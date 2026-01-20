//! Queue trigger (message queue).
//!
//! In-memory message queue trigger for testing and lightweight use cases.
//! For production, consider using Kafka or a dedicated message broker.

use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use xerv_core::error::{Result, XervError};
use xerv_core::traits::{Trigger, TriggerConfig, TriggerEvent, TriggerFuture, TriggerType};
use xerv_core::types::RelPtr;

/// Message in the queue.
#[derive(Debug, Clone)]
pub struct QueueMessage {
    /// Message payload.
    pub payload: Vec<u8>,
    /// Optional message key.
    pub key: Option<String>,
    /// Optional headers.
    pub headers: Vec<(String, String)>,
}

impl QueueMessage {
    /// Create a new message with payload.
    pub fn new(payload: impl Into<Vec<u8>>) -> Self {
        Self {
            payload: payload.into(),
            key: None,
            headers: Vec::new(),
        }
    }

    /// Create a message from a string.
    pub fn from_string(s: impl Into<String>) -> Self {
        Self::new(s.into().into_bytes())
    }

    /// Create a message from JSON.
    pub fn from_json<T: serde::Serialize>(value: &T) -> Result<Self> {
        let bytes = serde_json::to_vec(value).map_err(|e| XervError::ConfigValue {
            field: "payload".to_string(),
            cause: format!("Failed to serialize JSON: {}", e),
        })?;
        Ok(Self::new(bytes))
    }

    /// Set the message key.
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Add a header.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }
}

/// State for the queue trigger.
struct QueueState {
    /// Whether the trigger is running.
    running: AtomicBool,
    /// Whether the trigger is paused.
    paused: AtomicBool,
    /// Shutdown signal sender.
    shutdown_tx: RwLock<Option<tokio::sync::oneshot::Sender<()>>>,
    /// Message sender for pushing messages to the queue.
    message_tx: RwLock<Option<mpsc::Sender<QueueMessage>>>,
}

/// Queue handle for sending messages.
#[derive(Clone)]
pub struct QueueHandle {
    tx: mpsc::Sender<QueueMessage>,
}

impl QueueHandle {
    /// Send a message to the queue.
    pub async fn send(&self, message: QueueMessage) -> Result<()> {
        self.tx.send(message).await.map_err(|e| XervError::Network {
            cause: format!("Failed to send message: {}", e),
        })
    }

    /// Send a string message.
    pub async fn send_string(&self, s: impl Into<String>) -> Result<()> {
        self.send(QueueMessage::from_string(s)).await
    }

    /// Send a JSON message.
    pub async fn send_json<T: serde::Serialize>(&self, value: &T) -> Result<()> {
        self.send(QueueMessage::from_json(value)?).await
    }
}

/// In-memory queue trigger.
///
/// Provides an in-memory message queue for testing and lightweight scenarios.
/// Messages can be pushed via the `QueueHandle`.
///
/// # Configuration
///
/// ```yaml
/// triggers:
///   - id: event_queue
///     type: trigger::queue
///     params:
///       buffer_size: 1000
/// ```
///
/// # Parameters
///
/// - `buffer_size` - Maximum messages to buffer (default: 100)
pub struct QueueTrigger {
    /// Trigger ID.
    id: String,
    /// Buffer size for the queue.
    buffer_size: usize,
    /// Internal state.
    state: Arc<QueueState>,
}

impl QueueTrigger {
    /// Create a new queue trigger.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            buffer_size: 100,
            state: Arc::new(QueueState {
                running: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                shutdown_tx: RwLock::new(None),
                message_tx: RwLock::new(None),
            }),
        }
    }

    /// Create from configuration.
    pub fn from_config(config: &TriggerConfig) -> Result<Self> {
        let buffer_size = config.get_i64("buffer_size").unwrap_or(100) as usize;

        Ok(Self {
            id: config.id.clone(),
            buffer_size,
            state: Arc::new(QueueState {
                running: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                shutdown_tx: RwLock::new(None),
                message_tx: RwLock::new(None),
            }),
        })
    }

    /// Set the buffer size.
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Get a handle to send messages to this queue.
    ///
    /// Returns `None` if the trigger hasn't been started yet.
    pub fn handle(&self) -> Option<QueueHandle> {
        self.state
            .message_tx
            .read()
            .as_ref()
            .map(|tx| QueueHandle { tx: tx.clone() })
    }
}

