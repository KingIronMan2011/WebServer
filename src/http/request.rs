//! Lightweight request metadata used for routing and logging.

use hyper::Request;

use crate::config::{is_safe_host_name, normalise_host};

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
            .and_then(authority_host)
            .or_else(|| {
                request
                    .uri()
                    .authority()
                    .and_then(|authority| authority_host(authority.as_str()))
            })
            .map(|host| normalise_host(&host))
            .unwrap_or_default();
        Self {
            host,
            path: request.uri().path().to_owned(),
        }
    }
}

pub(crate) fn authority_host(value: &str) -> Option<String> {
    // URI authorities permit userinfo, but the HTTP Host field does not. If it
    // were accepted here, `user@admin.example` could normalize to the protected
    // admin hostname in the underlying authority parser.
    if value.contains('@') {
        return None;
    }
    let has_valid_port = if value.starts_with('[') {
        let closing_bracket = value.find(']')?;
        let suffix = &value[closing_bracket + 1..];
        suffix.is_empty()
            || suffix
                .strip_prefix(':')
                .is_some_and(|port| !port.is_empty() && port.parse::<u16>().is_ok())
    } else {
        match value.matches(':').count() {
            0 => true,
            1 => value
                .rsplit_once(':')
                .is_some_and(|(_, port)| !port.is_empty() && port.parse::<u16>().is_ok()),
            _ => false,
        }
    };
    if !has_valid_port {
        return None;
    }

    let authority = value.parse::<hyper::http::uri::Authority>().ok()?;
    let host = normalise_host(authority.host());
    is_safe_host_name(&host).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::RequestContext;
    use hyper::Request;

    #[test]
    fn rejects_an_invalid_host_authority() {
        for host in [
            "example.test:not-a-port",
            "example.test:65536",
            "user@example.test",
            "[2001:db8::1]:8443",
        ] {
            let request = Request::builder().header("host", host).body(()).unwrap();
            assert!(
                RequestContext::from_request(&request).host.is_empty(),
                "unexpectedly accepted host authority: {host}"
            );
        }
    }

    #[test]
    fn accepts_hosts_with_valid_ports() {
        for (host, expected) in [
            ("example.test:443", "example.test"),
            ("LOCALHOST", "localhost"),
            ("127.0.0.1:8080", "127.0.0.1"),
        ] {
            let request = Request::builder().header("host", host).body(()).unwrap();
            assert_eq!(RequestContext::from_request(&request).host, expected);
        }
    }
}
