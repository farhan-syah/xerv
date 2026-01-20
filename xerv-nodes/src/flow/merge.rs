//! Merge node (N→1 barrier).
//!
//! The merge node waits for all expected inputs before continuing.
//! It collects data from multiple upstream nodes and merges them into
//! a single output.

use std::collections::HashMap;
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput, Port, PortDirection};
use xerv_core::types::RelPtr;

/// Configuration for merge behavior.
#[derive(Debug, Clone, Default)]
pub enum MergeStrategy {
    /// Wait for all inputs (barrier semantics).
    #[default]
    WaitAll,
    /// Proceed when any input arrives (race semantics).
    FirstArrival,
    /// Wait for a specific number of inputs.
    WaitN(usize),
}

/// Merge node - N→1 barrier.
///
/// Waits for multiple inputs to arrive before proceeding.
/// The output contains all merged inputs.
///
/// # Ports
/// - Input: Multiple named inputs (e.g., "in_0", "in_1", "in_2")
/// - Output: "out" - Merged data containing all inputs
///
/// # Example Configuration
/// ```yaml
/// nodes:
///   merge_results:
///     type: std::merge
///     config:
///       strategy: wait_all  # or first_arrival, wait_n
///       wait_count: 3       # for wait_n strategy
///     inputs:
///       - from: branch_a.out -> in_0
///       - from: branch_b.out -> in_1
///       - from: branch_c.out -> in_2
/// ```
#[derive(Debug)]
pub struct MergeNode {
    /// Number of expected inputs.
    expected_inputs: usize,
    /// Merge strategy.
    strategy: MergeStrategy,
}

impl MergeNode {
    /// Create a new merge node expecting the given number of inputs.
    pub fn new(expected_inputs: usize) -> Self {
        Self {
            expected_inputs,
            strategy: MergeStrategy::WaitAll,
        }
    }

    /// Create a merge node with custom strategy.
    pub fn with_strategy(expected_inputs: usize, strategy: MergeStrategy) -> Self {
        Self {
            expected_inputs,
            strategy,
        }
    }
}

impl Node for MergeNode {
    fn info(&self) -> NodeInfo {
        let mut inputs = Vec::with_capacity(self.expected_inputs);
        for i in 0..self.expected_inputs {
            inputs.push(Port::named(
                format!("in_{}", i),
                PortDirection::Input,
                "Any",
            ));
        }

        NodeInfo::new("std", "merge")
            .with_description("N→1 barrier that waits for all inputs before continuing")
            .with_inputs(inputs)
            .with_outputs(vec![Port::output("Any"), Port::error()])
    }

    fn execute<'a>(&'a self, _ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a> {
        Box::pin(async move {
            let received = inputs.len();

            match &self.strategy {
                MergeStrategy::WaitAll => {
                    if received < self.expected_inputs {
                        tracing::debug!(
                            expected = self.expected_inputs,
                            received = received,
                            "Merge waiting for more inputs"
                        );
                        // In a real implementation, this would block until all inputs arrive.
                        // For now, we proceed with what we have.
                    }
                }
                MergeStrategy::FirstArrival => {
                    // Proceed as soon as any input arrives
                }
                MergeStrategy::WaitN(n) => {
                    if received < *n {
                        tracing::debug!(
                            expected = n,
                            received = received,
                            "Merge waiting for more inputs (wait_n)"
                        );
                    }
                }
            }

            // For now, return the first input as the merged output.
            // A proper implementation would combine all inputs into a single structure.
            if let Some((_, ptr)) = inputs.into_iter().next() {
                Ok(NodeOutput::out(ptr))
            } else {
                Ok(NodeOutput::out(RelPtr::<()>::null()))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_node_info() {
        let node = MergeNode::new(3);
        let info = node.info();

        assert_eq!(info.name, "std::merge");
        assert_eq!(info.inputs.len(), 3);
        assert_eq!(info.inputs[0].name, "in_0");
        assert_eq!(info.inputs[1].name, "in_1");
        assert_eq!(info.inputs[2].name, "in_2");
    }

    #[test]
    fn merge_strategy_default() {
        let strategy = MergeStrategy::default();
        assert!(matches!(strategy, MergeStrategy::WaitAll));
    }
}
