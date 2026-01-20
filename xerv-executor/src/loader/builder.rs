//! FlowBuilder - converts FlowDefinition to FlowGraph.

use crate::scheduler::{Edge, FlowGraph, GraphNode};
use std::collections::HashMap;
use xerv_core::error::{Result, XervError};
use xerv_core::flow::{FlowDefinition, NodeDefinition, TriggerDefinition};
use xerv_core::types::NodeId;

/// Builder that converts a FlowDefinition to a FlowGraph.
pub struct FlowBuilder {
    /// Map from string node IDs to numeric NodeIds.
    node_ids: HashMap<String, NodeId>,
    /// Counter for generating NodeIds.
    next_id: u32,
    /// The graph being built.
    graph: FlowGraph,
}

impl FlowBuilder {
    /// Create a new flow builder.
    pub fn new() -> Self {
        Self {
            node_ids: HashMap::new(),
            next_id: 0,
            graph: FlowGraph::new(),
        }
    }

    /// Build a FlowGraph from a FlowDefinition.
    pub fn build(mut self, flow: &FlowDefinition) -> Result<FlowGraph> {
        // First pass: register all node IDs (triggers and nodes)
        self.register_ids(flow)?;

        // Second pass: add nodes to graph
        self.add_triggers(flow)?;
        self.add_nodes(flow)?;

        // Third pass: add edges
        self.add_edges(flow)?;

        // Validate the resulting graph
        self.graph.validate()?;

        Ok(self.graph)
    }

    /// Allocate a new NodeId.
    fn allocate_id(&mut self, name: &str) -> NodeId {
        let id = NodeId::new(self.next_id);
        self.next_id += 1;
        self.node_ids.insert(name.to_string(), id);
        id
    }

    /// Get a NodeId by name.
    fn get_id(&self, name: &str) -> Option<NodeId> {
        self.node_ids.get(name).copied()
    }

    /// Register all node IDs (triggers and processing nodes).
    fn register_ids(&mut self, flow: &FlowDefinition) -> Result<()> {
        // Register trigger IDs
        for trigger in &flow.triggers {
            if !trigger.enabled {
                continue;
            }
            self.allocate_id(&trigger.id);
        }

        // Register node IDs
        for (node_id, node) in &flow.nodes {
            if !node.enabled {
                continue;
            }
            self.allocate_id(node_id);
        }

        Ok(())
    }

    /// Add triggers as entry point nodes.
    fn add_triggers(&mut self, flow: &FlowDefinition) -> Result<()> {
        for trigger in &flow.triggers {
            if !trigger.enabled {
                continue;
            }

            let id = self
                .get_id(&trigger.id)
                .ok_or_else(|| XervError::ConfigValue {
                    field: format!("triggers.{}", trigger.id),
                    cause: "trigger ID not registered".to_string(),
                })?;

            let node_type = format!("trigger::{}", trigger.trigger_type);
            let graph_node = GraphNode::new(id, node_type);
            self.graph.add_node(graph_node);
        }

        Ok(())
    }

    /// Add processing nodes.
    fn add_nodes(&mut self, flow: &FlowDefinition) -> Result<()> {
        for (node_id, node) in &flow.nodes {
            if !node.enabled {
                continue;
            }

            let id = self.get_id(node_id).ok_or_else(|| XervError::ConfigValue {
                field: format!("nodes.{}", node_id),
                cause: "node ID not registered".to_string(),
            })?;

            let graph_node = GraphNode::new(id, &node.node_type);
            self.graph.add_node(graph_node);

            // If this is a loop node, we'll need to track back-edges
            if node.node_type == "std::loop" {
                // Loop back-edges will be declared when processing edges
            }
        }

        Ok(())
    }

    /// Add edges between nodes.
    fn add_edges(&mut self, flow: &FlowDefinition) -> Result<()> {
        for edge_def in &flow.edges {
            let from_node_name = edge_def.from_node();
            let from_port = edge_def.from_port();
            let to_node_name = edge_def.to_node();
            let to_port = edge_def.to_port();

            let from_id = self
                .get_id(from_node_name)
                .ok_or_else(|| XervError::InvalidEdge {
                    from_node: NodeId::new(0),
                    from_port: from_port.to_string(),
                    to_node: NodeId::new(0),
                    to_port: to_port.to_string(),
                })?;

            let to_id = self
                .get_id(to_node_name)
                .ok_or_else(|| XervError::InvalidEdge {
                    from_node: from_id,
                    from_port: from_port.to_string(),
                    to_node: NodeId::new(0),
                    to_port: to_port.to_string(),
                })?;

            let edge = Edge::new(from_id, from_port, to_id, to_port);
            self.graph.add_edge(edge);

            // Declare loop back-edges
            if edge_def.loop_back {
                self.graph.declare_loop_edge(from_id, to_id);
            }
        }

        Ok(())
    }
}

