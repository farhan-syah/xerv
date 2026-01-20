//! Flow graph representation and analysis.

use std::collections::{HashMap, HashSet, VecDeque};
use xerv_core::error::{Result, XervError};
use xerv_core::types::NodeId;

/// A directed edge in the flow graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Edge {
    /// Source node ID.
    pub from_node: NodeId,
    /// Source port name.
    pub from_port: String,
    /// Target node ID.
    pub to_node: NodeId,
    /// Target port name.
    pub to_port: String,
}

impl Edge {
    /// Create a new edge.
    pub fn new(
        from_node: NodeId,
        from_port: impl Into<String>,
        to_node: NodeId,
        to_port: impl Into<String>,
    ) -> Self {
        Self {
            from_node,
            from_port: from_port.into(),
            to_node,
            to_port: to_port.into(),
        }
    }
}

/// Information about a node in the graph.
#[derive(Debug, Clone)]
pub struct GraphNode {
    /// Node ID.
    pub id: NodeId,
    /// Node type (e.g., "std::switch", "plugins::fraud_model").
    pub node_type: String,
    /// Whether this node is a loop controller.
    pub is_loop_controller: bool,
    /// Whether this node is a merge/barrier node.
    pub is_merge: bool,
    /// Whether this is a trigger node (entry point).
    pub is_trigger: bool,
}

impl GraphNode {
    /// Create a new graph node.
    pub fn new(id: NodeId, node_type: impl Into<String>) -> Self {
        let node_type = node_type.into();
        let is_loop_controller = node_type == "std::loop";
        let is_merge = node_type == "std::merge";
        let is_trigger = node_type.starts_with("trigger::");

        Self {
            id,
            node_type,
            is_loop_controller,
            is_merge,
            is_trigger,
        }
    }
}

/// The flow graph representation.
#[derive(Debug)]
pub struct FlowGraph {
    /// Nodes in the graph, keyed by node ID.
    nodes: HashMap<NodeId, GraphNode>,
    /// All edges in the graph.
    edges: Vec<Edge>,
    /// Edges indexed by source node.
    outgoing: HashMap<NodeId, Vec<usize>>,
    /// Edges indexed by target node.
    incoming: HashMap<NodeId, Vec<usize>>,
    /// Entry points (trigger nodes).
    entry_points: Vec<NodeId>,
    /// Declared loop back-edges (from loop controllers).
    loop_back_edges: HashSet<(NodeId, NodeId)>,
}

