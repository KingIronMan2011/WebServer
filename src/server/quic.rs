//! HTTP/3 listener built on QUIC.  It deliberately shares the normal routing
//! pipeline so HTTP/3 has the same virtual hosts and route behaviour as TCP.

use std::sync::Arc;

use bytes::{Buf, BytesMut};
use http_body_util::{BodyExt, Full};
use hyper::Request;
use quinn::crypto::rustls::QuicServerConfig;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{config::Config, server::connection, tls::TlsManager};

pub fn spawn(
    bind: std::net::SocketAddr,
    tls: Arc<TlsManager>,
    state: Arc<RwLock<Config>>,
    shutdown: CancellationToken,
) -> std::result::Result<JoinHandle<()>, String> {
    let mut rustls = (*tls.server_config()).clone();
    // HTTP/3 uses its own ALPN value and must not advertise TCP protocols.
    rustls.alpn_protocols = vec![b"h3".to_vec()];
    let crypto = QuicServerConfig::try_from(rustls).map_err(|error| error.to_string())?;
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    #[cfg(unix)]
    let endpoint = {
        let socket = crate::server::listener::bind_udp(bind).map_err(|error| error.to_string())?;
        let runtime = quinn::default_runtime().ok_or("no QUIC runtime available")?;
        quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(server_config),
            socket,
            runtime,
        )
        .map_err(|error| error.to_string())?
    };
    #[cfg(not(unix))]
    let endpoint =
        quinn::Endpoint::server(server_config, bind).map_err(|error| error.to_string())?;
    let address = endpoint.local_addr().map_err(|error| error.to_string())?;
    tracing::info!(%address, "listening for HTTP/3 connections over QUIC");

    Ok(tokio::spawn(async move {
        loop {
            let incoming = tokio::select! {
                _ = shutdown.cancelled() => return,
                incoming = endpoint.accept() => incoming,
            };
            let Some(incoming) = incoming else { return };
            let state = Arc::clone(&state);
            tokio::spawn(async move {
                let connection = match incoming.await {
                    Ok(connection) => connection,
                    Err(error) => {
                        tracing::debug!(%error, "QUIC handshake failed");
                        return;
                    }
                };
                let peer = connection.remote_address();
                let mut h3 = match h3::server::builder()
                    .build(h3_quinn::Connection::new(connection))
                    .await
                {
                    Ok(connection) => connection,
                    Err(error) => {
                        tracing::debug!(%peer, %error, "HTTP/3 connection setup failed");
                        return;
                    }
                };
                loop {
                    let resolver = match h3.accept().await {
                        Ok(Some(resolver)) => resolver,
                        Ok(None) => return,
                        Err(error) => {
                            tracing::debug!(%peer, %error, "HTTP/3 connection closed");
                            return;
                        }
                    };
                    let state = Arc::clone(&state);
                    tokio::spawn(async move {
                        let (request, mut stream) = match resolver.resolve_request().await {
                            Ok(request) => request,
                            Err(error) => {
                                tracing::debug!(%error, "invalid HTTP/3 request");
                                return;
                            }
                        };
                        let mut body = BytesMut::new();
                        loop {
                            match stream.recv_data().await {
                                Ok(Some(mut chunk)) => {
                                    body.extend_from_slice(&chunk.copy_to_bytes(chunk.remaining()))
                                }
                                Ok(None) => break,
                                Err(error) => {
                                    tracing::debug!(%error, "HTTP/3 request body failed");
                                    return;
                                }
                            }
                        }
                        let (parts, _) = request.into_parts();
                        let request = Request::from_parts(parts, Full::new(body.freeze()));
                        let response = connection::handle(request, peer, state, None, true)
                            .await
                            .expect("HTTP request handler is infallible");
                        let (parts, mut body) = response.into_parts();
                        if let Err(error) = stream
                            .send_response(hyper::Response::from_parts(parts, ()))
                            .await
                        {
                            tracing::debug!(%error, "HTTP/3 response headers failed");
                            return;
                        }
                        while let Some(frame) = body.frame().await {
                            match frame {
                                Ok(frame) => {
                                    if let Ok(data) = frame.into_data()
                                        && stream.send_data(data).await.is_err()
                                    {
                                        return;
                                    }
                                }
                                Err(error) => {
                                    tracing::debug!(%error, "HTTP/3 response body failed");
                                    return;
                                }
                            }
                        }
                        let _ = stream.finish().await;
                    });
                }
            });
        }
    }))
}
