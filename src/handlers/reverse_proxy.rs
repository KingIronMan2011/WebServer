//! Reverse-proxy handler for HTTP upstreams.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use crate::{
    http::{Body, response},
    upstream::client,
};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response, StatusCode, Uri,
    body::Incoming,
    header::{CONNECTION, HOST, HeaderName, HeaderValue},
};
use hyper_util::rt::TokioIo;

pub async fn serve(
    mut request: Request<Incoming>,
    upstreams: &[(&str, u32)],
    strategy: crate::config::LoadBalancing,
    route_prefix: &str,
    base_path: Option<&str>,
    rewrite_prefix: Option<&str>,
    max_connections: usize,
    retries: u32,
    retry_backoff_ms: u64,
    peer: SocketAddr,
    original_host: &str,
    timeout_secs: u64,
) -> Response<Body> {
    let websocket = request.headers().get(CONNECTION).is_some_and(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        })
    }) && request
        .headers()
        .get("upgrade")
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"));
    let client_upgrade = websocket.then(|| hyper::upgrade::on(&mut request));
    if !websocket {
        return serve_buffered(
            request,
            upstreams,
            strategy,
            route_prefix,
            base_path,
            rewrite_prefix,
            max_connections,
            retries,
            retry_backoff_ms,
            peer,
            original_host,
            timeout_secs,
        )
        .await;
    }
    let upstream = match select_upstream(upstreams, strategy) {
        Some(upstream) => upstream,
        None => return response::error(StatusCode::BAD_GATEWAY),
    };
    if !reserve(upstream, max_connections) {
        return response::error(StatusCode::SERVICE_UNAVAILABLE);
    }
    let target_uri = match upstream_uri(
        upstream,
        request.uri(),
        route_prefix,
        base_path,
        rewrite_prefix,
    ) {
        Ok(uri) => uri,
        Err(error) => {
            tracing::error!(%error, "invalid proxy target");
            release(upstream);
            return response::error(StatusCode::BAD_GATEWAY);
        }
    };
    if !websocket {
        remove_hop_by_hop_headers(request.headers_mut());
    }
    add_forwarded_headers(request.headers_mut(), peer, original_host);
    if let Some(authority) = target_uri.authority()
        && let Ok(value) = HeaderValue::from_str(authority.as_str())
    {
        request.headers_mut().insert(HOST, value);
    }
    *request.uri_mut() = target_uri;

    let mut upstream_response = match tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        client::shared().request(request.map(|body| body.boxed())),
    )
    .await
    {
        Ok(Ok(response)) => {
            success(upstream);
            response
        }
        Ok(Err(error)) => {
            tracing::warn!(%error, "upstream request failed");
            failure(upstream);
            release(upstream);
            return response::error(StatusCode::BAD_GATEWAY);
        }
        Err(_) => {
            failure(upstream);
            release(upstream);
            return response::error(StatusCode::GATEWAY_TIMEOUT);
        }
    };
    release(upstream);
    if websocket && upstream_response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let upstream_upgrade = hyper::upgrade::on(&mut upstream_response);
        tokio::spawn(async move {
            let (Ok(client), Ok(upstream)) = (
                client_upgrade.expect("websocket upgrade").await,
                upstream_upgrade.await,
            ) else {
                return;
            };
            let _ = tokio::io::copy_bidirectional(
                &mut TokioIo::new(client),
                &mut TokioIo::new(upstream),
            )
            .await;
        });
    }
    let (mut parts, body) = upstream_response.into_parts();
    if !websocket {
        remove_hop_by_hop_headers(&mut parts.headers);
    }
    Response::from_parts(parts, body.boxed())
}

