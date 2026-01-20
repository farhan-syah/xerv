//! Shared application state for API handlers.

use crate::listener::ListenerPool;
use crate::pipeline::PipelineController;
use crate::suspension::{MemorySuspensionStore, SuspensionStore};
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use xerv_core::auth::AuthConfig;
use xerv_core::logging::BufferedCollector;
use xerv_core::types::TraceId;

/// Maximum number of traces to keep in history.
const MAX_TRACE_HISTORY: usize = 1000;

/// Maximum number of log events to keep in the buffer.
const MAX_LOG_EVENTS: usize = 10_000;

/// Shared application state passed to all handlers.
pub struct AppState {
    /// Pipeline controller for managing pipelines.
    pub controller: Arc<PipelineController>,
    /// Listener pool for trigger management.
    pub listener_pool: Arc<ListenerPool>,
    /// Server start time.
    pub start_time: Instant,
    /// Recent trace history.
    pub trace_history: RwLock<TraceHistory>,
    /// Log event collector for trace execution logs.
    pub log_collector: Arc<BufferedCollector>,
    /// Authentication configuration.
    pub auth_config: AuthConfig,
    /// Suspension store for human-in-the-loop workflows.
    pub suspension_store: Arc<dyn SuspensionStore>,
}

impl AppState {
    /// Create new application state.
    pub fn new(controller: Arc<PipelineController>, listener_pool: Arc<ListenerPool>) -> Self {
        Self {
            controller,
            listener_pool,
            start_time: Instant::now(),
            trace_history: RwLock::new(TraceHistory::new(MAX_TRACE_HISTORY)),
            log_collector: Arc::new(BufferedCollector::new(MAX_LOG_EVENTS)),
            auth_config: AuthConfig::default(),
            suspension_store: Arc::new(MemorySuspensionStore::new()),
        }
    }

    /// Create application state with authentication configuration.
    pub fn with_auth(
        controller: Arc<PipelineController>,
        listener_pool: Arc<ListenerPool>,
        auth_config: AuthConfig,
    ) -> Self {
        Self {
            controller,
            listener_pool,
            start_time: Instant::now(),
            trace_history: RwLock::new(TraceHistory::new(MAX_TRACE_HISTORY)),
            log_collector: Arc::new(BufferedCollector::new(MAX_LOG_EVENTS)),
            auth_config,
            suspension_store: Arc::new(MemorySuspensionStore::new()),
        }
    }

    /// Create application state with a custom log collector.
    pub fn with_log_collector(
        controller: Arc<PipelineController>,
        listener_pool: Arc<ListenerPool>,
        log_collector: Arc<BufferedCollector>,
    ) -> Self {
        Self {
            controller,
            listener_pool,
            start_time: Instant::now(),
            trace_history: RwLock::new(TraceHistory::new(MAX_TRACE_HISTORY)),
            log_collector,
            auth_config: AuthConfig::default(),
            suspension_store: Arc::new(MemorySuspensionStore::new()),
        }
    }

    /// Create application state with auth and custom log collector.
    pub fn with_auth_and_log_collector(
        controller: Arc<PipelineController>,
        listener_pool: Arc<ListenerPool>,
        auth_config: AuthConfig,
        log_collector: Arc<BufferedCollector>,
    ) -> Self {
        Self {
            controller,
            listener_pool,
            start_time: Instant::now(),
            trace_history: RwLock::new(TraceHistory::new(MAX_TRACE_HISTORY)),
            log_collector,
            auth_config,
            suspension_store: Arc::new(MemorySuspensionStore::new()),
        }
    }

    /// Create application state with a custom suspension store.
    pub fn with_suspension_store(
        controller: Arc<PipelineController>,
        listener_pool: Arc<ListenerPool>,
        suspension_store: Arc<dyn SuspensionStore>,
    ) -> Self {
        Self {
            controller,
            listener_pool,
            start_time: Instant::now(),
            trace_history: RwLock::new(TraceHistory::new(MAX_TRACE_HISTORY)),
            log_collector: Arc::new(BufferedCollector::new(MAX_LOG_EVENTS)),
            auth_config: AuthConfig::default(),
            suspension_store,
        }
    }

