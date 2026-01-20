#![allow(clippy::module_inception)]
#![allow(clippy::await_holding_lock)]

//! Multi-head trigger support via ListenerPool.
//!
//! The `ListenerPool` enables multiple pipelines to share a single trigger instance.
//! When a trigger fires, the pool broadcasts the event to all registered listeners,
//! generating a unique trace ID for each.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                       ListenerPool                               │
//! │  ┌─────────────┐                                                │
//! │  │   Trigger   │─────┬──► Listener 1 (Pipeline A, trace_id_1)   │
//! │  │  (Webhook/  │     ├──► Listener 2 (Pipeline B, trace_id_2)   │
//! │  │   Kafka)    │     └──► Listener 3 (Pipeline C, trace_id_3)   │
//! │  └─────────────┘                                                │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! let pool = ListenerPool::new();
//!
//! // Register a trigger
//! pool.register_trigger(webhook_trigger)?;
//!
//! // Subscribe multiple pipelines to the same trigger
//! let listener1 = Listener::new(pipeline_a_id, |event| { /* handle */ });
//! let listener2 = Listener::new(pipeline_b_id, |event| { /* handle */ });
//!
//! pool.subscribe("webhook_8080", listener1)?;
//! pool.subscribe("webhook_8080", listener2)?;
//!
//! // Start all triggers
//! pool.start().await?;
//! ```

mod listener;

pub use listener::{Listener, ListenerId};

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use xerv_core::error::{Result, XervError};
use xerv_core::traits::{Trigger, TriggerEvent};
use xerv_core::types::TraceId;

/// Pool of triggers with multi-head listener support.
///
/// Manages trigger lifecycles and broadcasts events to multiple listeners.
pub struct ListenerPool {
    /// Active triggers keyed by trigger ID.
    triggers: RwLock<HashMap<String, Arc<dyn Trigger>>>,
    /// Listeners keyed by trigger ID, stored as Arc for sharing.
    listeners: Arc<RwLock<HashMap<String, Vec<ListenerEntry>>>>,
    /// Running state.
    running: RwLock<bool>,
}

/// Internal listener entry that can be cloned for broadcasting.
struct ListenerEntry {
    id: ListenerId,
    callback: Arc<dyn Fn(TriggerEvent) + Send + Sync>,
}

impl ListenerPool {
    /// Create a new empty listener pool.
    pub fn new() -> Self {
        Self {
            triggers: RwLock::new(HashMap::new()),
            listeners: Arc::new(RwLock::new(HashMap::new())),
            running: RwLock::new(false),
        }
    }

    /// Register a trigger with the pool.
    ///
    /// If a trigger with the same ID already exists, this returns an error.
    /// Use `get_trigger` to check if a trigger exists before registering.
    pub fn register_trigger(&self, trigger: Arc<dyn Trigger>) -> Result<()> {
        let trigger_id = trigger.id().to_string();

        let mut triggers = self.triggers.write();
        if triggers.contains_key(&trigger_id) {
            return Err(XervError::ConfigValue {
                field: format!("trigger:{}", trigger_id),
                cause: "Trigger already registered".to_string(),
            });
        }

        triggers.insert(trigger_id.clone(), trigger);

        // Initialize empty listener list
        let mut listeners = self.listeners.write();
        listeners.entry(trigger_id).or_default();

        Ok(())
    }

    /// Get a trigger by ID.
    pub fn get_trigger(&self, trigger_id: &str) -> Option<Arc<dyn Trigger>> {
        self.triggers.read().get(trigger_id).cloned()
    }

    /// Subscribe a listener to a trigger.
    ///
    /// Returns the listener ID for later unsubscription.
    pub fn subscribe(&self, trigger_id: &str, listener: Listener) -> Result<ListenerId> {
        let triggers = self.triggers.read();
        if !triggers.contains_key(trigger_id) {
            return Err(XervError::ConfigValue {
                field: format!("trigger:{}", trigger_id),
                cause: "Trigger not found".to_string(),
            });
        }
        drop(triggers);

        let listener_id = listener.id;
        let entry = ListenerEntry {
            id: listener_id,
            callback: listener.into_callback(),
        };

        let mut listeners = self.listeners.write();
        listeners
            .entry(trigger_id.to_string())
            .or_default()
            .push(entry);

        tracing::debug!(
            trigger_id = %trigger_id,
            listener_id = %listener_id,
            "Listener subscribed"
        );

        Ok(listener_id)
    }

