//! Reverse-proxy handler for HTTP upstreams.

use std::{
    collections::HashMap,
    net::IpAddr,
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
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode, Uri,
    body::Body as HttpBody,
    header::{CONNECTION, COOKIE, HOST, HeaderName, HeaderValue},
};

// The arguments mirror one proxy route's independently configurable fields;
// grouping them would obscure the connection between the route contract and
// its execution path.
#[allow(clippy::too_many_arguments)]
pub async fn serve<B>(
    request: Request<B>,
    upstreams: &[(&str, u32)],
    strategy: crate::config::LoadBalancing,
    route_prefix: &str,
    base_path: Option<&str>,
    rewrite_prefix: Option<&str>,
    max_connections: usize,
    retries: u32,
    retry_backoff_ms: u64,
    client_ip: IpAddr,
    original_host: &str,
    is_tls: bool,
    max_body_bytes: u64,
    timeout_secs: u64,
) -> Response<Body>
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    serve_buffered(
        request,
        upstreams,
        strategy,
        route_prefix,
        base_path,
        rewrite_prefix,
        max_connections,
        retries,
        retry_backoff_ms,
        client_ip,
        original_host,
        is_tls,
        max_body_bytes,
        timeout_secs,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn serve_buffered<B>(
    request: Request<B>,
    upstreams: &[(&str, u32)],
    strategy: crate::config::LoadBalancing,
    route_prefix: &str,
    base_path: Option<&str>,
    rewrite_prefix: Option<&str>,
    max_connections: usize,
    retries: u32,
    retry_backoff_ms: u64,
    client_ip: IpAddr,
    original_host: &str,
    is_tls: bool,
    max_body_bytes: u64,
    timeout_secs: u64,
) -> Response<Body>
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let (parts, body) = request.into_parts();
    let collected = tokio::time::timeout(
        Duration::from_secs(30),
        Limited::new(body, usize::try_from(max_body_bytes).unwrap_or(usize::MAX)).collect(),
    )
    .await;
    let body = match collected {
        Ok(Ok(body)) => body.to_bytes(),
        Ok(Err(error))
            if error
                .downcast_ref::<http_body_util::LengthLimitError>()
                .is_some() =>
        {
            return response::error(StatusCode::PAYLOAD_TOO_LARGE);
        }
        Ok(Err(_)) => return response::error(StatusCode::BAD_REQUEST),
        Err(_) => return response::error(StatusCode::REQUEST_TIMEOUT),
    };
    // Retrying a POST/PATCH after an upstream processed it but dropped the
    // response can duplicate payments or other state changes.
    let attempts = if is_idempotent(&parts.method) {
        retries.saturating_add(1)
    } else {
        1
    };
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
        remove_management_cookie(&mut headers);
        set_forwarded_headers(&mut headers, client_ip, original_host, is_tls);
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

fn is_idempotent(method: &Method) -> bool {
    method == Method::GET
        || method == Method::HEAD
        || method == Method::OPTIONS
        || method == Method::PUT
        || method == Method::DELETE
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
    last_used: Option<Instant>,
}

const MAX_UPSTREAM_STATES: usize = 65_536;

struct UpstreamStates {
    entries: HashMap<String, UpstreamState>,
    last_pruned: Instant,
}

fn states() -> &'static Mutex<UpstreamStates> {
    static STATES: OnceLock<Mutex<UpstreamStates>> = OnceLock::new();
    STATES.get_or_init(|| {
        Mutex::new(UpstreamStates {
            entries: HashMap::new(),
            last_pruned: Instant::now(),
        })
    })
}

fn prune_states(states: &mut UpstreamStates, now: Instant) {
    if now.duration_since(states.last_pruned) >= Duration::from_secs(60) {
        states.entries.retain(|_, state| {
            state.active != 0
                || state.last_used.is_some_and(|last_used| {
                    now.duration_since(last_used) < Duration::from_secs(600)
                })
        });
        states.last_pruned = now;
    }
}

