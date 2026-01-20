//! Command application logic.

use crate::command::ClusterCommand;
use xerv_core::traits::PipelineState;

use super::state::ClusterState;
use super::types::{
    ClusterNodeInfo, ClusterResponse, PipelineStateInfo, TraceStateInfo, TraceStatus,
};

/// Apply a command to the cluster state.
///
/// This function is the core of the state machine. It takes a command and
/// applies it to the state, returning a response indicating success or failure.
pub fn apply_command(state: &mut ClusterState, cmd: ClusterCommand) -> ClusterResponse {
    match cmd {
        ClusterCommand::StartTrace {
            trace_id,
            pipeline_id,
            started_at_ms,
            ..
        } => apply_start_trace(state, trace_id, pipeline_id, started_at_ms),

        ClusterCommand::CompleteNode {
            trace_id, node_id, ..
        } => apply_complete_node(state, trace_id, node_id),

        ClusterCommand::CompleteTrace {
            trace_id,
            completed_at_ms,
        } => apply_complete_trace(state, trace_id, completed_at_ms),

        ClusterCommand::FailTrace {
            trace_id,
            error,
            failed_at_ms,
            ..
        } => apply_fail_trace(state, trace_id, error, failed_at_ms),

        ClusterCommand::SuspendTrace {
            trace_id,
            suspended_at_node,
            ..
        } => apply_suspend_trace(state, trace_id, suspended_at_node),

        ClusterCommand::ResumeTrace { trace_id, .. } => apply_resume_trace(state, trace_id),

        ClusterCommand::DeployPipeline {
            pipeline_id,
            config,
            deployed_at_ms,
        } => apply_deploy_pipeline(state, pipeline_id, config, deployed_at_ms),

        ClusterCommand::StartPipeline { pipeline_id, .. } => {
            apply_pipeline_state_change(state, &pipeline_id, PipelineState::Running)
        }

        ClusterCommand::PausePipeline { pipeline_id, .. } => {
            apply_pipeline_state_change(state, &pipeline_id, PipelineState::Paused)
        }

        ClusterCommand::ResumePipeline { pipeline_id, .. } => {
            apply_resume_pipeline(state, &pipeline_id)
        }

        ClusterCommand::DrainPipeline { pipeline_id, .. } => {
            apply_pipeline_state_change(state, &pipeline_id, PipelineState::Draining)
        }

        ClusterCommand::StopPipeline { pipeline_id, .. } => {
            apply_pipeline_state_change(state, &pipeline_id, PipelineState::Stopped)
        }

        ClusterCommand::UndeployPipeline { pipeline_id, .. } => {
            apply_undeploy_pipeline(state, &pipeline_id)
        }

        ClusterCommand::RegisterNode {
            cluster_node_id,
            address,
            registered_at_ms,
        } => apply_register_node(state, cluster_node_id, address, registered_at_ms),

        ClusterCommand::DeregisterNode {
            cluster_node_id, ..
        } => apply_deregister_node(state, cluster_node_id),
    }
}

fn apply_start_trace(
    state: &mut ClusterState,
    trace_id: xerv_core::types::TraceId,
    pipeline_id: xerv_core::types::PipelineId,
    started_at_ms: u64,
) -> ClusterResponse {
    // Check pipeline exists and is running
    if let Some(pipeline) = state.pipelines.get_mut(&pipeline_id) {
        if pipeline.state != PipelineState::Running {
            return ClusterResponse::err(format!("Pipeline {} is not running", pipeline_id));
        }
        pipeline.active_traces += 1;
    } else {
        return ClusterResponse::err(format!("Pipeline {} not found", pipeline_id));
    }

    state.traces.insert(
        trace_id,
        TraceStateInfo {
            pipeline_id,
            status: TraceStatus::Running,
            completed_nodes: Vec::new(),
            current_node: None,
            started_at_ms,
            completed_at_ms: None,
            error: None,
        },
    );
    ClusterResponse::ok()
}

fn apply_complete_node(
    state: &mut ClusterState,
    trace_id: xerv_core::types::TraceId,
    node_id: xerv_core::types::NodeId,
) -> ClusterResponse {
    if let Some(trace) = state.traces.get_mut(&trace_id) {
        trace.completed_nodes.push(node_id);
        trace.current_node = None;
        ClusterResponse::ok()
    } else {
        ClusterResponse::err(format!("Trace {} not found", trace_id))
    }
}

