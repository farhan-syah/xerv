//! Trace execution engine.
//!
//! This module provides the core execution engine for XERV traces with
//! **concurrent DAG execution**. Nodes are executed in parallel when their
//! dependencies are satisfied, maximizing throughput.
//!
//! ## Architecture
//!
//! The executor uses a work-stealing approach:
//! 1. Find all "ready" nodes (dependencies satisfied, not yet executing)
//! 2. Spawn concurrent tasks for ready nodes up to `max_concurrent_nodes`
//! 3. Wait for any task to complete
//! 4. Update trace state and repeat
//!
//! This ensures maximum parallelism while respecting the DAG constraints.
//!
//! ## Concurrency Control
//!
//! - `max_concurrent_traces`: Limits total active traces
//! - `max_concurrent_nodes`: Limits concurrent node executions per trace
//! - Backpressure via semaphore prevents resource exhaustion
//!
//! All public methods are instrumented with tracing spans for distributed tracing.

use super::graph::FlowGraph;
use crate::suspension::SuspensionStore;
use crate::trace::TraceState;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::instrument;
use xerv_core::arena::{Arena, ArenaConfig, ArenaReader, ArenaWriter};
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
    /// Maximum concurrent node executions per trace.
    ///
    /// This controls how many nodes can execute in parallel within a single trace.
    /// Higher values increase throughput but also memory usage.
    /// Set to 1 for sequential execution (debugging).
    pub max_concurrent_nodes: usize,
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
            max_concurrent_nodes: 16, // Reasonable default for most workloads
            node_timeout_ms: 30_000,
            arena_config: ArenaConfig::default(),
            wal_config: WalConfig::default(),
            dispatch_config: DispatchConfig::default(),
        }
    }
}

