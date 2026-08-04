//! Global server configuration and nginx-style per-site configuration files.

use std::{
    collections::HashSet,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use hyper::Uri;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub sites: Vec<SiteConfig>,
    source_path: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct GlobalConfig {
    #[serde(default)]
    server: ServerConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: SocketAddr,
    #[serde(default = "default_timeout")]
    pub upstream_timeout_secs: u64,
    #[serde(default = "default_max_header_bytes")]
    pub max_header_bytes: usize,
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            upstream_timeout_secs: default_timeout(),
            max_header_bytes: default_max_header_bytes(),
            max_body_bytes: default_max_body_bytes(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SiteConfig {
    pub host: String,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
    #[serde(skip)]
    source_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RouteConfig {
    pub path_prefix: String,
    #[serde(flatten)]
    pub target: RouteTarget,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RouteTarget {
    Static {
        root: PathBuf,
        #[serde(default = "default_index_file")]
        index_file: String,
    },
    Proxy {
        upstream: String,
    },
}

fn default_bind() -> SocketAddr {
    "0.0.0.0:80".parse().expect("default bind address is valid")
}
fn default_timeout() -> u64 {
    30
}
fn default_max_header_bytes() -> usize {
    32 * 1024
}
fn default_max_body_bytes() -> u64 {
    10 * 1024 * 1024
}
fn default_index_file() -> String {
    "index.html".into()
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let source_path = config_path(path.as_ref());
        let contents = fs::read_to_string(&source_path)?;
        let global: GlobalConfig =
            toml::from_str(&contents).map_err(|error| Error::Config(error.to_string()))?;
        let sites_directory = source_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("sites");
        if !sites_directory.is_dir() {
            return Err(Error::Config(format!(
                "sites directory does not exist: {}",
                sites_directory.display()
            )));
        }

        let mut site_paths: Vec<PathBuf> = fs::read_dir(&sites_directory)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                entry
                    .file_type()
                    .ok()
                    .filter(|kind| kind.is_file())
                    .map(|_| entry.path())
            })
            .collect();
        site_paths.sort();

        let sites = site_paths
            .into_iter()
            .map(|site_path| load_site(&site_path))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            server: global.server,
            sites,
            source_path,
        })
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub fn sites_directory(&self) -> PathBuf {
        self.source_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("sites")
    }

    pub fn save(&self) -> Result<()> {
        let global = GlobalConfig {
            server: self.server.clone(),
        };
        fs::write(
            &self.source_path,
            toml::to_string_pretty(&global).map_err(|error| Error::Config(error.to_string()))?,
        )?;
        fs::create_dir_all(self.sites_directory())?;
        for site in &self.sites {
            save_site(site)?;
        }
        Ok(())
    }

    pub fn add_site(&mut self, host: String) -> Result<()> {
        let host = normalise_host(&host);
        if host.is_empty() || !is_safe_host_name(&host) {
            return Err(Error::Config("site host must be a valid hostname".into()));
        }
        if self
            .sites
            .iter()
            .any(|site| normalise_host(&site.host) == host)
        {
            return Err(Error::Config(format!("site already exists: {host}")));
        }
        let source_path = self.sites_directory().join(format!("{host}.conf"));
        if source_path.exists() {
            return Err(Error::Config(format!(
                "site configuration already exists: {}",
                source_path.display()
            )));
        }
        self.sites.push(SiteConfig {
            host,
            routes: Vec::new(),
            source_path,
        });
        Ok(())
    }

    pub fn remove_site(&mut self, host: &str) -> Result<()> {
        let host = normalise_host(host);
        let index = self
            .sites
            .iter()
            .position(|site| normalise_host(&site.host) == host)
            .ok_or_else(|| Error::Config(format!("site does not exist: {host}")))?;
        let site = self.sites.remove(index);
        fs::remove_file(site.source_path)?;
        Ok(())
    }

    pub fn add_route(&mut self, host: &str, route: RouteConfig) -> Result<()> {
        let host = normalise_host(host);
        let index = self
            .sites
            .iter()
            .position(|site| normalise_host(&site.host) == host)
            .ok_or_else(|| Error::Config(format!("site does not exist: {host}")))?;
        self.validate_route(&self.sites[index], &route)?;
        let site = &mut self.sites[index];
        if site
            .routes
            .iter()
            .any(|existing| existing.path_prefix == route.path_prefix)
        {
            return Err(Error::Config(format!(
                "route already exists for {host}: {}",
                route.path_prefix
            )));
        }
        site.routes.push(route);
        Ok(())
    }

    pub fn remove_route(&mut self, host: &str, path_prefix: &str) -> Result<()> {
        let host = normalise_host(host);
        let site = self
            .sites
            .iter_mut()
            .find(|site| normalise_host(&site.host) == host)
            .ok_or_else(|| Error::Config(format!("site does not exist: {host}")))?;
        let count_before = site.routes.len();
        site.routes.retain(|route| route.path_prefix != path_prefix);
        if site.routes.len() == count_before {
            return Err(Error::Config(format!(
                "route does not exist for {host}: {path_prefix}"
            )));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.server.max_header_bytes < 8 * 1024 {
            return Err(Error::Config(
                "server.max_header_bytes must be at least 8192".into(),
            ));
        }
        if self.server.max_body_bytes == 0 {
            return Err(Error::Config(
                "server.max_body_bytes must be greater than zero".into(),
            ));
        }
        if self.sites.is_empty() {
            return Err(Error::Config(format!(
                "no site files found in {}",
                self.sites_directory().display()
            )));
        }
        let mut hosts = HashSet::new();
        for site in &self.sites {
            let host = normalise_host(&site.host);
            if host.is_empty() || !is_safe_host_name(&host) {
                return Err(Error::Config(format!("invalid site host: {}", site.host)));
            }
            if !hosts.insert(host) {
                return Err(Error::Config(format!("duplicate site host: {}", site.host)));
            }
            if site.routes.is_empty() {
                return Err(Error::Config(format!("site {} has no routes", site.host)));
            }
            for route in &site.routes {
                self.validate_route(site, route)?;
            }
        }
        Ok(())
    }

    fn validate_route(&self, site: &SiteConfig, route: &RouteConfig) -> Result<()> {
        if !route.path_prefix.starts_with('/') {
            return Err(Error::Config(format!(
                "route '{}' must start with '/'",
                route.path_prefix
            )));
        }
        match &route.target {
            RouteTarget::Static { root, index_file } => {
                if index_file.is_empty() {
                    return Err(Error::Config(
                        "static route index_file must not be empty".into(),
                    ));
                }
                let root = site.static_path(root);
                if !root.is_dir() {
                    return Err(Error::Config(format!(
                        "static root does not exist or is not a directory: {}",
                        root.display()
                    )));
                }
            }
            RouteTarget::Proxy { upstream } => {
                let uri: Uri = upstream
                    .parse()
                    .map_err(|_| Error::Config(format!("invalid upstream URI: {upstream}")))?;
                if uri.scheme().is_none() || uri.authority().is_none() {
                    return Err(Error::Config(format!(
                        "upstream must include scheme and host: {upstream}"
                    )));
                }
            }
        }
        Ok(())
    }
}

impl SiteConfig {
    pub fn static_path(&self, root: &Path) -> PathBuf {
        if root.is_absolute() {
            root.to_path_buf()
        } else {
            self.source_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(root)
        }
    }
}

fn config_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("webserver.toml")
    } else {
        path.to_path_buf()
    }
}

fn load_site(path: &Path) -> Result<SiteConfig> {
    let contents = fs::read_to_string(path)?;
    let mut site: SiteConfig = toml::from_str(&contents)
        .map_err(|error| Error::Config(format!("{}: {error}", path.display())))?;
    site.source_path = path.to_path_buf();
    Ok(site)
}

fn save_site(site: &SiteConfig) -> Result<()> {
    fs::write(
        &site.source_path,
        toml::to_string_pretty(site).map_err(|error| Error::Config(error.to_string()))?,
    )?;
    Ok(())
}

fn is_safe_host_name(host: &str) -> bool {
    host.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

pub fn normalise_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}
