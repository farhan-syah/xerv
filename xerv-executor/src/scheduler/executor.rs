//! Trace execution engine.

use super::graph::FlowGraph;
use crate::trace::TraceState;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use xerv_core::arena::{Arena, ArenaConfig};
use xerv_core::error::{Result, XervError};
use xerv_core::logging::{BufferedCollector, LogCategory, LogCollector, LogEvent};
use xerv_core::traits::{Context, Node, NodeOutput, TriggerEvent};
use xerv_core::types::{NodeId, RelPtr, TraceId};
use xerv_core::wal::{Wal, WalConfig, WalRecord};

/// Message sent to signal a node execution.
#[derive(Debug)]
pub struct ExecutionSignal {
    /// Trace ID.
    pub trace_id: TraceId,
    /// Node to execute.
    pub node_id: NodeId,
    /// Input pointers from upstream nodes (port -> pointer).
    pub inputs: HashMap<String, RelPtr<()>>,
}

/// Configuration for the executor.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// Maximum concurrent traces.
    pub max_concurrent_traces: usize,
    /// Timeout per node execution in milliseconds.
    pub node_timeout_ms: u64,
    /// Arena configuration.
    pub arena_config: ArenaConfig,
    /// WAL configuration.
    pub wal_config: WalConfig,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_traces: 100,
            node_timeout_ms: 30_000,
            arena_config: ArenaConfig::default(),
            wal_config: WalConfig::default(),
        }
    }
}

/// The main execution engine.
pub struct Executor {
    /// Configuration.
    config: ExecutorConfig,
    /// The flow graph.
    graph: Arc<FlowGraph>,
    /// Node implementations.
    nodes: Arc<HashMap<NodeId, Box<dyn Node>>>,
    /// Active traces.
    traces: Arc<DashMap<TraceId, TraceState>>,
    /// The WAL for durability.
    wal: Arc<Wal>,
    /// Execution order from topological sort.
    execution_order: Vec<NodeId>,
    /// Log collector for structured logging with correlation IDs.
    log_collector: Arc<BufferedCollector>,
    /// Pipeline ID for logging context.
    pipeline_id: Option<String>,
}

impl Executor {
    /// Create a new executor.
    pub fn new(
        config: ExecutorConfig,
        graph: FlowGraph,
        nodes: HashMap<NodeId, Box<dyn Node>>,
        wal: Arc<Wal>,
        log_collector: Arc<BufferedCollector>,
        pipeline_id: Option<String>,
    ) -> Result<Self> {
        // Validate graph and compute execution order
        graph.validate()?;
        let execution_order = graph.topological_sort()?;

        Ok(Self {
            config,
            graph: Arc::new(graph),
            nodes: Arc::new(nodes),
            traces: Arc::new(DashMap::new()),
            wal,
            execution_order,
            log_collector,
            pipeline_id,
        })
    }

    /// Start a new trace from a trigger event.
    pub async fn start_trace(&self, event: TriggerEvent) -> Result<TraceId> {
        let trace_id = event.trace_id;

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

        // Create arena for this trace
        let arena = Arena::create(trace_id, &self.config.arena_config)?;

        // Initialize trace state
        let trigger_id = event.trigger_id.clone();
        let trace_state = TraceState::new(trace_id, arena, event.trigger_id, event.data);
        self.traces.insert(trace_id, trace_state);

        // Log to WAL
        let record = WalRecord::trace_start(trace_id);
        self.wal.write(&record)?;

        // Log to collector with correlation IDs
        let mut log_event = LogEvent::info(LogCategory::Trace, "Trace started")
            .with_trace_id(trace_id)
            .with_field("trigger_id", trigger_id);
        if let Some(ref pid) = self.pipeline_id {
            log_event = log_event.with_pipeline_id(pid);
        }
        self.log_collector.collect(log_event);

        tracing::info!(trace_id = %trace_id, "Started trace");

        Ok(trace_id)
    }