    /// Unsubscribe a listener by ID.
    pub fn unsubscribe(&self, listener_id: ListenerId) -> Result<()> {
        let mut listeners = self.listeners.write();

        for listener_list in listeners.values_mut() {
            if let Some(pos) = listener_list.iter().position(|l| l.id == listener_id) {
                listener_list.remove(pos);
                tracing::debug!(listener_id = %listener_id, "Listener unsubscribed");
                return Ok(());
            }
        }

        Err(XervError::ConfigValue {
            field: format!("listener:{}", listener_id),
            cause: "Listener not found".to_string(),
        })
    }

    /// Get the number of listeners for a trigger.
    pub fn listener_count(&self, trigger_id: &str) -> usize {
        self.listeners
            .read()
            .get(trigger_id)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    /// Start all registered triggers.
    pub async fn start(&self) -> Result<()> {
        {
            let mut running = self.running.write();
            if *running {
                return Ok(());
            }
            *running = true;
        }

        let triggers: Vec<_> = self
            .triggers
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect();

        for (trigger_id, trigger) in triggers {
            let listeners = Arc::clone(&self.listeners);
            let tid = trigger_id.clone();

            // Create the broadcast callback
            let callback = move |event: TriggerEvent| {
                let listeners_guard = listeners.read();
                if let Some(listener_list) = listeners_guard.get(&tid) {
                    for entry in listener_list {
                        // Create a new event with a unique trace ID for each listener
                        let listener_event = TriggerEvent {
                            trigger_id: event.trigger_id.clone(),
                            trace_id: TraceId::new(),
                            data: event.data,
                            schema_hash: event.schema_hash,
                            timestamp_ns: event.timestamp_ns,
                            metadata: event.metadata.clone(),
                        };

                        (entry.callback)(listener_event);
                    }
                }
            };

            // Start the trigger with the broadcast callback
            let trigger_clone = Arc::clone(&trigger);
            let tid_log = trigger_id.clone();
            tokio::spawn(async move {
                if let Err(e) = trigger_clone.start(Box::new(callback)).await {
                    tracing::error!(
                        trigger_id = %tid_log,
                        error = %e,
                        "Trigger failed"
                    );
                }
            });

            tracing::info!(trigger_id = %trigger_id, "Trigger started");
        }

        Ok(())
    }

    /// Stop all triggers.
    pub async fn stop(&self) -> Result<()> {
        {
            let mut running = self.running.write();
            if !*running {
                return Ok(());
            }
            *running = false;
        }

        let triggers: Vec<_> = self
            .triggers
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect();

        for (trigger_id, trigger) in triggers {
            if let Err(e) = trigger.stop().await {
                tracing::error!(
                    trigger_id = %trigger_id,
                    error = %e,
                    "Failed to stop trigger"
                );
            }
            tracing::info!(trigger_id = %trigger_id, "Trigger stopped");
        }

        Ok(())
    }

    /// Pause all triggers.
    pub async fn pause(&self) -> Result<()> {
        let triggers: Vec<_> = self
            .triggers
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect();

        for (trigger_id, trigger) in triggers {
            if let Err(e) = trigger.pause().await {
                tracing::error!(
                    trigger_id = %trigger_id,
                    error = %e,
                    "Failed to pause trigger"
                );
            }
        }

        Ok(())
    }

    /// Resume all triggers.
    pub async fn resume(&self) -> Result<()> {
        let triggers: Vec<_> = self
            .triggers
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect();

        for (trigger_id, trigger) in triggers {
            if let Err(e) = trigger.resume().await {
                tracing::error!(
                    trigger_id = %trigger_id,
                    error = %e,
                    "Failed to resume trigger"
                );
            }
        }

        Ok(())
    }

    /// Check if the pool is running.
    pub fn is_running(&self) -> bool {
        *self.running.read()
    }

    /// Get all registered trigger IDs.
    pub fn trigger_ids(&self) -> Vec<String> {
        self.triggers.read().keys().cloned().collect()
    }

    /// Remove a trigger from the pool.
    ///
    /// This also removes all listeners for the trigger.
    pub async fn remove_trigger(&self, trigger_id: &str) -> Result<()> {
        // Stop the trigger if running
        if let Some(trigger) = self.triggers.read().get(trigger_id) {
            if trigger.is_running() {
                trigger.stop().await?;
            }
        }

        // Remove trigger and listeners
        self.triggers.write().remove(trigger_id);
        self.listeners.write().remove(trigger_id);

        Ok(())
    }
}

impl Default for ListenerPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use xerv_core::traits::{TriggerFuture, TriggerType};
    use xerv_core::types::PipelineId;