fn apply_complete_trace(
    state: &mut ClusterState,
    trace_id: xerv_core::types::TraceId,
    completed_at_ms: u64,
) -> ClusterResponse {
    if let Some(trace) = state.traces.get_mut(&trace_id) {
        trace.status = TraceStatus::Completed;
        trace.completed_at_ms = Some(completed_at_ms);

        // Decrement active traces on pipeline
        if let Some(pipeline) = state.pipelines.get_mut(&trace.pipeline_id) {
            pipeline.active_traces = pipeline.active_traces.saturating_sub(1);
        }
        ClusterResponse::ok()
    } else {
        ClusterResponse::err(format!("Trace {} not found", trace_id))
    }
}

fn apply_fail_trace(
    state: &mut ClusterState,
    trace_id: xerv_core::types::TraceId,
    error: String,
    failed_at_ms: u64,
) -> ClusterResponse {
    if let Some(trace) = state.traces.get_mut(&trace_id) {
        trace.status = TraceStatus::Failed;
        trace.completed_at_ms = Some(failed_at_ms);
        trace.error = Some(error);

        // Decrement active traces on pipeline
        if let Some(pipeline) = state.pipelines.get_mut(&trace.pipeline_id) {
            pipeline.active_traces = pipeline.active_traces.saturating_sub(1);
        }
        ClusterResponse::ok()
    } else {
        ClusterResponse::err(format!("Trace {} not found", trace_id))
    }
}

fn apply_suspend_trace(
    state: &mut ClusterState,
    trace_id: xerv_core::types::TraceId,
    suspended_at_node: xerv_core::types::NodeId,
) -> ClusterResponse {
    if let Some(trace) = state.traces.get_mut(&trace_id) {
        trace.status = TraceStatus::Suspended;
        trace.current_node = Some(suspended_at_node);
        ClusterResponse::ok()
    } else {
        ClusterResponse::err(format!("Trace {} not found", trace_id))
    }
}

fn apply_resume_trace(
    state: &mut ClusterState,
    trace_id: xerv_core::types::TraceId,
) -> ClusterResponse {
    if let Some(trace) = state.traces.get_mut(&trace_id) {
        if trace.status != TraceStatus::Suspended {
            return ClusterResponse::err(format!("Trace {} is not suspended", trace_id));
        }
        trace.status = TraceStatus::Running;
        ClusterResponse::ok()
    } else {
        ClusterResponse::err(format!("Trace {} not found", trace_id))
    }
}

fn apply_deploy_pipeline(
    state: &mut ClusterState,
    pipeline_id: xerv_core::types::PipelineId,
    config: Vec<u8>,
    deployed_at_ms: u64,
) -> ClusterResponse {
    state.pipelines.insert(
        pipeline_id,
        PipelineStateInfo {
            state: PipelineState::Initializing,
            config,
            deployed_at_ms,
            active_traces: 0,
        },
    );
    ClusterResponse::ok()
}

fn apply_pipeline_state_change(
    state: &mut ClusterState,
    pipeline_id: &xerv_core::types::PipelineId,
    new_state: PipelineState,
) -> ClusterResponse {
    if let Some(pipeline) = state.pipelines.get_mut(pipeline_id) {
        pipeline.state = new_state;
        ClusterResponse::ok()
    } else {
        ClusterResponse::err(format!("Pipeline {} not found", pipeline_id))
    }
}

fn apply_resume_pipeline(
    state: &mut ClusterState,
    pipeline_id: &xerv_core::types::PipelineId,
) -> ClusterResponse {
    if let Some(pipeline) = state.pipelines.get_mut(pipeline_id) {
        if pipeline.state != PipelineState::Paused {
            return ClusterResponse::err(format!("Pipeline {} is not paused", pipeline_id));
        }
        pipeline.state = PipelineState::Running;
        ClusterResponse::ok()
    } else {
        ClusterResponse::err(format!("Pipeline {} not found", pipeline_id))
    }
}

fn apply_undeploy_pipeline(
    state: &mut ClusterState,
    pipeline_id: &xerv_core::types::PipelineId,
) -> ClusterResponse {
    if state.pipelines.remove(pipeline_id).is_some() {
        ClusterResponse::ok()
    } else {
        ClusterResponse::err(format!("Pipeline {} not found", pipeline_id))
    }
}

fn apply_register_node(
    state: &mut ClusterState,
    cluster_node_id: u64,
    address: String,
    registered_at_ms: u64,
) -> ClusterResponse {
    state.nodes.insert(
        cluster_node_id,
        ClusterNodeInfo {
            cluster_node_id,
            address,
            registered_at_ms,
        },
    );
    ClusterResponse::ok()
}

fn apply_deregister_node(state: &mut ClusterState, cluster_node_id: u64) -> ClusterResponse {
    if state.nodes.remove(&cluster_node_id).is_some() {
        ClusterResponse::ok()
    } else {
        ClusterResponse::err(format!("Node {} not found", cluster_node_id))
    }
}
