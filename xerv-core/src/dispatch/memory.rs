//! In-memory dispatch backend for single-node deployments.
//!
//! This backend is useful for:
//! - Development and testing
//! - Single-node deployments
//! - Scenarios where persistence isn't required

use super::config::MemoryConfig;
use super::request::TraceRequest;
use super::traits::{DispatchBackend, DispatchError, DispatchFuture, DispatchResult};
use crate::types::{PipelineId, TraceId};
use parking_lot::Mutex;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Notify;

/// In-memory dispatch backend.
///
/// Uses a simple queue (FIFO or priority) to manage trace requests.
/// All state is lost on restart.
pub struct MemoryDispatch {
    config: MemoryConfig,
    /// Pending traces (either FIFO queue or priority heap).
    queue: Mutex<QueueInner>,
    /// Traces currently being processed.
    in_flight: Mutex<HashMap<TraceId, TraceRequest>>,
    /// Notification for waiting workers.
    notify: Arc<Notify>,
    /// Whether we're shutting down.
    shutting_down: AtomicBool,
    /// Total enqueued count (for metrics).
    total_enqueued: AtomicUsize,
    /// Total dequeued count (for metrics).
    total_dequeued: AtomicUsize,
}

enum QueueInner {
    Fifo(VecDeque<TraceRequest>),
    Priority(BinaryHeap<PrioritizedRequest>),
}

/// Wrapper for priority queue ordering.
struct PrioritizedRequest(TraceRequest);

impl PartialEq for PrioritizedRequest {
    fn eq(&self, other: &Self) -> bool {
        self.0.trace_id == other.0.trace_id
    }
}

impl Eq for PrioritizedRequest {}

impl PartialOrd for PrioritizedRequest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedRequest {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Higher priority first, then older requests first
        self.0
            .priority
            .cmp(&other.0.priority)
            .then_with(|| other.0.created_at_ms.cmp(&self.0.created_at_ms))
    }
}

impl MemoryDispatch {
    /// Create a new in-memory dispatch with the given config.
    pub fn new(config: MemoryConfig) -> Self {
        let queue = if config.priority_queue {
            QueueInner::Priority(BinaryHeap::new())
        } else {
            QueueInner::Fifo(VecDeque::new())
        };

        Self {
            config,
            queue: Mutex::new(queue),
            in_flight: Mutex::new(HashMap::new()),
            notify: Arc::new(Notify::new()),
            shutting_down: AtomicBool::new(false),
            total_enqueued: AtomicUsize::new(0),
            total_dequeued: AtomicUsize::new(0),
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(MemoryConfig::default())
    }

    /// Get the current queue length.
    pub fn len(&self) -> usize {
        let queue = self.queue.lock();
        match &*queue {
            QueueInner::Fifo(q) => q.len(),
            QueueInner::Priority(q) => q.len(),
        }
    }

    /// Check if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get total enqueued count.
    pub fn total_enqueued(&self) -> usize {
        self.total_enqueued.load(Ordering::Relaxed)
    }

    /// Get total dequeued count.
    pub fn total_dequeued(&self) -> usize {
        self.total_dequeued.load(Ordering::Relaxed)
    }

    fn push(&self, request: TraceRequest) -> DispatchResult<()> {
        let mut queue = self.queue.lock();
        let len = match &*queue {
            QueueInner::Fifo(q) => q.len(),
            QueueInner::Priority(q) => q.len(),
        };

        if len >= self.config.max_queue_size {
            return Err(DispatchError::QueueFull {
                current: len,
                max: self.config.max_queue_size,
            });
        }

        match &mut *queue {
            QueueInner::Fifo(q) => q.push_back(request),
            QueueInner::Priority(q) => q.push(PrioritizedRequest(request)),
        }

        self.total_enqueued.fetch_add(1, Ordering::Relaxed);
        self.notify.notify_one();
        Ok(())
    }

    fn pop(&self) -> Option<TraceRequest> {
        let mut queue = self.queue.lock();
        let request = match &mut *queue {
            QueueInner::Fifo(q) => q.pop_front(),
            QueueInner::Priority(q) => q.pop().map(|p| p.0),
        };

        if request.is_some() {
            self.total_dequeued.fetch_add(1, Ordering::Relaxed);
        }

        request
    }
}

impl Default for MemoryDispatch {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl DispatchBackend for MemoryDispatch {
    fn enqueue(&self, request: TraceRequest) -> DispatchFuture<'_, TraceId> {
        Box::pin(async move {
            if self.shutting_down.load(Ordering::SeqCst) {
                return Err(DispatchError::ShuttingDown);
            }

            let trace_id = request.trace_id;
            self.push(request)?;

            tracing::debug!(trace_id = %trace_id, "Enqueued trace");
            Ok(trace_id)
        })
    }

