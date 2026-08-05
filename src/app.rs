//! Composition root for the v0.1 server.

use std::path::Path;

use clap::CommandFactory;
use clap_complete::generate;

use crate::{
    cli::{Cli, Command},
    config::{Config, RouteConfig, RouteTarget},
    error::Result,
    server::listener,
};

pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Run { config } => {
            let config = Config::load(&config)?;
            config.validate()?;
            listener::run(config).await
        }
        Command::Check { config } => {
            let config = Config::load(&config)?;
            config.validate()?;
            println!("Configuration is valid: {}", config.source_path().display());
            Ok(())
        }
        Command::Init { config } => write_example_config(&config),
        Command::SiteAdd { config, host } => {
            let mut config = Config::load(&config)?;
            config.add_site(host.clone())?;
            config.save()?;
            println!("Added site: {host}");
            Ok(())
        }
        Command::SiteRemove { config, host } => {
            let mut config = Config::load(&config)?;
            config.remove_site(&host)?;
            config.save()?;
            println!("Removed site: {host}");
            Ok(())
        }
        Command::RouteAdd {
            config,
            host,
            path,
            static_root,
            upstream,
        } => {
            let mut config = Config::load(&config)?;
            let target = match (static_root, upstream) {
                (Some(root), None) => {
                    let root = if root.is_absolute() {
                        root
                    } else {
                        config
                            .source_path()
                            .parent()
                            .unwrap_or_else(|| Path::new("."))
                            .join(root)
                    };
                    std::fs::create_dir_all(&root)?;
                    RouteTarget::Static {
                        root,
                        index_file: "index.html".into(),
                    }
                }
                (None, Some(upstream)) => RouteTarget::Proxy {
                    upstream: Some(upstream),
                    upstreams: Vec::new(),
                    load_balancing: crate::config::LoadBalancing::RoundRobin,
                    retries: 1,
                    retry_backoff_ms: 100,
                    max_connections_per_upstream: 0,
                    base_path: None,
                    rewrite_prefix: None,
                    health_check: None,
                    dns_discovery: None,
                },
                _ => {
                    return Err(crate::error::Error::Config(
                        "choose exactly one of --static or --upstream".into(),
                    ));
                }
            };
            config.add_route(
                &host,
                RouteConfig {
                    path_prefix: path.clone(),
                    response_headers: Default::default(),
                    error_pages: Default::default(),
                    cors: None,
                    target,
                },
            )?;
            config.save()?;
            println!("Added route {path} to {host}");
            Ok(())
        }
        Command::RouteRemove { config, host, path } => {
            let mut config = Config::load(&config)?;
            config.remove_route(&host, &path)?;
            config.save()?;
            println!("Removed route {path} from {host}");
            Ok(())
        }
        Command::Completion { shell } => {
            generate(
                shell,
                &mut Cli::command(),
                "webserver",
                &mut std::io::stdout(),
            );
            Ok(())
        }
    }
}

fn write_example_config(path: &Path) -> Result<()> {
    if path.exists() {
        return Err(crate::error::Error::Config(format!(
            "refusing to overwrite existing file: {}",
            path.display()
        )));
    }

    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(directory)?;
    let sites = directory.join("sites");
    std::fs::create_dir_all(&sites)?;
    let public = directory.join("public");
    std::fs::create_dir_all(&public)?;
    let index = public.join("index.html");
    if !index.exists() {
        std::fs::write(
            &index,
            "<!doctype html><title>Webserver</title><h1>It works!</h1>\n",
        )?;
    }
    crate::config::atomic_write(path, starter_config())?;
    crate::config::atomic_write(&sites.join("localhost.conf"), starter_site_config())?;
    println!("Created example configuration: {}", path.display());
    Ok(())
}

fn starter_config() -> &'static str {
    r#"[server]
bind = "0.0.0.0:80"
upstream_timeout_secs = 30
max_header_bytes = 32768
max_body_bytes = 10485760
"#
}

fn starter_site_config() -> &'static str {
    r#"host = "localhost"

[[routes]]
path_prefix = "/"
kind = "static"
root = "../public"
index_file = "index.html"
"#
}