impl FlowGraph {
    /// Create a new empty flow graph.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
            entry_points: Vec::new(),
            loop_back_edges: HashSet::new(),
        }
    }

    /// Add a node to the graph.
    pub fn add_node(&mut self, node: GraphNode) {
        let id = node.id;
        if node.is_trigger {
            self.entry_points.push(id);
        }
        self.nodes.insert(id, node);
        self.outgoing.entry(id).or_default();
        self.incoming.entry(id).or_default();
    }

    /// Add an edge to the graph.
    pub fn add_edge(&mut self, edge: Edge) {
        let idx = self.edges.len();
        self.outgoing.entry(edge.from_node).or_default().push(idx);
        self.incoming.entry(edge.to_node).or_default().push(idx);
        self.edges.push(edge);
    }

    /// Declare a loop back-edge (to be excluded from cycle detection).
    pub fn declare_loop_edge(&mut self, from: NodeId, to: NodeId) {
        self.loop_back_edges.insert((from, to));
    }

    /// Get a node by ID.
    pub fn get_node(&self, id: NodeId) -> Option<&GraphNode> {
        self.nodes.get(&id)
    }

    /// Get all nodes.
    pub fn nodes(&self) -> impl Iterator<Item = &GraphNode> {
        self.nodes.values()
    }

    /// Get all node IDs.
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.keys().copied()
    }

    /// Get outgoing edges from a node.
    pub fn outgoing_edges(&self, node: NodeId) -> impl Iterator<Item = &Edge> {
        self.outgoing
            .get(&node)
            .into_iter()
            .flat_map(|indices| indices.iter().map(|&i| &self.edges[i]))
    }

    /// Get incoming edges to a node.
    pub fn incoming_edges(&self, node: NodeId) -> impl Iterator<Item = &Edge> {
        self.incoming
            .get(&node)
            .into_iter()
            .flat_map(|indices| indices.iter().map(|&i| &self.edges[i]))
    }

    /// Get entry points (trigger nodes).
    pub fn entry_points(&self) -> &[NodeId] {
        &self.entry_points
    }

    /// Manually set a node as an entry point.
    ///
    /// This is useful for testing when you don't want to use trigger:: prefixed nodes.
    pub fn set_entry_point(&mut self, node_id: NodeId) {
        if !self.entry_points.contains(&node_id) {
            self.entry_points.push(node_id);
        }
    }

    /// Check if an edge is a declared loop back-edge.
    pub fn is_loop_back_edge(&self, from: NodeId, to: NodeId) -> bool {
        self.loop_back_edges.contains(&(from, to))
    }

    /// Perform topological sort using Kahn's algorithm.
    ///
    /// Returns nodes in execution order, or an error if there are undeclared cycles.
    pub fn topological_sort(&self) -> Result<Vec<NodeId>> {
        // Calculate in-degree for each node, excluding loop back-edges
        let mut in_degree: HashMap<NodeId, usize> = HashMap::new();
        for node_id in self.nodes.keys() {
            in_degree.insert(*node_id, 0);
        }

        for edge in &self.edges {
            if !self.is_loop_back_edge(edge.from_node, edge.to_node) {
                *in_degree.entry(edge.to_node).or_default() += 1;
            }
        }

        // Queue nodes with no incoming edges (entry points)
        let mut queue: VecDeque<NodeId> = in_degree
            .iter()
            .filter(|&(_, degree)| *degree == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut sorted = Vec::with_capacity(self.nodes.len());
        let mut visited = 0;

        while let Some(node_id) = queue.pop_front() {
            sorted.push(node_id);
            visited += 1;

            // Reduce in-degree of downstream nodes
            for edge in self.outgoing_edges(node_id) {
                if !self.is_loop_back_edge(edge.from_node, edge.to_node) {
                    if let Some(degree) = in_degree.get_mut(&edge.to_node) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(edge.to_node);
                        }
                    }
                }
            }
        }

        // Check for undeclared cycles
        if visited != self.nodes.len() {
            let cyclic_nodes: Vec<NodeId> = self
                .nodes
                .keys()
                .filter(|id| !sorted.contains(id))
                .copied()
                .collect();

            return Err(XervError::UncontrolledCycle {
                nodes: cyclic_nodes,
            });
        }

        Ok(sorted)
    }

    /// Find all nodes reachable from entry points.
    pub fn reachable_nodes(&self) -> HashSet<NodeId> {
        let mut visited = HashSet::new();
        let mut queue: VecDeque<NodeId> = self.entry_points.iter().copied().collect();

        while let Some(node_id) = queue.pop_front() {
            if visited.insert(node_id) {
                for edge in self.outgoing_edges(node_id) {
                    queue.push_back(edge.to_node);
                }
            }
        }

        visited
    }

    /// Find predecessors of a node (excluding loop back-edges).
    pub fn predecessors(&self, node: NodeId) -> Vec<NodeId> {
        self.incoming_edges(node)
            .filter(|edge| !self.is_loop_back_edge(edge.from_node, edge.to_node))
            .map(|edge| edge.from_node)
            .collect()
    }

    /// Find successors of a node.
    pub fn successors(&self, node: NodeId) -> Vec<NodeId> {
        self.outgoing_edges(node).map(|edge| edge.to_node).collect()
    }

    /// Validate the graph structure.
    pub fn validate(&self) -> Result<()> {
        // Check for unreachable nodes
        let reachable = self.reachable_nodes();
        for node_id in self.nodes.keys() {
            if !reachable.contains(node_id) {
                let node = self.nodes.get(node_id).unwrap();
                if !node.is_trigger {
                    tracing::warn!(
                        node_id = %node_id,
                        node_type = %node.node_type,
                        "Node is unreachable from entry points"
                    );
                }
            }
        }

        // Validate topological sort (catches undeclared cycles)
        self.topological_sort()?;

        // Validate edge references
        for edge in &self.edges {
            if !self.nodes.contains_key(&edge.from_node) {
                return Err(XervError::InvalidEdge {
                    from_node: edge.from_node,
                    from_port: edge.from_port.clone(),
                    to_node: edge.to_node,
                    to_port: edge.to_port.clone(),
                });
            }
            if !self.nodes.contains_key(&edge.to_node) {
                return Err(XervError::InvalidEdge {
                    from_node: edge.from_node,
                    from_port: edge.from_port.clone(),
                    to_node: edge.to_node,
                    to_port: edge.to_port.clone(),
                });
            }
        }

        Ok(())
    }
}

