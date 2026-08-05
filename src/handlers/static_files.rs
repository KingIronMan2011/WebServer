//! Static-file handler with streaming, validators, and byte ranges.

use std::{
    collections::BTreeMap,
    convert::Infallible,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_compression::tokio::bufread::{BrotliEncoder, GzipEncoder};
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Empty, StreamBody};
use hyper::{
    Method, Request, Response, StatusCode,
    body::Frame,
    header::{
        ACCEPT_RANGES, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_MODIFIED_SINCE,
        IF_NONE_MATCH, LAST_MODIFIED, RANGE, VARY,
    },
};
use percent_encoding::percent_decode_str;
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader, SeekFrom};
use tokio_util::io::ReaderStream;

use crate::http::{Body, response};

pub async fn serve<B>(
    request: &Request<B>,
    root: &Path,
    index_file: &str,
    prefix: &str,
) -> Response<Body> {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return response::error(StatusCode::METHOD_NOT_ALLOWED);
    }

    let relative = request
        .uri()
        .path()
        .strip_prefix(prefix)
        .unwrap_or(request.uri().path())
        .trim_start_matches('/');
    let decoded = match percent_decode_str(relative).decode_utf8() {
        Ok(path) => path,
        Err(_) => return response::error(StatusCode::BAD_REQUEST),
    };
    let mut path = match safe_path(root, &decoded) {
        Some(path) => path,
        None => return response::error(StatusCode::FORBIDDEN),
    };
    if path.is_dir() {
        path.push(index_file);
    }

    let root = match tokio::fs::canonicalize(root).await {
        Ok(root) => root,
        Err(error) => {
            tracing::error!(%error, root = %root.display(), "failed to canonicalize static root");
            return response::error(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let path = match tokio::fs::canonicalize(&path).await {
        Ok(path) if path.starts_with(&root) => path,
        Ok(_) => return response::error(StatusCode::FORBIDDEN),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return response::error(StatusCode::NOT_FOUND);
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return response::error(StatusCode::FORBIDDEN);
        }
        Err(error) => {
            tracing::error!(%error, "failed to resolve static file");
            return response::error(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return response::error(StatusCode::NOT_FOUND),
        Err(error) => {
            tracing::error!(%error, path = %path.display(), "failed to read static file metadata");
            return response::error(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let size = metadata.len();
    let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
    let etag = etag(size, modified);
    let last_modified = httpdate::fmt_http_date(modified);
    let content_type = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .essence_str()
        .to_owned();

    if not_modified(request, &etag, modified) {
        return file_headers(
            empty(StatusCode::NOT_MODIFIED),
            &etag,
            &last_modified,
            None,
            size,
        );
    }

    let range = match request.headers().get(RANGE).map(|value| value.to_str()) {
        Some(Ok(value)) => match parse_range(value, size) {
            Some(range) => Some(range),
            None => {
                let mut response = file_headers(
                    empty(StatusCode::RANGE_NOT_SATISFIABLE),
                    &etag,
                    &last_modified,
                    None,
                    size,
                );
                response.headers_mut().insert(
                    CONTENT_RANGE,
                    format!("bytes */{size}").parse().expect("valid range"),
                );
                return response;
            }
        },
        Some(Err(_)) => return response::error(StatusCode::BAD_REQUEST),
        None => None,
    };
    let (start, end) = range.unwrap_or((0, size.saturating_sub(1)));
    let length = if size == 0 { 0 } else { end - start + 1 };
    let status = if range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };

    if request.method() == Method::HEAD {
        return file_headers(
            response::empty_with_content_length(status, &content_type, length as usize),
            &etag,
            &last_modified,
            range,
            size,
        );
    }

    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(error) => {
            tracing::error!(%error, path = %path.display(), "failed to open static file");
            return response::error(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    if start != 0 && file.seek(SeekFrom::Start(start)).await.is_err() {
        return response::error(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let encoding = range
        .is_none()
        .then(|| accepted_encoding(request))
        .flatten();
    let mut builder = Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, content_type);
    if encoding.is_none() {
        builder = builder.header(CONTENT_LENGTH, length);
    }
    let body = match encoding {
        Some("br") => stream_body(BrotliEncoder::new(BufReader::new(file))),
        Some("gzip") => stream_body(GzipEncoder::new(BufReader::new(file))),
        None => stream_body(file.take(length)),
        Some(_) => unreachable!("known content encoding"),
    };
    let mut response = builder.body(body).expect("valid static response");
    if let Some(encoding) = encoding {
        response
            .headers_mut()
            .insert(CONTENT_ENCODING, encoding.parse().expect("valid encoding"));
        response
            .headers_mut()
            .insert(VARY, "Accept-Encoding".parse().expect("valid vary"));
    }
    file_headers(response, &etag, &last_modified, range, size)
}

pub async fn custom_error(
    status: StatusCode,
    pages: &BTreeMap<u16, PathBuf>,
    site: &crate::config::SiteConfig,
) -> Option<Response<Body>> {
    let path = site.static_path(pages.get(&status.as_u16())?);
    let bytes = tokio::fs::read(&path).await.ok()?;
    let content_type = mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_owned();
    Some(response::full(status, &content_type, Bytes::from(bytes)))
}

fn stream_body(reader: impl tokio::io::AsyncRead + Unpin + Send + Sync + 'static) -> Body {
    let stream = ReaderStream::with_capacity(reader, 64 * 1024).filter_map(|chunk| async move {
        chunk
            .ok()
            .map(|bytes| Ok::<_, Infallible>(Frame::data(bytes)))
    });
    StreamBody::new(stream)
        .map_err(|never| match never {})
        .boxed()
}

fn accepted_encoding<B>(request: &Request<B>) -> Option<&'static str> {
    let value = request
        .headers()
        .get("accept-encoding")?
        .to_str()
        .ok()?
        .to_ascii_lowercase();
    value.split(',').map(str::trim).find_map(|encoding| {
        let name = encoding.split(';').next().unwrap_or_default();
        match name {
            "br" => Some("br"),
            "gzip" => Some("gzip"),
            _ => None,
        }
    })
}

fn empty(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(
            Empty::<Bytes>::new()
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid empty response")
}

fn file_headers(
    mut response: Response<Body>,
    etag: &str,
    last_modified: &str,
    range: Option<(u64, u64)>,
    total_size: u64,
) -> Response<Body> {
    let headers = response.headers_mut();
    headers.insert(ACCEPT_RANGES, "bytes".parse().expect("valid header"));
    headers.insert(ETAG, etag.parse().expect("valid etag"));
    headers.insert(LAST_MODIFIED, last_modified.parse().expect("valid date"));
    if let Some((start, end)) = range {
        headers.insert(
            CONTENT_RANGE,
            format!("bytes {start}-{end}/{total_size}")
                .parse()
                .expect("valid range"),
        );
    }
    response
}

fn not_modified<B>(request: &Request<B>, etag: &str, modified: SystemTime) -> bool {
    if let Some(value) = request
        .headers()
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
    {
        return value
            .split(',')
            .any(|candidate| candidate.trim() == "*" || candidate.trim() == etag);
    }
    request
        .headers()
        .get(IF_MODIFIED_SINCE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| httpdate::parse_http_date(value).ok())
        .is_some_and(|since| modified <= since + Duration::from_secs(1))
}

fn etag(size: u64, modified: SystemTime) -> String {
    let stamp = modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("\"{size:x}-{stamp:x}\"")
}

fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') || size == 0 {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?;
        if suffix == 0 {
            return None;
        }
        return Some((size.saturating_sub(suffix), size - 1));
    }
    let start = start.parse::<u64>().ok()?;
    if start >= size {
        return None;
    }
    let end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>().ok()?.min(size - 1)
    };
    (start <= end).then_some((start, end))
}

fn safe_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut path = PathBuf::from(root);
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::{parse_range, safe_path};
    use std::path::Path;

    #[test]
    fn rejects_parent_directory_components() {
        assert!(safe_path(Path::new("public"), "../secret").is_none());
    }
    #[test]
    fn parses_single_byte_ranges() {
        assert_eq!(parse_range("bytes=1-4", 10), Some((1, 4)));
        assert_eq!(parse_range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(parse_range("bytes=8-", 10), Some((8, 9)));
        assert_eq!(parse_range("bytes=10-", 10), None);
    }
}
