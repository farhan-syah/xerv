//! Core dispatch traits and error types.

use super::request::TraceRequest;
use crate::types::{PipelineId, TraceId};
use std::future::Future;
use std::pin::Pin;
use thiserror::Error;

/// Errors that can occur during dispatch operations.
#[derive(Debug, Error)]
pub enum DispatchError {
    /// Failed to connect to the dispatch backend.
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// The dispatch queue is full.
    #[error("Queue full: {current}/{max} pending traces")]
    QueueFull {
        /// Current queue size.
        current: usize,
        /// Maximum queue size.
        max: usize,
    },

    /// Trace not found in the queue.
    #[error("Trace {0} not found in queue")]
    TraceNotFound(TraceId),

    /// Backend-specific error.
    #[error("Backend error: {0}")]
    BackendError(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Timeout waiting for operation.
    #[error("Operation timed out after {0}ms")]
    Timeout(u64),

    /// The backend is shutting down.
    #[error("Backend is shutting down")]
    ShuttingDown,

    /// Configuration error.
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

/// Result type for dispatch operations.
pub type DispatchResult<T> = Result<T, DispatchError>;

/// Type alias for async dispatch futures.
pub type DispatchFuture<'a, T> = Pin<Box<dyn Future<Output = DispatchResult<T>> + Send + 'a>>;

/// Trait for dispatch backends.
///
/// A dispatch backend is responsible for:
/// - Queueing trace requests from triggers
/// - Distributing traces to workers
/// - Tracking acknowledgments and retries
///
/// # Implementation Notes
///
/// - All methods are async and must be `Send`
/// - Backends should handle their own connection pooling
/// - Backends should implement backpressure via `enqueue` returning `QueueFull`
/// - `dequeue` should block (with timeout) waiting for work
pub trait DispatchBackend: Send + Sync {
    /// Enqueue a trace request for execution.
    ///
    /// Returns the trace ID on success. The trace will be picked up by a worker
    /// via `dequeue()`.
    ///
    /// # Errors
    ///
    /// - `QueueFull` if the queue has reached capacity
    /// - `BackendError` for backend-specific failures
    fn enqueue(&self, request: TraceRequest) -> DispatchFuture<'_, TraceId>;

    /// Dequeue a trace request for processing.
    ///
    /// This should be called by workers to get work. The request is marked as
    /// "in-flight" until `ack()` or `nack()` is called.
    ///
    /// Returns `None` if no work is available (after timeout).
    ///
    /// # Arguments
    ///
    /// * `timeout_ms` - Maximum time to wait for work (0 = don't wait)
    fn dequeue(&self, timeout_ms: u64) -> DispatchFuture<'_, Option<TraceRequest>>;

    /// Acknowledge successful processing of a trace.
    ///
    /// This removes the trace from the in-flight set. The trace will not be
    /// retried.
    fn ack(&self, trace_id: TraceId) -> DispatchFuture<'_, ()>;

    /// Negative acknowledge - trace processing failed.
    ///
    /// This returns the trace to the queue for retry (if retries are enabled)
    /// or marks it as failed.
    ///
    /// # Arguments
    ///
    /// * `trace_id` - The trace that failed
    /// * `error` - Error message for logging/debugging
    /// * `retry` - Whether to retry the trace
    fn nack(&self, trace_id: TraceId, error: String, retry: bool) -> DispatchFuture<'_, ()>;

    /// Get the current queue depth (pending traces).
    fn queue_depth(&self) -> DispatchFuture<'_, usize>;

    /// Get the number of in-flight traces (being processed).
    fn in_flight_count(&self) -> DispatchFuture<'_, usize>;

    /// Check if the backend is healthy and connected.
    fn health_check(&self) -> DispatchFuture<'_, bool>;

    /// Gracefully shutdown the backend.
    ///
    /// This should:
    /// 1. Stop accepting new enqueue requests
    /// 2. Wait for in-flight traces to complete (with timeout)
    /// 3. Close connections
    fn shutdown(&self) -> DispatchFuture<'_, ()>;

    /// Get traces for a specific pipeline (for monitoring).
    fn get_pending_by_pipeline(&self, pipeline_id: &PipelineId)
    -> DispatchFuture<'_, Vec<TraceId>>;
}

/// Extension trait for dispatch backends with subscription support.
///
/// Some backends (like NATS) support push-based delivery where workers
/// subscribe to receive traces rather than polling.
#[allow(dead_code)]
pub trait SubscribableDispatch: DispatchBackend {
    /// Subscribe to receive traces for a pipeline.
    ///
    /// Returns a stream of trace requests. The stream will yield traces
    /// as they are enqueued.
    fn subscribe(
        &self,
        pipeline_id: Option<PipelineId>,
    ) -> DispatchFuture<'_, Box<dyn TraceStream>>;
}

/// A stream of trace requests (for subscription-based backends).
#[allow(dead_code)]
pub trait TraceStream: Send {
    /// Get the next trace request.
    ///
    /// Returns `None` when the subscription is closed.
    fn next(&mut self) -> DispatchFuture<'_, Option<TraceRequest>>;

    /// Close the subscription.
    fn close(&mut self) -> DispatchFuture<'_, ()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ensure DispatchError is Send + Sync
    fn _assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn error_is_send_sync() {
        _assert_send_sync::<DispatchError>();
    }
}