fn state_mut<'a>(
    states: &'a mut UpstreamStates,
    url: &str,
    now: Instant,
) -> Option<&'a mut UpstreamState> {
    prune_states(states, now);
    if states.entries.len() >= MAX_UPSTREAM_STATES && !states.entries.contains_key(url) {
        return None;
    }
    let state = states.entries.entry(url.to_owned()).or_default();
    state.last_used = Some(now);
    Some(state)
}
fn usable(url: &str) -> bool {
    states()
        .lock()
        .expect("upstream state")
        .entries
        .get(url)
        .is_none_or(|state| state.open_until.is_none_or(|until| until <= Instant::now()))
}
fn active(url: &str) -> usize {
    states()
        .lock()
        .expect("upstream state")
        .entries
        .get(url)
        .map_or(0, |state| state.active)
}
fn reserve(url: &str, limit: usize) -> bool {
    let mut states = states().lock().expect("upstream state");
    let Some(state) = state_mut(&mut states, url, Instant::now()) else {
        return false;
    };
    if limit != 0 && state.active >= limit {
        return false;
    }
    state.active += 1;
    true
}
fn release(url: &str) {
    if let Some(state) = states()
        .lock()
        .expect("upstream state")
        .entries
        .get_mut(url)
    {
        state.active = state.active.saturating_sub(1);
        state.last_used = Some(Instant::now());
    }
}
fn success(url: &str) {
    let mut states = states().lock().expect("upstream state");
    let Some(state) = state_mut(&mut states, url, Instant::now()) else {
        return;
    };
    state.consecutive_failures = 0;
    state.open_until = None;
}
fn failure(url: &str) {
    let mut states = states().lock().expect("upstream state");
    let Some(state) = state_mut(&mut states, url, Instant::now()) else {
        return;
    };
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    if state.consecutive_failures >= 3 {
        state.open_until = Some(Instant::now() + Duration::from_secs(30));
    }
}

pub(crate) fn report_health(url: &str, healthy: bool) {
    if healthy { success(url) } else { failure(url) }
}

/// A safe snapshot for the management API. It intentionally exposes no
/// internal synchronization details or request data.
#[derive(Clone, Debug, serde::Serialize)]
pub struct UpstreamStatus {
    pub url: String,
    pub active_connections: usize,
    pub consecutive_failures: u32,
    pub circuit_open: bool,
}

pub fn status(url: &str) -> UpstreamStatus {
    let states = states().lock().expect("upstream state");
    let state = states.entries.get(url);
    UpstreamStatus {
        url: url.to_owned(),
        active_connections: state.map_or(0, |state| state.active),
        consecutive_failures: state.map_or(0, |state| state.consecutive_failures),
        circuit_open: state
            .is_some_and(|state| state.open_until.is_some_and(|until| until > Instant::now())),
    }
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

fn set_forwarded_headers(
    headers: &mut hyper::HeaderMap,
    client_ip: IpAddr,
    original_host: &str,
    is_tls: bool,
) {
    // Values received from an untrusted client must never be allowed to reach
    // an upstream authorization layer. `client_ip` was already resolved using
    // the configured trusted-proxy chain.
    headers.remove(HeaderName::from_static("forwarded"));
    headers.insert(
        HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_str(&client_ip.to_string()).expect("IP addresses are valid headers"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static(if is_tls { "https" } else { "http" }),
    );
    if let Ok(value) = HeaderValue::from_str(original_host) {
        headers.insert(HeaderName::from_static("x-forwarded-host"), value);
    }
}

fn remove_management_cookie(headers: &mut hyper::HeaderMap) {
    let cookies = headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|cookie| {
            cookie
                .split_once('=')
                .is_none_or(|(name, _)| name != crate::admin::SESSION_COOKIE)
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    headers.remove(COOKIE);
    if !cookies.is_empty()
        && let Ok(value) = HeaderValue::from_str(&cookies.join("; "))
    {
        headers.insert(COOKIE, value);
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
    use super::{remove_management_cookie, select_upstream, serve, set_forwarded_headers};
    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::{HeaderMap, Request, StatusCode};

    #[test]
    fn selection_always_returns_a_configured_upstream() {
        let upstreams = [("http://one.test", 1), ("http://two.test", 1)];
        for _ in 0..4 {
            let selected = select_upstream(&upstreams, crate::config::LoadBalancing::RoundRobin)
                .expect("target");
            assert!(upstreams.iter().any(|(url, _)| *url == selected));
        }
    }

    #[test]
    fn replaces_client_supplied_forwarding_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.99".parse().unwrap());
        headers.insert("forwarded", "for=203.0.113.99".parse().unwrap());

        set_forwarded_headers(
            &mut headers,
            "198.51.100.7".parse().unwrap(),
            "example.test",
            true,
        );

        assert_eq!(headers["x-forwarded-for"], "198.51.100.7");
        assert_eq!(headers["x-forwarded-proto"], "https");
        assert!(!headers.contains_key("forwarded"));
    }

    #[test]
    fn never_forwards_the_admin_session_cookie_to_public_upstreams() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            "theme=dark; __Host-webserver_admin=secret; session=public"
                .parse()
                .unwrap(),
        );

        remove_management_cookie(&mut headers);

        assert_eq!(headers["cookie"], "theme=dark; session=public");
    }

    #[tokio::test]
    async fn rejects_a_streamed_body_over_the_configured_limit() {
        let request = Request::builder()
            .uri("/upload")
            .body(Full::new(Bytes::from_static(b"too large")))
            .unwrap();
        let response = serve(
            request,
            &[("http://127.0.0.1:9", 1)],
            crate::config::LoadBalancing::RoundRobin,
            "/",
            None,
            None,
            0,
            0,
            0,
            "198.51.100.7".parse().unwrap(),
            "example.test",
            true,
            4,
            1,
        )
        .await;

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
