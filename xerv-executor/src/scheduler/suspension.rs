//! Suspension and resume logic for human-in-the-loop workflows.
//!
//! This module contains the methods for suspending and resuming traces when nodes
//! request human approval (e.g., WaitNode).

use super::Executor;
use crate::suspension::{ResumeDecision, SuspendedTraceState};
use crate::trace::TraceState;
use std::collections::HashMap;
use tracing::instrument;
use xerv_core::arena::Arena;
use xerv_core::error::{Result, XervError};
use xerv_core::logging::{LogCategory, LogCollector, LogEvent};
use xerv_core::traits::NodeOutput;
use xerv_core::types::{ArenaOffset, NodeId, RelPtr, TraceId};
use xerv_core::wal::WalRecord;

impl Executor {
    /// Suspend a trace for human-in-the-loop approval.
    ///
    /// This is called when a node returns `NodeOutput::Suspend`. The trace
    /// will be persisted to the suspension store and can be resumed via
    /// `resume_suspended_trace()`.
    #[instrument(
        skip(self, request),
        fields(
            otel.kind = "internal",
            hook_id = %request.hook_id,
            pending_size = %pending_size,
        )
    )]
    pub(crate) async fn suspend_trace(
        &self,
        trace_id: TraceId,
        node_id: NodeId,
        request: xerv_core::suspension::SuspensionRequest,
        pending_offset: ArenaOffset,
        pending_size: u32,
    ) -> Result<()> {
        // Get the suspension store
        let store = self
            .suspension_store
            .as_ref()
            .ok_or_else(|| XervError::SuspensionFailed {
                trace_id,
                cause: "No suspension store configured".to_string(),
            })?;

        // Get the trace and flush its arena
        let trace = self
            .traces
            .get(&trace_id)
            .ok_or_else(|| XervError::NodeExecution {
                node_id,
                trace_id,
                cause: "Trace not found during suspension".to_string(),
            })?;

        // Flush arena to disk
        trace.flush_arena()?;

        // Get arena path
        let arena_path = trace.arena_path();

        // Get pipeline ID
        let pipeline_id = self
            .pipeline_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        // Create suspended trace state
        let mut suspended = SuspendedTraceState::new(
            &request.hook_id,
            trace_id,
            node_id,
            arena_path.clone(),
            &pipeline_id,
        )
        .with_metadata(request.metadata.clone());

        // Apply timeout if configured
        if let Some(timeout_secs) = request.timeout_secs {
            suspended = suspended.with_timeout(timeout_secs, request.timeout_action);
        }

        // Store in suspension store
        store.suspend(suspended)?;

        // Write WAL record
        let metadata = serde_json::json!({
            "hook_id": request.hook_id,
            "pending_data_offset": pending_offset.as_u64(),
            "pending_data_size": pending_size,
            "timeout_secs": request.timeout_secs,
            "timeout_action": format!("{:?}", request.timeout_action),
        });

        let record =
            WalRecord::trace_suspended(trace_id, node_id).with_metadata(metadata.to_string());
        self.wal.write(&record)?;

        // Log suspension
        let mut suspend_event = LogEvent::info(LogCategory::Trace, "Trace suspended")
            .with_trace_id(trace_id)
            .with_node_id(node_id)
            .with_field("hook_id", &request.hook_id);
        if let Some(ref pid) = self.pipeline_id {
            suspend_event = suspend_event.with_pipeline_id(pid);
        }
        self.log_collector.collect(suspend_event);

        tracing::info!(
            trace_id = %trace_id,
            node_id = %node_id,
            hook_id = %request.hook_id,
            arena_path = %arena_path.display(),
            "Trace suspended, awaiting external resume"
        );

        // Drop trace guard and remove from active traces
        drop(trace);
        self.traces.remove(&trace_id);

        Ok(())
    }

    /// Resume a suspended trace.
    ///
    /// This is called when the external API receives a resume signal.
    /// The trace will be restored from the suspension store and execution
    /// will continue from the wait node.
    #[instrument(
        skip(self, decision),
        fields(
            otel.kind = "server",
            hook_id = %hook_id,
            decision_type = ?std::mem::discriminant(&decision),
        )
    )]
    pub async fn resume_suspended_trace(
        &self,
        hook_id: &str,
        decision: ResumeDecision,
    ) -> Result<TraceId> {
        // Get the suspension store
        let store = self
            .suspension_store
            .as_ref()
            .ok_or_else(|| XervError::HookNotFound {
                hook_id: hook_id.to_string(),
            })?;

        // Get the output port based on decision
        let output_port = decision.output_port().to_string();
        let decision_str = format!("{:?}", decision);

        // Resume the trace (this removes it from the store)
        let suspended = store.resume(hook_id, decision.clone())?;

        let trace_id = suspended.trace_id;
        let node_id = suspended.suspended_at;

        tracing::info!(
            trace_id = %trace_id,
            hook_id = %hook_id,
            decision = %decision_str,
            output_port = %output_port,
            "Resuming suspended trace"
        );

        // Open the arena from disk
        let arena = Arena::open(&suspended.arena_path).map_err(|e| XervError::ArenaCreate {
            path: suspended.arena_path.clone(),
            cause: format!("Failed to reopen arena for suspended trace: {}", e),
        })?;

        // Read WAL to get completed nodes
        let reader = self.wal.reader();
        let incomplete = reader.get_incomplete_traces()?;

        // Build completed outputs map from WAL
        let mut completed_outputs: HashMap<NodeId, NodeOutput> = HashMap::new();

        if let Some(recovery_state) = incomplete.get(&trace_id) {
            for (&nid, loc) in &recovery_state.completed_nodes {
                let ptr = RelPtr::new(loc.offset, loc.size);
                completed_outputs.insert(
                    nid,
                    NodeOutput::Complete {
                        port: "out".to_string(),
                        data: ptr,
                        schema_hash: loc.schema_hash,
                    },
                );
            }
        }

        // Create output for the wait node based on the decision
        let decision_data = match &decision {
            ResumeDecision::Approve { response_data } => response_data.clone(),
            ResumeDecision::Reject { reason } => Some(serde_json::json!({
                "rejected": true,
                "reason": reason,
            })),
            ResumeDecision::Escalate { details } => Some(serde_json::json!({
                "escalated": true,
                "details": details,
            })),
        };

        // Write decision data to arena and create wait node output
        let data_ptr = if let Some(data) = decision_data {
            let bytes = serde_json::to_vec(&data).map_err(|e| {
                XervError::Serialization(format!("Failed to serialize decision data: {}", e))
            })?;
            arena.write_bytes::<()>(&bytes)?
        } else {
            RelPtr::null()
        };

        completed_outputs.insert(
            node_id,
            NodeOutput::Complete {
                port: output_port,
                data: data_ptr,
                schema_hash: 0,
            },
        );

        // Reconstruct trace state
        let trace_state = TraceState::from_recovery(
            trace_id,
            arena,
            suspended.pipeline_id.clone(),
            RelPtr::null(),
            completed_outputs,
        );

        self.traces.insert(trace_id, trace_state);

        // Write WAL resume record
        let record = WalRecord::trace_resumed(trace_id);
        self.wal.write(&record)?;

        // Log resume
        let mut resume_event = LogEvent::info(LogCategory::Trace, "Trace resumed from suspension")
            .with_trace_id(trace_id)
            .with_node_id(node_id)
            .with_field("hook_id", hook_id)
            .with_field("decision", decision_str);
        if let Some(ref pid) = self.pipeline_id {
            resume_event = resume_event.with_pipeline_id(pid);
        }
        self.log_collector.collect(resume_event);

        // Continue execution
        self.execute_trace(trace_id).await?;

        Ok(trace_id)
    }

    /// Resume a trace from crash recovery.
    ///
    /// This method is called by the CrashReplayer to continue execution of a
    /// trace that was interrupted. It reconstructs the trace state from the
    /// recovered arena and WAL information, then continues execution.
    ///
    /// # Arguments
    /// * `arena` - The arena loaded from disk
    /// * `recovery_state` - State reconstructed from WAL records
    /// * `resume_from` - The node to resume from (or None to continue from last completed)
    #[instrument(
        skip(self, arena, recovery_state),
        fields(
            otel.kind = "internal",
            trace_id = %recovery_state.trace_id,
            completed_nodes = %recovery_state.completed_nodes.len(),
            resume_from = ?resume_from,
        )
    )]
    pub async fn resume_trace(
        &self,
        arena: xerv_core::arena::Arena,
        recovery_state: &xerv_core::wal::TraceRecoveryState,
        resume_from: Option<NodeId>,
    ) -> Result<()> {
        let trace_id = recovery_state.trace_id;

        // Check concurrency limits
        if self.traces.len() >= self.config.max_concurrent_traces {
            return Err(XervError::ConcurrencyLimit {
                pipeline_id: self
                    .pipeline_id
                    .clone()
                    .unwrap_or_else(|| "executor".to_string()),
                current: self.traces.len() as u32,
                max: self.config.max_concurrent_traces as u32,
            });
        }

        // Reconstruct NodeOutput from the stored locations
        let mut completed_outputs: HashMap<NodeId, NodeOutput> = HashMap::new();
        for (&node_id, loc) in &recovery_state.completed_nodes {
            // Create a RelPtr from the stored offset/size
            let ptr = RelPtr::new(loc.offset, loc.size);
            completed_outputs.insert(
                node_id,
                NodeOutput::Complete {
                    port: "out".to_string(), // Default port; actual port would need WAL enhancement
                    data: ptr,
                    schema_hash: loc.schema_hash,
                },
            );
        }

        // Create trace state from recovery
        let trace_state = TraceState::from_recovery(
            trace_id,
            arena,
            "recovered".to_string(), // Trigger ID not stored in WAL; could be enhanced
            RelPtr::null(),
            completed_outputs,
        );

        self.traces.insert(trace_id, trace_state);

        // Log recovery to WAL
        let record = WalRecord::trace_resumed(trace_id);
        self.wal.write(&record)?;

        // Log trace recovery to collector
        let mut resume_event = LogEvent::info(LogCategory::Trace, "Trace resumed from recovery")
            .with_trace_id(trace_id)
            .with_field_i64(
                "completed_nodes",
                recovery_state.completed_nodes.len() as i64,
            );
        if let Some(ref pid) = self.pipeline_id {
            resume_event = resume_event.with_pipeline_id(pid);
        }
        self.log_collector.collect(resume_event);

        tracing::info!(
            trace_id = %trace_id,
            resume_from = ?resume_from,
            completed_nodes = recovery_state.completed_nodes.len(),
            "Resuming trace from recovery"
        );

        // Execute remaining nodes
        self.execute_trace(trace_id).await
    }
}
