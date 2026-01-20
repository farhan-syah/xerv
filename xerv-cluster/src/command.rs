//! Cluster commands - operations that go through Raft consensus.
//!
//! These commands represent all state mutations that need to be replicated
//! across the cluster. Each command is serialized into the Raft log and
//! applied to all nodes' state machines in the same order.

use serde::{Deserialize, Serialize};
use xerv_core::types::{NodeId, PipelineId, TraceId};

/// Commands that are replicated through Raft consensus.
///
/// Every state mutation in the cluster goes through this enum.
/// Commands are serialized to the Raft log and applied deterministically
/// on all nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClusterCommand {
    // ==================== Trace Operations ====================
    /// Start a new trace execution.
    StartTrace {
        /// Unique trace identifier.
        trace_id: TraceId,
        /// Pipeline this trace belongs to.
        pipeline_id: PipelineId,
        /// Serialized trigger event data.
        trigger_data: Vec<u8>,
        /// Timestamp when trace was initiated.
        started_at_ms: u64,
    },

    /// Mark a node within a trace as completed.
    CompleteNode {
        /// Trace identifier.
        trace_id: TraceId,
        /// Node that completed.
        node_id: NodeId,
        /// Arena offset where output was written.
        output_offset: u64,
        /// Execution duration in microseconds.
        duration_us: u64,
    },

    /// Mark a trace as successfully completed.
    CompleteTrace {
        /// Trace identifier.
        trace_id: TraceId,
        /// Completion timestamp.
        completed_at_ms: u64,
    },

    /// Mark a trace as failed.
    FailTrace {
        /// Trace identifier.
        trace_id: TraceId,
        /// Node where failure occurred (if applicable).
        failed_node: Option<NodeId>,
        /// Error message.
        error: String,
        /// Failure timestamp.
        failed_at_ms: u64,
    },

    /// Suspend a trace (human-in-the-loop).
    SuspendTrace {
        /// Trace identifier.
        trace_id: TraceId,
        /// Node where suspension occurred.
        suspended_at_node: NodeId,
        /// Reason for suspension.
        reason: String,
        /// Suspension timestamp.
        suspended_at_ms: u64,
    },

    /// Resume a suspended trace.
    ResumeTrace {
        /// Trace identifier.
        trace_id: TraceId,
        /// Approval data (if applicable).
        approval_data: Option<Vec<u8>>,
        /// Resume timestamp.
        resumed_at_ms: u64,
    },

    // ==================== Pipeline Operations ====================
    /// Deploy a new pipeline.
    DeployPipeline {
        /// Pipeline identifier.
        pipeline_id: PipelineId,
        /// Serialized pipeline configuration.
        config: Vec<u8>,
        /// Deployment timestamp.
        deployed_at_ms: u64,
    },

    /// Start a pipeline (begin accepting triggers).
    StartPipeline {
        /// Pipeline identifier.
        pipeline_id: PipelineId,
        /// Start timestamp.
        started_at_ms: u64,
    },

    /// Pause a pipeline (stop accepting new triggers).
    PausePipeline {
        /// Pipeline identifier.
        pipeline_id: PipelineId,
        /// Pause timestamp.
        paused_at_ms: u64,
    },

    /// Resume a paused pipeline.
    ResumePipeline {
        /// Pipeline identifier.
        pipeline_id: PipelineId,
        /// Resume timestamp.
        resumed_at_ms: u64,
    },

    /// Drain a pipeline (stop new triggers, wait for in-flight).
    DrainPipeline {
        /// Pipeline identifier.
        pipeline_id: PipelineId,
        /// Drain initiation timestamp.
        drain_at_ms: u64,
    },

    /// Stop a pipeline.
    StopPipeline {
        /// Pipeline identifier.
        pipeline_id: PipelineId,
        /// Stop timestamp.
        stopped_at_ms: u64,
    },

    /// Undeploy a pipeline (remove from cluster).
    UndeployPipeline {
        /// Pipeline identifier.
        pipeline_id: PipelineId,
        /// Undeploy timestamp.
        undeployed_at_ms: u64,
    },

    // ==================== Cluster Management ====================
    /// Register a new node joining the cluster.
    RegisterNode {
        /// Node's cluster ID.
        cluster_node_id: u64,
        /// Node's address for RPC.
        address: String,
        /// Registration timestamp.
        registered_at_ms: u64,
    },

    /// Deregister a node leaving the cluster.
    DeregisterNode {
        /// Node's cluster ID.
        cluster_node_id: u64,
        /// Deregistration timestamp.
        deregistered_at_ms: u64,
    },
}

impl ClusterCommand {
    /// Get a human-readable name for this command type.
    pub fn name(&self) -> &'static str {
        match self {
            ClusterCommand::StartTrace { .. } => "StartTrace",
            ClusterCommand::CompleteNode { .. } => "CompleteNode",
            ClusterCommand::CompleteTrace { .. } => "CompleteTrace",
            ClusterCommand::FailTrace { .. } => "FailTrace",
            ClusterCommand::SuspendTrace { .. } => "SuspendTrace",
            ClusterCommand::ResumeTrace { .. } => "ResumeTrace",
            ClusterCommand::DeployPipeline { .. } => "DeployPipeline",
            ClusterCommand::StartPipeline { .. } => "StartPipeline",
            ClusterCommand::PausePipeline { .. } => "PausePipeline",
            ClusterCommand::ResumePipeline { .. } => "ResumePipeline",
            ClusterCommand::DrainPipeline { .. } => "DrainPipeline",
            ClusterCommand::StopPipeline { .. } => "StopPipeline",
            ClusterCommand::UndeployPipeline { .. } => "UndeployPipeline",
            ClusterCommand::RegisterNode { .. } => "RegisterNode",
            ClusterCommand::DeregisterNode { .. } => "DeregisterNode",
        }
    }

    /// Get the trace ID if this command is trace-related.
    pub fn trace_id(&self) -> Option<&TraceId> {
        match self {
            ClusterCommand::StartTrace { trace_id, .. }
            | ClusterCommand::CompleteNode { trace_id, .. }
            | ClusterCommand::CompleteTrace { trace_id, .. }
            | ClusterCommand::FailTrace { trace_id, .. }
            | ClusterCommand::SuspendTrace { trace_id, .. }
            | ClusterCommand::ResumeTrace { trace_id, .. } => Some(trace_id),
            _ => None,
        }
    }

    /// Get the pipeline ID if this command is pipeline-related.
    pub fn pipeline_id(&self) -> Option<&PipelineId> {
        match self {
            ClusterCommand::StartTrace { pipeline_id, .. }
            | ClusterCommand::DeployPipeline { pipeline_id, .. }
            | ClusterCommand::StartPipeline { pipeline_id, .. }
            | ClusterCommand::PausePipeline { pipeline_id, .. }
            | ClusterCommand::ResumePipeline { pipeline_id, .. }
            | ClusterCommand::DrainPipeline { pipeline_id, .. }
            | ClusterCommand::StopPipeline { pipeline_id, .. }
            | ClusterCommand::UndeployPipeline { pipeline_id, .. } => Some(pipeline_id),
            _ => None,
        }
    }
}
