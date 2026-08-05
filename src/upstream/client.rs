//! HTTP client used by the reverse-proxy handler.

use std::sync::OnceLock;

use crate::http::Body;
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};

static CLIENT: OnceLock<Client<HttpConnector, Body>> = OnceLock::new();

/// A shared Hyper client. Reusing it also reuses eligible upstream connections.
pub fn shared() -> &'static Client<HttpConnector, Body> {
    CLIENT.get_or_init(|| Client::builder(TokioExecutor::new()).build_http())
}
