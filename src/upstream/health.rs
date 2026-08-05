//! Periodic active health checks for configured proxy targets.

use std::time::Duration;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{Config, RouteTarget, proxy_upstreams},
    handlers::reverse_proxy::report_health,
};

pub fn spawn(config: &Config, shutdown: CancellationToken) {
    for site in &config.sites {
        for route in &site.routes {
            let RouteTarget::Proxy {
                upstream,
                upstreams,
                health_check: Some(check),
                ..
            } = &route.target
            else {
                continue;
            };
            for (url, _) in proxy_upstreams(upstream.as_deref(), upstreams) {
                let url = url.to_owned();
                let check = check.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move {
                    loop {
                        let healthy = check_once(&url, &check.path, check.timeout_secs).await;
                        report_health(&url, healthy);
                        tokio::select! {
                            _ = shutdown.cancelled() => return,
                            _ = tokio::time::sleep(Duration::from_secs(check.interval_secs)) => {},
                        }
                    }
                });
            }
        }
    }
}

async fn check_once(url: &str, path: &str, timeout_secs: u64) -> bool {
    let Ok(uri) = url.parse::<hyper::Uri>() else {
        return false;
    };
    if uri.scheme_str() != Some("http") {
        return false;
    }
    let Some(authority) = uri.authority() else {
        return false;
    };
    let address = authority.as_str();
    let result = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        let mut stream = TcpStream::connect(address).await?;
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        let mut response = [0_u8; 32];
        let count = stream.read(&mut response).await?;
        Ok::<_, std::io::Error>(
            response[..count].starts_with(b"HTTP/1.1 2")
                || response[..count].starts_with(b"HTTP/1.0 2"),
        )
    })
    .await;
    matches!(result, Ok(Ok(true)))
}