impl Default for FlowGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32, node_type: &str) -> GraphNode {
        GraphNode::new(NodeId::new(id), node_type)
    }

    #[test]
    fn linear_graph_topo_sort() {
        let mut graph = FlowGraph::new();
        graph.add_node(node(0, "trigger::webhook"));
        graph.add_node(node(1, "std::json_parse"));
        graph.add_node(node(2, "std::log"));

        graph.add_edge(Edge::new(NodeId::new(0), "out", NodeId::new(1), "in"));
        graph.add_edge(Edge::new(NodeId::new(1), "out", NodeId::new(2), "in"));

        let sorted = graph.topological_sort().unwrap();
        assert_eq!(sorted, vec![NodeId::new(0), NodeId::new(1), NodeId::new(2)]);
    }

    #[test]
    fn diamond_graph_topo_sort() {
        let mut graph = FlowGraph::new();
        graph.add_node(node(0, "trigger::webhook"));
        graph.add_node(node(1, "std::branch_a"));
        graph.add_node(node(2, "std::branch_b"));
        graph.add_node(node(3, "std::merge"));

        graph.add_edge(Edge::new(NodeId::new(0), "out", NodeId::new(1), "in"));
        graph.add_edge(Edge::new(NodeId::new(0), "out", NodeId::new(2), "in"));
        graph.add_edge(Edge::new(NodeId::new(1), "out", NodeId::new(3), "in"));
        graph.add_edge(Edge::new(NodeId::new(2), "out", NodeId::new(3), "in"));

        let sorted = graph.topological_sort().unwrap();
        // Node 0 must come first, node 3 must come last
        assert_eq!(sorted[0], NodeId::new(0));
        assert_eq!(sorted[3], NodeId::new(3));
    }

    #[test]
    fn undeclared_cycle_detected() {
        let mut graph = FlowGraph::new();
        graph.add_node(node(0, "trigger::webhook"));
        graph.add_node(node(1, "std::a"));
        graph.add_node(node(2, "std::b"));

        graph.add_edge(Edge::new(NodeId::new(0), "out", NodeId::new(1), "in"));
        graph.add_edge(Edge::new(NodeId::new(1), "out", NodeId::new(2), "in"));
        graph.add_edge(Edge::new(NodeId::new(2), "out", NodeId::new(1), "in")); // Cycle!

        let result = graph.topological_sort();
        assert!(matches!(result, Err(XervError::UncontrolledCycle { .. })));
    }

    #[test]
    fn declared_loop_allowed() {
        let mut graph = FlowGraph::new();
        graph.add_node(node(0, "trigger::webhook"));
        graph.add_node(node(1, "std::loop"));
        graph.add_node(node(2, "std::process"));

        graph.add_edge(Edge::new(NodeId::new(0), "out", NodeId::new(1), "in"));
        graph.add_edge(Edge::new(NodeId::new(1), "continue", NodeId::new(2), "in"));
        graph.add_edge(Edge::new(NodeId::new(2), "out", NodeId::new(1), "in")); // Back-edge

        // Declare the back-edge as a loop
        graph.declare_loop_edge(NodeId::new(2), NodeId::new(1));

        let sorted = graph.topological_sort();
        assert!(sorted.is_ok());
    }
}
