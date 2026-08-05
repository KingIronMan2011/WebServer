//! Lightweight request metadata used for routing and logging.

use hyper::Request;

use crate::config::normalise_host;

pub struct RequestContext {
    pub host: String,
    pub path: String,
}

impl RequestContext {
    pub fn from_request<B>(request: &Request<B>) -> Self {
        let host = request
            .headers()
            .get(hyper::header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(host_name)
            .map(|host| normalise_host(&host))
            .unwrap_or_default();
        Self {
            host,
            path: request.uri().path().to_owned(),
        }
    }
}

fn host_name(host: &str) -> String {
    host.parse::<hyper::http::uri::Authority>()
        .map(|authority| authority.host().to_owned())
        .unwrap_or_else(|_| {
            host.split_once(':')
                .map_or(host, |(name, _)| name)
                .to_owned()
        })
}
