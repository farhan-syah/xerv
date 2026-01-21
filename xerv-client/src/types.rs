//! Type definitions for the XERV client.
//!
//! Re-exports shared types from xerv-core and defines client-specific response types.

use serde::{Deserialize, Serialize};

// Re-export core types
pub use xerv_core::types::{NodeId, TraceId};

/// Information about a deployed pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineInfo {
    /// Unique pipeline identifier.
    pub pipeline_id: String,
    /// Pipeline name.
    pub name: String,
    /// Pipeline version.
    pub version: String,
    /// Current pipeline status.
    pub status: PipelineStatus,
    /// Number of triggers.
    pub trigger_count: usize,
    /// Number of nodes.
    pub node_count: usize,
}

/// Pipeline lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStatus {
    /// Pipeline is deployed but not started.
    Deployed,
    /// Pipeline is running and accepting triggers.
    Running,
    /// Pipeline is paused.
    Paused,
    /// Pipeline is draining (no new triggers, finishing existing traces).
    Draining,
    /// Pipeline has stopped.
    Stopped,
}

/// Information about a trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerInfo {
    /// Trigger identifier.
    pub id: String,
    /// Trigger type (e.g., "webhook", "cron", "manual").
    pub trigger_type: String,
    /// Whether the trigger is enabled.
    pub enabled: bool,
}

/// Response from firing a trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerResponse {
    /// ID of the created trace.
    pub trace_id: TraceId,
    /// Pipeline ID.
    pub pipeline_id: String,
    /// Trigger ID that was fired.
    pub trigger_id: String,
}

/// Information about a trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceInfo {
    /// Trace identifier.
    pub trace_id: TraceId,
    /// Associated pipeline ID.
    pub pipeline_id: String,
    /// Current trace status.
    pub status: TraceStatus,
    /// ISO 8601 timestamp when trace started.
    pub started_at: String,
    /// ISO 8601 timestamp when trace completed (if finished).
    pub completed_at: Option<String>,
}

/// Detailed trace information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceDetail {
    /// Trace identifier.
    pub trace_id: TraceId,
    /// Associated pipeline ID.
    pub pipeline_id: String,
    /// Current trace status.
    pub status: TraceStatus,
    /// ISO 8601 timestamp when trace started.
    pub started_at: String,
    /// ISO 8601 timestamp when trace completed (if finished).
    pub completed_at: Option<String>,
    /// Number of nodes executed.
    pub nodes_executed: usize,
    /// Error message if trace failed.
    pub error: Option<String>,
}

/// Trace execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceStatus {
    /// Trace is queued for execution.
    Queued,
    /// Trace is currently running.
    Running,
    /// Trace completed successfully.
    Completed,
    /// Trace failed with an error.
    Failed,
    /// Trace is suspended awaiting external input.
    Suspended,
}

/// Health status of the XERV server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Server status (e.g., "healthy").
    pub status: String,
    /// Server version.
    pub version: String,
}

/// Log entry from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Log entry ID.
    pub id: u64,
    /// ISO 8601 timestamp.
    pub timestamp: String,
    /// Log level (trace, debug, info, warn, error).
    pub level: String,
    /// Log category.
    pub category: String,
    /// Log message.
    pub message: String,
    /// Associated trace ID (if any).
    pub trace_id: Option<TraceId>,
    /// Associated node ID (if any).
    pub node_id: Option<NodeId>,
    /// Associated pipeline ID (if any).
    pub pipeline_id: Option<String>,
    /// Additional structured fields.
    #[serde(default)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}
