//! Crash recovery and trace replay.
//!
//! This module provides crash recovery functionality using the WAL (Write-Ahead Log).
//! On startup, the `CrashReplayer` scans the WAL for incomplete traces and determines
//! the appropriate recovery action for each.
//!
//! # Recovery Actions
//!
//! - **ResumeFrom**: Resume execution from after the last completed node
//! - **RetryNodes**: Retry specific nodes that were in progress when the crash occurred
//! - **AwaitResume**: Trace was suspended (at a wait node), needs manual resume
//! - **Skip**: Trace cannot be recovered (terminal failure)

mod report;

pub use report::RecoveryReport;

use crate::scheduler::Executor;
use std::sync::Arc;
use xerv_core::error::{Result, XervError};
use xerv_core::types::{NodeId, TraceId};
use xerv_core::wal::{TraceRecoveryState, Wal, WalRecord};

/// Action to take for recovering a trace.
#[derive(Debug, Clone)]
pub enum RecoveryAction {
    /// Resume execution from after the last completed node.
    ResumeFrom {
        /// The node ID to resume from (next node after this will execute).
        node_id: NodeId,
    },
    /// Retry specific nodes that were in-progress when crash occurred.
    RetryNodes {
        /// List of node IDs to retry.
        nodes: Vec<NodeId>,
    },
    /// Trace was suspended at a wait node, needs manual resume.
    AwaitResume {
        /// The node ID where trace is suspended.
        suspended_at: NodeId,
    },
    /// Skip recovery due to terminal failure.
    Skip {
        /// Reason for skipping.
        reason: String,
    },
}

/// Crash recovery handler.
///
/// Scans the WAL for incomplete traces and determines recovery actions.
/// Works in conjunction with the `Executor` to resume trace execution.
pub struct CrashReplayer {
    wal: Arc<Wal>,
    executor: Arc<Executor>,
}

impl CrashReplayer {
    /// Create a new crash replayer.
    pub fn new(wal: Arc<Wal>, executor: Arc<Executor>) -> Self {
        Self { wal, executor }
    }

    /// Recover all incomplete traces on startup.
    ///
    /// This method should be called before starting any triggers to ensure
    /// crash recovery completes first.
    pub async fn recover_all(&self) -> Result<RecoveryReport> {
        let reader = self.wal.reader();
        let incomplete = reader.get_incomplete_traces()?;

        let mut report = RecoveryReport::new();

        for (trace_id, state) in incomplete {
            let action = self.determine_action(&state);

            match action {
                RecoveryAction::ResumeFrom { node_id } => {
                    match self.resume_from(trace_id, node_id, &state).await {
                        Ok(()) => {
                            report.add_recovered(trace_id);
                            tracing::info!(
                                trace_id = %trace_id,
                                from_node = %node_id,
                                completed_nodes = state.completed_nodes.len(),
                                "Trace recovered and resumed"
                            );
                        }
                        Err(e) => {
                            report.add_skipped(trace_id, format!("Resume failed: {}", e));
                            tracing::warn!(
                                trace_id = %trace_id,
                                error = %e,
                                "Failed to resume trace"
                            );
                        }
                    }
                }
                RecoveryAction::RetryNodes { nodes } => {
                    match self.retry_nodes(trace_id, &nodes, &state).await {
                        Ok(()) => {
                            report.add_recovered(trace_id);
                            tracing::info!(
                                trace_id = %trace_id,
                                retry_nodes = ?nodes,
                                completed_nodes = state.completed_nodes.len(),
                                "Trace recovered with node retry"
                            );
                        }
                        Err(e) => {
                            report.add_skipped(trace_id, format!("Retry failed: {}", e));
                            tracing::warn!(
                                trace_id = %trace_id,
                                error = %e,
                                "Failed to retry nodes"
                            );
                        }
                    }
                }
                RecoveryAction::AwaitResume { suspended_at } => {
                    report.add_awaiting_resume(trace_id);
                    tracing::info!(
                        trace_id = %trace_id,
                        suspended_at = %suspended_at,
                        "Trace awaiting manual resume"
                    );
                }
                RecoveryAction::Skip { reason } => {
                    report.add_skipped(trace_id, reason.clone());
                    // Log trace as failed in WAL
                    let record = WalRecord::trace_failed(trace_id, &reason);
                    if let Err(e) = self.wal.write(&record) {
                        tracing::error!(
                            trace_id = %trace_id,
                            error = %e,
                            "Failed to write skip record to WAL"
                        );
                    }
                    tracing::warn!(
                        trace_id = %trace_id,
                        reason = %reason,
                        "Trace skipped during recovery"
                    );
                }
            }
        }

        Ok(report)
    }