    /// Execute a trace to completion.
    pub async fn execute_trace(&self, trace_id: TraceId) -> Result<()> {
        // Get mutable access to trace state
        let mut trace = self
            .traces
            .get_mut(&trace_id)
            .ok_or_else(|| XervError::NodeExecution {
                node_id: NodeId::new(0),
                trace_id,
                cause: "Trace not found".to_string(),
            })?;

        // Execute nodes in topological order
        for &node_id in &self.execution_order {
            // Skip nodes that haven't been activated
            if !trace.is_node_ready(node_id, &self.graph) {
                continue;
            }

            // Get the node implementation
            let node = self
                .nodes
                .get(&node_id)
                .ok_or_else(|| XervError::NodeNotFound {
                    node_name: format!("{}", node_id),
                })?;

            // Collect inputs from upstream nodes
            let inputs = trace.collect_inputs(node_id, &self.graph);

            // Create execution context
            let (reader, writer) = trace.arena_handles();
            let ctx = Context::new(trace_id, node_id, reader, writer, Arc::clone(&self.wal));

            // Log node execution start
            let node_info = node.info();
            let mut start_event = LogEvent::debug(
                LogCategory::Node,
                format!("Executing node: {}", node_info.name),
            )
            .with_trace_id(trace_id)
            .with_node_id(node_id)
            .with_field("namespace", &node_info.namespace);
            if let Some(ref pid) = self.pipeline_id {
                start_event = start_event.with_pipeline_id(pid);
            }
            self.log_collector.collect(start_event);

            tracing::debug!(
                trace_id = %trace_id,
                node_id = %node_id,
                "Executing node"
            );

            let result = tokio::time::timeout(
                std::time::Duration::from_millis(self.config.node_timeout_ms),
                node.execute(ctx, inputs),
            )
            .await;

            match result {
                Ok(Ok(output)) => {
                    // Get arena location before moving output
                    let (offset, size) = output.arena_location();
                    let schema_hash = output.schema_hash;

                    // Store output and update trace state
                    trace.record_output(node_id, output);

                    // Log to WAL with actual output location
                    let record = WalRecord::node_done(trace_id, node_id, offset, size, schema_hash);
                    self.wal.write(&record)?;

                    // Log node completion
                    let mut done_event = LogEvent::info(
                        LogCategory::Node,
                        format!("Node completed: {}", node_info.name),
                    )
                    .with_trace_id(trace_id)
                    .with_node_id(node_id)
                    .with_field("output_size", size.to_string());
                    if let Some(ref pid) = self.pipeline_id {
                        done_event = done_event.with_pipeline_id(pid);
                    }
                    self.log_collector.collect(done_event);

                    tracing::debug!(
                        trace_id = %trace_id,
                        node_id = %node_id,
                        "Node completed"
                    );
                }
                Ok(Err(e)) => {
                    // Node execution error
                    let record = WalRecord::node_error(trace_id, node_id, e.to_string());
                    self.wal.write(&record)?;

                    // Log node error
                    let mut error_event = LogEvent::error(
                        LogCategory::Node,
                        format!("Node failed: {}", node_info.name),
                    )
                    .with_trace_id(trace_id)
                    .with_node_id(node_id)
                    .with_field("error", e.to_string());
                    if let Some(ref pid) = self.pipeline_id {
                        error_event = error_event.with_pipeline_id(pid);
                    }
                    self.log_collector.collect(error_event);

                    tracing::error!(
                        trace_id = %trace_id,
                        node_id = %node_id,
                        error = %e,
                        "Node failed"
                    );

                    return Err(XervError::NodeExecution {
                        node_id,
                        trace_id,
                        cause: e.to_string(),
                    });
                }
                Err(_) => {
                    // Timeout - log error
                    let record = WalRecord::node_error(trace_id, node_id, "Timeout");
                    self.wal.write(&record)?;

                    let mut timeout_event = LogEvent::error(
                        LogCategory::Node,
                        format!("Node timeout: {}", node_info.name),
                    )
                    .with_trace_id(trace_id)
                    .with_node_id(node_id)
                    .with_field_i64("timeout_ms", self.config.node_timeout_ms as i64);
                    if let Some(ref pid) = self.pipeline_id {
                        timeout_event = timeout_event.with_pipeline_id(pid);
                    }
                    self.log_collector.collect(timeout_event);

                    return Err(XervError::NodeTimeout {
                        node_id,
                        trace_id,
                        timeout_ms: self.config.node_timeout_ms,
                    });
                }
            }
        }

        // Mark trace as complete
        let record = WalRecord::trace_complete(trace_id);
        self.wal.write(&record)?;

        // Log trace completion
        let mut complete_event =
            LogEvent::info(LogCategory::Trace, "Trace completed").with_trace_id(trace_id);
        if let Some(ref pid) = self.pipeline_id {
            complete_event = complete_event.with_pipeline_id(pid);
        }
        self.log_collector.collect(complete_event);

        tracing::info!(trace_id = %trace_id, "Trace completed");

        // Remove trace from active set
        self.traces.remove(&trace_id);

        Ok(())
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
        let mut completed_outputs: HashMap<NodeId, xerv_core::traits::NodeOutput> = HashMap::new();
        for (&node_id, loc) in &recovery_state.completed_nodes {
            // Create a RelPtr from the stored offset/size
            let ptr = RelPtr::new(loc.offset, loc.size);
            completed_outputs.insert(
                node_id,
                NodeOutput {
                    port: "out".to_string(), // Default port; actual port would need WAL enhancement
                    data: ptr,
                    schema_hash: loc.schema_hash,
                    error_message: None, // Recovery only handles successful completions
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

    /// Get the number of active traces.
    pub fn active_trace_count(&self) -> usize {
        self.traces.len()
    }

    /// Get execution order for debugging.
    pub fn execution_order(&self) -> &[NodeId] {
        &self.execution_order
    }

    /// Shutdown the executor.
    pub async fn shutdown(&self) {
        // Notify all nodes of shutdown
        for node in self.nodes.values() {
            node.shutdown();
        }

        tracing::info!("Executor shutdown complete");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::graph::{Edge, GraphNode};
    use std::collections::HashMap;
    use xerv_core::traits::{NodeFuture, NodeInfo};

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
    async fn executor_creation() {
        let mut graph = FlowGraph::new();
        graph.add_node(GraphNode::new(NodeId::new(0), "trigger::memory"));
        graph.add_node(GraphNode::new(NodeId::new(1), "test::passthrough"));
        graph.add_edge(Edge::new(NodeId::new(0), "out", NodeId::new(1), "in"));

        let mut nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();
        nodes.insert(NodeId::new(0), Box::new(PassthroughNode));
        nodes.insert(NodeId::new(1), Box::new(PassthroughNode));

        let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
        let log_collector = Arc::new(BufferedCollector::with_default_capacity());

        let executor = Executor::new(
            ExecutorConfig::default(),
            graph,
            nodes,
            wal,
            log_collector,
            Some("test_pipeline".to_string()),
        );
        assert!(executor.is_ok());
    }
}
