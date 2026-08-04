//! HTTP client used by the reverse-proxy handler.

use std::sync::OnceLock;

use hyper::body::Incoming;
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};

static CLIENT: OnceLock<Client<HttpConnector, Incoming>> = OnceLock::new();

/// A shared Hyper client. Reusing it also reuses eligible upstream connections.
pub fn shared() -> &'static Client<HttpConnector, Incoming> {
    CLIENT.get_or_init(|| Client::builder(TokioExecutor::new()).build_http())
}
