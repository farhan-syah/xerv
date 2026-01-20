//! HTTP server setup and connection handling.

use super::router;
use super::state::AppState;
use crate::listener::ListenerPool;
use crate::pipeline::PipelineController;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use xerv_core::error::Result;

/// Configuration for the API server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Host to bind to.
    pub host: String,
    /// Port to listen on.
    pub port: u16,
}

impl ServerConfig {
    /// Create a new server configuration.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    /// Get the socket address.
    pub fn socket_addr(&self) -> SocketAddr {
        let host: std::net::IpAddr = self.host.parse().unwrap_or([0, 0, 0, 0].into());
        SocketAddr::new(host, self.port)
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
        }
    }
}

/// HTTP API server for XERV.
pub struct ApiServer {
    /// Server configuration.
    config: ServerConfig,
    /// Shared application state.
    state: Arc<AppState>,
    /// Shutdown signal sender.
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl ApiServer {
    /// Create a new API server.
    pub fn new(
        config: ServerConfig,
        controller: Arc<PipelineController>,
        listener_pool: Arc<ListenerPool>,
    ) -> Self {
        let state = Arc::new(AppState::new(controller, listener_pool));

        Self {
            config,
            state,
            shutdown_tx: None,
        }
    }

    /// Get a reference to the application state.
    pub fn state(&self) -> Arc<AppState> {
        Arc::clone(&self.state)
    }

    /// Run the server until shutdown signal is received.
    pub async fn run(&mut self) -> Result<()> {
        let addr = self.config.socket_addr();
        let listener =
            TcpListener::bind(addr)
                .await
                .map_err(|e| xerv_core::error::XervError::Io {
                    path: std::path::PathBuf::from(format!(
                        "{}:{}",
                        self.config.host, self.config.port
                    )),
                    cause: e.to_string(),
                })?;

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        tracing::info!(
            host = %self.config.host,
            port = %self.config.port,
            "API server started"
        );

        loop {
            tokio::select! {
                result = listener.accept() => {
                    let (stream, remote_addr) = result.map_err(|e| {
                        xerv_core::error::XervError::Network {
                            cause: e.to_string(),
                        }
                    })?;

                    let io = TokioIo::new(stream);
                    let state = Arc::clone(&self.state);

                    tokio::spawn(async move {
                        let service = service_fn(move |req| {
                            let state = Arc::clone(&state);
                            async move { router::route(req, state).await }
                        });

                        if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                            if !e.is_incomplete_message() {
                                tracing::warn!(
                                    remote = %remote_addr,
                                    error = %e,
                                    "HTTP connection error"
                                );
                            }
                        }
                    });
                }
                _ = &mut shutdown_rx => {
                    tracing::info!("API server shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Shutdown the server.
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
    }

    #[test]
    fn server_config_socket_addr() {
        let config = ServerConfig::new("127.0.0.1", 9000);
        let addr = config.socket_addr();

        assert_eq!(addr.port(), 9000);
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
    }
}
