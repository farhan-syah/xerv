//! Memory trigger (benchmarking).
//!
//! Direct memory injection trigger for performance testing and benchmarks.
//! Events are injected directly without any I/O overhead.

use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::mpsc;
use xerv_core::error::{Result, XervError};
use xerv_core::traits::{Trigger, TriggerConfig, TriggerEvent, TriggerFuture, TriggerType};
use xerv_core::types::RelPtr;

/// State for the memory trigger.
struct MemoryState {
    /// Whether the trigger is running.
    running: AtomicBool,
    /// Whether the trigger is paused.
    paused: AtomicBool,
    /// Shutdown signal sender.
    shutdown_tx: RwLock<Option<tokio::sync::oneshot::Sender<()>>>,
    /// Event injection channel.
    inject_tx: RwLock<Option<mpsc::Sender<RelPtr<()>>>>,
    /// Event counter for statistics.
    event_count: AtomicU64,
}

/// Handle for injecting events into a memory trigger.
#[derive(Clone)]
pub struct MemoryInjector {
    tx: mpsc::Sender<RelPtr<()>>,
}

impl MemoryInjector {
    /// Inject an event with the given data pointer.
    pub async fn inject(&self, data: RelPtr<()>) -> Result<()> {
        self.tx.send(data).await.map_err(|e| XervError::Network {
            cause: format!("Failed to inject event: {}", e),
        })
    }

    /// Inject an event with a null data pointer.
    pub async fn inject_empty(&self) -> Result<()> {
        self.inject(RelPtr::null()).await
    }

    /// Inject multiple events.
    pub async fn inject_batch(&self, count: usize) -> Result<()> {
        for _ in 0..count {
            self.inject_empty().await?;
        }
        Ok(())
    }
}

/// Memory trigger for benchmarking.
///
/// Provides zero-overhead event injection for performance testing.
/// Events are injected directly via `MemoryInjector`.
///
/// # Configuration
///
/// ```yaml
/// triggers:
///   - id: bench_trigger
///     type: trigger::memory
///     params:
///       buffer_size: 10000
/// ```
///
/// # Parameters
///
/// - `buffer_size` - Maximum events to buffer (default: 1000)
///
/// # Usage
///
/// ```ignore
/// let trigger = MemoryTrigger::new("bench");
/// // Start the trigger...
/// let injector = trigger.injector().unwrap();
///
/// // Inject events for benchmarking
/// for _ in 0..1000 {
///     injector.inject_empty().await?;
/// }
/// ```
pub struct MemoryTrigger {
    /// Trigger ID.
    id: String,
    /// Buffer size.
    buffer_size: usize,
    /// Internal state.
    state: Arc<MemoryState>,
}

impl MemoryTrigger {
    /// Create a new memory trigger.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            buffer_size: 1000,
            state: Arc::new(MemoryState {
                running: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                shutdown_tx: RwLock::new(None),
                inject_tx: RwLock::new(None),
                event_count: AtomicU64::new(0),
            }),
        }
    }

    /// Create from configuration.
    pub fn from_config(config: &TriggerConfig) -> Result<Self> {
        let buffer_size = config.get_i64("buffer_size").unwrap_or(1000) as usize;

        Ok(Self {
            id: config.id.clone(),
            buffer_size,
            state: Arc::new(MemoryState {
                running: AtomicBool::new(false),
                paused: AtomicBool::new(false),
                shutdown_tx: RwLock::new(None),
                inject_tx: RwLock::new(None),
                event_count: AtomicU64::new(0),
            }),
        })
    }

    /// Set the buffer size.
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Get an injector for sending events.
    ///
    /// Returns `None` if the trigger hasn't been started.
    pub fn injector(&self) -> Option<MemoryInjector> {
        self.state
            .inject_tx
            .read()
            .as_ref()
            .map(|tx| MemoryInjector { tx: tx.clone() })
    }

    /// Get the number of events processed.
    pub fn event_count(&self) -> u64 {
        self.state.event_count.load(Ordering::SeqCst)
    }

    /// Reset the event counter.
    pub fn reset_count(&self) {
        self.state.event_count.store(0, Ordering::SeqCst);
    }
}

impl Trigger for MemoryTrigger {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Memory
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
                "Memory trigger started"
            );

            state.running.store(true, Ordering::SeqCst);
            state.event_count.store(0, Ordering::SeqCst);

            let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
            *state.shutdown_tx.write() = Some(shutdown_tx);

            // Create injection channel
            let (inject_tx, mut inject_rx) = mpsc::channel(buffer_size);
            *state.inject_tx.write() = Some(inject_tx);

            let callback = Arc::new(callback);

            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        tracing::info!(
                            trigger_id = %trigger_id,
                            total_events = state.event_count.load(Ordering::SeqCst),
                            "Memory trigger shutting down"
                        );
                        break;
                    }
                    Some(data) = inject_rx.recv() => {
                        if state.paused.load(Ordering::SeqCst) {
                            continue;
                        }

                        // Increment counter
                        let count = state.event_count.fetch_add(1, Ordering::SeqCst) + 1;

                        // Create event
                        let event = TriggerEvent::new(&trigger_id, data)
                            .with_metadata(format!("event_number={}", count));

                        callback(event);
                    }
                }
            }

            // Clean up
            *state.inject_tx.write() = None;
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
                tracing::info!(trigger_id = %trigger_id, "Memory trigger stopped");
            }
            *state.inject_tx.write() = None;
            state.running.store(false, Ordering::SeqCst);
            Ok(())
        })
    }

    fn pause<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(true, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Memory trigger paused");
            Ok(())
        })
    }

    fn resume<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = self.state.clone();
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(false, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Memory trigger resumed");
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
    fn memory_trigger_creation() {
        let trigger = MemoryTrigger::new("bench_trigger");
        assert_eq!(trigger.id(), "bench_trigger");
        assert_eq!(trigger.trigger_type(), TriggerType::Memory);
        assert!(!trigger.is_running());
        assert_eq!(trigger.event_count(), 0);
    }

    #[test]
    fn memory_trigger_from_config() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("buffer_size".to_string()),
            serde_yaml::Value::Number(5000.into()),
        );

        let config = TriggerConfig::new("mem_test", TriggerType::Memory)
            .with_params(serde_yaml::Value::Mapping(params));

        let trigger = MemoryTrigger::from_config(&config).unwrap();
        assert_eq!(trigger.id(), "mem_test");
        assert_eq!(trigger.buffer_size, 5000);
    }

    #[test]
    fn memory_trigger_no_injector_before_start() {
        let trigger = MemoryTrigger::new("test");
        assert!(trigger.injector().is_none());
    }

    #[test]
    fn memory_trigger_reset_count() {
        let trigger = MemoryTrigger::new("test");
        trigger.state.event_count.store(100, Ordering::SeqCst);
        assert_eq!(trigger.event_count(), 100);
        trigger.reset_count();
        assert_eq!(trigger.event_count(), 0);
    }
}