#[allow(clippy::too_many_arguments)]
async fn serve_buffered(
    request: Request<Incoming>,
    upstreams: &[(&str, u32)],
    strategy: crate::config::LoadBalancing,
    route_prefix: &str,
    base_path: Option<&str>,
    rewrite_prefix: Option<&str>,
    max_connections: usize,
    retries: u32,
    retry_backoff_ms: u64,
    peer: SocketAddr,
    original_host: &str,
    timeout_secs: u64,
) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let body = match body.collect().await {
        Ok(body) => body.to_bytes(),
        Err(_) => return response::error(StatusCode::BAD_REQUEST),
    };
    let attempts = retries.saturating_add(1);
    for attempt in 0..attempts {
        let Some(upstream) = select_upstream(upstreams, strategy) else {
            return response::error(StatusCode::BAD_GATEWAY);
        };
        if !reserve(upstream, max_connections) {
            continue;
        }
        let target = match upstream_uri(
            upstream,
            &parts.uri,
            route_prefix,
            base_path,
            rewrite_prefix,
        ) {
            Ok(uri) => uri,
            Err(_) => {
                release(upstream);
                return response::error(StatusCode::BAD_GATEWAY);
            }
        };
        let mut headers = parts.headers.clone();
        remove_hop_by_hop_headers(&mut headers);
        add_forwarded_headers(&mut headers, peer, original_host);
        if let Some(authority) = target
            .authority()
            .and_then(|value| HeaderValue::from_str(value.as_str()).ok())
        {
            headers.insert(HOST, authority);
        }
        let outgoing = Request::builder()
            .method(&parts.method)
            .version(parts.version)
            .uri(target)
            .body(
                Full::new(Bytes::copy_from_slice(&body))
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .expect("valid proxy request");
        let (mut outgoing_parts, outgoing_body) = outgoing.into_parts();
        outgoing_parts.headers = headers;
        match tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            client::shared().request(Request::from_parts(outgoing_parts, outgoing_body)),
        )
        .await
        {
            Ok(Ok(response)) => {
                success(upstream);
                release(upstream);
                let (mut parts, body) = response.into_parts();
                remove_hop_by_hop_headers(&mut parts.headers);
                return Response::from_parts(parts, body.boxed());
            }
            Ok(Err(error)) => tracing::warn!(%error, attempt, "upstream request failed"),
            Err(_) => tracing::warn!(attempt, "upstream request timed out"),
        }
        failure(upstream);
        release(upstream);
        if attempt + 1 < attempts {
            tokio::time::sleep(Duration::from_millis(
                retry_backoff_ms.saturating_mul(1_u64 << attempt.min(10)),
            ))
            .await;
        }
    }
    response::error(StatusCode::BAD_GATEWAY)
}

fn select_upstream<'a>(
    upstreams: &'a [(&str, u32)],
    strategy: crate::config::LoadBalancing,
) -> Option<&'a str> {
    static NEXT_UPSTREAM: AtomicUsize = AtomicUsize::new(0);
    let index = NEXT_UPSTREAM.fetch_add(1, Ordering::Relaxed);
    let available: Vec<_> = upstreams.iter().filter(|(url, _)| usable(url)).collect();
    if available.is_empty() {
        return None;
    }
    match strategy {
        crate::config::LoadBalancing::RoundRobin => {
            available.get(index % available.len()).map(|(url, _)| *url)
        }
        crate::config::LoadBalancing::LeastConnections => available
            .into_iter()
            .min_by_key(|(url, _)| active(url))
            .map(|(url, _)| *url),
        crate::config::LoadBalancing::WeightedRoundRobin => {
            let total_weight: usize = available.iter().map(|(_, weight)| *weight as usize).sum();
            let mut slot = index % total_weight;
            for (url, weight) in available {
                if slot < *weight as usize {
                    return Some(*url);
                }
                slot -= *weight as usize;
            }
            None
        }
    }
}

#[derive(Default)]
struct UpstreamState {
    active: usize,
    consecutive_failures: u32,
    open_until: Option<Instant>,
}
fn states() -> &'static Mutex<HashMap<String, UpstreamState>> {
    static STATES: OnceLock<Mutex<HashMap<String, UpstreamState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}
