//! Binds the TCP listener and accepts incoming connections.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[cfg(unix)]
use std::path::PathBuf;

use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::{
    sync::{Notify, RwLock},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{config::Config, error::Result, server::connection, tls::TlsManager};

pub async fn run(config: Config) -> Result<()> {
    let listener = TcpListener::bind(config.server.bind).await?;
    #[cfg(unix)]
    let config_path = config.source_path().to_path_buf();
    let max_header_bytes = config.server.max_header_bytes;
    let tls = start_tls(&config).await?;
    let state = Arc::new(RwLock::new(config));
    let shutdown = CancellationToken::new();
    let connections = Arc::new(ConnectionTracker::default());
    tracing::info!(address = %listener.local_addr()?, "listening for HTTP connections");
    let tls_listener = spawn_tls_listener(
        &state,
        tls.clone(),
        max_header_bytes,
        shutdown.clone(),
        Arc::clone(&connections),
    )
    .await?;

    #[cfg(unix)]
    let result = run_unix(
        listener,
        state,
        tls,
        connections.clone(),
        config_path,
        max_header_bytes,
    )
    .await;

    #[cfg(not(unix))]
    let result =
        run_without_reload(listener, state, tls, connections.clone(), max_header_bytes).await;

    shutdown.cancel();
    if let Some(listener) = tls_listener {
        let _ = listener.await;
    }
    connections.wait_for_all().await;
    result
}

#[cfg(windows)]
pub async fn run_service(
    config: Config,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let listener = TcpListener::bind(config.server.bind).await?;
    let max_header_bytes = config.server.max_header_bytes;
    let tls = start_tls(&config).await?;
    let state = Arc::new(RwLock::new(config));
    let shutdown_token = CancellationToken::new();
    let connections = Arc::new(ConnectionTracker::default());
    tracing::info!(address = %listener.local_addr()?, "Windows service is listening for HTTP connections");
    let tls_listener = spawn_tls_listener(
        &state,
        tls.clone(),
        max_header_bytes,
        shutdown_token.clone(),
        Arc::clone(&connections),
    )
    .await?;

    let result = loop {
        tokio::select! {
            result = accept_one(&listener, &state, tls.clone(), Arc::clone(&connections), max_header_bytes) => result?,
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    tracing::info!("received Windows service stop signal");
                    break Ok(());
                }
            }
        }
    };
    shutdown_token.cancel();
    if let Some(listener) = tls_listener {
        let _ = listener.await;
    }
    connections.wait_for_all().await;
    result
}

#[cfg(not(unix))]
async fn run_without_reload(
    listener: TcpListener,
    state: Arc<RwLock<Config>>,
    tls: Option<Arc<TlsManager>>,
    connections: Arc<ConnectionTracker>,
    max_header_bytes: usize,
) -> Result<()> {
    accept_loop(listener, state, tls, connections, max_header_bytes).await
}

