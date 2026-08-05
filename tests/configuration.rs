//! Configuration validation for protocol and discovery features.

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use webserver::config::Config;

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("webserver-config-test-{unique}"));
        fs::create_dir_all(&path).expect("create temporary directory");
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_site(directory: &TempDirectory, contents: &str) -> PathBuf {
    let sites = directory.0.join("sites");
    fs::create_dir_all(&sites).expect("create sites directory");
    fs::write(sites.join("example.test.conf"), contents).expect("write site configuration");
    let config = directory.0.join("webserver.toml");
    fs::write(&config, "[server]\nbind = \"127.0.0.1:0\"\n").expect("write global config");
    config
}

#[test]
fn accepts_combined_dns_docker_and_kubernetes_discovery() {
    let directory = TempDirectory::new();
    let config = write_site(
        &directory,
        r#"
host = "example.test"

[[routes]]
path_prefix = "/api"
kind = "proxy"

[routes.dns_discovery]
host = "api.internal.example"
port = 3000

[routes.docker_discovery]
labels = { "webserver.discovery" = "api" }
port = 3000
refresh_secs = 15

[routes.kubernetes_discovery]
service = "api"
namespace = "production"
port = 3000
"#,
    );

    let config = Config::load(&config).expect("load configuration");
    config.validate().expect("valid discovery configuration");
}

#[test]
fn rejects_http3_without_tls() {
    let directory = TempDirectory::new();
    let config = write_site(
        &directory,
        r#"
host = "example.test"

[[routes]]
path_prefix = "/"
kind = "proxy"
upstream = "http://127.0.0.1:3000"
"#,
    );
    fs::write(
        &config,
        "[server]\nbind = \"127.0.0.1:0\"\n\n[tls]\nhttp3 = true\n",
    )
    .expect("write invalid global config");

    let config = Config::load(&config).expect("load configuration");
    let error = config
        .validate()
        .expect_err("HTTP/3 without TLS is invalid");
    assert!(error.to_string().contains("tls.http3 requires tls.enabled"));
}

#[test]
fn rejects_invalid_discovery_and_cors_settings() {
    let directory = TempDirectory::new();
    let config = write_site(
        &directory,
        r#"
host = "example.test"

[[routes]]
path_prefix = "/api"
kind = "proxy"
cors = { origins = ["*"], allow_credentials = true }

[routes.docker_discovery]
labels = { "webserver.discovery" = "api" }
port = 0
"#,
    );

    let config = Config::load(&config).expect("load configuration");
    let error = config
        .validate()
        .expect_err("invalid configuration is rejected");
    assert!(error.to_string().contains("CORS"));
}
