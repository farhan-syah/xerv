//! Manual trigger (testing).
//!
//! Allows explicit triggering via API calls.
//! Useful for testing pipelines and debugging.

use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use xerv_core::error::{Result, XervError};
use xerv_core::traits::{Trigger, TriggerConfig, TriggerEvent, TriggerFuture, TriggerType};
use xerv_core::types::RelPtr;
use xerv_core::value::Value;

/// State for the manual trigger.
struct ManualState {
    /// Whether the trigger is running.
    running: AtomicBool,
    /// Whether the trigger is paused.
    paused: AtomicBool,
    /// Shutdown signal sender.
    shutdown_tx: RwLock<Option<tokio::sync::oneshot::Sender<()>>>,
    /// Event fire channel.
    fire_tx: RwLock<Option<mpsc::Sender<ManualEvent>>>,
}

/// A manually fired event.
#[derive(Debug, Clone)]
pub struct ManualEvent {
    /// Event data as Value.
    pub data: Value,
    /// Optional metadata.
    pub metadata: Option<String>,
}

impl ManualEvent {
    /// Create an empty event.
    pub fn empty() -> Self {
        Self {
            data: Value::null(),
            metadata: None,
        }
    }

    /// Create an event with data.
    pub fn with_data(data: Value) -> Self {
        Self {
            data,
            metadata: None,
        }
    }

    /// Create an event from JSON.
    pub fn from_json(json: serde_json::Value) -> Self {
        Self {
            data: Value::from(json),
            metadata: None,
        }
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: impl Into<String>) -> Self {
        self.metadata = Some(metadata.into());
        self
    }
}

/// Handle for firing manual events.
#[derive(Clone)]
pub struct ManualFireHandle {
    tx: mpsc::Sender<ManualEvent>,
}

impl ManualFireHandle {
    /// Fire an event.
    pub async fn fire(&self, event: ManualEvent) -> Result<()> {
        self.tx.send(event).await.map_err(|e| XervError::Network {
            cause: format!("Failed to fire event: {}", e),
        })
    }

    /// Fire an empty event.
    pub async fn fire_empty(&self) -> Result<()> {
        self.fire(ManualEvent::empty()).await
    }

    /// Fire an event with JSON data.
    pub async fn fire_json(&self, json: serde_json::Value) -> Result<()> {
        self.fire(ManualEvent::from_json(json)).await
    }

    /// Fire an event with data.
    pub async fn fire_data(&self, data: Value) -> Result<()> {
        self.fire(ManualEvent::with_data(data)).await
    }
}

/// Manual trigger for testing.
///
/// Provides programmatic control over event firing for testing pipelines.
///
/// # Configuration
///
/// ```yaml
/// triggers:
///   - id: test_trigger
///     type: trigger::manual
///     params:
///       buffer_size: 10
/// ```
///
/// # Parameters
///
/// - `buffer_size` - Maximum events to buffer (default: 10)
///
/// # Usage
///
/// ```ignore
/// let trigger = ManualTrigger::new("test");
/// // Start the trigger...
/// let handle = trigger.fire_handle().unwrap();
///
/// // Fire events manually
/// handle.fire_empty().await?;
/// handle.fire_json(json!({"key": "value"})).await?;
/// ```
pub struct ManualTrigger {
    /// Trigger ID.
    id: String,
    /// Buffer size.
    buffer_size: usize,
    /// Internal state.
    state: Arc<ManualState>,
}

impl ManualTrigger {
    /// Create a new manual trigger.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            buffer_size: 10,
            state: Arc::new(ManualState {
                running: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                shutdown_tx: RwLock::new(None),
                fire_tx: RwLock::new(None),
            }),
        }
    }

    /// Create from configuration.
    pub fn from_config(config: &TriggerConfig) -> Result<Self> {
        let buffer_size = config.get_i64("buffer_size").unwrap_or(10) as usize;

        Ok(Self {
            id: config.id.clone(),
            buffer_size,
            state: Arc::new(ManualState {
                running: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                shutdown_tx: RwLock::new(None),
                fire_tx: RwLock::new(None),
            }),
        })
    }

    /// Set the buffer size.
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Get a handle to fire events.
    ///
    /// Returns `None` if the trigger hasn't been started.
    pub fn fire_handle(&self) -> Option<ManualFireHandle> {
        self.state
            .fire_tx
            .read()
            .as_ref()
            .map(|tx| ManualFireHandle { tx: tx.clone() })
    }
}

impl Trigger for ManualTrigger {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Manual
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
                "Manual trigger started"
            );

            state.running.store(true, Ordering::SeqCst);

            let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
            *state.shutdown_tx.write() = Some(shutdown_tx);

            // Create fire channel
            let (fire_tx, mut fire_rx) = mpsc::channel(buffer_size);
            *state.fire_tx.write() = Some(fire_tx);

            let callback = Arc::new(callback);

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        tracing::info!(trigger_id = %trigger_id, "Manual trigger shutting down");
                        break;
                    }
                    Some(manual_event) = fire_rx.recv() => {
                        if state.paused.load(Ordering::SeqCst) {
                            tracing::debug!(trigger_id = %trigger_id, "Trigger paused, dropping event");
                            continue;
                        }

                        // Create trigger event
                        let mut event = TriggerEvent::new(&trigger_id, RelPtr::null());

                        if let Some(metadata) = manual_event.metadata {
                            event = event.with_metadata(metadata);
                        }

                        tracing::debug!(
                            trigger_id = %trigger_id,
                            trace_id = %event.trace_id,
                            "Manual event fired"
                        );

                        callback(event);
                    }
                }
            }

            // Clean up
            *state.fire_tx.write() = None;
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
                tracing::info!(trigger_id = %trigger_id, "Manual trigger stopped");
            }
            *state.fire_tx.write() = None;
            state.running.store(false, Ordering::SeqCst);
            Ok(())
        })
    }

    fn pause<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(true, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Manual trigger paused");
            Ok(())
        })
    }

    fn resume<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(false, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Manual trigger resumed");
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
    fn manual_trigger_creation() {
        let trigger = ManualTrigger::new("test_manual");
        assert_eq!(trigger.id(), "test_manual");
        assert_eq!(trigger.trigger_type(), TriggerType::Manual);
        assert!(!trigger.is_running());
    }

    #[test]
    fn manual_trigger_from_config() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("buffer_size".to_string()),
            serde_yaml::Value::Number(50.into()),
        );

        let config = TriggerConfig::new("manual_test", TriggerType::Manual)
            .with_params(serde_yaml::Value::Mapping(params));

        let trigger = ManualTrigger::from_config(&config).unwrap();
        assert_eq!(trigger.id(), "manual_test");
        assert_eq!(trigger.buffer_size, 50);
    }

    #[test]
    fn manual_event_creation() {
        let event = ManualEvent::empty();
        assert!(event.metadata.is_none());

        let event = ManualEvent::from_json(serde_json::json!({"key": "value"}))
            .with_metadata("test metadata");
        assert_eq!(event.metadata, Some("test metadata".to_string()));
    }

    #[test]
    fn manual_trigger_no_handle_before_start() {
        let trigger = ManualTrigger::new("test");
        assert!(trigger.fire_handle().is_none());
    }
}
