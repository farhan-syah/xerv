//! Trace execution engine.
//!
//! This module provides the core execution engine for XERV traces.
//! All public methods are instrumented with tracing spans for distributed tracing.

use super::graph::FlowGraph;
use crate::suspension::SuspensionStore;
use crate::trace::TraceState;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::instrument;
use xerv_core::arena::{Arena, ArenaConfig};
use xerv_core::dispatch::{DispatchBackend, DispatchConfig, MemoryDispatch};
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
    /// Dispatch backend configuration.
    pub dispatch_config: DispatchConfig,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_traces: 100,
            node_timeout_ms: 30_000,
            arena_config: ArenaConfig::default(),
            wal_config: WalConfig::default(),
            dispatch_config: DispatchConfig::default(),
        }
    }
}

impl ExecutorConfig {
    /// Set the dispatch configuration.
    pub fn with_dispatch(mut self, config: DispatchConfig) -> Self {
        self.dispatch_config = config;
        self
    }
}

/// The main execution engine.
pub struct Executor {
    /// Configuration.
    pub(crate) config: ExecutorConfig,
    /// The flow graph.
    pub(crate) graph: Arc<FlowGraph>,
    /// Node implementations.
    pub(crate) nodes: Arc<HashMap<NodeId, Box<dyn Node>>>,
    /// Active traces.
    pub(crate) traces: Arc<DashMap<TraceId, TraceState>>,
    /// The WAL for durability.
    pub(crate) wal: Arc<Wal>,
    /// Execution order from topological sort.
    pub(crate) execution_order: Vec<NodeId>,
    /// Log collector for structured logging with correlation IDs.
    pub(crate) log_collector: Arc<BufferedCollector>,
    /// Pipeline ID for logging context.
    pub(crate) pipeline_id: Option<String>,
    /// Suspension store for human-in-the-loop workflows.
    pub(crate) suspension_store: Option<Arc<dyn SuspensionStore>>,
    /// Dispatch backend for trace queueing (pluggable: Memory, Raft, Redis, NATS).
    pub(crate) dispatch: Arc<dyn DispatchBackend>,
}

impl Executor {
    /// Create a new executor.
    ///
    /// Uses in-memory dispatch by default. Call `set_dispatch` to use a different backend.
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

        // Create default in-memory dispatch backend
        let dispatch: Arc<dyn DispatchBackend> =
            Arc::new(MemoryDispatch::new(config.dispatch_config.memory_config()));

