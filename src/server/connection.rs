//! Reads requests from one client connection and writes responses.

use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Instant};

use hyper::{
    Request,
    body::Incoming,
    header::{CONTENT_LENGTH, TRANSFER_ENCODING},
};
use tokio::sync::RwLock;

use crate::{
    config::{Config, RouteTarget},
    handlers::{reverse_proxy, static_files},
    http::{Body, request::RequestContext, response, status::StatusCode},
    routing::router,
};

pub async fn handle(
    request: Request<Incoming>,
    peer: SocketAddr,
    config: Arc<RwLock<Config>>,
) -> Result<hyper::Response<Body>, Infallible> {
    let started = Instant::now();
    let config = config.read().await.clone();
    let context = RequestContext::from_request(&request);
    let method = request.method().clone();
    let path = context.path.clone();

    let response = match request_size_error(&request, config.server.max_body_bytes) {
        Some(response) => response,
        None => match router::find(&config, &context.host, &context.path) {
            None => response::error(StatusCode::NOT_FOUND),
            Some(matched) => match &matched.route.target {
                RouteTarget::Static { root, index_file } => {
                    let root = matched.site.static_path(root);
                    static_files::serve(&request, &root, index_file, &matched.route.path_prefix)
                        .await
                }
                RouteTarget::Proxy { upstream } => {
                    reverse_proxy::serve(
                        request,
                        upstream,
                        peer,
                        &context.host,
                        config.server.upstream_timeout_secs,
                    )
                    .await
                }
            },
        },
    };

    tracing::info!(%peer, %method, %path, status = response.status().as_u16(), elapsed_ms = started.elapsed().as_millis(), "request completed");
    Ok(response)
}

fn request_size_error(
    request: &Request<Incoming>,
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
