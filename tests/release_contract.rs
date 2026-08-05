//! Stable v1 command-line and starter-configuration contract tests.

use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("webserver-release-contract-{suffix}"));
        fs::create_dir_all(&path).expect("create temporary directory");
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_webserver")
}

#[test]
fn stable_v1_cli_surface_and_starter_configuration_work_together() {
    let help = Command::new(binary())
        .arg("--help")
        .output()
        .expect("run CLI help");
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).expect("UTF-8 help");
    for command in [
        "admin",
        "init",
        "check",
        "run",
        "site-add",
        "site-remove",
        "route-add",
        "route-remove",
        "completion",
    ] {
        assert!(help.contains(command), "missing stable command {command}");
    }

    let directory = TemporaryDirectory::new();
    let config = directory.0.join("webserver.toml");
    let init = Command::new(binary())
        .args(["init", "--config"])
        .arg(&config)
        .output()
        .expect("initialize starter configuration");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let check = Command::new(binary())
        .args(["check", "--config"])
        .arg(&config)
        .output()
        .expect("validate starter configuration");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(directory.0.join("sites/localhost.conf").is_file());
    assert!(directory.0.join("public/index.html").is_file());
}
