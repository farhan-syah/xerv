//! Trace execution state.

use crate::scheduler::FlowGraph;
use std::collections::{HashMap, HashSet};
use xerv_core::arena::{Arena, ArenaReader, ArenaWriter};
use xerv_core::traits::NodeOutput;
use xerv_core::types::{NodeId, RelPtr, TraceId};

/// State of a running trace.
pub struct TraceState {
    /// Trace ID.
    pub trace_id: TraceId,
    /// The arena for this trace.
    arena: Arena,
    /// Trigger that started this trace.
    pub trigger_id: String,
    /// Initial input data.
    pub initial_input: RelPtr<()>,
    /// Outputs from each node (node_id -> (port, pointer)).
    outputs: HashMap<NodeId, NodeOutput>,
    /// Nodes that have completed execution.
    completed: HashSet<NodeId>,
    /// Nodes that are currently executing.
    executing: HashSet<NodeId>,
    /// Loop iteration state (for std::loop nodes).
    loop_state: HashMap<NodeId, LoopState>,
}

/// Loop iteration state.
#[derive(Debug, Clone)]
pub struct LoopState {
    /// Current iteration number.
    pub iteration: u32,
    /// Maximum iterations allowed.
    pub max_iterations: u32,
    /// Whether the loop has exited.
    pub exited: bool,
}

impl std::fmt::Debug for TraceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceState")
            .field("trace_id", &self.trace_id)
            .field("trigger_id", &self.trigger_id)
            .field("outputs", &self.outputs.len())
            .field("completed", &self.completed.len())
            .field("executing", &self.executing.len())
            .finish()
    }
}

impl TraceState {
    /// Create a new trace state.
    pub fn new(
        trace_id: TraceId,
        arena: Arena,
        trigger_id: String,
        initial_input: RelPtr<()>,
    ) -> Self {
        Self {
            trace_id,
            arena,
            trigger_id,
            initial_input,
            outputs: HashMap::new(),
            completed: HashSet::new(),
            executing: HashSet::new(),
            loop_state: HashMap::new(),
        }
    }

    /// Create a trace state for crash recovery.
    ///
    /// Reconstructs state from a recovered arena and WAL information about
    /// which nodes were previously completed.
    pub fn from_recovery(
        trace_id: TraceId,
        arena: Arena,
        trigger_id: String,
        initial_input: RelPtr<()>,
        completed_nodes: HashMap<NodeId, NodeOutput>,
    ) -> Self {
        let completed: HashSet<NodeId> = completed_nodes.keys().copied().collect();
        Self {
            trace_id,
            arena,
            trigger_id,
            initial_input,
            outputs: completed_nodes,
            completed,
            executing: HashSet::new(),
            loop_state: HashMap::new(),
        }
    }

    /// Get arena reader and writer handles.
    pub fn arena_handles(&self) -> (ArenaReader, ArenaWriter) {
        (self.arena.reader(), self.arena.writer())
    }

    /// Flush the arena to disk.
    ///
    /// This is called before suspension to ensure all data is persisted.
    pub fn flush_arena(&self) -> xerv_core::error::Result<()> {
        self.arena.flush()
    }

    /// Get the path to the arena file.
    pub fn arena_path(&self) -> std::path::PathBuf {
        self.arena.path()
    }

    /// Check if a node is ready to execute.
    ///
    /// A node is ready if all its predecessors have completed (or it's an entry point).
    pub fn is_node_ready(&self, node_id: NodeId, graph: &FlowGraph) -> bool {
        // Check if this is an entry point (trigger)
        if graph.entry_points().contains(&node_id) {
            return !self.completed.contains(&node_id) && !self.executing.contains(&node_id);
        }

        // Check if this node was activated by an upstream node
        let has_input = self.has_pending_input(node_id, graph);

        // Check if all required predecessors have completed
        let predecessors = graph.predecessors(node_id);
        let all_preds_complete = predecessors
            .iter()
            .all(|pred| self.completed.contains(pred));

        has_input
            && all_preds_complete
            && !self.completed.contains(&node_id)
            && !self.executing.contains(&node_id)
    }

    /// Check if a node has pending input from any upstream node.
    fn has_pending_input(&self, node_id: NodeId, graph: &FlowGraph) -> bool {
        for edge in graph.incoming_edges(node_id) {
            if let Some(output) = self.outputs.get(&edge.from_node) {
                if output.matches_port(&edge.from_port) {
                    return true;
                }
            }
        }
        false
    }