#[cfg(unix)]
async fn run_unix(
    listener: TcpListener,
    state: Arc<RwLock<Config>>,
    tls: Option<Arc<TlsManager>>,
    connections: Arc<ConnectionTracker>,
    config_path: PathBuf,
    max_header_bytes: usize,
) -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut reload = signal(SignalKind::hangup())?;
    loop {
        tokio::select! {
            result = accept_one(&listener, &state, tls.clone(), Arc::clone(&connections), max_header_bytes) => result?,
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
    tls: Option<Arc<TlsManager>>,
    connections: Arc<ConnectionTracker>,
    max_header_bytes: usize,
) -> Result<()> {
    loop {
        tokio::select! {
            result = accept_one(&listener, &state, tls.clone(), Arc::clone(&connections), max_header_bytes) => result?,
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
    tls: Option<Arc<TlsManager>>,
    connections: Arc<ConnectionTracker>,
    max_header_bytes: usize,
) -> Result<()> {
    let (stream, peer) = listener.accept().await?;
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let _connection = connections.track();
        let service = service_fn(move |request| {
            connection::handle(request, peer, Arc::clone(&state), tls.clone(), false)
        });
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

async fn start_tls(config: &Config) -> Result<Option<Arc<TlsManager>>> {
    if config.tls.enabled {
        Ok(Some(TlsManager::start(config).await?))
    } else {
        Ok(None)
    }
}

async fn spawn_tls_listener(
    state: &Arc<RwLock<Config>>,
    tls: Option<Arc<TlsManager>>,
    max_header_bytes: usize,
    shutdown: CancellationToken,
    connections: Arc<ConnectionTracker>,
) -> Result<Option<JoinHandle<()>>> {
    let Some(tls) = tls else {
        return Ok(None);
    };
    let bind = state.read().await.tls.bind;
    let listener = TcpListener::bind(bind).await?;
    tracing::info!(address = %listener.local_addr()?, "listening for HTTPS connections");
    let state = Arc::clone(state);
    Ok(Some(tokio::spawn(async move {
        loop {
            let (stream, peer) = match tokio::select! {
                _ = shutdown.cancelled() => return,
                connection = listener.accept() => connection,
            } {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::error!(%error, "failed to accept HTTPS connection");
                    continue;
                }
            };
            let state = Arc::clone(&state);
            let acceptor = tls.acceptor();
            let connections = Arc::clone(&connections);
            tokio::spawn(async move {
                let _connection = connections.track();
                let stream = match acceptor.accept(stream).await {
                    Ok(stream) => stream,
                    Err(error) => {
                        tracing::debug!(%peer, %error, "TLS handshake failed");
                        return;
                    }
                };
                let service = service_fn(move |request| {
                    connection::handle(request, peer, Arc::clone(&state), None, true)
                });
                if let Err(error) = hyper::server::conn::http1::Builder::new()
                    .max_buf_size(max_header_bytes)
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades()
                    .await
                {
                    tracing::debug!(%peer, %error, "connection closed with HTTPS error");
                }
            });
        }
    })))
}

#[derive(Default)]
struct ConnectionTracker {
    active: AtomicUsize,
    idle: Notify,
}

impl ConnectionTracker {
    fn track(self: &Arc<Self>) -> ConnectionGuard {
        self.active.fetch_add(1, Ordering::AcqRel);
        ConnectionGuard(Arc::clone(self))
    }

    async fn wait_for_all(&self) {
        loop {
            let notified = self.idle.notified();
            if self.active.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }
}

struct ConnectionGuard(Arc<ConnectionTracker>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        if self.0.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}

#[cfg(unix)]
async fn reload_config(state: &Arc<RwLock<Config>>, path: &PathBuf) {
    match Config::load(path).and_then(|config| {
        config.validate()?;
        Ok(config)
    }) {
        Ok(config) => {
            let mut current = state.write().await;
            let tls_changed = current.tls.enabled != config.tls.enabled
                || current.tls.bind != config.tls.bind
                || current.tls.email != config.tls.email
                || current.tls.certificate_cache != config.tls.certificate_cache;
            let domains_changed = current
                .sites
                .iter()
                .map(|site| &site.host)
                .ne(config.sites.iter().map(|site| &site.host));
            if tls_changed || (current.tls.enabled && domains_changed) {
                tracing::warn!(
                    config = %path.display(),
                    "TLS settings or TLS site names changed; restart the service to apply them"
                );
                return;
            }
            *current = config;
            tracing::info!(config = %path.display(), "configuration reloaded");
        }
        Err(error) => {
            tracing::error!(%error, config = %path.display(), "configuration reload rejected")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tracker_waits_for_existing_connections() {
        let tracker = Arc::new(ConnectionTracker::default());
        let connection = tracker.track();
        let waiter = tokio::spawn({
            let tracker = Arc::clone(&tracker);
            async move { tracker.wait_for_all().await }
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        drop(connection);
        waiter.await.expect("graceful shutdown waiter completes");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reload_keeps_previous_config_when_the_new_one_is_invalid() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("webserver-reload-test-{unique}"));
        fs::create_dir_all(directory.join("sites")).expect("create sites directory");
        fs::create_dir_all(directory.join("public")).expect("create public directory");
        let path = directory.join("webserver.toml");
        let site = directory.join("sites/localhost.conf");
        fs::write(
            &path,
            "[server]\nbind = \"127.0.0.1:8080\"\nupstream_timeout_secs = 30\n",
        )
        .expect("write config");
        fs::write(&site, "host = \"localhost\"\n[[routes]]\npath_prefix = \"/\"\nkind = \"static\"\nroot = \"../public\"\n").expect("write site");
        let state = Arc::new(RwLock::new(Config::load(&path).expect("load config")));

        fs::write(
            &path,
            "[server]\nbind = \"127.0.0.1:8080\"\nmax_header_bytes = 1\n",
        )
        .expect("write invalid config");
        reload_config(&state, &path).await;
        assert_eq!(state.read().await.server.max_header_bytes, 32 * 1024);

        fs::write(
            &path,
            "[server]\nbind = \"127.0.0.1:8080\"\nupstream_timeout_secs = 9\n",
        )
        .expect("write valid config");
        reload_config(&state, &path).await;
        assert_eq!(state.read().await.server.upstream_timeout_secs, 9);
        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
