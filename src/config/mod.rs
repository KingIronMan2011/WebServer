//! Global server configuration and nginx-style per-site configuration files.

use std::{
    collections::HashSet,
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use hyper::Uri;
use serde::{Deserialize, Serialize};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::error::{Error, Result};

#[derive(Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub tls: TlsConfig,
    pub sites: Vec<SiteConfig>,
    source_path: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct GlobalConfig {
    #[serde(default)]
    server: ServerConfig,
    #[serde(default)]
    tls: TlsConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TlsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_https_bind")]
    pub bind: SocketAddr,
    pub email: Option<String>,
    #[serde(default = "default_certificate_cache")]
    pub certificate_cache: PathBuf,
    #[serde(default)]
    pub certificates: Vec<LocalCertificateConfig>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_https_bind(),
            email: None,
            certificate_cache: default_certificate_cache(),
            certificates: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LocalCertificateConfig {
    pub hosts: Vec<String>,
    pub certificate: PathBuf,
    pub private_key: PathBuf,
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
fn default_https_bind() -> SocketAddr {
    "0.0.0.0:443"
        .parse()
        .expect("default HTTPS bind address is valid")
}
#[cfg(windows)]
fn default_certificate_cache() -> PathBuf {
    PathBuf::from(r"C:\ProgramData\Webserver\certificates\acme")
}

#[cfg(not(windows))]
fn default_certificate_cache() -> PathBuf {
    PathBuf::from("/etc/webserver/certificates/acme")
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
            tls: global.tls,
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
        fs::create_dir_all(self.sites_directory())?;
        for site in &self.sites {
            save_site_preserving_comments(site)?;
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
        if self.tls.enabled {
            self.validate_local_certificates(&hosts)?;
        }
        Ok(())
    }

    fn validate_local_certificates(&self, sites: &HashSet<String>) -> Result<()> {
        let mut local_hosts = HashSet::new();
        for certificate in &self.tls.certificates {
            if certificate.hosts.is_empty() {
                return Err(Error::Config(
                    "a local certificate needs at least one host".into(),
                ));
            }
            if !certificate.certificate.is_file() {
                return Err(Error::Config(format!(
                    "local certificate does not exist: {}",
                    certificate.certificate.display()
                )));
            }
            if !certificate.private_key.is_file() {
                return Err(Error::Config(format!(
                    "local private key does not exist: {}",
                    certificate.private_key.display()
                )));
            }
            ensure_private_key_permissions(&certificate.private_key)?;
            for host in &certificate.hosts {
                let host = normalise_host(host);
                if host.is_empty() || !is_safe_host_name(&host) || !sites.contains(&host) {
                    return Err(Error::Config(format!(
                        "local certificate host must match a configured site: {host}"
                    )));
                }
                if !local_hosts.insert(host) {
                    return Err(Error::Config(
                        "a host can only use one local certificate".into(),
                    ));
                }
            }
        }
        if self.tls.email.as_deref().is_none_or(str::is_empty)
            && sites.iter().any(|host| !local_hosts.contains(host))
        {
            return Err(Error::Config(
                "tls.email is required for sites without a local certificate".into(),
            ));
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

#[cfg(unix)]
fn ensure_private_key_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path)?.permissions().mode();
    if mode & 0o027 != 0 || mode & 0o004 != 0 {
        return Err(Error::Config(format!(
            "local private key must not be writable by group/others or readable by others: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_key_permissions(_path: &Path) -> Result<()> {
    Ok(())
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

fn save_site_preserving_comments(site: &SiteConfig) -> Result<()> {
    if !site.source_path.exists() {
        return atomic_write(
            &site.source_path,
            &toml::to_string_pretty(site).map_err(|error| Error::Config(error.to_string()))?,
        );
    }

    let original = fs::read_to_string(&site.source_path)?;
    let current = load_site(&site.source_path)?;
    let current_paths: HashSet<&str> = current
        .routes
        .iter()
        .map(|route| route.path_prefix.as_str())
        .collect();
    let desired_paths: HashSet<&str> = site
        .routes
        .iter()
        .map(|route| route.path_prefix.as_str())
        .collect();
    if current_paths == desired_paths {
        return Ok(());
    }

    let mut document = original
        .parse::<DocumentMut>()
        .map_err(|error| Error::Config(format!("{}: {error}", site.source_path.display())))?;
    if document["routes"].is_none()
        || document["routes"]
            .as_array()
            .is_some_and(|routes| routes.is_empty())
    {
        document["routes"] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    let routes = document["routes"].as_array_of_tables_mut().ok_or_else(|| {
        Error::Config(format!(
            "{}: routes must be an array of tables",
            site.source_path.display()
        ))
    })?;
    routes.retain(|route| {
        route["path_prefix"]
            .as_str()
            .is_some_and(|path| desired_paths.contains(path))
    });
    for route in &site.routes {
        if !current_paths.contains(route.path_prefix.as_str()) {
            routes.push(route_table(route));
        }
    }
    atomic_write(&site.source_path, &document.to_string())?;
    Ok(())
}

fn route_table(route: &RouteConfig) -> Table {
    let mut table = Table::new();
    table["path_prefix"] = value(&route.path_prefix);
    match &route.target {
        RouteTarget::Static { root, index_file } => {
            table["kind"] = value("static");
            table["root"] = value(root.to_string_lossy().to_string());
            table["index_file"] = value(index_file);
        }
        RouteTarget::Proxy { upstream } => {
            table["kind"] = value("proxy");
            table["upstream"] = value(upstream);
        }
    }
    table
}

pub fn atomic_write(path: &Path, contents: &str) -> Result<()> {
    static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            Error::Config(format!(
                "configuration path has no valid file name: {}",
                path.display()
            ))
        })?;
    let temporary = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let write_result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    replace_file_atomically(&temporary, path)?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_file_atomically(temporary: &Path, path: &Path) -> std::io::Result<()> {
    fs::rename(temporary, path)
}

#[cfg(windows)]
fn replace_file_atomically(temporary: &Path, path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let encode = |value: &Path| {
        value
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let temporary = encode(temporary);
    let path = encode(path);
    if unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            path.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn is_safe_host_name(host: &str) -> bool {
    host.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

pub fn normalise_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_edits_preserve_comments_and_leave_no_temporary_file() {
        let directory =
            std::env::temp_dir().join(format!("webserver-config-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("sites")).expect("create test configuration directory");
        let config_path = directory.join("webserver.toml");
        let site_path = directory.join("sites/example.test.conf");
        fs::write(
            &config_path,
            "# global comment\n[server]\nbind = \"127.0.0.1:8080\"\n",
        )
        .expect("write global configuration");
        fs::write(
            &site_path,
            "# site comment\nhost = \"example.test\"\n\n# retained route comment\n[[routes]]\npath_prefix = \"/\"\nkind = \"proxy\"\nupstream = \"http://127.0.0.1:3000\"\n",
        )
        .expect("write site configuration");

        let mut config = Config::load(&config_path).expect("load configuration");
        config
            .add_route(
                "example.test",
                RouteConfig {
                    path_prefix: "/api".into(),
                    target: RouteTarget::Proxy {
                        upstream: "http://127.0.0.1:4000".into(),
                    },
                },
            )
            .expect("add route");
        config.save().expect("atomically save added route");
        let edited = fs::read_to_string(&site_path).expect("read edited configuration");
        assert!(edited.contains("# site comment"));
        assert!(edited.contains("# retained route comment"));
        assert!(edited.contains("path_prefix = \"/api\""));

        config
            .remove_route("example.test", "/api")
            .expect("remove route");
        config.save().expect("atomically save removed route");
        let edited = fs::read_to_string(&site_path).expect("read edited configuration");
        assert!(edited.contains("# retained route comment"));
        assert!(!edited.contains("path_prefix = \"/api\""));
        assert!(
            fs::read_dir(&directory)
                .expect("read test directory")
                .all(|entry| !entry
                    .expect("read entry")
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp"))
        );
        fs::remove_dir_all(directory).expect("remove test configuration directory");
    }
}
