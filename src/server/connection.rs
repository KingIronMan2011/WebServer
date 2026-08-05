//! Reads requests from one client connection and writes responses.

use std::{
    convert::Infallible,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Instant,
};

use bytes::Bytes;
use hyper::{
    Request,
    body::Body as HttpBody,
    header::{CONTENT_LENGTH, HeaderValue, ORIGIN, TRANSFER_ENCODING},
};
use tokio::sync::RwLock;

use crate::{
    config::{Config, RouteTarget},
    handlers::{reverse_proxy, static_files},
    http::{Body, request::RequestContext, response, status::StatusCode},
    routing::router,
};

pub async fn handle<B>(
    request: Request<B>,
    peer: SocketAddr,
    config: Arc<RwLock<Config>>,
    tls: Option<Arc<crate::tls::TlsManager>>,
    is_tls: bool,
) -> Result<hyper::Response<Body>, Infallible>
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let started = Instant::now();
    let config = config.read().await.clone();
    let context = RequestContext::from_request(&request);
    let method = request.method().clone();
    let path = context.path.clone();
    let origin = request
        .headers()
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    if !is_tls && let Some(tls) = tls {
        if let Some(token) = context.path.strip_prefix("/.well-known/acme-challenge/")
            && let Some(key_authorization) = tls.challenge_response(token)
        {
            return Ok(with_http3_alt_svc(
                response::plain(StatusCode::OK, key_authorization),
                &config,
                is_tls,
            ));
        }
        let host = context.host;
        if host.is_empty() {
            return Ok(with_http3_alt_svc(
                response::error(StatusCode::BAD_REQUEST),
                &config,
                is_tls,
            ));
        }
        let target = request
            .uri()
            .path_and_query()
            .map(|value| value.as_str())
            .unwrap_or("/");
        return Ok(with_http3_alt_svc(
            response::redirect(format!("https://{host}{target}")),
            &config,
            is_tls,
        ));
    }

    if config.server.metrics_path.as_deref() == Some(context.path.as_str()) {
        return Ok(with_http3_alt_svc(
            response::plain(StatusCode::OK, crate::observability::metrics::prometheus()),
            &config,
            is_tls,
        ));
    }

    let client_ip = client_ip(&request, peer, &config);
    if config
        .server
        .deny_ips
        .iter()
        .any(|network| network.contains(&client_ip))
        || (!config.server.allow_ips.is_empty()
            && !config
                .server
                .allow_ips
                .iter()
                .any(|network| network.contains(&client_ip)))
    {
        return Ok(with_http3_alt_svc(
            response::error(StatusCode::FORBIDDEN),
            &config,
            is_tls,
        ));
    }
    if !crate::server::limits::allow(client_ip, config.server.rate_limit_per_minute) {
        return Ok(with_http3_alt_svc(
            response::error(StatusCode::TOO_MANY_REQUESTS),
            &config,
            is_tls,
        ));
    }

    let matched = router::find(&config, &context.host, &context.path);
    if let Some(ref matched) = matched
        && request.method() == hyper::Method::OPTIONS
        && let Some(cors) = &matched.route.cors
    {
        let mut response =
            response::empty_with_content_length(StatusCode::NO_CONTENT, "text/plain", 0);
        apply_cors(&mut response, origin.as_deref(), cors);
        return Ok(with_http3_alt_svc(response, &config, is_tls));
    }
    let mut response = match request_size_error(&request, config.server.max_body_bytes) {
        Some(response) => response,
        None => match matched {
            None => response::error(StatusCode::NOT_FOUND),
            Some(ref matched) => match &matched.route.target {
                RouteTarget::Static { root, index_file } => {
                    let root = matched.site.static_path(root);
                    static_files::serve(&request, &root, index_file, &matched.route.path_prefix)
                        .await
                }
                RouteTarget::Proxy {
                    upstream,
                    upstreams,
                    load_balancing,
                    base_path,
                    rewrite_prefix,
                    max_connections_per_upstream,
                    retries,
                    retry_backoff_ms,
                    dns_discovery,
                    docker_discovery,
                    kubernetes_discovery,
                    ..
                } => {
                    let mut targets: Vec<(String, u32)> =
                        crate::config::proxy_upstreams(upstream.as_deref(), upstreams)
                            .into_iter()
                            .map(|(url, weight)| (url.to_owned(), weight))
                            .collect();
                    if let Some(discovery) = dns_discovery {
                        targets.extend(
                            crate::upstream::discovery::dns(discovery)
                                .await
                                .into_iter()
                                .map(|url| (url, 1)),
                        );
                    }
                    if let Some(discovery) = docker_discovery {
                        targets.extend(
                            crate::upstream::discovery::docker(discovery)
                                .await
                                .into_iter()
                                .map(|url| (url, 1)),
                        );
                    }
                    if let Some(discovery) = kubernetes_discovery {
                        targets.extend(
                            crate::upstream::discovery::kubernetes(discovery)
                                .await
                                .into_iter()
                                .map(|url| (url, 1)),
                        );
                    }
                    let target_refs: Vec<(&str, u32)> = targets
                        .iter()
                        .map(|(url, weight)| (url.as_str(), *weight))
                        .collect();
                    reverse_proxy::serve(
                        request,
                        &target_refs,
                        *load_balancing,
                        &matched.route.path_prefix,
                        base_path.as_deref(),
                        rewrite_prefix.as_deref(),
                        *max_connections_per_upstream,
                        *retries,
                        *retry_backoff_ms,
                        peer,
                        &context.host,
                        config.server.upstream_timeout_secs,
                    )
                    .await
                }
                RouteTarget::Redirect { location, status } => {
                    response::redirect_with_status(location, *status)
                }
            },
        },
    };

    if let Some(matched) = matched {
        if (response.status().is_client_error() || response.status().is_server_error())
            && let Some(page) = static_files::custom_error(
                response.status(),
                &matched.route.error_pages,
                matched.site,
            )
            .await
        {
            response = page;
        }
        response::apply_headers(&mut response, &matched.route.response_headers);
        if let Some(cors) = &matched.route.cors {
            apply_cors(&mut response, origin.as_deref(), cors);
        }
    }
    advertise_http3(&mut response, &config, is_tls);

    tracing::info!(%peer, %method, %path, status = response.status().as_u16(), elapsed_ms = started.elapsed().as_millis(), "request completed");
    crate::observability::metrics::record(response.status().as_u16());
    Ok(response)
}

