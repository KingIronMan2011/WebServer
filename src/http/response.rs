//! Response builders shared by server handlers.

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full};
use hyper::{
    Response, StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE},
};

use super::Body;
use super::error_pages;

pub fn empty_with_content_length(
    status: StatusCode,
    content_type: &str,
    content_length: usize,
) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, content_length)
        .body(
            Empty::<Bytes>::new()
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid response")
}

pub fn error(status: StatusCode) -> Response<Body> {
    let bytes = Bytes::from(error_pages::document(status));
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header(CONTENT_LENGTH, bytes.len())
        .body(Full::new(bytes).map_err(|never| match never {}).boxed())
        .expect("valid error response")
}

pub fn full(status: StatusCode, content_type: &str, bytes: Bytes) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, bytes.len())
        .body(Full::new(bytes).map_err(|never| match never {}).boxed())
        .expect("valid response")
}
