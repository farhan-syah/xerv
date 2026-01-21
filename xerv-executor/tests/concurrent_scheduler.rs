//! Integration tests for the concurrent DAG scheduler.
//!
//! Tests verify that:
//! - Independent nodes execute in parallel
//! - Dependencies are respected
//! - Concurrency limits are enforced
//! - Errors propagate correctly
//! - Timeouts are handled

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Barrier;
use xerv_core::arena::ArenaConfig;
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput, TriggerEvent};
use xerv_core::types::{NodeId, RelPtr};
use xerv_core::wal::{Wal, WalConfig};
use xerv_executor::scheduler::{Edge, Executor, ExecutorConfig, FlowGraph, GraphNode};

use common::{TriggerNode, test_log_collector};

/// A node that tracks concurrent execution using a barrier.
struct ConcurrencyTrackingNode {
    /// Barrier to synchronize concurrent executions.
    barrier: Arc<Barrier>,
    /// Counter for how many times this node has been executed.
    exec_count: Arc<AtomicU32>,
}

impl ConcurrencyTrackingNode {
    fn new(barrier: Arc<Barrier>, exec_count: Arc<AtomicU32>) -> Self {
        Self {
            barrier,
            exec_count,
        }
    }
}

impl Node for ConcurrencyTrackingNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("test", "concurrency_tracker")
    }

    fn execute<'a>(&'a self, _ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a> {
        let barrier = Arc::clone(&self.barrier);
        let exec_count = Arc::clone(&self.exec_count);

        Box::pin(async move {
            exec_count.fetch_add(1, Ordering::SeqCst);

            // Wait at the barrier - this will only succeed if all expected
            // concurrent executions reach this point
            barrier.wait().await;

            let input = inputs.get("in").cloned().unwrap_or_else(RelPtr::null);
            Ok(NodeOutput::out(input))
        })
    }
}

/// A node that sleeps for a specified duration.
struct SlowNode {
    sleep_ms: u64,
}

impl SlowNode {
    fn new(sleep_ms: u64) -> Self {
        Self { sleep_ms }
    }
}

impl Node for SlowNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("test", "slow")
    }

    fn execute<'a>(&'a self, _ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a> {
        let sleep_ms = self.sleep_ms;
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
            let input = inputs.get("in").cloned().unwrap_or_else(RelPtr::null);
            Ok(NodeOutput::out(input))
        })
    }
}

/// A node that tracks the maximum concurrent executions.
struct MaxConcurrencyNode {
    /// Current number of concurrent executions.
    current: Arc<AtomicUsize>,
    /// Maximum observed concurrent executions.
    max_observed: Arc<AtomicUsize>,
    /// How long to hold the execution.
    hold_ms: u64,
}

impl MaxConcurrencyNode {
    fn new(current: Arc<AtomicUsize>, max_observed: Arc<AtomicUsize>, hold_ms: u64) -> Self {
        Self {
            current,
            max_observed,
            hold_ms,
        }
    }
}

impl Node for MaxConcurrencyNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("test", "max_concurrency")
    }

    fn execute<'a>(&'a self, _ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a> {
        let current = Arc::clone(&self.current);
        let max_observed = Arc::clone(&self.max_observed);
        let hold_ms = self.hold_ms;

        Box::pin(async move {
            // Increment current
            let prev = current.fetch_add(1, Ordering::SeqCst);
            let new_current = prev + 1;

            // Update max if this is higher
            max_observed.fetch_max(new_current, Ordering::SeqCst);

            // Hold execution
            tokio::time::sleep(Duration::from_millis(hold_ms)).await;

            // Decrement current
            current.fetch_sub(1, Ordering::SeqCst);

            let input = inputs.get("in").cloned().unwrap_or_else(RelPtr::null);
            Ok(NodeOutput::out(input))
        })
    }
}

/// A node that returns an error.
struct FailingNode {
    message: String,
}

impl FailingNode {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Node for FailingNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("test", "failing")
    }

    fn execute<'a>(
        &'a self,
        _ctx: Context,
        _inputs: HashMap<String, RelPtr<()>>,
    ) -> NodeFuture<'a> {
        let message = self.message.clone();
        Box::pin(async move { Ok(NodeOutput::error_with_message(message)) })
    }
}

fn test_executor_config_with_concurrency(max_concurrent_nodes: usize) -> ExecutorConfig {
    ExecutorConfig {
        max_concurrent_traces: 10,
        max_concurrent_nodes,
        node_timeout_ms: 5_000,
        arena_config: ArenaConfig::in_memory(),
        wal_config: WalConfig::in_memory(),
        dispatch_config: xerv_core::dispatch::DispatchConfig::default(),
    }
}

/// Build a wide flow with tracking nodes for concurrency verification.
type WideFlowWithTracking = (
    FlowGraph,
    HashMap<NodeId, Box<dyn Node>>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
);