        Ok(Self {
            config,
            graph: Arc::new(graph),
            nodes: Arc::new(nodes),
            traces: Arc::new(DashMap::new()),
            wal,
            execution_order,
            log_collector,
            pipeline_id,
            suspension_store: None,
            dispatch,
        })
    }

    /// Create executor with a suspension store for human-in-the-loop workflows.
    pub fn with_suspension_store(
        config: ExecutorConfig,
        graph: FlowGraph,
        nodes: HashMap<NodeId, Box<dyn Node>>,
        wal: Arc<Wal>,
        log_collector: Arc<BufferedCollector>,
        pipeline_id: Option<String>,
        suspension_store: Arc<dyn SuspensionStore>,
    ) -> Result<Self> {
        // Validate graph and compute execution order
        graph.validate()?;
        let execution_order = graph.topological_sort()?;

        // Create default in-memory dispatch backend
        let dispatch: Arc<dyn DispatchBackend> =
            Arc::new(MemoryDispatch::new(config.dispatch_config.memory_config()));

        Ok(Self {
            config,
            graph: Arc::new(graph),
            nodes: Arc::new(nodes),
            traces: Arc::new(DashMap::new()),
            wal,
            execution_order,
            log_collector,
            pipeline_id,
            suspension_store: Some(suspension_store),
            dispatch,
        })
    }

    /// Set the suspension store after construction.
    pub fn set_suspension_store(&mut self, store: Arc<dyn SuspensionStore>) {
        self.suspension_store = Some(store);
    }

    /// Set a custom dispatch backend.
    ///
    /// Use this to switch from the default in-memory dispatch to Redis, NATS, or Raft.
    /// Must be called before any traces are started.
    pub fn set_dispatch(&mut self, dispatch: Arc<dyn DispatchBackend>) {
        self.dispatch = dispatch;
    }

    /// Get a reference to the dispatch backend.
    pub fn dispatch(&self) -> &Arc<dyn DispatchBackend> {
        &self.dispatch
    }

    /// Start a new trace from a trigger event.
    #[instrument(
        skip(self, event),
        fields(
            otel.kind = "server",
            trace_id = %event.trace_id,
            trigger_id = %event.trigger_id,
            pipeline_id = ?self.pipeline_id,
        )
    )]
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
    #[instrument(
        skip(self),
        fields(
            otel.kind = "internal",
            pipeline_id = ?self.pipeline_id,
            node_count = %self.execution_order.len(),
        )
    )]
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

            // Create a span for this node execution for distributed tracing
            let node_span = tracing::info_span!(
                "node_execution",
                otel.kind = "internal",
                trace_id = %trace_id,
                node_id = %node_id,
                node_type = %node_info.name,
                timeout_ms = %self.config.node_timeout_ms,
            );

            tracing::debug!(
                parent: &node_span,
                "Executing node"
            );

            let result = node_span
                .in_scope(|| async {
                    tokio::time::timeout(
                        std::time::Duration::from_millis(self.config.node_timeout_ms),
                        node.execute(ctx, inputs),
                    )
                    .await
                })
                .await;

            match result {
                Ok(Ok(output)) => {
                    // Handle the output based on its variant
                    match output {
                        NodeOutput::Complete {
                            ref port,
                            ref data,
                            schema_hash,
                        } => {
                            let (offset, size) = (data.offset(), data.size());
                            let port_clone = port.clone();

                            // Log to WAL with actual output location
                            let record =
                                WalRecord::node_done(trace_id, node_id, offset, size, schema_hash);
                            self.wal.write(&record)?;

                            // Log node completion
                            let mut done_event = LogEvent::info(
                                LogCategory::Node,
                                format!("Node completed: {}", node_info.name),
                            )
                            .with_trace_id(trace_id)
                            .with_node_id(node_id)
                            .with_field("output_port", port_clone.clone())
                            .with_field("output_size", size.to_string());
                            if let Some(ref pid) = self.pipeline_id {
                                done_event = done_event.with_pipeline_id(pid);
                            }
                            self.log_collector.collect(done_event);

                            tracing::debug!(
                                trace_id = %trace_id,
                                node_id = %node_id,
                                port = %port_clone,
                                "Node completed"
                            );

                            // Store output and update trace state
                            trace.record_output(node_id, output);
                        }

                        NodeOutput::Error { ref message, .. } => {
                            let message_clone = message.clone();

                            // Log to WAL
                            let record =
                                WalRecord::node_error(trace_id, node_id, message_clone.clone());
                            self.wal.write(&record)?;

                            // Log node error
                            let mut error_event = LogEvent::error(
                                LogCategory::Node,
                                format!("Node returned error: {}", node_info.name),
                            )
                            .with_trace_id(trace_id)
                            .with_node_id(node_id)
                            .with_field("error", message_clone.clone());
                            if let Some(ref pid) = self.pipeline_id {
                                error_event = error_event.with_pipeline_id(pid);
                            }
                            self.log_collector.collect(error_event);

                            tracing::warn!(
                                trace_id = %trace_id,
                                node_id = %node_id,
                                error = %message_clone,
                                "Node returned error output"
                            );

                            // Store the error output for downstream error handling
                            trace.record_output(node_id, output);
                        }

                        NodeOutput::Suspend {
                            ref request,
                            ref pending_data,
                        } => {
                            // Handle suspension - node requests human-in-the-loop approval
                            let hook_id = request.hook_id.clone();
                            let request_clone = request.clone();
                            let pending_offset = pending_data.offset();
                            let pending_size = pending_data.size();

                            tracing::info!(
                                trace_id = %trace_id,
                                node_id = %node_id,
                                hook_id = %hook_id,
                                "Node requested suspension"
                            );

                            // Drop the trace guard before calling suspend_trace
                            // which needs to remove the trace from the map
                            drop(trace);

                            // Suspend the trace
                            return self
                                .suspend_trace(
                                    trace_id,
                                    node_id,
                                    request_clone,
                                    pending_offset,
                                    pending_size,
                                )
                                .await;
                        }
                    }
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

                    // Remove trace from active set before returning error
                    drop(trace);
                    self.traces.remove(&trace_id);

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

                    // Remove trace from active set before returning error
                    drop(trace);
                    self.traces.remove(&trace_id);

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

        // Remove trace from active set and cleanup arena file
        if let Some((_, trace_state)) = self.traces.remove(&trace_id) {
            if let Err(e) = trace_state.cleanup() {
                tracing::warn!(trace_id = %trace_id, error = %e, "Failed to cleanup arena file");
            }
        }

        Ok(())
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
    ///
    /// This will:
    /// 1. Notify all nodes of shutdown
    /// 2. Cleanup all active traces and their arena files
    #[instrument(skip(self), fields(active_traces = %self.traces.len()))]
    pub async fn shutdown(&self) {
        // Notify all nodes of shutdown
        for node in self.nodes.values() {
            node.shutdown();
        }

        // Cleanup all active traces
        let trace_ids: Vec<TraceId> = self.traces.iter().map(|r| *r.key()).collect();
        let trace_count = trace_ids.len();
        for trace_id in trace_ids {
            if let Some((_, trace_state)) = self.traces.remove(&trace_id) {
                if let Err(e) = trace_state.cleanup() {
                    tracing::warn!(
                        trace_id = %trace_id,
                        error = %e,
                        "Failed to cleanup trace during shutdown"
                    );
                }
            }
        }

        tracing::info!(
            "Executor shutdown complete, cleaned up {} traces",
            trace_count
        );
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

        let wal = Arc::new(Wal::open(WalConfig::in_memory()).expect("in-memory WAL should open"));
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