    fn dequeue(&self, timeout_ms: u64) -> DispatchFuture<'_, Option<TraceRequest>> {
        Box::pin(async move {
            if self.shutting_down.load(Ordering::SeqCst) {
                return Ok(None);
            }

            // Try immediate pop
            if let Some(mut request) = self.pop() {
                request.increment_attempt();
                let trace_id = request.trace_id;
                self.in_flight.lock().insert(trace_id, request.clone());
                tracing::debug!(trace_id = %trace_id, "Dequeued trace");
                return Ok(Some(request));
            }

            // Wait for notification
            if timeout_ms > 0 {
                let timeout = std::time::Duration::from_millis(timeout_ms);
                let result = tokio::time::timeout(timeout, self.notify.notified()).await;

                if result.is_ok() {
                    // Notified, try again
                    if let Some(mut request) = self.pop() {
                        request.increment_attempt();
                        let trace_id = request.trace_id;
                        self.in_flight.lock().insert(trace_id, request.clone());
                        tracing::debug!(trace_id = %trace_id, "Dequeued trace after wait");
                        return Ok(Some(request));
                    }
                }
            }

            Ok(None)
        })
    }

    fn ack(&self, trace_id: TraceId) -> DispatchFuture<'_, ()> {
        Box::pin(async move {
            self.in_flight.lock().remove(&trace_id);
            tracing::debug!(trace_id = %trace_id, "Acknowledged trace");
            Ok(())
        })
    }

    fn nack(&self, trace_id: TraceId, error: String, retry: bool) -> DispatchFuture<'_, ()> {
        Box::pin(async move {
            let request = self.in_flight.lock().remove(&trace_id);

            if let Some(request) = request {
                if retry && !request.is_exhausted() {
                    tracing::debug!(
                        trace_id = %trace_id,
                        attempt = request.attempt,
                        error = %error,
                        "Retrying trace"
                    );
                    // Re-enqueue for retry
                    let _ = self.push(request);
                } else {
                    tracing::warn!(
                        trace_id = %trace_id,
                        attempt = request.attempt,
                        error = %error,
                        "Trace failed permanently"
                    );
                }
            }

            Ok(())
        })
    }

    fn queue_depth(&self) -> DispatchFuture<'_, usize> {
        Box::pin(async move { Ok(self.len()) })
    }

    fn in_flight_count(&self) -> DispatchFuture<'_, usize> {
        Box::pin(async move { Ok(self.in_flight.lock().len()) })
    }

    fn health_check(&self) -> DispatchFuture<'_, bool> {
        Box::pin(async move { Ok(!self.shutting_down.load(Ordering::SeqCst)) })
    }

    fn shutdown(&self) -> DispatchFuture<'_, ()> {
        Box::pin(async move {
            self.shutting_down.store(true, Ordering::SeqCst);
            // Notify all waiting workers
            self.notify.notify_waiters();
            tracing::info!("Memory dispatch shutdown complete");
            Ok(())
        })
    }

    fn get_pending_by_pipeline(
        &self,
        pipeline_id: &PipelineId,
    ) -> DispatchFuture<'_, Vec<TraceId>> {
        let pipeline_id = pipeline_id.clone();
        Box::pin(async move {
            let queue = self.queue.lock();
            let traces = match &*queue {
                QueueInner::Fifo(q) => q
                    .iter()
                    .filter(|r| r.pipeline_id == pipeline_id)
                    .map(|r| r.trace_id)
                    .collect(),
                QueueInner::Priority(q) => q
                    .iter()
                    .filter(|r| r.0.pipeline_id == pipeline_id)
                    .map(|r| r.0.trace_id)
                    .collect(),
            };
            Ok(traces)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_request(name: &str) -> TraceRequest {
        TraceRequest::new(
            TraceId::new(),
            PipelineId::new(name, 1),
            "test-trigger",
            vec![],
        )
    }

    #[tokio::test]
    async fn basic_enqueue_dequeue() {
        let dispatch = MemoryDispatch::with_defaults();

        let request = create_request("test");
        let trace_id = dispatch.enqueue(request).await.unwrap();

        let dequeued = dispatch.dequeue(0).await.unwrap();
        assert!(dequeued.is_some());
        assert_eq!(dequeued.unwrap().trace_id, trace_id);

        // Queue should be empty now
        let empty = dispatch.dequeue(0).await.unwrap();
        assert!(empty.is_none());
    }

    #[tokio::test]
    async fn ack_removes_from_in_flight() {
        let dispatch = MemoryDispatch::with_defaults();

        let request = create_request("test");
        let trace_id = dispatch.enqueue(request).await.unwrap();

        let _ = dispatch.dequeue(0).await.unwrap();
        assert_eq!(dispatch.in_flight_count().await.unwrap(), 1);

        dispatch.ack(trace_id).await.unwrap();
        assert_eq!(dispatch.in_flight_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn nack_with_retry() {
        let dispatch = MemoryDispatch::with_defaults();

        let mut request = create_request("test");
        request.max_attempts = 3;
        let trace_id = dispatch.enqueue(request).await.unwrap();

        // Dequeue and nack
        let _ = dispatch.dequeue(0).await.unwrap();
        dispatch
            .nack(trace_id, "test error".to_string(), true)
            .await
            .unwrap();

        // Should be back in queue
        assert_eq!(dispatch.queue_depth().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn nack_exhausted() {
        let dispatch = MemoryDispatch::with_defaults();

        let mut request = create_request("test");
        request.max_attempts = 1;
        request.attempt = 1; // Already at max
        let trace_id = dispatch.enqueue(request).await.unwrap();

        let _ = dispatch.dequeue(0).await.unwrap();
        dispatch
            .nack(trace_id, "test error".to_string(), true)
            .await
            .unwrap();

        // Should NOT be back in queue (exhausted)
        assert_eq!(dispatch.queue_depth().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn queue_full() {
        let config = MemoryConfig {
            max_queue_size: 2,
            priority_queue: false,
        };
        let dispatch = MemoryDispatch::new(config);

        dispatch.enqueue(create_request("test1")).await.unwrap();
        dispatch.enqueue(create_request("test2")).await.unwrap();

        let result = dispatch.enqueue(create_request("test3")).await;
        assert!(matches!(result, Err(DispatchError::QueueFull { .. })));
    }

    #[tokio::test]
    async fn priority_queue() {
        let config = MemoryConfig {
            max_queue_size: 100,
            priority_queue: true,
        };
        let dispatch = MemoryDispatch::new(config);

        // Enqueue low priority first
        let mut low = create_request("low");
        low.priority = 0;
        dispatch.enqueue(low).await.unwrap();

        // Enqueue high priority second
        let mut high = create_request("high");
        high.priority = 10;
        let high_id = dispatch.enqueue(high).await.unwrap();

        // High priority should come out first
        let dequeued = dispatch.dequeue(0).await.unwrap().unwrap();
        assert_eq!(dequeued.trace_id, high_id);
    }

    #[tokio::test]
    async fn shutdown_stops_dequeue() {
        let dispatch = MemoryDispatch::with_defaults();

        dispatch.shutdown().await.unwrap();

        let result = dispatch.dequeue(100).await.unwrap();
        assert!(result.is_none());

        let result = dispatch.enqueue(create_request("test")).await;
        assert!(matches!(result, Err(DispatchError::ShuttingDown)));
    }

    #[tokio::test]
    async fn get_pending_by_pipeline() {
        let dispatch = MemoryDispatch::with_defaults();

        let pipeline_a = PipelineId::new("pipeline-a", 1);
        let pipeline_b = PipelineId::new("pipeline-b", 1);

        // Enqueue mixed pipelines
        dispatch
            .enqueue(TraceRequest::new(
                TraceId::new(),
                pipeline_a.clone(),
                "t",
                vec![],
            ))
            .await
            .unwrap();
        dispatch
            .enqueue(TraceRequest::new(
                TraceId::new(),
                pipeline_b.clone(),
                "t",
                vec![],
            ))
            .await
            .unwrap();
        dispatch
            .enqueue(TraceRequest::new(
                TraceId::new(),
                pipeline_a.clone(),
                "t",
                vec![],
            ))
            .await
            .unwrap();

        let a_traces = dispatch.get_pending_by_pipeline(&pipeline_a).await.unwrap();
        assert_eq!(a_traces.len(), 2);

        let b_traces = dispatch.get_pending_by_pipeline(&pipeline_b).await.unwrap();
        assert_eq!(b_traces.len(), 1);
    }

    #[tokio::test]
    async fn health_check() {
        let dispatch = MemoryDispatch::with_defaults();
        assert!(dispatch.health_check().await.unwrap());

        dispatch.shutdown().await.unwrap();
        assert!(!dispatch.health_check().await.unwrap());
    }
}
