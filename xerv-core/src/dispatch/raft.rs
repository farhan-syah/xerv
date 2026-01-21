//! Raft-based dispatch backend for consensus-driven execution.
//!
//! This backend provides a local queue implementation with hooks for integration
//! with external Raft consensus systems (like xerv-cluster).
//!
//! # Features
//!
//! - **Zero Dependencies**: No external services required for local mode
//! - **Pluggable Consensus**: Trait-based integration with Raft implementations
//! - **Edge Deployments**: Works standalone in air-gapped environments
//!
//! # Architecture
//!
//! The `RaftDispatch` can operate in two modes:
//!
//! 1. **Local Queue Mode** (default): Uses an in-memory queue for single-node deployments
//! 2. **Cluster Mode**: Integrates with a `RaftClusterProvider` for distributed consensus
//!
//! The cluster integration happens at a higher level (e.g., xerv-executor) to avoid
//! circular dependencies.

use super::config::RaftConfig;
use super::request::TraceRequest;
use super::traits::{DispatchBackend, DispatchError, DispatchFuture, DispatchResult};
use crate::types::{PipelineId, TraceId};
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Trait for Raft cluster integration.
///
/// This trait abstracts the cluster operations needed by RaftDispatch,
/// allowing xerv-cluster or other implementations to provide the actual
/// Raft consensus logic without creating circular dependencies.
pub trait RaftClusterProvider: Send + Sync {
    /// Get the node ID of this cluster member.
    fn node_id(&self) -> u64;

