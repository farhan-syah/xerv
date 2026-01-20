//! Webhook trigger (HTTP endpoint).
//!
//! Listens for incoming HTTP requests and converts them to trigger events.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use parking_lot::RwLock;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::TcpListener;
use xerv_core::error::{Result, XervError};
use xerv_core::traits::{Trigger, TriggerConfig, TriggerEvent, TriggerFuture, TriggerType};
use xerv_core::types::RelPtr;

/// State for the webhook trigger.
struct WebhookState {
    /// Whether the trigger is running.
    running: AtomicBool,
    /// Whether the trigger is paused.
    paused: AtomicBool,
    /// Shutdown signal sender.
    shutdown_tx: RwLock<Option<tokio::sync::oneshot::Sender<()>>>,
}

/// HTTP webhook trigger.
///
/// Listens on a specified host:port for incoming HTTP requests.
/// Converts request body to a trigger event.
///
/// # Configuration
///
/// ```yaml
/// triggers:
///   - id: webhook_orders
///     type: trigger::webhook
///     params:
///       host: "0.0.0.0"
///       port: 8080
///       path: "/orders"
///       method: "POST"
/// ```
///
/// # Parameters
///
/// - `host` - Host to bind to (default: "0.0.0.0")
/// - `port` - Port to listen on (default: 8080)
/// - `path` - URL path to match (default: "/")
/// - `method` - HTTP method to accept (default: "POST")
pub struct WebhookTrigger {
    /// Trigger ID.
    id: String,
    /// Host to bind to.
    host: String,
    /// Port to listen on.
    port: u16,
    /// URL path to match.
    path: String,
    /// HTTP method to accept.
    method: Method,
    /// Internal state.
    state: Arc<WebhookState>,
}

impl WebhookTrigger {
    /// Create a new webhook trigger.
    pub fn new(id: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self {
            id: id.into(),
            host: host.into(),
            port,
            path: "/".to_string(),
            method: Method::POST,
            state: Arc::new(WebhookState {
                running: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                shutdown_tx: RwLock::new(None),
            }),
        }
    }

    /// Create from configuration.
    pub fn from_config(config: &TriggerConfig) -> Result<Self> {
        let host = config.get_string("host").unwrap_or("0.0.0.0").to_string();
        let port = config.get_i64("port").unwrap_or(8080) as u16;
        let path = config.get_string("path").unwrap_or("/").to_string();
        let method_str = config.get_string("method").unwrap_or("POST");

        let method = match method_str.to_uppercase().as_str() {
            "GET" => Method::GET,
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            "DELETE" => Method::DELETE,
            "PATCH" => Method::PATCH,
            _ => {
                return Err(XervError::ConfigValue {
                    field: "method".to_string(),
                    cause: format!("Invalid HTTP method: {}", method_str),
                });
            }
        };

        Ok(Self {
            id: config.id.clone(),
            host,
            port,
            path,
            method,
            state: Arc::new(WebhookState {
                running: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                shutdown_tx: RwLock::new(None),
            }),
        })
    }

    /// Set the URL path.
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    /// Set the HTTP method.
    pub fn with_method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }
}

