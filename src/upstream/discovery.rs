//! Runtime discovery sources for proxy upstreams.

#[cfg(unix)]
use std::{collections::HashMap, sync::{Mutex, OnceLock}, time::Instant};

use crate::config::{DnsDiscoveryConfig, DockerDiscoveryConfig, KubernetesDiscoveryConfig};

pub async fn dns(config: &DnsDiscoveryConfig) -> Vec<String> {
    let Ok(addresses) = tokio::net::lookup_host((config.host.as_str(), config.port)).await else {
        return Vec::new();
    };
    addresses
        .map(|address| format!("{}://{}", config.scheme, address))
        .collect()
}

/// Resolves running Docker containers through the local Docker Engine socket.
/// Results are short-lived cached, so an application request never causes a
/// Docker API round-trip unless the configured refresh interval elapsed.
pub async fn docker(config: &DockerDiscoveryConfig) -> Vec<String> {
    #[cfg(unix)]
    {
        let key = format!(
            "{}|{}|{}|{:?}",
            config.socket.display(), config.port, config.scheme, config.labels
        );
        if let Some(cached) = docker_cache()
            .lock()
            .expect("docker discovery cache")
            .get(&key)
            .filter(|cached| cached.created.elapsed().as_secs() < config.refresh_secs)
        {
            return cached.targets.clone();
        }
        let targets = docker_query(config).await;
        docker_cache().lock().expect("docker discovery cache").insert(
            key,
            CachedDocker {
                created: Instant::now(),
                targets: targets.clone(),
            },
        );
        targets
    }
    #[cfg(not(unix))]
    {
        let _ = config;
        tracing::warn!("Docker discovery is only available on Unix hosts");
        Vec::new()
    }
}

/// Resolves Kubernetes Service records through the configured cluster DNS.
pub async fn kubernetes(config: &KubernetesDiscoveryConfig) -> Vec<String> {
    let host = format!(
        "{}.{}.svc.{}",
        config.service, config.namespace, config.cluster_domain
    );
    let Ok(addresses) = tokio::net::lookup_host((host.as_str(), config.port)).await else {
        tracing::warn!(%host, "Kubernetes service DNS lookup failed");
        return Vec::new();
    };
    addresses
        .map(|address| format!("{}://{}", config.scheme, address))
        .collect()
}

#[cfg(unix)]
struct CachedDocker {
    created: Instant,
    targets: Vec<String>,
}

#[cfg(unix)]
fn docker_cache() -> &'static Mutex<HashMap<String, CachedDocker>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedDocker>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(unix)]
async fn docker_query(config: &DockerDiscoveryConfig) -> Vec<String> {
    use http_body_util::{BodyExt, Empty};
    use hyper::{Request, body::Bytes, client::conn::http1};
    use hyper_util::rt::TokioIo;

    let stream = match tokio::net::UnixStream::connect(&config.socket).await {
        Ok(stream) => stream,
        Err(error) => {
            tracing::warn!(socket = %config.socket.display(), %error, "Docker discovery could not connect to Docker Engine");
            return Vec::new();
        }
    };
    let (mut sender, connection) = match http1::handshake(TokioIo::new(stream)).await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(%error, "Docker discovery could not establish an HTTP connection");
            return Vec::new();
        }
    };
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::debug!(%error, "Docker Engine connection closed");
        }
    });
    let request = Request::builder()
        .uri("http://docker/containers/json")
        .body(Empty::<Bytes>::new())
        .expect("valid Docker API request");
    let response = match sender.send_request(request).await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            tracing::warn!(status = %response.status(), "Docker discovery API request failed");
            return Vec::new();
        }
        Err(error) => {
            tracing::warn!(%error, "Docker discovery API request failed");
            return Vec::new();
        }
    };
    let payload = match response.into_body().collect().await {
        Ok(payload) => payload.to_bytes(),
        Err(error) => {
            tracing::warn!(%error, "Docker discovery response body failed");
            return Vec::new();
        }
    };
    let containers: Vec<DockerContainer> = match serde_json::from_slice(&payload) {
        Ok(containers) => containers,
        Err(error) => {
            tracing::warn!(%error, "Docker discovery returned invalid JSON");
            return Vec::new();
        }
    };
    containers
        .into_iter()
        .filter(|container| {
            config.labels.iter().all(|(key, value)| {
                container.labels.get(key).is_some_and(|actual| actual == value)
            })
        })
        .flat_map(|container| container.network_settings.networks.into_values())
        .filter_map(|network| (!network.ip_address.is_empty()).then_some(network.ip_address))
        .map(|ip| format!("{}://{}:{}", config.scheme, ip, config.port))
        .collect()
}

#[cfg(unix)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerContainer {
    #[serde(default)]
    labels: HashMap<String, String>,
    network_settings: DockerNetworkSettings,
}

#[cfg(unix)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerNetworkSettings {
    #[serde(default)]
    networks: HashMap<String, DockerNetwork>,
}

#[cfg(unix)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerNetwork {
    #[serde(default)]
    ip_address: String,
}
