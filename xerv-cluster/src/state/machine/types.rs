//! Types for cluster state management.

use crate::types::ClusterSnapshotMeta;
use serde::{Deserialize, Serialize};
use xerv_core::traits::PipelineState;
use xerv_core::types::{NodeId, PipelineId};

/// Response from applying a command to the state machine.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClusterResponse {
    /// Whether the operation succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
    /// Additional data (serialized).
    pub data: Option<Vec<u8>>,
}

impl ClusterResponse {
    /// Create a success response.
    pub fn ok() -> Self {
        Self {
            success: true,
            error: None,
            data: None,
        }
    }

    /// Create an error response.
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(msg.into()),
            data: None,
        }
    }

    /// Create a success response with data.
    pub fn with_data(data: Vec<u8>) -> Self {
        Self {
            success: true,
            error: None,
            data: Some(data),
        }
    }
}

/// State of a trace in the cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStateInfo {
    /// Pipeline this trace belongs to.
    pub pipeline_id: PipelineId,
    /// Current status.
    pub status: TraceStatus,
    /// Nodes that have completed.
    pub completed_nodes: Vec<NodeId>,
    /// Node where trace is currently executing.
    pub current_node: Option<NodeId>,
    /// Start timestamp.
    pub started_at_ms: u64,
    /// Completion timestamp (if completed).
    pub completed_at_ms: Option<u64>,
    /// Error message (if failed).
    pub error: Option<String>,
}

/// Status of a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceStatus {
    /// Trace is running.
    Running,
    /// Trace is suspended (waiting for human input).
    Suspended,
    /// Trace completed successfully.
    Completed,
    /// Trace failed.
    Failed,
}

/// State of a pipeline in the cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStateInfo {
    /// Current pipeline state.
    pub state: PipelineState,
    /// Serialized pipeline configuration.
    pub config: Vec<u8>,
    /// Deployment timestamp.
    pub deployed_at_ms: u64,
    /// Active trace count.
    pub active_traces: u64,
}

/// Information about a cluster node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNodeInfo {
    /// Node's cluster ID.
    pub cluster_node_id: u64,
    /// Node's address.
    pub address: String,
    /// Registration timestamp.
    pub registered_at_ms: u64,
}

/// Stored snapshot data.
#[derive(Debug)]
pub struct StoredSnapshot {
    /// Snapshot metadata.
    pub meta: ClusterSnapshotMeta,
    /// Serialized state data.
    pub data: Vec<u8>,
}