fn with_http3_alt_svc(
    mut response: hyper::Response<Body>,
    config: &Config,
    is_tls: bool,
) -> hyper::Response<Body> {
    advertise_http3(&mut response, config, is_tls);
    response
}

/// Announces the UDP HTTP/3 endpoint only from secure HTTP/1.1 and HTTP/2
/// responses. Browsers ignore Alt-Svc received over plain HTTP by design.
fn advertise_http3(response: &mut hyper::Response<Body>, config: &Config, is_tls: bool) {
    if !is_tls || !config.tls.http3 {
        return;
    }
    let port = config.tls.quic_bind.unwrap_or(config.tls.bind).port();
    let value = format!(r#"h3=\":{port}\"; ma=86400"#);
    if let Ok(value) = HeaderValue::from_str(&value) {
        response.headers_mut().insert("alt-svc", value);
    }
}

fn apply_cors(
    response: &mut hyper::Response<Body>,
    origin: Option<&str>,
    cors: &crate::config::CorsConfig,
) {
    let Some(origin) = origin.filter(|origin| {
        cors.origins
            .iter()
            .any(|allowed| allowed == "*" || allowed == origin)
    }) else {
        return;
    };
    let headers = response.headers_mut();
    let value = if cors.origins.iter().any(|allowed| allowed == "*") {
        "*"
    } else {
        origin
    };
    if let Ok(value) = value.parse() {
        headers.insert("access-control-allow-origin", value);
    }
    if let Ok(value) = cors.methods.join(", ").parse() {
        headers.insert("access-control-allow-methods", value);
    }
    if !cors.headers.is_empty()
        && let Ok(value) = cors.headers.join(", ").parse()
    {
        headers.insert("access-control-allow-headers", value);
    }
    if cors.allow_credentials {
        headers.insert(
            "access-control-allow-credentials",
            "true".parse().expect("valid header"),
        );
    }
    if cors.max_age_secs > 0
        && let Ok(value) = cors.max_age_secs.to_string().parse()
    {
        headers.insert("access-control-max-age", value);
    }
    headers.insert("vary", "Origin".parse().expect("valid header"));
}

fn client_ip<B>(request: &Request<B>, peer: SocketAddr, config: &Config) -> IpAddr {
    if !config
        .server
        .trusted_proxies
        .iter()
        .any(|network| network.contains(&peer.ip()))
    {
        return peer.ip();
    }
    request
        .headers()
        .get("x-forwarded-for")
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.split(',').next())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(peer.ip())
}

fn request_size_error<B>(
    request: &Request<B>,
    max_body_bytes: u64,
) -> Option<hyper::Response<Body>> {
    if request.headers().contains_key(TRANSFER_ENCODING) {
        return Some(response::error(StatusCode::LENGTH_REQUIRED));
    }
    let content_length = request.headers().get(CONTENT_LENGTH)?;
    let content_length = match content_length.to_str().ok()?.parse::<u64>() {
        Ok(length) => length,
        Err(_) => {
            return Some(response::error(StatusCode::BAD_REQUEST));
        }
    };
    (content_length > max_body_bytes).then(|| response::error(StatusCode::PAYLOAD_TOO_LARGE))
}
