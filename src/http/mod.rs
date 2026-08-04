//! HTTP/1.1 request and response primitives.

pub mod request;
pub mod response;
pub mod status;

mod error_pages;

pub type Body = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;
