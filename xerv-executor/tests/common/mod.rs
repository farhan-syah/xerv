//! Common test utilities for integration tests.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use xerv_core::arena::ArenaConfig;
use xerv_core::flow::{FlowDefinition, TriggerDefinition};
use xerv_core::logging::BufferedCollector;
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput};
use xerv_core::types::{NodeId, RelPtr};
use xerv_core::wal::WalConfig;
use xerv_executor::scheduler::{Edge, ExecutorConfig, FlowGraph, GraphNode};

/// A passthrough node for testing.
pub struct PassthroughNode;

impl Node for PassthroughNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("test", "passthrough")
    }

    fn execute<'a>(&'a self, _ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a> {
        Box::pin(async move {
            let input = inputs.get("in").cloned().unwrap_or_else(RelPtr::null);
            Ok(NodeOutput::out(input))
        })
    }
}

/// A trigger node (entry point).
pub struct TriggerNode;

impl Node for TriggerNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("trigger", "memory")
    }

    fn execute<'a>(&'a self, _ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a> {
        Box::pin(async move {
            let input = inputs.get("in").cloned().unwrap_or_else(RelPtr::null);
            Ok(NodeOutput::out(input))
        })
    }
}

/// Create an in-memory WAL config for testing.
pub fn test_wal_config() -> WalConfig {
    WalConfig::in_memory()
}

/// Create an in-memory arena config for testing.
pub fn test_arena_config() -> ArenaConfig {
    ArenaConfig::in_memory()
}

/// Create a default executor config for testing.
pub fn test_executor_config() -> ExecutorConfig {
    ExecutorConfig {
        max_concurrent_traces: 10,
        max_concurrent_nodes: 4, // Reasonable concurrency for testing
        node_timeout_ms: 1_000,
        arena_config: test_arena_config(),
        wal_config: test_wal_config(),
        dispatch_config: xerv_core::dispatch::DispatchConfig::default(),
    }
}

/// Create a log collector for testing.
pub fn test_log_collector() -> Arc<BufferedCollector> {
    Arc::new(BufferedCollector::with_default_capacity())
}

/// Build a simple linear flow: trigger -> node1 -> node2 -> ... -> nodeN
pub fn build_linear_flow(node_count: usize) -> (FlowGraph, HashMap<NodeId, Box<dyn Node>>) {
    let mut graph = FlowGraph::new();
    let mut nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();

    // Add trigger node
    let trigger_id = NodeId::new(0);
    graph.add_node(GraphNode::new(trigger_id, "trigger::memory"));
    nodes.insert(trigger_id, Box::new(TriggerNode));

    // Add processing nodes
    for i in 1..=node_count {
        let node_id = NodeId::new(i as u32);
        graph.add_node(GraphNode::new(node_id, "test::passthrough"));
        nodes.insert(node_id, Box::new(PassthroughNode));

        // Connect to previous node
        let prev_id = NodeId::new((i - 1) as u32);
        graph.add_edge(Edge::new(prev_id, "out", node_id, "in"));
    }

    (graph, nodes)
}

/// Build a diamond flow: trigger -> A -> [B, C] -> D
pub fn build_diamond_flow() -> (FlowGraph, HashMap<NodeId, Box<dyn Node>>) {
    let mut graph = FlowGraph::new();
    let mut nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();

    let trigger_id = NodeId::new(0);
    let node_a = NodeId::new(1);
    let node_b = NodeId::new(2);
    let node_c = NodeId::new(3);
    let node_d = NodeId::new(4);

    // Add nodes
    graph.add_node(GraphNode::new(trigger_id, "trigger::memory"));
    graph.add_node(GraphNode::new(node_a, "test::passthrough"));
    graph.add_node(GraphNode::new(node_b, "test::passthrough"));
    graph.add_node(GraphNode::new(node_c, "test::passthrough"));
    graph.add_node(GraphNode::new(node_d, "test::passthrough"));

    // Add edges
    graph.add_edge(Edge::new(trigger_id, "out", node_a, "in"));
    graph.add_edge(Edge::new(node_a, "out", node_b, "in"));
    graph.add_edge(Edge::new(node_a, "out", node_c, "in"));
    graph.add_edge(Edge::new(node_b, "out", node_d, "in_b"));
    graph.add_edge(Edge::new(node_c, "out", node_d, "in_c"));

    // Add node implementations
    nodes.insert(trigger_id, Box::new(TriggerNode));
    nodes.insert(node_a, Box::new(PassthroughNode));
    nodes.insert(node_b, Box::new(PassthroughNode));
    nodes.insert(node_c, Box::new(PassthroughNode));
    nodes.insert(node_d, Box::new(PassthroughNode));

    (graph, nodes)
}

/// Create a minimal FlowDefinition for testing.
pub fn test_flow_definition(name: &str) -> FlowDefinition {
    FlowDefinition {
        name: name.to_string(),
        version: Some("1".to_string()),
        description: None,
        triggers: vec![TriggerDefinition::new("trigger", "memory")],
        nodes: HashMap::new(),
        edges: vec![],
        settings: Default::default(),
        metadata: None,
    }
}
