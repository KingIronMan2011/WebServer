//! Runtime DNS discovery for proxy upstreams.

use crate::config::DnsDiscoveryConfig;

pub async fn dns(config: &DnsDiscoveryConfig) -> Vec<String> {
    let Ok(addresses) = tokio::net::lookup_host((config.host.as_str(), config.port)).await else {
        return Vec::new();
    };
    addresses
        .map(|address| format!("{}://{}", config.scheme, address))
        .collect()
}