impl Default for FlowBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Metadata about the built flow.
#[derive(Debug, Clone)]
pub struct FlowMetadata {
    /// Flow name.
    pub name: String,
    /// Flow version.
    pub version: String,
    /// Flow description.
    pub description: Option<String>,
    /// Map from string node IDs to numeric NodeIds.
    pub node_ids: HashMap<String, NodeId>,
    /// Trigger definitions (for runtime instantiation).
    pub triggers: Vec<TriggerDefinition>,
    /// Node definitions (for runtime instantiation).
    pub nodes: HashMap<String, NodeDefinition>,
}

impl FlowMetadata {
    /// Create metadata from a FlowDefinition.
    pub fn from_definition(flow: &FlowDefinition, node_ids: HashMap<String, NodeId>) -> Self {
        Self {
            name: flow.name.clone(),
            version: flow.effective_version().to_string(),
            description: flow.description.clone(),
            node_ids,
            triggers: flow.triggers.clone(),
            nodes: flow.nodes.clone(),
        }
    }

    /// Get the NodeId for a string ID.
    pub fn get_node_id(&self, name: &str) -> Option<NodeId> {
        self.node_ids.get(name).copied()
    }

    /// Get the trigger definition for an ID.
    pub fn get_trigger(&self, id: &str) -> Option<&TriggerDefinition> {
        self.triggers.iter().find(|t| t.id == id)
    }

    /// Get the node definition for an ID.
    pub fn get_node(&self, id: &str) -> Option<&NodeDefinition> {
        self.nodes.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xerv_core::flow::{EdgeDefinition, TriggerDefinition};

    fn create_simple_flow() -> FlowDefinition {
        FlowDefinition::new("test_flow")
            .with_trigger(TriggerDefinition::new("webhook", "webhook"))
            .with_node("process", NodeDefinition::new("std::log"))
            .with_edge(EdgeDefinition::new("webhook", "process"))
    }

    #[test]
    fn build_simple_flow() {
        let flow = create_simple_flow();
        let graph = FlowBuilder::new().build(&flow).unwrap();

        // Should have 2 nodes: trigger and process
        let node_count = graph.node_ids().count();
        assert_eq!(node_count, 2);

        // Trigger should be an entry point
        assert_eq!(graph.entry_points().len(), 1);

        // Topological sort should work
        let sorted = graph.topological_sort().unwrap();
        assert_eq!(sorted.len(), 2);
    }

    #[test]
    fn build_diamond_flow() {
        let flow = FlowDefinition::new("diamond")
            .with_trigger(TriggerDefinition::new("webhook", "webhook"))
            .with_node("switch", NodeDefinition::new("std::switch"))
            .with_node("branch_a", NodeDefinition::new("std::log"))
            .with_node("branch_b", NodeDefinition::new("std::log"))
            .with_node("merge", NodeDefinition::new("std::merge"))
            .with_edge(EdgeDefinition::new("webhook", "switch"))
            .with_edge(EdgeDefinition::new("switch.true", "branch_a"))
            .with_edge(EdgeDefinition::new("switch.false", "branch_b"))
            .with_edge(EdgeDefinition::new("branch_a", "merge"))
            .with_edge(EdgeDefinition::new("branch_b", "merge"));

        let graph = FlowBuilder::new().build(&flow).unwrap();

        // Should have 5 nodes
        assert_eq!(graph.node_ids().count(), 5);

        // Should be able to topologically sort
        let sorted = graph.topological_sort().unwrap();
        assert_eq!(sorted.len(), 5);
    }

    #[test]
    fn build_with_loop() {
        let flow = FlowDefinition::new("loop_flow")
            .with_trigger(TriggerDefinition::new("webhook", "webhook"))
            .with_node("loop", NodeDefinition::new("std::loop"))
            .with_node("process", NodeDefinition::new("std::log"))
            .with_edge(EdgeDefinition::new("webhook", "loop"))
            .with_edge(EdgeDefinition::new("loop.continue", "process"))
            .with_edge(EdgeDefinition::new("process", "loop").as_loop_back());

        let graph = FlowBuilder::new().build(&flow).unwrap();

        // Should still be able to topologically sort (back-edge excluded)
        let sorted = graph.topological_sort().unwrap();
        assert_eq!(sorted.len(), 3);
    }

    #[test]
    fn disabled_nodes_excluded() {
        let mut flow = FlowDefinition::new("test")
            .with_trigger(TriggerDefinition::new("webhook", "webhook"))
            .with_node("enabled", NodeDefinition::new("std::log"))
            .with_node("disabled", NodeDefinition::new("std::log").disabled());

        // Only connect to enabled node
        flow.edges.push(EdgeDefinition::new("webhook", "enabled"));

        let graph = FlowBuilder::new().build(&flow).unwrap();

        // Should only have 2 nodes (trigger + enabled)
        assert_eq!(graph.node_ids().count(), 2);
    }

    #[test]
    fn invalid_edge_reference() {
        let flow = FlowDefinition::new("invalid")
            .with_trigger(TriggerDefinition::new("webhook", "webhook"))
            .with_edge(EdgeDefinition::new("webhook", "nonexistent"));

        let result = FlowBuilder::new().build(&flow);
        assert!(result.is_err());
    }
}
