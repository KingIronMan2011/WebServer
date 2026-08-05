//! End-to-end coverage for access controls that sit in front of route handling.

use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("webserver-security-test-{unique}"));
        fs::create_dir_all(&path).expect("create temporary directory");
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct ChildProcess(Child);

impl Drop for ChildProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn unused_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("read port").port()
}

fn start_server(config: &std::path::Path) -> ChildProcess {
    ChildProcess(
        Command::new(env!("CARGO_BIN_EXE_webserver"))
            .args(["run", "--config"])
            .arg(config)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start server"),
    )
}

fn request(port: u16, payload: &str) -> String {
    for _ in 0..30 {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .expect("set read timeout");
            stream.write_all(payload.as_bytes()).expect("send request");
            let mut response = String::new();
            stream.read_to_string(&mut response).expect("read response");
            return response;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("server did not start");
}

#[test]
fn applies_cors_preflight_before_serving_the_route_and_rate_limits_clients() {
    let directory = TempDirectory::new();
    let public = directory.0.join("public");
    let sites = directory.0.join("sites");
    fs::create_dir_all(&public).expect("create static directory");
    fs::create_dir_all(&sites).expect("create sites directory");
    fs::write(public.join("index.html"), "ok").expect("write static response");
    let port = unused_port();
    let config = directory.0.join("webserver.toml");
    fs::write(
        &config,
        format!("[server]\nbind = \"127.0.0.1:{port}\"\nrate_limit_per_minute = 1\n"),
    )
    .expect("write global configuration");
    fs::write(
        sites.join("localhost.conf"),
        r#"
host = "localhost"

[[routes]]
path_prefix = "/"
kind = "static"
root = "../public"

[routes.cors]
origins = ["https://app.example"]
methods = ["GET", "OPTIONS"]
headers = ["authorization"]
max_age_secs = 600
"#,
    )
    .expect("write site configuration");

    let _server = start_server(&config);
    let preflight = request(
        port,
        "OPTIONS / HTTP/1.1\r\nHost: localhost\r\nOrigin: https://app.example\r\nConnection: close\r\n\r\n",
    );
    assert!(preflight.starts_with("HTTP/1.1 204 No Content"));
    assert!(preflight.contains("access-control-allow-origin: https://app.example"));
    assert!(preflight.contains("access-control-allow-methods: GET, OPTIONS"));
    assert!(preflight.contains("access-control-max-age: 600"));

    let limited = request(
        port,
        "GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(limited.starts_with("HTTP/1.1 429 Too Many Requests"));
}
