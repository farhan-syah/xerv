//! Integration tests for ListenerPool multi-head trigger support.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use xerv_core::traits::{Trigger, TriggerEvent, TriggerFuture, TriggerType};
use xerv_core::types::{PipelineId, RelPtr};
use xerv_executor::listener::{Listener, ListenerId, ListenerPool};

/// A test trigger that can be manually fired.
struct ManualTestTrigger {
    id: String,
    running: std::sync::atomic::AtomicBool,
    #[allow(clippy::type_complexity)]
    callback: parking_lot::RwLock<Option<Arc<dyn Fn(TriggerEvent) + Send + Sync>>>,
}

impl ManualTestTrigger {
    fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            running: std::sync::atomic::AtomicBool::new(false),
            callback: parking_lot::RwLock::new(None),
        }
    }

    fn fire(&self) {
        if let Some(callback) = self.callback.read().as_ref() {
            let event = TriggerEvent::new(&self.id, RelPtr::null());
            callback(event);
        }
    }
}

impl Trigger for ManualTestTrigger {
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
        self.running.store(true, Ordering::SeqCst);
        *self.callback.write() = Some(Arc::from(callback));
        Box::pin(async { Ok(()) })
    }

    fn stop<'a>(&'a self) -> TriggerFuture<'a, ()> {
        self.running.store(false, Ordering::SeqCst);
        *self.callback.write() = None;
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
async fn listener_pool_basic_subscribe() {
    let pool = ListenerPool::new();

    let trigger = Arc::new(ManualTestTrigger::new("test_trigger"));
    pool.register_trigger(trigger).unwrap();

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = Arc::clone(&counter);

    let listener = Listener::new(PipelineId::new("pipeline_a", 1), move |_event| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
    });

    let listener_id = pool.subscribe("test_trigger", listener).unwrap();
    assert_eq!(pool.listener_count("test_trigger"), 1);

    pool.unsubscribe(listener_id).unwrap();
    assert_eq!(pool.listener_count("test_trigger"), 0);
}

#[tokio::test]
async fn listener_pool_multiple_listeners() {
    let pool = ListenerPool::new();

    let trigger = Arc::new(ManualTestTrigger::new("shared_trigger"));
    pool.register_trigger(Arc::clone(&trigger) as Arc<dyn Trigger>)
        .unwrap();

    // Subscribe three pipelines
    let counter_a = Arc::new(AtomicUsize::new(0));
    let counter_b = Arc::new(AtomicUsize::new(0));
    let counter_c = Arc::new(AtomicUsize::new(0));

    let counter_a_clone = Arc::clone(&counter_a);
    let counter_b_clone = Arc::clone(&counter_b);
    let counter_c_clone = Arc::clone(&counter_c);

    let listener_a = Listener::new(PipelineId::new("pipeline_a", 1), move |_| {
        counter_a_clone.fetch_add(1, Ordering::SeqCst);
    });
    let listener_b = Listener::new(PipelineId::new("pipeline_b", 1), move |_| {
        counter_b_clone.fetch_add(1, Ordering::SeqCst);
    });
    let listener_c = Listener::new(PipelineId::new("pipeline_c", 1), move |_| {
        counter_c_clone.fetch_add(1, Ordering::SeqCst);
    });

    pool.subscribe("shared_trigger", listener_a).unwrap();
    pool.subscribe("shared_trigger", listener_b).unwrap();
    pool.subscribe("shared_trigger", listener_c).unwrap();

    assert_eq!(pool.listener_count("shared_trigger"), 3);

    // Start the pool and fire the trigger
    pool.start().await.unwrap();

    // Give the spawned task time to register the callback
    tokio::task::yield_now().await;

    // Fire the trigger
    trigger.fire();

    // Give async tasks time to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Each listener should have received one event
    assert_eq!(counter_a.load(Ordering::SeqCst), 1);
    assert_eq!(counter_b.load(Ordering::SeqCst), 1);
    assert_eq!(counter_c.load(Ordering::SeqCst), 1);

    pool.stop().await.unwrap();
}

#[tokio::test]
async fn listener_pool_duplicate_trigger_rejected() {
    let pool = ListenerPool::new();

    let trigger1 = Arc::new(ManualTestTrigger::new("same_id"));
    let trigger2 = Arc::new(ManualTestTrigger::new("same_id"));

    pool.register_trigger(trigger1).unwrap();

    let result = pool.register_trigger(trigger2);
    assert!(result.is_err());
}

#[tokio::test]
async fn listener_pool_subscribe_to_nonexistent() {
    let pool = ListenerPool::new();

    let listener = Listener::new(PipelineId::new("test", 1), |_| {});

    let result = pool.subscribe("nonexistent", listener);
    assert!(result.is_err());
}

#[tokio::test]
async fn listener_pool_lifecycle() {
    let pool = ListenerPool::new();

    let trigger = Arc::new(ManualTestTrigger::new("lifecycle_trigger"));
    pool.register_trigger(trigger).unwrap();

    assert!(!pool.is_running());

    pool.start().await.unwrap();
    assert!(pool.is_running());

    pool.stop().await.unwrap();
    assert!(!pool.is_running());
}

#[tokio::test]
async fn listener_pool_remove_trigger() {
    let pool = ListenerPool::new();

    let trigger = Arc::new(ManualTestTrigger::new("removable"));
    pool.register_trigger(trigger).unwrap();

    let listener = Listener::new(PipelineId::new("test", 1), |_| {});
    pool.subscribe("removable", listener).unwrap();

    assert!(pool.get_trigger("removable").is_some());
    assert_eq!(pool.listener_count("removable"), 1);

    pool.remove_trigger("removable").await.unwrap();

    assert!(pool.get_trigger("removable").is_none());
    assert_eq!(pool.listener_count("removable"), 0);
}

#[tokio::test]
async fn listener_ids_are_unique() {
    let id1 = ListenerId::new();
    let id2 = ListenerId::new();
    let id3 = ListenerId::new();

    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);
}

#[tokio::test]
async fn listener_pool_trigger_ids() {
    let pool = ListenerPool::new();

    pool.register_trigger(Arc::new(ManualTestTrigger::new("trigger_a")))
        .unwrap();
    pool.register_trigger(Arc::new(ManualTestTrigger::new("trigger_b")))
        .unwrap();
    pool.register_trigger(Arc::new(ManualTestTrigger::new("trigger_c")))
        .unwrap();

    let ids = pool.trigger_ids();
    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&"trigger_a".to_string()));
    assert!(ids.contains(&"trigger_b".to_string()));
    assert!(ids.contains(&"trigger_c".to_string()));
}
