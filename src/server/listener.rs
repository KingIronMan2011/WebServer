//! Binds the TCP listener and accepts incoming connections.

use std::sync::Arc;

#[cfg(unix)]
use std::path::PathBuf;

use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::RwLock;

use crate::{config::Config, error::Result, server::connection};

pub async fn run(config: Config) -> Result<()> {
    let listener = TcpListener::bind(config.server.bind).await?;
    #[cfg(unix)]
    let config_path = config.source_path().to_path_buf();
    let max_header_bytes = config.server.max_header_bytes;
    let state = Arc::new(RwLock::new(config));
    tracing::info!(address = %listener.local_addr()?, "listening for HTTP connections");

    #[cfg(unix)]
    return run_unix(listener, state, config_path, max_header_bytes).await;

    #[cfg(not(unix))]
    run_without_reload(listener, state, max_header_bytes).await
}

#[cfg(windows)]
pub async fn run_service(
    config: Config,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let listener = TcpListener::bind(config.server.bind).await?;
    let max_header_bytes = config.server.max_header_bytes;
    let state = Arc::new(RwLock::new(config));
    tracing::info!(address = %listener.local_addr()?, "Windows service is listening for HTTP connections");

    loop {
        tokio::select! {
            result = accept_one(&listener, &state, max_header_bytes) => result?,
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    tracing::info!("received Windows service stop signal");
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(not(unix))]
async fn run_without_reload(
    listener: TcpListener,
    state: Arc<RwLock<Config>>,
    max_header_bytes: usize,
) -> Result<()> {
    accept_loop(listener, state, max_header_bytes).await
}

#[cfg(unix)]
async fn run_unix(
    listener: TcpListener,
    state: Arc<RwLock<Config>>,
    config_path: PathBuf,
    max_header_bytes: usize,
) -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut reload = signal(SignalKind::hangup())?;
    loop {
        tokio::select! {
            result = accept_one(&listener, &state, max_header_bytes) => result?,
            _ = reload.recv() => reload_config(&state, &config_path).await,
            signal = tokio::signal::ctrl_c() => {
                signal.expect("failed to install Ctrl+C handler");
                tracing::info!("received shutdown signal");
                return Ok(());
            }
        }
    }
}

#[cfg(not(unix))]
async fn accept_loop(
    listener: TcpListener,
    state: Arc<RwLock<Config>>,
    max_header_bytes: usize,
) -> Result<()> {
    loop {
        tokio::select! {
            result = accept_one(&listener, &state, max_header_bytes) => result?,
            signal = tokio::signal::ctrl_c() => {
                signal.expect("failed to install Ctrl+C handler");
                tracing::info!("received shutdown signal");
                return Ok(());
            }
        }
    }
}

async fn accept_one(
    listener: &TcpListener,
    state: &Arc<RwLock<Config>>,
    max_header_bytes: usize,
) -> Result<()> {
    let (stream, peer) = listener.accept().await?;
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let service =
            service_fn(move |request| connection::handle(request, peer, Arc::clone(&state)));
        if let Err(error) = hyper::server::conn::http1::Builder::new()
            .max_buf_size(max_header_bytes)
            .serve_connection(TokioIo::new(stream), service)
            .with_upgrades()
            .await
        {
            tracing::debug!(%peer, %error, "connection closed with HTTP error");
        }
    });
    Ok(())
}

#[cfg(unix)]
async fn reload_config(state: &Arc<RwLock<Config>>, path: &PathBuf) {
    match Config::load(path).and_then(|config| {
        config.validate()?;
        Ok(config)
    }) {
        Ok(config) => {
            *state.write().await = config;
            tracing::info!(config = %path.display(), "configuration reloaded");
        }
        Err(error) => {
            tracing::error!(%error, config = %path.display(), "configuration reload rejected")
        }
    }
}