impl Trigger for WebhookTrigger {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Webhook
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn start<'a>(
        &'a self,
        callback: Box<dyn Fn(TriggerEvent) + Send + Sync + 'static>,
    ) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let host = self.host.clone();
        let port = self.port;
        let path = self.path.clone();
        let method = self.method.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            if state.running.load(Ordering::SeqCst) {
                return Err(XervError::ConfigValue {
                    field: "trigger".to_string(),
                    cause: "Trigger is already running".to_string(),
                });
            }

            let addr: SocketAddr =
                format!("{}:{}", host, port)
                    .parse()
                    .map_err(|e| XervError::ConfigValue {
                        field: "host/port".to_string(),
                        cause: format!("Invalid address: {}", e),
                    })?;

            let listener = TcpListener::bind(addr)
                .await
                .map_err(|e| XervError::Network {
                    cause: format!("Failed to bind to {}: {}", addr, e),
                })?;

            tracing::info!(trigger_id = %trigger_id, addr = %addr, "Webhook trigger started");
            state.running.store(true, Ordering::SeqCst);

            let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
            *state.shutdown_tx.write() = Some(shutdown_tx);

            let callback = Arc::new(callback);

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        tracing::info!(trigger_id = %trigger_id, "Webhook trigger shutting down");
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok((stream, remote_addr)) => {
                                if state.paused.load(Ordering::SeqCst) {
                                    tracing::debug!(trigger_id = %trigger_id, "Trigger paused, ignoring request");
                                    continue;
                                }

                                let callback = callback.clone();
                                let trigger_id = trigger_id.clone();
                                let path = path.clone();
                                let method = method.clone();
                                let state = state.clone();

                                tokio::spawn(async move {
                                    let io = TokioIo::new(stream);

                                    let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                                        let callback = callback.clone();
                                        let trigger_id = trigger_id.clone();
                                        let path = path.clone();
                                        let method = method.clone();
                                        let state = state.clone();

                                        async move {
                                            // Check if paused
                                            if state.paused.load(Ordering::SeqCst) {
                                                return Ok::<_, hyper::Error>(Response::builder()
                                                    .status(StatusCode::SERVICE_UNAVAILABLE)
                                                    .body(Full::new(Bytes::from("Trigger paused")))
                                                    .unwrap());
                                            }

                                            // Check path
                                            if req.uri().path() != path {
                                                return Ok(Response::builder()
                                                    .status(StatusCode::NOT_FOUND)
                                                    .body(Full::new(Bytes::from("Not found")))
                                                    .unwrap());
                                            }

                                            // Check method
                                            if req.method() != method {
                                                return Ok(Response::builder()
                                                    .status(StatusCode::METHOD_NOT_ALLOWED)
                                                    .body(Full::new(Bytes::from("Method not allowed")))
                                                    .unwrap());
                                            }

                                            // Read body
                                            let body = match req.collect().await {
                                                Ok(collected) => collected.to_bytes(),
                                                Err(e) => {
                                                    tracing::error!(error = %e, "Failed to read request body");
                                                    return Ok(Response::builder()
                                                        .status(StatusCode::BAD_REQUEST)
                                                        .body(Full::new(Bytes::from("Failed to read body")))
                                                        .unwrap());
                                                }
                                            };

                                            // Create event with null pointer (data will be written by pipeline)
                                            let event = TriggerEvent::new(&trigger_id, RelPtr::null())
                                                .with_metadata(format!("body_size={}", body.len()));

                                            tracing::debug!(
                                                trigger_id = %trigger_id,
                                                trace_id = %event.trace_id,
                                                body_size = body.len(),
                                                "Webhook received request"
                                            );

                                            // Call the callback
                                            callback(event);

                                            Ok(Response::builder()
                                                .status(StatusCode::ACCEPTED)
                                                .body(Full::new(Bytes::from("Event accepted")))
                                                .unwrap())
                                        }
                                    });

                                    if let Err(e) = http1::Builder::new()
                                        .serve_connection(io, service)
                                        .await
                                    {
                                        tracing::error!(
                                            error = %e,
                                            remote_addr = %remote_addr,
                                            "HTTP connection error"
                                        );
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Failed to accept connection");
                            }
                        }
                    }
                }
            }

            state.running.store(false, Ordering::SeqCst);
            Ok(())
        })
    }

    fn stop<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            if let Some(tx) = state.shutdown_tx.write().take() {
                let _ = tx.send(());
                tracing::info!(trigger_id = %trigger_id, "Webhook trigger stopped");
            }
            state.running.store(false, Ordering::SeqCst);
            Ok(())
        })
    }

    fn pause<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(true, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Webhook trigger paused");
            Ok(())
        })
    }

    fn resume<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(false, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Webhook trigger resumed");
            Ok(())
        })
    }

    fn is_running(&self) -> bool {
        self.state.running.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_trigger_creation() {
        let trigger = WebhookTrigger::new("test_webhook", "127.0.0.1", 8080);
        assert_eq!(trigger.id(), "test_webhook");
        assert_eq!(trigger.trigger_type(), TriggerType::Webhook);
        assert!(!trigger.is_running());
    }

    #[test]
    fn webhook_trigger_from_config() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("host".to_string()),
            serde_yaml::Value::String("localhost".to_string()),
        );
        params.insert(
            serde_yaml::Value::String("port".to_string()),
            serde_yaml::Value::Number(9090.into()),
        );
        params.insert(
            serde_yaml::Value::String("path".to_string()),
            serde_yaml::Value::String("/api/events".to_string()),
        );
        params.insert(
            serde_yaml::Value::String("method".to_string()),
            serde_yaml::Value::String("POST".to_string()),
        );

        let config = TriggerConfig::new("webhook_test", TriggerType::Webhook)
            .with_params(serde_yaml::Value::Mapping(params));

        let trigger = WebhookTrigger::from_config(&config).unwrap();
        assert_eq!(trigger.id(), "webhook_test");
        assert_eq!(trigger.host, "localhost");
        assert_eq!(trigger.port, 9090);
        assert_eq!(trigger.path, "/api/events");
        assert_eq!(trigger.method, Method::POST);
    }

    #[test]
    fn webhook_trigger_builder() {
        let trigger = WebhookTrigger::new("builder_test", "0.0.0.0", 8080)
            .with_path("/webhook")
            .with_method(Method::PUT);

        assert_eq!(trigger.path, "/webhook");
        assert_eq!(trigger.method, Method::PUT);
    }
}