impl Trigger for QueueTrigger {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Queue
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn start<'a>(
        &'a self,
        callback: Box<dyn Fn(TriggerEvent) + Send + Sync + 'static>,
    ) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let buffer_size = self.buffer_size;
        let trigger_id = self.id.clone();

        Box::pin(async move {
            if state.running.load(Ordering::SeqCst) {
                return Err(XervError::ConfigValue {
                    field: "trigger".to_string(),
                    cause: "Trigger is already running".to_string(),
                });
            }

            tracing::info!(
                trigger_id = %trigger_id,
                buffer_size = buffer_size,
                "Queue trigger started"
            );

            state.running.store(true, Ordering::SeqCst);

            let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
            *state.shutdown_tx.write() = Some(shutdown_tx);

            // Create message channel
            let (msg_tx, mut msg_rx) = mpsc::channel(buffer_size);
            *state.message_tx.write() = Some(msg_tx);

            let callback = Arc::new(callback);

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        tracing::info!(trigger_id = %trigger_id, "Queue trigger shutting down");
                        break;
                    }
                    Some(message) = msg_rx.recv() => {
                        if state.paused.load(Ordering::SeqCst) {
                            tracing::debug!(trigger_id = %trigger_id, "Trigger paused, dropping message");
                            continue;
                        }

                        let metadata = format!(
                            "payload_size={},key={}",
                            message.payload.len(),
                            message.key.as_deref().unwrap_or("none")
                        );

                        // Create trigger event
                        let event = TriggerEvent::new(&trigger_id, RelPtr::null())
                            .with_metadata(metadata);

                        tracing::debug!(
                            trigger_id = %trigger_id,
                            trace_id = %event.trace_id,
                            payload_size = message.payload.len(),
                            key = ?message.key,
                            "Queue message received"
                        );

                        callback(event);
                    }
                }
            }

            // Clean up
            *state.message_tx.write() = None;
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
                tracing::info!(trigger_id = %trigger_id, "Queue trigger stopped");
            }
            *state.message_tx.write() = None;
            state.running.store(false, Ordering::SeqCst);
            Ok(())
        })
    }

    fn pause<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(true, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Queue trigger paused");
            Ok(())
        })
    }

    fn resume<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(false, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Queue trigger resumed");
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
    fn queue_trigger_creation() {
        let trigger = QueueTrigger::new("test_queue");
        assert_eq!(trigger.id(), "test_queue");
        assert_eq!(trigger.trigger_type(), TriggerType::Queue);
        assert!(!trigger.is_running());
    }

    #[test]
    fn queue_trigger_from_config() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("buffer_size".to_string()),
            serde_yaml::Value::Number(500.into()),
        );

        let config = TriggerConfig::new("queue_test", TriggerType::Queue)
            .with_params(serde_yaml::Value::Mapping(params));

        let trigger = QueueTrigger::from_config(&config).unwrap();
        assert_eq!(trigger.id(), "queue_test");
        assert_eq!(trigger.buffer_size, 500);
    }

    #[test]
    fn queue_message_creation() {
        let msg = QueueMessage::new(vec![1, 2, 3])
            .with_key("test-key")
            .with_header("content-type", "application/json");

        assert_eq!(msg.payload, vec![1, 2, 3]);
        assert_eq!(msg.key, Some("test-key".to_string()));
        assert_eq!(msg.headers.len(), 1);
    }

    #[test]
    fn queue_message_from_string() {
        let msg = QueueMessage::from_string("hello world");
        assert_eq!(msg.payload, b"hello world");
    }

    #[test]
    fn queue_message_from_json() {
        let data = serde_json::json!({"key": "value"});
        let msg = QueueMessage::from_json(&data).unwrap();
        assert!(!msg.payload.is_empty());
    }

    #[test]
    fn queue_trigger_no_handle_before_start() {
        let trigger = QueueTrigger::new("test");
        assert!(trigger.handle().is_none());
    }
}