    /// Simple test trigger that can be manually fired.
    struct TestTrigger {
        id: String,
        running: std::sync::atomic::AtomicBool,
    }

    impl TestTrigger {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                running: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl Trigger for TestTrigger {
        fn trigger_type(&self) -> TriggerType {
            TriggerType::Manual
        }

        fn id(&self) -> &str {
            &self.id
        }

        fn start<'a>(
            &'a self,
            _callback: Box<dyn Fn(TriggerEvent) + Send + Sync + 'static>,
        ) -> TriggerFuture<'a, ()> {
            self.running.store(true, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }

        fn stop<'a>(&'a self) -> TriggerFuture<'a, ()> {
            self.running.store(false, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }

        fn pause<'a>(&'a self) -> TriggerFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn resume<'a>(&'a self) -> TriggerFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn is_running(&self) -> bool {
            self.running.load(Ordering::SeqCst)
        }
    }

    #[tokio::test]
    async fn register_and_subscribe() {
        let pool = ListenerPool::new();

        let trigger = Arc::new(TestTrigger::new("test_trigger"));
        pool.register_trigger(trigger).unwrap();

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let listener = Listener::new(PipelineId::new("test_pipeline", 1), move |_event| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        let listener_id = pool.subscribe("test_trigger", listener).unwrap();
        assert_eq!(pool.listener_count("test_trigger"), 1);

        pool.unsubscribe(listener_id).unwrap();
        assert_eq!(pool.listener_count("test_trigger"), 0);
    }

    #[tokio::test]
    async fn multiple_listeners_same_trigger() {
        let pool = ListenerPool::new();

        let trigger = Arc::new(TestTrigger::new("shared_trigger"));
        pool.register_trigger(trigger).unwrap();

        let listener1 = Listener::new(PipelineId::new("pipeline_a", 1), |_| {});
        let listener2 = Listener::new(PipelineId::new("pipeline_b", 1), |_| {});
        let listener3 = Listener::new(PipelineId::new("pipeline_c", 1), |_| {});

        pool.subscribe("shared_trigger", listener1).unwrap();
        pool.subscribe("shared_trigger", listener2).unwrap();
        pool.subscribe("shared_trigger", listener3).unwrap();

        assert_eq!(pool.listener_count("shared_trigger"), 3);
    }

    #[tokio::test]
    async fn start_stop_lifecycle() {
        let pool = ListenerPool::new();

        let trigger = Arc::new(TestTrigger::new("lifecycle_trigger"));
        pool.register_trigger(trigger).unwrap();

        assert!(!pool.is_running());

        pool.start().await.unwrap();
        assert!(pool.is_running());

        pool.stop().await.unwrap();
        assert!(!pool.is_running());
    }

    #[tokio::test]
    async fn remove_trigger() {
        let pool = ListenerPool::new();

        let trigger = Arc::new(TestTrigger::new("removable_trigger"));
        pool.register_trigger(trigger).unwrap();

        let listener = Listener::new(PipelineId::new("test", 1), |_| {});
        pool.subscribe("removable_trigger", listener).unwrap();

        assert!(pool.get_trigger("removable_trigger").is_some());
        assert_eq!(pool.listener_count("removable_trigger"), 1);

        pool.remove_trigger("removable_trigger").await.unwrap();

        assert!(pool.get_trigger("removable_trigger").is_none());
        assert_eq!(pool.listener_count("removable_trigger"), 0);
    }

    #[tokio::test]
    async fn duplicate_trigger_registration() {
        let pool = ListenerPool::new();

        let trigger1 = Arc::new(TestTrigger::new("dupe_trigger"));
        let trigger2 = Arc::new(TestTrigger::new("dupe_trigger"));

        pool.register_trigger(trigger1).unwrap();

        let result = pool.register_trigger(trigger2);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn subscribe_to_nonexistent_trigger() {
        let pool = ListenerPool::new();

        let listener = Listener::new(PipelineId::new("test", 1), |_| {});

        let result = pool.subscribe("nonexistent", listener);
        assert!(result.is_err());
    }
}