    /// Collect inputs for a node from upstream outputs.
    pub fn collect_inputs(
        &self,
        node_id: NodeId,
        graph: &FlowGraph,
    ) -> HashMap<String, RelPtr<()>> {
        let mut inputs = HashMap::new();

        // For entry points, use the initial input
        if graph.entry_points().contains(&node_id) {
            inputs.insert("in".to_string(), self.initial_input);
            return inputs;
        }

        // Collect from upstream nodes
        for edge in graph.incoming_edges(node_id) {
            if let Some(output) = self.outputs.get(&edge.from_node) {
                if output.matches_port(&edge.from_port) {
                    inputs.insert(edge.to_port.clone(), output.data());
                }
            }
        }

        inputs
    }

    /// Record a node's output.
    pub fn record_output(&mut self, node_id: NodeId, output: NodeOutput) {
        self.executing.remove(&node_id);
        self.completed.insert(node_id);
        self.outputs.insert(node_id, output);
    }

    /// Mark a node as executing.
    pub fn mark_executing(&mut self, node_id: NodeId) {
        self.executing.insert(node_id);
    }

    /// Check if a node has completed.
    pub fn is_completed(&self, node_id: NodeId) -> bool {
        self.completed.contains(&node_id)
    }

    /// Get the output of a node.
    pub fn get_output(&self, node_id: NodeId) -> Option<&NodeOutput> {
        self.outputs.get(&node_id)
    }

    /// Initialize loop state for a loop controller node.
    pub fn init_loop(&mut self, node_id: NodeId, max_iterations: u32) {
        self.loop_state.insert(
            node_id,
            LoopState {
                iteration: 0,
                max_iterations,
                exited: false,
            },
        );
    }

    /// Increment loop iteration.
    ///
    /// Returns `true` if the loop can continue, `false` if max iterations reached.
    pub fn increment_loop(&mut self, node_id: NodeId) -> bool {
        if let Some(state) = self.loop_state.get_mut(&node_id) {
            state.iteration += 1;
            state.iteration < state.max_iterations
        } else {
            false
        }
    }

    /// Mark a loop as exited.
    pub fn exit_loop(&mut self, node_id: NodeId) {
        if let Some(state) = self.loop_state.get_mut(&node_id) {
            state.exited = true;
        }
    }

    /// Get loop state for a node.
    pub fn loop_state(&self, node_id: NodeId) -> Option<&LoopState> {
        self.loop_state.get(&node_id)
    }

    /// Reset node completion state for loop re-execution.
    ///
    /// This allows nodes in a loop body to be re-executed.
    pub fn reset_loop_body(&mut self, nodes: &[NodeId]) {
        for &node_id in nodes {
            self.completed.remove(&node_id);
            self.executing.remove(&node_id);
            self.outputs.remove(&node_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xerv_core::arena::ArenaConfig;

    #[test]
    fn trace_state_creation() {
        let trace_id = TraceId::new();
        let arena = Arena::create(trace_id, &ArenaConfig::in_memory()).unwrap();
        let state = TraceState::new(trace_id, arena, "test_trigger".to_string(), RelPtr::null());

        assert_eq!(state.trace_id, trace_id);
        assert_eq!(state.trigger_id, "test_trigger");
    }

    #[test]
    fn loop_state_tracking() {
        let trace_id = TraceId::new();
        let arena = Arena::create(trace_id, &ArenaConfig::in_memory()).unwrap();
        let mut state = TraceState::new(trace_id, arena, "test".to_string(), RelPtr::null());

        let loop_node = NodeId::new(1);
        state.init_loop(loop_node, 5);

        // Can iterate 5 times
        assert!(state.increment_loop(loop_node)); // 1
        assert!(state.increment_loop(loop_node)); // 2
        assert!(state.increment_loop(loop_node)); // 3
        assert!(state.increment_loop(loop_node)); // 4
        assert!(!state.increment_loop(loop_node)); // 5 - max reached

        // Check state
        let loop_state = state.loop_state(loop_node).unwrap();
        assert_eq!(loop_state.iteration, 5);
        assert!(!loop_state.exited);

        // Mark as exited
        state.exit_loop(loop_node);
        let loop_state = state.loop_state(loop_node).unwrap();
        assert!(loop_state.exited);
    }
}