    /// Get server uptime in seconds.
    pub fn uptime_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Get the number of active pipelines.
    pub fn pipeline_count(&self) -> usize {
        self.controller.list().len()
    }
}

/// Trace execution record.
#[derive(Debug, Clone)]
pub struct TraceRecord {
    /// Trace ID.
    pub trace_id: TraceId,
    /// Pipeline that owns this trace.
    pub pipeline_id: String,
    /// Status of the trace.
    pub status: TraceStatus,
    /// When the trace was started.
    pub started_at: Instant,
    /// When the trace completed (if finished).
    pub completed_at: Option<Instant>,
    /// Error message if failed.
    pub error: Option<String>,
}

impl TraceRecord {
    /// Create a new pending trace record.
    pub fn new(trace_id: TraceId, pipeline_id: String) -> Self {
        Self {
            trace_id,
            pipeline_id,
            status: TraceStatus::Running,
            started_at: Instant::now(),
            completed_at: None,
            error: None,
        }
    }

    /// Mark the trace as completed.
    pub fn complete(&mut self) {
        self.status = TraceStatus::Completed;
        self.completed_at = Some(Instant::now());
    }

    /// Mark the trace as failed.
    pub fn fail(&mut self, error: String) {
        self.status = TraceStatus::Failed;
        self.completed_at = Some(Instant::now());
        self.error = Some(error);
    }

    /// Get elapsed time in milliseconds.
    pub fn elapsed_ms(&self) -> u64 {
        let end = self.completed_at.unwrap_or_else(Instant::now);
        end.duration_since(self.started_at).as_millis() as u64
    }
}

/// Status of a trace execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceStatus {
    /// Trace is currently running.
    Running,
    /// Trace completed successfully.
    Completed,
    /// Trace failed.
    Failed,
}

impl TraceStatus {
    /// Convert to string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// Ring buffer of recent trace records.
pub struct TraceHistory {
    /// Trace records.
    records: VecDeque<TraceRecord>,
    /// Maximum capacity.
    capacity: usize,
}

impl TraceHistory {
    /// Create a new trace history with given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            records: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Add a trace record.
    pub fn push(&mut self, record: TraceRecord) {
        if self.records.len() >= self.capacity {
            self.records.pop_front();
        }
        self.records.push_back(record);
    }

    /// Get a mutable reference to a trace by ID.
    pub fn get_mut(&mut self, trace_id: TraceId) -> Option<&mut TraceRecord> {
        self.records.iter_mut().find(|r| r.trace_id == trace_id)
    }

    /// Get a trace by ID.
    pub fn get(&self, trace_id: TraceId) -> Option<&TraceRecord> {
        self.records.iter().find(|r| r.trace_id == trace_id)
    }

    /// Get all traces for a pipeline.
    pub fn by_pipeline(&self, pipeline_id: &str) -> Vec<&TraceRecord> {
        self.records
            .iter()
            .filter(|r| r.pipeline_id == pipeline_id)
            .collect()
    }

    /// Get all traces (most recent first).
    pub fn all(&self) -> impl Iterator<Item = &TraceRecord> {
        self.records.iter().rev()
    }

    /// Get the number of traces.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_history_capacity() {
        let mut history = TraceHistory::new(3);

        for i in 0..5 {
            history.push(TraceRecord::new(TraceId::new(), format!("pipeline_{}", i)));
        }

        assert_eq!(history.len(), 3);
    }

    #[test]
    fn trace_record_lifecycle() {
        let mut record = TraceRecord::new(TraceId::new(), "test".to_string());
        assert_eq!(record.status, TraceStatus::Running);

        record.complete();
        assert_eq!(record.status, TraceStatus::Completed);
        assert!(record.completed_at.is_some());
    }

    #[test]
    fn trace_record_failure() {
        let mut record = TraceRecord::new(TraceId::new(), "test".to_string());

        record.fail("test error".to_string());
        assert_eq!(record.status, TraceStatus::Failed);
        assert_eq!(record.error.as_deref(), Some("test error"));
    }

    #[test]
    fn trace_history_lookup() {
        let mut history = TraceHistory::new(10);
        let trace_id = TraceId::new();

        history.push(TraceRecord::new(trace_id, "test".to_string()));

        assert!(history.get(trace_id).is_some());
        assert!(history.get(TraceId::new()).is_none());
    }
}