    /// Record a trace start in the Raft log.
    fn start_trace(
        &self,
        trace_id: TraceId,
        pipeline_id: PipelineId,
        trigger_data: Vec<u8>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>>;

    /// Record a trace completion in the Raft log.
    fn complete_trace(
        &self,
        trace_id: TraceId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>>;

    /// Record a trace failure in the Raft log.
    fn fail_trace(
        &self,
        trace_id: TraceId,
        error: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>>;

    /// Check if the cluster is healthy (has a leader).
    fn is_healthy(&self) -> bool;
}

/// Raft-based dispatch backend.
///
/// This backend can operate in two modes:
///
/// 1. **Local Queue Mode**: When no cluster provider is attached, uses an in-memory
///    queue for single-node deployments.
///
/// 2. **Cluster Mode**: When a `RaftClusterProvider` is attached via `with_cluster()`,
///    all dispatch operations are recorded in the Raft log for strong consistency.
pub struct RaftDispatch {
    config: RaftConfig,
    /// The cluster provider (when using full Raft integration)
    cluster: Option<Arc<dyn RaftClusterProvider>>,
    /// Local pending traces queue
    pending: Arc<Mutex<VecDeque<TraceRequest>>>,
    /// In-flight traces being processed
    in_flight: Arc<Mutex<HashMap<TraceId, TraceRequest>>>,
    /// Shutdown flag
    shutting_down: AtomicBool,
}

impl RaftDispatch {
    /// Create a new Raft dispatch backend in local queue mode.
    ///
    /// For distributed consensus, call `with_cluster()` after creation to attach
    /// a cluster provider.
    pub fn new(config: RaftConfig) -> DispatchResult<Self> {
        tracing::info!(
            node_id = config.node_id,
            "RaftDispatch created in local queue mode"
        );

        Ok(Self {
            config,
            cluster: None,
            pending: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
            shutting_down: AtomicBool::new(false),
        })
    }

    /// Attach a cluster provider for full Raft consensus integration.
    ///
    /// Once attached, all dispatch operations go through the Raft log
    /// for strong consistency across the cluster.
    pub fn with_cluster(mut self, provider: Arc<dyn RaftClusterProvider>) -> Self {
        tracing::info!(
            node_id = provider.node_id(),
            "RaftDispatch attached to cluster provider"
        );
        self.cluster = Some(provider);
        self
    }

    /// Set the cluster provider (alternative to builder pattern).
    pub fn set_cluster(&mut self, provider: Arc<dyn RaftClusterProvider>) {
        tracing::info!(
            node_id = provider.node_id(),
            "RaftDispatch cluster provider updated"
        );
        self.cluster = Some(provider);
    }

    /// Check if cluster integration is active.
    pub fn is_cluster_mode(&self) -> bool {
        self.cluster.is_some()
    }

    /// Get the node ID from config.
    pub fn node_id(&self) -> u64 {
        self.config.node_id
    }

    /// Execute enqueue via Raft consensus.
    async fn enqueue_via_raft(&self, request: &TraceRequest) -> DispatchResult<()> {
        let cluster = self
            .cluster
            .as_ref()
            .ok_or_else(|| DispatchError::ConnectionFailed("Raft cluster not connected".into()))?;

        cluster
            .start_trace(
                request.trace_id,
                request.pipeline_id.clone(),
                request.payload.clone(),
            )
            .await
            .map_err(|e| DispatchError::BackendError(format!("Raft error: {}", e)))
    }

    /// Execute ack via Raft consensus.
    async fn ack_via_raft(&self, trace_id: TraceId) -> DispatchResult<()> {
        let cluster = self
            .cluster
            .as_ref()
            .ok_or_else(|| DispatchError::ConnectionFailed("Raft cluster not connected".into()))?;

        cluster
            .complete_trace(trace_id)
            .await
            .map_err(|e| DispatchError::BackendError(format!("Raft error: {}", e)))
    }

    /// Execute nack via Raft consensus.
    async fn nack_via_raft(&self, trace_id: TraceId, error: String) -> DispatchResult<()> {
        let cluster = self
            .cluster
            .as_ref()
            .ok_or_else(|| DispatchError::ConnectionFailed("Raft cluster not connected".into()))?;

        cluster
            .fail_trace(trace_id, error)
            .await
            .map_err(|e| DispatchError::BackendError(format!("Raft error: {}", e)))
    }

    /// Check Raft cluster health.
    fn check_cluster_health(&self) -> bool {
        if let Some(cluster) = &self.cluster {
            cluster.is_healthy()
        } else {
            // No cluster attached = healthy (local mode)
            true
        }
    }
}

impl DispatchBackend for RaftDispatch {
    fn enqueue(&self, request: TraceRequest) -> DispatchFuture<'_, TraceId> {
        Box::pin(async move {
            if self.shutting_down.load(Ordering::SeqCst) {
                return Err(DispatchError::ShuttingDown);
            }

            let trace_id = request.trace_id;

            if self.cluster.is_some() {
                // Full Raft consensus path
                self.enqueue_via_raft(&request).await?;
                // Also track locally for dequeue
                self.pending.lock().push_back(request);
                tracing::debug!(trace_id = %trace_id, "Enqueued trace via Raft consensus");
                return Ok(trace_id);
            }

            // Local queue fallback
            self.pending.lock().push_back(request);
            tracing::debug!(trace_id = %trace_id, "Enqueued trace (local queue)");
            Ok(trace_id)
        })
    }

    fn dequeue(&self, _timeout_ms: u64) -> DispatchFuture<'_, Option<TraceRequest>> {
        Box::pin(async move {
            if self.shutting_down.load(Ordering::SeqCst) {
                return Ok(None);
            }

            // Dequeue from local queue (both modes use this for worker assignment)
            let mut pending = self.pending.lock();
            if let Some(mut request) = pending.pop_front() {
                request.increment_attempt();
                let trace_id = request.trace_id;
                self.in_flight.lock().insert(trace_id, request.clone());
                tracing::debug!(trace_id = %trace_id, "Dequeued trace for processing");
                Ok(Some(request))
            } else {
                Ok(None)
            }
        })
    }

    fn ack(&self, trace_id: TraceId) -> DispatchFuture<'_, ()> {
        Box::pin(async move {
            // Remove from in-flight tracking
            self.in_flight.lock().remove(&trace_id);

            if self.cluster.is_some() {
                // Record completion via Raft
                self.ack_via_raft(trace_id).await?;
                tracing::debug!(trace_id = %trace_id, "Acknowledged trace via Raft");
                return Ok(());
            }

            tracing::debug!(trace_id = %trace_id, "Acknowledged trace (local)");
            Ok(())
        })
    }

