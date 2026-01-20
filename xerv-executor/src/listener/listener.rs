//! Listener types for multi-head trigger support.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use xerv_core::traits::TriggerEvent;
use xerv_core::types::PipelineId;

/// Unique identifier for a listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerId(u64);

impl ListenerId {
    /// Create a new unique listener ID.
    pub fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Get the raw ID value.
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Default for ListenerId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ListenerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "listener_{}", self.0)
    }
}

/// A listener represents a subscription to a trigger.
///
/// When the trigger fires, the listener's callback is invoked with the event.
/// Each listener receives its own copy of the event with a unique trace ID.
pub struct Listener {
    /// Unique identifier for this listener.
    pub id: ListenerId,
    /// The pipeline that owns this listener.
    pub pipeline_id: PipelineId,
    /// Callback invoked when the trigger fires (stored as Arc for sharing).
    callback: Arc<dyn Fn(TriggerEvent) + Send + Sync>,
}

impl Listener {
    /// Create a new listener.
    pub fn new(
        pipeline_id: PipelineId,
        callback: impl Fn(TriggerEvent) + Send + Sync + 'static,
    ) -> Self {
        Self {
            id: ListenerId::new(),
            pipeline_id,
            callback: Arc::new(callback),
        }
    }

    /// Create a listener with a specific ID (for testing).
    pub fn with_id(
        id: ListenerId,
        pipeline_id: PipelineId,
        callback: impl Fn(TriggerEvent) + Send + Sync + 'static,
    ) -> Self {
        Self {
            id,
            pipeline_id,
            callback: Arc::new(callback),
        }
    }

    /// Invoke the listener's callback with an event.
    pub fn notify(&self, event: TriggerEvent) {
        (self.callback)(event);
    }

    /// Convert into the callback Arc for use in ListenerPool.
    pub fn into_callback(self) -> Arc<dyn Fn(TriggerEvent) + Send + Sync> {
        self.callback
    }
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("id", &self.id)
            .field("pipeline_id", &self.pipeline_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use xerv_core::types::RelPtr;

    #[test]
    fn listener_id_uniqueness() {
        let id1 = ListenerId::new();
        let id2 = ListenerId::new();
        let id3 = ListenerId::new();

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    #[test]
    fn listener_callback() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let listener = Listener::new(PipelineId::new("test", 1), move |_event| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        let event = TriggerEvent::new("test_trigger", RelPtr::null());

        listener.notify(event.clone());
        listener.notify(event.clone());
        listener.notify(event);

        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