fn build_wide_flow_with_tracking(width: usize) -> WideFlowWithTracking {
    let current = Arc::new(AtomicUsize::new(0));
    let max_observed = Arc::new(AtomicUsize::new(0));

    let mut graph = FlowGraph::new();
    let mut nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();

    let trigger_id = NodeId::new(0);
    graph.add_node(GraphNode::new(trigger_id, "trigger::memory"));
    nodes.insert(trigger_id, Box::new(TriggerNode));

    for i in 1..=width {
        let node_id = NodeId::new(i as u32);
        graph.add_node(GraphNode::new(node_id, "test::max_concurrency"));
        nodes.insert(
            node_id,
            Box::new(MaxConcurrencyNode::new(
                Arc::clone(&current),
                Arc::clone(&max_observed),
                50, // Hold for 50ms
            )),
        );
        graph.add_edge(Edge::new(trigger_id, "out", node_id, "in"));
    }

    (graph, nodes, current, max_observed)
}

#[tokio::test]
async fn diamond_dag_executes_correctly() {
    // Diamond: trigger -> A -> [B, C] -> D
    // B and C should be able to run in parallel
    let barrier = Arc::new(Barrier::new(2)); // B and C
    let exec_count = Arc::new(AtomicU32::new(0));

    let mut graph = FlowGraph::new();
    let mut nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();

    let trigger_id = NodeId::new(0);
    let node_a = NodeId::new(1);
    let node_b = NodeId::new(2);
    let node_c = NodeId::new(3);
    let node_d = NodeId::new(4);

    graph.add_node(GraphNode::new(trigger_id, "trigger::memory"));
    graph.add_node(GraphNode::new(node_a, "test::passthrough"));
    graph.add_node(GraphNode::new(node_b, "test::concurrency_tracker"));
    graph.add_node(GraphNode::new(node_c, "test::concurrency_tracker"));
    graph.add_node(GraphNode::new(node_d, "test::passthrough"));

    graph.add_edge(Edge::new(trigger_id, "out", node_a, "in"));
    graph.add_edge(Edge::new(node_a, "out", node_b, "in"));
    graph.add_edge(Edge::new(node_a, "out", node_c, "in"));
    graph.add_edge(Edge::new(node_b, "out", node_d, "in_b"));
    graph.add_edge(Edge::new(node_c, "out", node_d, "in_c"));

    nodes.insert(trigger_id, Box::new(TriggerNode));
    nodes.insert(node_a, Box::new(common::PassthroughNode));
    nodes.insert(
        node_b,
        Box::new(ConcurrencyTrackingNode::new(
            Arc::clone(&barrier),
            Arc::clone(&exec_count),
        )),
    );
    nodes.insert(
        node_c,
        Box::new(ConcurrencyTrackingNode::new(
            Arc::clone(&barrier),
            Arc::clone(&exec_count),
        )),
    );
    nodes.insert(node_d, Box::new(common::PassthroughNode));

    let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
    let log_collector = test_log_collector();

    let executor = Executor::new(
        test_executor_config_with_concurrency(4),
        graph,
        nodes,
        wal,
        log_collector,
        Some("test_diamond".to_string()),
    )
    .unwrap();

    // Start a trace
    let trigger_event = TriggerEvent::new("trigger::memory", RelPtr::null());
    let trace_id = trigger_event.trace_id;

    executor.start_trace(trigger_event).await.unwrap();

    // Execute with timeout (barrier requires both B and C to run concurrently)
    let result =
        tokio::time::timeout(Duration::from_secs(5), executor.execute_trace(trace_id)).await;

    // If the barrier didn't trip (B and C weren't concurrent), this would timeout
    assert!(
        result.is_ok(),
        "Execution should complete (B and C ran concurrently)"
    );
    assert!(result.unwrap().is_ok());

    // Both B and C should have executed
    assert_eq!(exec_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn concurrency_limit_respected() {
    // Create a wide flow with 10 parallel nodes
    let width = 10;
    let max_concurrent = 3;

    let (graph, nodes, _current, max_observed) = build_wide_flow_with_tracking(width);

    let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
    let log_collector = test_log_collector();

    let executor = Executor::new(
        test_executor_config_with_concurrency(max_concurrent),
        graph,
        nodes,
        wal,
        log_collector,
        Some("test_concurrency_limit".to_string()),
    )
    .unwrap();

    let trigger_event = TriggerEvent::new("trigger::memory", RelPtr::null());
    let trace_id = trigger_event.trace_id;

    executor.start_trace(trigger_event).await.unwrap();
    executor.execute_trace(trace_id).await.unwrap();

    // Maximum concurrent executions should not exceed the limit
    let observed_max = max_observed.load(Ordering::SeqCst);
    assert!(
        observed_max <= max_concurrent,
        "Max concurrent nodes ({}) exceeded limit ({})",
        observed_max,
        max_concurrent
    );
}

#[tokio::test]
async fn sequential_execution_with_limit_one() {
    // With max_concurrent_nodes=1, nodes should execute sequentially
    let width = 5;
    let (graph, nodes, _current, max_observed) = build_wide_flow_with_tracking(width);

    let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
    let log_collector = test_log_collector();

    let executor = Executor::new(
        test_executor_config_with_concurrency(1), // Sequential!
        graph,
        nodes,
        wal,
        log_collector,
        Some("test_sequential".to_string()),
    )
    .unwrap();

    let trigger_event = TriggerEvent::new("trigger::memory", RelPtr::null());
    let trace_id = trigger_event.trace_id;

    executor.start_trace(trigger_event).await.unwrap();
    executor.execute_trace(trace_id).await.unwrap();

    // With limit 1, max concurrent should be exactly 1 (excluding trigger)
    let observed_max = max_observed.load(Ordering::SeqCst);
    assert_eq!(
        observed_max, 1,
        "Sequential execution should have max concurrency of 1, got {}",
        observed_max
    );
}

#[tokio::test]
async fn parallel_execution_is_faster_than_sequential() {
    // Create a flow where parallel execution should be significantly faster
    let width = 4;
    let sleep_ms = 100; // Each node sleeps 100ms

    let mut graph = FlowGraph::new();
    let mut nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();

    let trigger_id = NodeId::new(0);
    graph.add_node(GraphNode::new(trigger_id, "trigger::memory"));
    nodes.insert(trigger_id, Box::new(TriggerNode));

    for i in 1..=width {
        let node_id = NodeId::new(i as u32);
        graph.add_node(GraphNode::new(node_id, "test::slow"));
        nodes.insert(node_id, Box::new(SlowNode::new(sleep_ms)));
        graph.add_edge(Edge::new(trigger_id, "out", node_id, "in"));
    }

    let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
    let log_collector = test_log_collector();

    let executor = Executor::new(
        test_executor_config_with_concurrency(width), // Full parallelism
        graph,
        nodes,
        wal,
        log_collector,
        Some("test_parallel_speed".to_string()),
    )
    .unwrap();

    let trigger_event = TriggerEvent::new("trigger::memory", RelPtr::null());
    let trace_id = trigger_event.trace_id;

    executor.start_trace(trigger_event).await.unwrap();

    let start = Instant::now();
    executor.execute_trace(trace_id).await.unwrap();
    let elapsed = start.elapsed();

    // With full parallelism, 4 nodes sleeping 100ms each should complete in ~100ms
    // Sequential would take ~400ms
    // Allow some overhead, but it should be well under 300ms
    assert!(
        elapsed < Duration::from_millis(300),
        "Parallel execution took {:?}, expected < 300ms (sequential would be ~400ms)",
        elapsed
    );
}

#[tokio::test]
async fn error_in_node_does_not_crash_execution() {
    // A node returning Error variant should be recorded, not crash execution
    let mut graph = FlowGraph::new();
    let mut nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();

    let trigger_id = NodeId::new(0);
    let failing_node = NodeId::new(1);

    graph.add_node(GraphNode::new(trigger_id, "trigger::memory"));
    graph.add_node(GraphNode::new(failing_node, "test::failing"));
    graph.add_edge(Edge::new(trigger_id, "out", failing_node, "in"));

    nodes.insert(trigger_id, Box::new(TriggerNode));
    nodes.insert(failing_node, Box::new(FailingNode::new("Test error")));

    let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
    let log_collector = test_log_collector();

    let executor = Executor::new(
        test_executor_config_with_concurrency(4),
        graph,
        nodes,
        wal,
        log_collector,
        Some("test_error_handling".to_string()),
    )
    .unwrap();

    let trigger_event = TriggerEvent::new("trigger::memory", RelPtr::null());
    let trace_id = trigger_event.trace_id;

    executor.start_trace(trigger_event).await.unwrap();

    // Should complete successfully (error is an output variant, not an execution failure)
    let result = executor.execute_trace(trace_id).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn multiple_traces_execute_independently() {
    let (graph, nodes) = common::build_linear_flow(3);

    let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
    let log_collector = test_log_collector();

    let executor = Arc::new(
        Executor::new(
            test_executor_config_with_concurrency(4),
            graph,
            nodes,
            wal,
            log_collector,
            Some("test_multiple_traces".to_string()),
        )
        .unwrap(),
    );

    // Start multiple traces
    let mut handles = vec![];
    for _ in 0..5 {
        let executor = Arc::clone(&executor);
        handles.push(tokio::spawn(async move {
            let trigger_event = TriggerEvent::new("trigger::memory", RelPtr::null());
            let trace_id = trigger_event.trace_id;

            executor.start_trace(trigger_event).await.unwrap();
            executor.execute_trace(trace_id).await
        }));
    }

    // All should complete successfully
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.unwrap();
        assert!(result.is_ok(), "Trace {} failed: {:?}", i, result);
    }
}

#[tokio::test]
async fn shutdown_cleans_up_resources() {
    let (graph, nodes) = common::build_linear_flow(2);

    let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
    let log_collector = test_log_collector();

    let executor = Executor::new(
        test_executor_config_with_concurrency(4),
        graph,
        nodes,
        wal,
        log_collector,
        Some("test_shutdown".to_string()),
    )
    .unwrap();

    // Start a trace but don't execute it
    let trigger_event = TriggerEvent::new("trigger::memory", RelPtr::null());
    executor.start_trace(trigger_event).await.unwrap();

    assert_eq!(executor.active_trace_count(), 1);

    // Shutdown should cleanup
    executor.shutdown().await;

    assert_eq!(executor.active_trace_count(), 0);
}