impl ExecutorConfig {
    /// Create configuration from environment variables.
    ///
    /// Reads the following environment variables:
    /// - `XERV_DATA_DIR`: Base directory for arena and WAL files
    /// - `XERV_ARENA_SYNC`: Enable arena sync on write
    /// - `XERV_WAL_SYNC`: Enable WAL sync on write
    /// - `XERV_WAL_GROUP_COMMIT`: Enable WAL group commit
    /// - `XERV_DISPATCH_BACKEND`: Dispatch backend (memory, raft, redis, nats)
    /// - `XERV_MAX_CONCURRENT_NODES`: Maximum concurrent node executions
    /// - `XERV_MAX_CONCURRENT_TRACES`: Maximum concurrent traces
    /// - `XERV_NODE_TIMEOUT_MS`: Node execution timeout in milliseconds
    ///
    /// # Example
    ///
    /// ```bash
    /// export XERV_DATA_DIR=/var/lib/xerv
    /// export XERV_ARENA_SYNC=true
    /// export XERV_WAL_SYNC=true
    /// export XERV_MAX_CONCURRENT_NODES=32
    /// ```
    pub fn from_env() -> Self {
        let max_concurrent_nodes = std::env::var("XERV_MAX_CONCURRENT_NODES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(16);

        let max_concurrent_traces = std::env::var("XERV_MAX_CONCURRENT_TRACES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(100);

        let node_timeout_ms = std::env::var("XERV_NODE_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30_000);

        Self {
            max_concurrent_traces,
            max_concurrent_nodes,
            node_timeout_ms,
            arena_config: ArenaConfig::from_env_or_default(),
            wal_config: WalConfig::from_env_or_default(),
            dispatch_config: DispatchConfig::from_env_or_default(),
        }
    }

    /// Create configuration from environment variables, or use defaults.
    ///
    /// Same as `from_env()` but always returns a valid configuration.
    pub fn from_env_or_default() -> Self {
        Self::from_env()
    }

    /// Set maximum concurrent nodes per trace.
    pub fn with_max_concurrent_nodes(mut self, max: usize) -> Self {
        self.max_concurrent_nodes = max.max(1); // At least 1
        self
    }

    /// Create a production-ready configuration.
    ///
    /// This configuration:
    /// - Reads from environment variables first (respects XERV_DATA_DIR)
    /// - Falls back to production defaults (`/var/lib/xerv`)
    /// - Enables sync-on-write for crash safety
    /// - Sets reasonable concurrency limits
    ///
    /// # Example
    /// ```rust,ignore
    /// // With environment variable set:
    /// // XERV_DATA_DIR=/mnt/fast-storage
    /// let config = ExecutorConfig::production()
    ///     .with_max_concurrent_nodes(32);
    /// // Will use /mnt/fast-storage/arenas and /mnt/fast-storage/wal
    /// ```
    pub fn production() -> Self {
        // Start with environment-aware defaults
        let mut config = Self::from_env();

        // Override with production-specific settings if XERV_DATA_DIR is not set
        if std::env::var("XERV_DATA_DIR").is_err() {
            config.arena_config = ArenaConfig::default()
                .with_directory("/var/lib/xerv/arenas")
                .with_sync(true);
            config.wal_config = WalConfig::default()
                .with_directory("/var/lib/xerv/wal")
                .with_sync(true);
        }

        // Ensure production timeout is at least 60 seconds
        config.node_timeout_ms = config.node_timeout_ms.max(60_000);

        config
    }
}

impl ExecutorConfig {
    /// Set the dispatch configuration.
    pub fn with_dispatch(mut self, config: DispatchConfig) -> Self {
        self.dispatch_config = config;
        self
    }

    /// Set the node timeout.
    pub fn with_node_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.node_timeout_ms = timeout_ms;
        self
    }
}

/// Result of a single node execution task.
#[derive(Debug)]
struct NodeExecutionResult {
    node_id: NodeId,
    result: Result<NodeOutput>,
}

/// Captured context needed to execute a node in a spawned task.
///
/// This struct contains everything needed to execute a node without
/// holding any references to TraceState.
struct NodeExecutionContext {
    trace_id: TraceId,
    node_id: NodeId,
    reader: ArenaReader,
    writer: ArenaWriter,
    wal: Arc<Wal>,
    inputs: HashMap<String, RelPtr<()>>,
}

/// The main execution engine.
///
/// The executor manages trace lifecycle and schedules node executions concurrently.
/// It uses a work-stealing approach where nodes are executed as soon as their
/// dependencies are satisfied, up to a configurable concurrency limit.
pub struct Executor {
    /// Configuration.
    pub(crate) config: ExecutorConfig,
    /// The flow graph.
    pub(crate) graph: Arc<FlowGraph>,
    /// Node implementations (Arc-wrapped for concurrent task spawning).
    pub(crate) nodes: HashMap<NodeId, Arc<Box<dyn Node>>>,
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

        // Wrap nodes in Arc for concurrent task spawning
        let nodes: HashMap<NodeId, Arc<Box<dyn Node>>> = nodes
            .into_iter()
            .map(|(id, node)| (id, Arc::new(node)))
            .collect();

        // Create default in-memory dispatch backend
        let dispatch: Arc<dyn DispatchBackend> =
            Arc::new(MemoryDispatch::new(config.dispatch_config.memory_config()));

        Ok(Self {
            config,
            graph: Arc::new(graph),
            nodes,
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

        // Wrap nodes in Arc for concurrent task spawning
        let nodes: HashMap<NodeId, Arc<Box<dyn Node>>> = nodes
            .into_iter()
            .map(|(id, node)| (id, Arc::new(node)))
            .collect();

        // Create default in-memory dispatch backend
        let dispatch: Arc<dyn DispatchBackend> =
            Arc::new(MemoryDispatch::new(config.dispatch_config.memory_config()));

        Ok(Self {
            config,
            graph: Arc::new(graph),
            nodes,
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

    /// Execute a trace to completion using concurrent DAG execution.
    ///
    /// This method executes nodes in parallel when their dependencies are satisfied,
    /// maximizing throughput while respecting the DAG constraints.
    ///
    /// ## Algorithm
    ///
    /// 1. Create a semaphore with `max_concurrent_nodes` permits
    /// 2. Loop:
    ///    a. Find all ready nodes (dependencies met, not executing)
    ///    b. For each ready node, acquire a semaphore permit and spawn a task
    ///    c. Wait for any task to complete
    ///    d. Process the result (update trace state, handle errors/suspension)
    ///    e. Repeat until no more nodes to execute
    ///
    /// ## Error Handling
    ///
    /// On node failure, all running tasks are aborted and the trace is cleaned up.
    /// On suspension request, running tasks complete before suspension.
    #[instrument(
        skip(self),
        fields(
            otel.kind = "internal",
            pipeline_id = ?self.pipeline_id,
            node_count = %self.execution_order.len(),
            max_concurrent = %self.config.max_concurrent_nodes,
        )
    )]
    pub async fn execute_trace(&self, trace_id: TraceId) -> Result<()> {
        // Verify trace exists
        if !self.traces.contains_key(&trace_id) {
            return Err(XervError::NodeExecution {
                node_id: NodeId::new(0),
                trace_id,
                cause: "Trace not found".to_string(),
            });
        }

        // Semaphore for limiting concurrent node executions
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_nodes));

        // JoinSet to track spawned tasks
        let mut tasks: JoinSet<NodeExecutionResult> = JoinSet::new();

        // Track which nodes have tasks spawned (to avoid double-spawning)
        let mut spawned: std::collections::HashSet<NodeId> = std::collections::HashSet::new();

        loop {
            // Find all ready nodes that haven't been spawned yet
            let ready_nodes = self.find_ready_nodes(trace_id, &spawned)?;

            // Spawn tasks for ready nodes
            for node_id in ready_nodes {
                // Mark as spawned to prevent double-spawning
                spawned.insert(node_id);

                // Prepare execution context (captures data without holding trace guard)
                let exec_ctx = self.prepare_node_execution(trace_id, node_id)?;

                // Get node reference
                let node = Arc::clone(self.nodes.get(&node_id).ok_or_else(|| {
                    XervError::NodeNotFound {
                        node_name: format!("{}", node_id),
                    }
                })?);

                // Mark node as executing in trace state
                if let Some(mut trace) = self.traces.get_mut(&trace_id) {
                    trace.mark_executing(node_id);
                }

                // Clone what we need for the spawned task
                let semaphore = Arc::clone(&semaphore);
                let timeout_ms = self.config.node_timeout_ms;
                let log_collector = Arc::clone(&self.log_collector);
                let pipeline_id = self.pipeline_id.clone();
                let wal = Arc::clone(&self.wal);

                // Spawn the task
                tasks.spawn(async move {
                    // Acquire semaphore permit (backpressure)
                    let _permit = semaphore
                        .acquire()
                        .await
                        .expect("semaphore should not be closed");

                    // Execute the node
                    let result = Self::execute_single_node(
                        exec_ctx,
                        node,
                        timeout_ms,
                        &log_collector,
                        pipeline_id.as_deref(),
                        &wal,
                    )
                    .await;

                    NodeExecutionResult { node_id, result }
                });
            }

            // If no tasks running and no ready nodes, we're done
            if tasks.is_empty() {
                break;
            }

            // Wait for any task to complete
            let Some(join_result) = tasks.join_next().await else {
                break;
            };

            // Handle task result
            let execution_result = match join_result {
                Ok(result) => result,
                Err(join_error) => {
                    // Task panicked - treat as fatal error
                    let error_msg = if join_error.is_panic() {
                        "Node task panicked".to_string()
                    } else {
                        "Node task was cancelled".to_string()
                    };

                    tracing::error!(
                        trace_id = %trace_id,
                        error = %error_msg,
                        "Task join error"
                    );

                    // Abort all remaining tasks
                    tasks.abort_all();

                    // Cleanup trace
                    if let Some((_, trace_state)) = self.traces.remove(&trace_id) {
                        let _ = trace_state.cleanup();
                    }

                    return Err(XervError::NodePanic {
                        node_id: NodeId::new(0),
                        trace_id,
                        message: error_msg,
                    });
                }
            };

            // Process the execution result
            match self
                .process_node_result(trace_id, execution_result, &mut tasks)
                .await?
            {
                ExecutionAction::Continue => {
                    // Normal case - continue the loop
                }
                ExecutionAction::Suspend => {
                    // Trace was suspended - we're done (suspension handled inside)
                    return Ok(());
                }
                ExecutionAction::Error(e) => {
                    // Abort all remaining tasks
                    tasks.abort_all();

                    // Cleanup trace
                    if let Some((_, trace_state)) = self.traces.remove(&trace_id) {
                        let _ = trace_state.cleanup();
                    }

                    return Err(e);
                }
            }
        }

        // All nodes completed - finalize trace
        self.finalize_trace(trace_id).await
    }

    /// Find all nodes that are ready to execute.
    ///
    /// A node is ready if:
    /// - All its dependencies have completed
    /// - It hasn't been spawned yet
    /// - It's not already executing
    fn find_ready_nodes(
        &self,
        trace_id: TraceId,
        already_spawned: &std::collections::HashSet<NodeId>,
    ) -> Result<Vec<NodeId>> {
        let trace = self
            .traces
            .get(&trace_id)
            .ok_or_else(|| XervError::NodeExecution {
                node_id: NodeId::new(0),
                trace_id,
                cause: "Trace not found".to_string(),
            })?;

        let ready: Vec<NodeId> = self
            .execution_order
            .iter()
            .filter(|&&node_id| {
                !already_spawned.contains(&node_id) && trace.is_node_ready(node_id, &self.graph)
            })
            .copied()
            .collect();

        Ok(ready)
    }

    /// Prepare execution context for a node.
    ///
    /// This captures all data needed to execute the node without holding
    /// any references to TraceState.
    fn prepare_node_execution(
        &self,
        trace_id: TraceId,
        node_id: NodeId,
    ) -> Result<NodeExecutionContext> {
        let trace = self
            .traces
            .get(&trace_id)
            .ok_or_else(|| XervError::NodeExecution {
                node_id,
                trace_id,
                cause: "Trace not found".to_string(),
            })?;

        let (reader, writer) = trace.arena_handles();
        let inputs = trace.collect_inputs(node_id, &self.graph);

        Ok(NodeExecutionContext {
            trace_id,
            node_id,
            reader,
            writer,
            wal: Arc::clone(&self.wal),
            inputs,
        })
    }

    /// Execute a single node with timeout.
    ///
    /// This is the actual node execution logic, isolated for spawning.
    async fn execute_single_node(
        exec_ctx: NodeExecutionContext,
        node: Arc<Box<dyn Node>>,
        timeout_ms: u64,
        log_collector: &BufferedCollector,
        pipeline_id: Option<&str>,
        wal: &Wal,
    ) -> Result<NodeOutput> {
        let trace_id = exec_ctx.trace_id;
        let node_id = exec_ctx.node_id;

        // Create execution context
        let ctx = Context::new(
            trace_id,
            node_id,
            exec_ctx.reader,
            exec_ctx.writer,
            exec_ctx.wal,
        );

        // Log node execution start
        let node_info = node.info();
        let mut start_event = LogEvent::debug(
            LogCategory::Node,
            format!("Executing node: {}", node_info.name),
        )
        .with_trace_id(trace_id)
        .with_node_id(node_id)
        .with_field("namespace", &node_info.namespace);
        if let Some(pid) = pipeline_id {
            start_event = start_event.with_pipeline_id(pid);
        }
        log_collector.collect(start_event);

        // Create tracing span
        let node_span = tracing::info_span!(
            "node_execution",
            otel.kind = "internal",
            trace_id = %trace_id,
            node_id = %node_id,
            node_type = %node_info.name,
            timeout_ms = %timeout_ms,
        );

        tracing::debug!(parent: &node_span, "Executing node");

        // Execute with timeout
        let result = node_span
            .in_scope(|| async {
                tokio::time::timeout(
                    std::time::Duration::from_millis(timeout_ms),
                    node.execute(ctx, exec_ctx.inputs),
                )
                .await
            })
            .await;

        match result {
            Ok(Ok(output)) => {
                // Log completion based on output type
                match &output {
                    NodeOutput::Complete {
                        port,
                        data,
                        schema_hash,
                    } => {
                        let (offset, size) = (data.offset(), data.size());

                        // Log to WAL
                        let record =
                            WalRecord::node_done(trace_id, node_id, offset, size, *schema_hash);
                        wal.write(&record)?;

                        let mut done_event = LogEvent::info(
                            LogCategory::Node,
                            format!("Node completed: {}", node_info.name),
                        )
                        .with_trace_id(trace_id)
                        .with_node_id(node_id)
                        .with_field("output_port", port.clone())
                        .with_field("output_size", size.to_string());
                        if let Some(pid) = pipeline_id {
                            done_event = done_event.with_pipeline_id(pid);
                        }
                        log_collector.collect(done_event);

                        tracing::debug!(
                            trace_id = %trace_id,
                            node_id = %node_id,
                            port = %port,
                            "Node completed"
                        );
                    }
                    NodeOutput::Error { message, .. } => {
                        // Log to WAL
                        let record = WalRecord::node_error(trace_id, node_id, message.clone());
                        wal.write(&record)?;

                        let mut error_event = LogEvent::error(
                            LogCategory::Node,
                            format!("Node returned error: {}", node_info.name),
                        )
                        .with_trace_id(trace_id)
                        .with_node_id(node_id)
                        .with_field("error", message.clone());
                        if let Some(pid) = pipeline_id {
                            error_event = error_event.with_pipeline_id(pid);
                        }
                        log_collector.collect(error_event);

                        tracing::warn!(
                            trace_id = %trace_id,
                            node_id = %node_id,
                            error = %message,
                            "Node returned error output"
                        );
                    }
                    NodeOutput::Suspend { request, .. } => {
                        tracing::info!(
                            trace_id = %trace_id,
                            node_id = %node_id,
                            hook_id = %request.hook_id,
                            "Node requested suspension"
                        );
                    }
                }
                Ok(output)
            }
            Ok(Err(e)) => {
                // Node execution error
                let record = WalRecord::node_error(trace_id, node_id, e.to_string());
                wal.write(&record)?;

                let mut error_event = LogEvent::error(
                    LogCategory::Node,
                    format!("Node failed: {}", node_info.name),
                )
                .with_trace_id(trace_id)
                .with_node_id(node_id)
                .with_field("error", e.to_string());
                if let Some(pid) = pipeline_id {
                    error_event = error_event.with_pipeline_id(pid);
                }
                log_collector.collect(error_event);

                tracing::error!(
                    trace_id = %trace_id,
                    node_id = %node_id,
                    error = %e,
                    "Node failed"
                );

                Err(e)
            }
            Err(_timeout) => {
                // Timeout
                let record = WalRecord::node_error(trace_id, node_id, "Timeout");
                wal.write(&record)?;

                let mut timeout_event = LogEvent::error(
                    LogCategory::Node,
                    format!("Node timeout: {}", node_info.name),
                )
                .with_trace_id(trace_id)
                .with_node_id(node_id)
                .with_field_i64("timeout_ms", timeout_ms as i64);
                if let Some(pid) = pipeline_id {
                    timeout_event = timeout_event.with_pipeline_id(pid);
                }
                log_collector.collect(timeout_event);

                Err(XervError::NodeTimeout {
                    node_id,
                    trace_id,
                    timeout_ms,
                })
            }
        }
    }

    /// Process the result of a node execution.
    ///
    /// Updates trace state and determines what action to take next.
    async fn process_node_result(
        &self,
        trace_id: TraceId,
        result: NodeExecutionResult,
        tasks: &mut JoinSet<NodeExecutionResult>,
    ) -> Result<ExecutionAction> {
        let node_id = result.node_id;

        match result.result {
            Ok(output) => {
                match &output {
                    NodeOutput::Complete { .. } | NodeOutput::Error { .. } => {
                        // Update trace state
                        if let Some(mut trace) = self.traces.get_mut(&trace_id) {
                            trace.record_output(node_id, output);
                        }
                        Ok(ExecutionAction::Continue)
                    }
                    NodeOutput::Suspend {
                        request,
                        pending_data,
                    } => {
                        // Drain remaining tasks before suspension
                        // (let them complete, but don't start new ones)
                        while let Some(join_result) = tasks.join_next().await {
                            if let Ok(other_result) = join_result {
                                // Record completed node outputs
                                if let Ok(other_output) = other_result.result {
                                    if let Some(mut trace) = self.traces.get_mut(&trace_id) {
                                        if !other_output.is_suspended() {
                                            trace.record_output(other_result.node_id, other_output);
                                        }
                                    }
                                }
                            }
                        }

                        // Now suspend the trace
                        self.suspend_trace(
                            trace_id,
                            node_id,
                            request.clone(),
                            pending_data.offset(),
                            pending_data.size(),
                        )
                        .await?;

                        Ok(ExecutionAction::Suspend)
                    }
                }
            }
            Err(e) => {
                // Node failed - determine if this is a fatal error
                Ok(ExecutionAction::Error(XervError::NodeExecution {
                    node_id,
                    trace_id,
                    cause: e.to_string(),
                }))
            }
        }
    }

    /// Finalize a completed trace.
    async fn finalize_trace(&self, trace_id: TraceId) -> Result<()> {
        // Mark trace as complete in WAL
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

/// Action to take after processing a node result.
enum ExecutionAction {
    /// Continue executing more nodes.
    Continue,
    /// Trace was suspended.
    Suspend,
    /// Fatal error occurred.
    Error(XervError),
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