    fn nack(&self, trace_id: TraceId, error: String, retry: bool) -> DispatchFuture<'_, ()> {
        Box::pin(async move {
            let maybe_request = self.in_flight.lock().remove(&trace_id);

            if let Some(request) = maybe_request {
                if retry && !request.is_exhausted() {
                    // Re-queue for retry
                    self.pending.lock().push_back(request);
                    tracing::debug!(
                        trace_id = %trace_id,
                        error = %error,
                        "NACK'd trace, re-queued for retry"
                    );
                } else {
                    // Permanent failure
                    if self.cluster.is_some() {
                        self.nack_via_raft(trace_id, error.clone()).await?;
                    }

                    tracing::warn!(
                        trace_id = %trace_id,
                        error = %error,
                        "Trace failed permanently"
                    );
                }
            }

            Ok(())
        })
    }

    fn queue_depth(&self) -> DispatchFuture<'_, usize> {
        Box::pin(async move { Ok(self.pending.lock().len()) })
    }

    fn in_flight_count(&self) -> DispatchFuture<'_, usize> {
        Box::pin(async move { Ok(self.in_flight.lock().len()) })
    }

    fn health_check(&self) -> DispatchFuture<'_, bool> {
        Box::pin(async move {
            if self.shutting_down.load(Ordering::SeqCst) {
                return Ok(false);
            }

            Ok(self.check_cluster_health())
        })
    }

    fn shutdown(&self) -> DispatchFuture<'_, ()> {
        Box::pin(async move {
            self.shutting_down.store(true, Ordering::SeqCst);

            if let Some(cluster) = &self.cluster {
                tracing::info!(node_id = cluster.node_id(), "Shutting down Raft dispatch");
            }

            tracing::info!("Raft dispatch shutdown complete");
            Ok(())
        })
    }

    fn get_pending_by_pipeline(
        &self,
        pipeline_id: &PipelineId,
    ) -> DispatchFuture<'_, Vec<TraceId>> {
        let pipeline_id = pipeline_id.clone();
        Box::pin(async move {
            let pending = self.pending.lock();
            let in_flight = self.in_flight.lock();

            let mut traces: Vec<TraceId> = pending
                .iter()
                .filter(|r| r.pipeline_id == pipeline_id)
                .map(|r| r.trace_id)
                .collect();

            traces.extend(
                in_flight
                    .iter()
                    .filter(|(_, r)| r.pipeline_id == pipeline_id)
                    .map(|(id, _)| *id),
            );

            Ok(traces)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn raft_dispatch_basic() {
        let config = RaftConfig::new(1);
        let dispatch = RaftDispatch::new(config).unwrap();

        // Should start in local mode
        assert!(!dispatch.is_cluster_mode());

        // Enqueue
        let request = TraceRequest::new(
            TraceId::new(),
            PipelineId::new("test", 1),
            "trigger",
            vec![],
        );
        let trace_id = dispatch.enqueue(request).await.unwrap();

        // Check queue depth
        assert_eq!(dispatch.queue_depth().await.unwrap(), 1);

        // Dequeue
        let dequeued = dispatch.dequeue(1000).await.unwrap();
        assert!(dequeued.is_some());
        assert_eq!(dequeued.unwrap().trace_id, trace_id);

        // Ack
        dispatch.ack(trace_id).await.unwrap();
        assert_eq!(dispatch.in_flight_count().await.unwrap(), 0);

        dispatch.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn raft_dispatch_nack_retry() {
        let config = RaftConfig::new(1);
        let dispatch = RaftDispatch::new(config).unwrap();

        // Enqueue with max_attempts = 3
        let mut request = TraceRequest::new(
            TraceId::new(),
            PipelineId::new("test", 1),
            "trigger",
            vec![],
        );
        request.max_attempts = 3;
        let trace_id = dispatch.enqueue(request).await.unwrap();

        // Dequeue and nack with retry
        let _ = dispatch.dequeue(1000).await.unwrap();
        dispatch
            .nack(trace_id, "test error".into(), true)
            .await
            .unwrap();

        // Should be back in queue
        assert_eq!(dispatch.queue_depth().await.unwrap(), 1);
        assert_eq!(dispatch.in_flight_count().await.unwrap(), 0);

        dispatch.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn raft_dispatch_health_check() {
        let config = RaftConfig::new(1);
        let dispatch = RaftDispatch::new(config).unwrap();

        // Should be healthy initially
        assert!(dispatch.health_check().await.unwrap());

        // Should be unhealthy after shutdown
        dispatch.shutdown().await.unwrap();
        assert!(!dispatch.health_check().await.unwrap());
    }

    #[tokio::test]
    async fn raft_dispatch_get_pending_by_pipeline() {
        let config = RaftConfig::new(1);
        let dispatch = RaftDispatch::new(config).unwrap();

        let pipeline_a = PipelineId::new("pipeline-a", 1);
        let pipeline_b = PipelineId::new("pipeline-b", 1);

        // Enqueue traces for different pipelines
        let req_a1 = TraceRequest::new(TraceId::new(), pipeline_a.clone(), "trigger", vec![]);
        let req_a2 = TraceRequest::new(TraceId::new(), pipeline_a.clone(), "trigger", vec![]);
        let req_b = TraceRequest::new(TraceId::new(), pipeline_b.clone(), "trigger", vec![]);

        let trace_a1 = dispatch.enqueue(req_a1).await.unwrap();
        let trace_a2 = dispatch.enqueue(req_a2).await.unwrap();
        let _trace_b = dispatch.enqueue(req_b).await.unwrap();

        // Check pending by pipeline
        let pending_a = dispatch.get_pending_by_pipeline(&pipeline_a).await.unwrap();
        assert_eq!(pending_a.len(), 2);
        assert!(pending_a.contains(&trace_a1));
        assert!(pending_a.contains(&trace_a2));

        let pending_b = dispatch.get_pending_by_pipeline(&pipeline_b).await.unwrap();
        assert_eq!(pending_b.len(), 1);

        dispatch.shutdown().await.unwrap();
    }
}