    /// Determine the recovery action for a trace based on its state.
    pub fn determine_action(&self, state: &TraceRecoveryState) -> RecoveryAction {
        // If trace was suspended at a wait node, don't auto-recover
        if let Some(suspended_at) = state.suspended_at {
            return RecoveryAction::AwaitResume { suspended_at };
        }

        // If nodes were mid-execution, retry them
        if !state.started_nodes.is_empty() {
            return RecoveryAction::RetryNodes {
                nodes: state.started_nodes.clone(),
            };
        }

        // Resume from after last completed node
        if let Some(last) = state.last_completed_node {
            return RecoveryAction::ResumeFrom { node_id: last };
        }

        // No progress made, restart from beginning (node 0)
        RecoveryAction::ResumeFrom {
            node_id: NodeId::new(0),
        }
    }

    /// Resume trace execution from a specific node.
    ///
    /// The trace will continue from the node after `node_id` in the execution order.
    async fn resume_from(
        &self,
        trace_id: TraceId,
        node_id: NodeId,
        state: &TraceRecoveryState,
    ) -> Result<()> {
        // Load the arena from disk
        let arena = self.load_arena(trace_id)?;

        tracing::debug!(
            trace_id = %trace_id,
            resume_from = %node_id,
            completed_nodes = state.completed_nodes.len(),
            "Loading arena and resuming trace"
        );

        // Resume execution via the executor
        self.executor
            .resume_trace(arena, state, Some(node_id))
            .await
    }

    /// Retry specific nodes that were in progress.
    async fn retry_nodes(
        &self,
        trace_id: TraceId,
        nodes: &[NodeId],
        state: &TraceRecoveryState,
    ) -> Result<()> {
        // Load the arena from disk
        let arena = self.load_arena(trace_id)?;

        tracing::debug!(
            trace_id = %trace_id,
            retry_nodes = ?nodes,
            completed_nodes = state.completed_nodes.len(),
            "Loading arena and retrying nodes"
        );

        // For retry, we create a modified state that excludes the nodes to retry
        // from the completed set, so they will be re-executed
        let mut modified_state = state.clone();
        for node_id in nodes {
            modified_state.completed_nodes.remove(node_id);
        }

        // Resume execution via the executor
        self.executor
            .resume_trace(arena, &modified_state, None)
            .await
    }

    /// Load the arena file for a trace from disk.
    fn load_arena(&self, trace_id: TraceId) -> Result<xerv_core::arena::Arena> {
        // The arena directory is typically the same as WAL directory's parent
        // or can be configured. For now, use a standard location.
        let arena_dir = std::path::PathBuf::from("/tmp/xerv");
        let arena_path = arena_dir.join(format!("trace_{}.bin", trace_id.as_uuid()));

        if !arena_path.exists() {
            return Err(XervError::ArenaCreate {
                path: arena_path,
                cause: "Arena file not found for recovery".to_string(),
            });
        }

        xerv_core::arena::Arena::open(&arena_path)
    }

    /// Get the recovery state for a specific trace.
    ///
    /// Useful for diagnostics and debugging.
    pub fn get_trace_state(&self, trace_id: TraceId) -> Result<Option<TraceRecoveryState>> {
        let reader = self.wal.reader();
        let incomplete = reader.get_incomplete_traces()?;
        Ok(incomplete.get(&trace_id).cloned())
    }

