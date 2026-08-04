//! Reverse-proxy handler for HTTP upstreams.

use std::{net::SocketAddr, time::Duration};

use crate::{
    http::{Body, response},
    upstream::client,
};
use http_body_util::BodyExt;
use hyper::{
    Request, Response, StatusCode, Uri,
    body::Incoming,
    header::{CONNECTION, HOST, HeaderName, HeaderValue},
};

pub async fn serve(
    mut request: Request<Incoming>,
    upstream: &str,
    peer: SocketAddr,
    original_host: &str,
    timeout_secs: u64,
) -> Response<Body> {
    let target_uri = match upstream_uri(upstream, request.uri()) {
        Ok(uri) => uri,
        Err(error) => {
            tracing::error!(%error, "invalid proxy target");
            return response::error(StatusCode::BAD_GATEWAY);
        }
    };
    remove_hop_by_hop_headers(request.headers_mut());
    add_forwarded_headers(request.headers_mut(), peer, original_host);
    if let Some(authority) = target_uri.authority()
        && let Ok(value) = HeaderValue::from_str(authority.as_str())
    {
        request.headers_mut().insert(HOST, value);
    }
    *request.uri_mut() = target_uri;

    let upstream_response = match tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        client::shared().request(request),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            tracing::warn!(%error, "upstream request failed");
            return response::error(StatusCode::BAD_GATEWAY);
        }
        Err(_) => {
            return response::error(StatusCode::GATEWAY_TIMEOUT);
        }
    };
    let (mut parts, body) = upstream_response.into_parts();
    remove_hop_by_hop_headers(&mut parts.headers);
    Response::from_parts(parts, body.boxed())
}

fn upstream_uri(upstream: &str, request_uri: &Uri) -> Result<Uri, hyper::http::uri::InvalidUri> {
    let base: Uri = upstream.parse()?;
    let scheme = base.scheme_str().unwrap_or("http");
    let authority = base
        .authority()
        .expect("configuration validation requires an authority");
    let path_and_query = request_uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
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