fn usable(url: &str) -> bool {
    states()
        .lock()
        .expect("upstream state")
        .get(url)
        .is_none_or(|state| state.open_until.is_none_or(|until| until <= Instant::now()))
}
fn active(url: &str) -> usize {
    states()
        .lock()
        .expect("upstream state")
        .get(url)
        .map_or(0, |state| state.active)
}
fn reserve(url: &str, limit: usize) -> bool {
    let mut states = states().lock().expect("upstream state");
    let state = states.entry(url.to_owned()).or_default();
    if limit != 0 && state.active >= limit {
        return false;
    }
    state.active += 1;
    true
}
fn release(url: &str) {
    if let Some(state) = states().lock().expect("upstream state").get_mut(url) {
        state.active = state.active.saturating_sub(1);
    }
}
fn success(url: &str) {
    let mut states = states().lock().expect("upstream state");
    let state = states.entry(url.to_owned()).or_default();
    state.consecutive_failures = 0;
    state.open_until = None;
}
fn failure(url: &str) {
    let mut states = states().lock().expect("upstream state");
    let state = states.entry(url.to_owned()).or_default();
    state.consecutive_failures += 1;
    if state.consecutive_failures >= 3 {
        state.open_until = Some(Instant::now() + Duration::from_secs(30));
    }
}

pub(crate) fn report_health(url: &str, healthy: bool) {
    if healthy { success(url) } else { failure(url) }
}

fn upstream_uri(
    upstream: &str,
    request_uri: &Uri,
    route_prefix: &str,
    base_path: Option<&str>,
    rewrite_prefix: Option<&str>,
) -> Result<Uri, hyper::http::uri::InvalidUri> {
    let base: Uri = upstream.parse()?;
    let scheme = base.scheme_str().unwrap_or("http");
    let authority = base
        .authority()
        .expect("configuration validation requires an authority");
    let path_and_query = request_uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let (path, query) = path_and_query
        .split_once('?')
        .unwrap_or((path_and_query, ""));
    let path = if let Some(replacement) = rewrite_prefix {
        path.strip_prefix(route_prefix)
            .map(|suffix| format!("{}{}", replacement.trim_end_matches('/'), suffix))
            .unwrap_or_else(|| path.to_owned())
    } else {
        path.to_owned()
    };
    let path = if let Some(base) = base_path {
        format!(
            "{}{}",
            base.trim_end_matches('/'),
            if path.starts_with('/') {
                path
            } else {
                format!("/{path}")
            }
        )
    } else {
        path
    };
    let path_and_query = if query.is_empty() {
        path
    } else {
        format!("{path}?{query}")
    };
    format!("{scheme}://{authority}{path_and_query}").parse()
}

fn add_forwarded_headers(headers: &mut hyper::HeaderMap, peer: SocketAddr, original_host: &str) {
    append_csv(
        headers,
        HeaderName::from_static("x-forwarded-for"),
        &peer.ip().to_string(),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("http"),
    );
    if let Ok(value) = HeaderValue::from_str(original_host) {
        headers.insert(HeaderName::from_static("x-forwarded-host"), value);
    }
}

fn append_csv(headers: &mut hyper::HeaderMap, name: HeaderName, value: &str) {
    let combined = headers
        .get(&name)
        .and_then(|old| old.to_str().ok())
        .map(|old| format!("{old}, {value}"))
        .unwrap_or_else(|| value.to_owned());
    if let Ok(value) = HeaderValue::from_str(&combined) {
        headers.insert(name, value);
    }
}

fn remove_hop_by_hop_headers(headers: &mut hyper::HeaderMap) {
    let connection_headers: Vec<HeaderName> = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|value| HeaderName::from_bytes(value.trim().as_bytes()).ok())
        .collect();
    for name in [
        CONNECTION,
        HeaderName::from_static("keep-alive"),
        HeaderName::from_static("proxy-authenticate"),
        HeaderName::from_static("proxy-authorization"),
        HeaderName::from_static("te"),
        HeaderName::from_static("trailer"),
        HeaderName::from_static("transfer-encoding"),
        HeaderName::from_static("upgrade"),
    ] {
        headers.remove(name);
    }
    for name in connection_headers {
        headers.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::select_upstream;

    #[test]
    fn selection_always_returns_a_configured_upstream() {
        let upstreams = [("http://one.test", 1), ("http://two.test", 1)];
        for _ in 0..4 {
            let selected = select_upstream(&upstreams, crate::config::LoadBalancing::RoundRobin)
                .expect("target");
            assert!(upstreams.iter().any(|(url, _)| *url == selected));
        }
    }
}
