use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "webserver-integration-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary test directory");
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

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_webserver")
}

fn unused_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("read local address").port()
}

fn request(port: u16, request: &str) -> String {
    String::from_utf8_lossy(&request_bytes(port, request)).into_owned()
}

fn request_bytes(port: u16, request: &str) -> Vec<u8> {
    let mut last_error = None;
    for _ in 0..30 {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut stream) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .expect("set test read timeout");
                stream.write_all(request.as_bytes()).expect("write request");
                let mut response = Vec::new();
                stream.read_to_end(&mut response).expect("read response");
                return response;
            }
            Err(error) => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
    panic!("server did not start: {last_error:?}");
}

fn start_server(config: &Path) -> ChildProcess {
    let mut child = Command::new(binary())
        .args(["run", "--config"])
        .arg(config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start webserver");
    thread::sleep(Duration::from_millis(100));
    if let Some(status) = child.try_wait().expect("read server status") {
        let mut stderr = String::new();
        let mut stdout = String::new();
        child
            .stdout
            .take()
            .expect("capture server stdout")
            .read_to_string(&mut stdout)
            .expect("read server stdout");
        child
            .stderr
            .take()
            .expect("capture server stderr")
            .read_to_string(&mut stderr)
            .expect("read server stderr");
        panic!("server exited early with {status}: {stdout}{stderr}");
    }
    ChildProcess(child)
}

#[test]
fn serves_static_files_and_proxies_requests() {
    let directory = TempDirectory::new();
    let public = directory.0.join("public");
    let errors = directory.0.join("errors");
    fs::create_dir_all(&public).expect("create public directory");
    fs::create_dir_all(&errors).expect("create errors directory");
    fs::write(public.join("index.html"), "static response").expect("write static fixture");
    fs::write(errors.join("not-found.html"), "custom missing page").expect("write error fixture");

    let upstream = TcpListener::bind("127.0.0.1:0").expect("start upstream listener");
    let upstream_port = upstream.local_addr().expect("read upstream address").port();
    let upstream_thread = thread::spawn(move || {
        let (mut stream, _) = upstream.accept().expect("accept proxy request");
        let mut request = [0_u8; 4096];
        let count = stream.read(&mut request).expect("read proxy request");
        let received = String::from_utf8_lossy(&request[..count]);
        assert!(received.contains("x-forwarded-host: localhost"));
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 17\r\nConnection: close\r\n\r\nproxied response\n")
            .expect("write upstream response");
    });

    let port = unused_port();
    let config = directory.0.join("webserver.toml");
    let sites = directory.0.join("sites");
    fs::create_dir_all(&sites).expect("create sites directory");
    fs::write(&config, format!("[server]\nbind = \"127.0.0.1:{port}\"\n"))
        .expect("write global configuration");
    fs::write(
        sites.join("localhost.conf"),
        format!(
            "host = \"localhost\"\n\n[[routes]]\npath_prefix = \"/\"\nkind = \"static\"\nroot = \"../public\"\nresponse_headers = {{ cache-control = \"public, max-age=3600\" }}\nerror_pages = {{ \"404\" = \"../errors/not-found.html\" }}\n\n[[routes]]\npath_prefix = \"/go\"\nkind = \"redirect\"\nlocation = \"https://example.test/target\"\nstatus = 302\n\n[[routes]]\npath_prefix = \"/api\"\nkind = \"proxy\"\nupstream = \"http://127.0.0.1:{upstream_port}\"\n"
        ),
    )
    .expect("write site configuration");

    let _server = start_server(&config);
    let static_response = request(
        port,
        "GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(
        static_response.starts_with("HTTP/1.1 200 OK"),
        "unexpected static response: {static_response}"
    );
    assert!(static_response.ends_with("static response"));
    assert!(static_response.contains("cache-control: public, max-age=3600"));
    let etag = static_response
        .lines()
        .find_map(|line| line.strip_prefix("etag: "))
        .expect("static response has an ETag")
        .to_owned();

    let range_response = request(
        port,
        "GET / HTTP/1.1\r\nHost: localhost\r\nRange: bytes=0-5\r\nConnection: close\r\n\r\n",
    );
    assert!(range_response.starts_with("HTTP/1.1 206 Partial Content"));
    assert!(range_response.contains("content-range: bytes 0-5/15"));
    assert!(range_response.ends_with("static"));

    let compressed_response = request(
        port,
        "GET / HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n",
    );
    assert!(compressed_response.starts_with("HTTP/1.1 200 OK"));
    assert!(compressed_response.contains("content-encoding: gzip"));
    assert!(compressed_response.contains("vary: Accept-Encoding"));

    let conditional_response = request(
        port,
        &format!(
            "GET / HTTP/1.1\r\nHost: localhost\r\nIf-None-Match: {etag}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(conditional_response.starts_with("HTTP/1.1 304 Not Modified"));

    let redirect_response = request(
        port,
        "GET /go HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(redirect_response.starts_with("HTTP/1.1 302 Found"));
    assert!(redirect_response.contains("location: https://example.test/target"));

    let missing_response = request(
        port,
        "GET /missing HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(missing_response.starts_with("HTTP/1.1 404 Not Found"));
    assert!(missing_response.contains("text/html; charset=utf-8"));
    assert!(missing_response.ends_with("custom missing page"));

    let proxy_response = request(
        port,
        "GET /api HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(proxy_response.starts_with("HTTP/1.1 200 OK"));
    assert!(proxy_response.ends_with("proxied response\n"));
    upstream_thread.join().expect("upstream thread completed");
}

#[test]
fn cli_manages_sites_and_routes() {
    let directory = TempDirectory::new();
    let config = directory.0.join("webserver.toml");

    for arguments in [
        vec!["init", "--config"],
        vec!["site-add", "--config"],
        vec!["route-add", "--config"],
        vec!["check", "--config"],
    ] {
        let mut command = Command::new(binary());
        command.args(&arguments).arg(&config);
        if command.get_args().any(|argument| argument == "site-add") {
            command.args(["--host", "example.test"]);
        }
        if command.get_args().any(|argument| argument == "route-add") {
            command.args([
                "--host",
                "example.test",
                "--path",
                "/",
                "--static",
                "./assets",
            ]);
        }
        let output = command.output().expect("run CLI command");
        assert!(
            output.status.success(),
            "command {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
fn enforces_body_limits_and_returns_error_pages() {
    let directory = TempDirectory::new();
    let public = directory.0.join("public");
    fs::create_dir_all(&public).expect("create public directory");
    fs::write(public.join("index.html"), "ok").expect("write static fixture");
    let port = unused_port();
    let config = directory.0.join("webserver.toml");
    let sites = directory.0.join("sites");
    fs::create_dir_all(&sites).expect("create sites directory");
    fs::write(
        &config,
        format!("[server]\nbind = \"127.0.0.1:{port}\"\nmax_body_bytes = 10\n"),
    )
    .expect("write global configuration");
    fs::write(
        sites.join("localhost.conf"),
        "host = \"localhost\"\n\n[[routes]]\npath_prefix = \"/\"\nkind = \"static\"\nroot = \"../public\"\n",
    )
    .expect("write site configuration");

    let _server = start_server(&config);
    let oversized = request(
        port,
        "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 11\r\nConnection: close\r\n\r\n01234567890",
    );
    assert!(oversized.starts_with("HTTP/1.1 413 Payload Too Large"));
    assert!(oversized.contains("text/html; charset=utf-8"));

    let chunked = request(
        port,
        "POST / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\n",
    );
    assert!(chunked.starts_with("HTTP/1.1 411 Length Required"));

    let missing_host = request(
        port,
        "GET / HTTP/1.1\r\nHost: unknown.test\r\nConnection: close\r\n\r\n",
    );
    assert!(missing_host.starts_with("HTTP/1.1 404 Not Found"));
}