    /// Check if there are any incomplete traces.
    pub fn has_incomplete_traces(&self) -> Result<bool> {
        let reader = self.wal.reader();
        let incomplete = reader.get_incomplete_traces()?;
        Ok(!incomplete.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::{ExecutorConfig, FlowGraph, GraphNode};
    use std::collections::HashMap;
    use xerv_core::logging::BufferedCollector;
    use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput};
    use xerv_core::types::RelPtr;
    use xerv_core::wal::WalConfig;

    fn test_log_collector() -> Arc<BufferedCollector> {
        Arc::new(BufferedCollector::with_default_capacity())
    }

    struct PassthroughNode;

    impl Node for PassthroughNode {
        fn info(&self) -> NodeInfo {
            NodeInfo::new("test", "passthrough")
        }

        fn execute<'a>(
            &'a self,
            _ctx: Context,
            inputs: HashMap<String, RelPtr<()>>,
        ) -> NodeFuture<'a> {
            Box::pin(async move {
                let input = inputs.get("in").cloned().unwrap_or_else(RelPtr::null);
                Ok(NodeOutput::out(input))
            })
        }
    }

    #[tokio::test]
    async fn crash_replayer_no_incomplete_traces() {
        let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());

        // Create a minimal flow graph
        let mut graph = FlowGraph::new();
        graph.add_node(GraphNode::new(NodeId::new(0), "test::passthrough"));

        let mut nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();
        nodes.insert(NodeId::new(0), Box::new(PassthroughNode));

        let executor = Arc::new(
            Executor::new(
                ExecutorConfig::default(),
                graph,
                nodes,
                Arc::clone(&wal),
                test_log_collector(),
                Some("test_pipeline".to_string()),
            )
            .unwrap(),
        );

        let replayer = CrashReplayer::new(wal, executor);

        assert!(!replayer.has_incomplete_traces().unwrap());

        let report = replayer.recover_all().await.unwrap();
        assert_eq!(report.total_processed(), 0);
    }

    #[tokio::test]
    async fn determine_action_suspended() {
        let state = TraceRecoveryState {
            trace_id: TraceId::new(),
            last_completed_node: Some(NodeId::new(1)),
            suspended_at: Some(NodeId::new(2)),
            started_nodes: Vec::new(),
            completed_nodes: HashMap::new(),
        };

        let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
        let mut graph = FlowGraph::new();
        graph.add_node(GraphNode::new(NodeId::new(0), "test::passthrough"));
        let nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();
        let executor = Arc::new(
            Executor::new(
                ExecutorConfig::default(),
                graph,
                nodes,
                Arc::clone(&wal),
                test_log_collector(),
                Some("test_pipeline".to_string()),
            )
            .unwrap(),
        );

        let replayer = CrashReplayer::new(wal, executor);
        let action = replayer.determine_action(&state);

        match action {
            RecoveryAction::AwaitResume { suspended_at } => {
                assert_eq!(suspended_at, NodeId::new(2));
            }
            _ => panic!("Expected AwaitResume action"),
        }
    }

    #[tokio::test]
    async fn determine_action_retry_nodes() {
        let state = TraceRecoveryState {
            trace_id: TraceId::new(),
            last_completed_node: Some(NodeId::new(1)),
            suspended_at: None,
            started_nodes: vec![NodeId::new(2), NodeId::new(3)],
            completed_nodes: HashMap::new(),
        };

        let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
        let mut graph = FlowGraph::new();
        graph.add_node(GraphNode::new(NodeId::new(0), "test::passthrough"));
        let nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();
        let executor = Arc::new(
            Executor::new(
                ExecutorConfig::default(),
                graph,
                nodes,
                Arc::clone(&wal),
                test_log_collector(),
                Some("test_pipeline".to_string()),
            )
            .unwrap(),
        );

        let replayer = CrashReplayer::new(wal, executor);
        let action = replayer.determine_action(&state);

        match action {
            RecoveryAction::RetryNodes { nodes: retry_nodes } => {
                assert_eq!(retry_nodes, vec![NodeId::new(2), NodeId::new(3)]);
            }
            _ => panic!("Expected RetryNodes action"),
        }
    }

    #[tokio::test]
    async fn determine_action_resume_from() {
        let state = TraceRecoveryState {
            trace_id: TraceId::new(),
            last_completed_node: Some(NodeId::new(5)),
            suspended_at: None,
            started_nodes: Vec::new(),
            completed_nodes: HashMap::new(),
        };

        let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
        let mut graph = FlowGraph::new();
        graph.add_node(GraphNode::new(NodeId::new(0), "test::passthrough"));
        let nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();
        let executor = Arc::new(
            Executor::new(
                ExecutorConfig::default(),
                graph,
                nodes,
                Arc::clone(&wal),
                test_log_collector(),
                Some("test_pipeline".to_string()),
            )
            .unwrap(),
        );

        let replayer = CrashReplayer::new(wal, executor);
        let action = replayer.determine_action(&state);

        match action {
            RecoveryAction::ResumeFrom { node_id } => {
                assert_eq!(node_id, NodeId::new(5));
            }
            _ => panic!("Expected ResumeFrom action"),
        }
    }
}
