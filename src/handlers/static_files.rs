//! Static-file handler.

use std::path::{Component, Path, PathBuf};

use bytes::Bytes;
use hyper::{Method, Request, StatusCode, body::Incoming};
use percent_encoding::percent_decode_str;

use crate::http::{Body, response};

pub async fn serve(
    request: &Request<Incoming>,
    root: &Path,
    index_file: &str,
    prefix: &str,
) -> hyper::Response<Body> {
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
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return response::error(StatusCode::NOT_FOUND);
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return response::error(StatusCode::FORBIDDEN);
        }
        Err(error) => {
            tracing::error!(%error, path = %path.display(), "failed to read static file");
            return response::error(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let content_type = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .essence_str()
        .to_owned();
    if request.method() == Method::HEAD {
        response::empty_with_content_length(StatusCode::OK, &content_type, bytes.len())
    } else {
        response::full(StatusCode::OK, &content_type, Bytes::from(bytes))
    }
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
    use super::safe_path;
    use std::path::Path;
    #[test]
    fn rejects_parent_directory_components() {
        assert!(safe_path(Path::new("public"), "../secret").is_none());
    }
}
