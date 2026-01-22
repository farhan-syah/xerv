//! Split node (fan-out iterator).
//!
//! Takes an array/collection and emits each element separately,
//! enabling parallel processing of individual items.

use crate::registry::{NodeCategory, NodeMetadata, PortDefinition, PortType, icons};
use std::collections::HashMap;
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput, Port, PortDirection};
use xerv_core::types::RelPtr;
use xerv_core::value::Value;

/// Mode for split processing.
#[derive(Debug, Clone, Default)]
pub enum SplitMode {
    /// Process items sequentially (one at a time).
    #[default]
    Sequential,
    /// Process all items in parallel (spawn concurrent traces).
    Parallel,
    /// Process items in batches of the specified size.
    Batched(usize),
}

/// Split node - fan-out iterator.
///
/// Takes an array field from the input and emits each element as a separate
/// output. This enables "for each item" processing patterns.
///
/// # Ports
/// - Input: "in" - Data containing an array field
/// - Output: "out" - Emitted for each item in the array
/// - Output: "done" - Emitted when all items have been processed
/// - Output: "error" - Emitted on errors
///
/// # Example Configuration
/// ```yaml
/// nodes:
///   process_items:
///     type: std::split
///     config:
///       field: $.items          # Array field to iterate
///       mode: parallel          # sequential | parallel | batched
///       batch_size: 10          # Only for batched mode
///     inputs:
///       - from: fetch_data.out -> in
///     outputs:
///       out: -> process_item.in
///       done: -> finalize.in
/// ```
#[derive(Debug)]
pub struct SplitNode {
    /// The field path containing the array to split.
    field: String,
    /// Processing mode.
    mode: SplitMode,
}

impl SplitNode {
    /// Create a split node that iterates over the specified field.
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            mode: SplitMode::Sequential,
        }
    }

    /// Create a split node with sequential processing.
    pub fn sequential(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            mode: SplitMode::Sequential,
        }
    }

    /// Create a split node with parallel processing.
    pub fn parallel(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            mode: SplitMode::Parallel,
        }
    }

    /// Create a split node with batched processing.
    pub fn batched(field: impl Into<String>, batch_size: usize) -> Self {
        Self {
            field: field.into(),
            mode: SplitMode::Batched(batch_size),
        }
    }

    /// Set the processing mode.
    pub fn with_mode(mut self, mode: SplitMode) -> Self {
        self.mode = mode;
        self
    }
}

impl Node for SplitNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("std", "split")
            .with_description("Fan-out iterator that processes each item in a collection")
            .with_inputs(vec![Port::input("Any")])
            .with_outputs(vec![
                Port::named("out", PortDirection::Output, "Any")
                    .with_description("Emitted for each item"),
                Port::named("done", PortDirection::Output, "Any")
                    .with_description("Emitted when all items processed"),
                Port::error(),
            ])
    }

    fn execute<'a>(&'a self, ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a> {
        Box::pin(async move {
            let input = inputs.get("in").copied().unwrap_or_else(RelPtr::null);

            // Read and parse input data
            let value = if input.is_null() {
                return Ok(NodeOutput::error_with_message("No input provided"));
            } else {
                match ctx.read_bytes(input) {
                    Ok(bytes) => match Value::from_bytes(&bytes) {
                        Ok(v) => v,
                        Err(e) => {
                            return Ok(NodeOutput::error_with_message(format!(
                                "Failed to parse input: {}",
                                e
                            )));
                        }
                    },
                    Err(e) => {
                        return Ok(NodeOutput::error_with_message(format!(
                            "Failed to read input: {}",
                            e
                        )));
                    }
                }
            };

            // Get the array field
            let array = match value.get_field(&self.field) {
                Some(field_value) => field_value,
                None => {
                    return Ok(NodeOutput::error_with_message(format!(
                        "Field '{}' not found",
                        self.field
                    )));
                }
            };

            // Check if it's an array
            let items = match array.inner().as_array() {
                Some(arr) => arr,
                None => {
                    return Ok(NodeOutput::error_with_message(format!(
                        "Field '{}' is not an array",
                        self.field
                    )));
                }
            };

            if items.is_empty() {
                // No items, emit done immediately
                tracing::debug!(field = %self.field, "Split: empty array, emitting done");
                return Ok(NodeOutput::new("done", input));
            }

            // Get current iteration index from context (using loop iteration tracking)
            let split_node_id = ctx.node_id();
            let iteration = ctx.get_loop_iteration(split_node_id) as usize;

            tracing::debug!(
                field = %self.field,
                iteration = iteration,
                total_items = items.len(),
                mode = ?self.mode,
                "Split: processing iteration"
            );

            // Check if we've emitted all items
            if iteration >= items.len() {
                // All items emitted, reset iteration counter and emit done
                ctx.reset_loop_iteration(split_node_id);
                tracing::debug!(
                    field = %self.field,
                    total_items = items.len(),
                    "Split: all items emitted, sending done"
                );
                return Ok(NodeOutput::new("done", input));
            }

            // Emit the current item
            let current_item = Value::from(items[iteration].clone());
            let item_bytes = match current_item.to_bytes() {
                Ok(bytes) => bytes,
                Err(e) => {
                    return Ok(NodeOutput::error_with_message(format!(
                        "Failed to serialize item at index {}: {}",
                        iteration, e
                    )));
                }
            };

            // Write the current item to arena
            let item_ptr = match ctx.write_bytes(&item_bytes) {
                Ok(ptr) => ptr,
                Err(e) => {
                    return Ok(NodeOutput::error_with_message(format!(
                        "Failed to write item at index {}: {}",
                        iteration, e
                    )));
                }
            };

            // Increment iteration counter for next invocation
            ctx.increment_loop_iteration(split_node_id);

            tracing::debug!(
                field = %self.field,
                iteration = iteration,
                total_items = items.len(),
                "Split: emitting item {}/{}", iteration + 1, items.len()
            );

            // Emit the current item on "out" port
            // The executor will see this output and should re-invoke the split node
            // to process the next item, or execute downstream nodes first depending on mode
            Ok(NodeOutput::new("out", item_ptr))
        })
    }
}

impl NodeMetadata for SplitNode {
    fn metadata() -> crate::registry::NodeInfo {
        crate::registry::NodeInfo {
            node_type: "std::split".to_string(),
            category: NodeCategory::Data,
            display_name: "Split".to_string(),
            description: "Fan-out iterator that processes each item in a collection separately. Takes an array field from the input and emits each element as a separate output.".to_string(),
            icon: icons::ICON_SPLIT,
            inputs: vec![PortDefinition::required("in", PortType::Any)],
            outputs: vec![
                PortDefinition::required("out", PortType::Any),
                PortDefinition::required("done", PortType::Any),
                PortDefinition::required("error", PortType::Any),
            ],
            config_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "field": {
                        "type": "string",
                        "description": "Array field path to iterate (e.g., $.items or data.list)"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["sequential", "parallel", "batched"],
                        "default": "sequential",
                        "description": "Processing mode: sequential (one at a time), parallel (concurrent), batched (groups)"
                    },
                    "batch_size": {
                        "type": "integer",
                        "description": "Batch size (only for batched mode)",
                        "minimum": 1
                    }
                },
                "required": ["field"]
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_node_info() {
        let node = SplitNode::new("items");
        let info = node.info();

        assert_eq!(info.name, "std::split");
        assert_eq!(info.inputs.len(), 1);
        assert_eq!(info.outputs.len(), 3);
        assert_eq!(info.outputs[0].name, "out");
        assert_eq!(info.outputs[1].name, "done");
        assert_eq!(info.outputs[2].name, "error");
    }

    #[test]
    fn split_mode_default() {
        let mode = SplitMode::default();
        assert!(matches!(mode, SplitMode::Sequential));
    }

    #[test]
    fn split_with_mode() {
        let node = SplitNode::new("items").with_mode(SplitMode::Parallel);
        assert!(matches!(node.mode, SplitMode::Parallel));
    }

    #[test]
    fn split_batched() {
        let node = SplitNode::batched("items", 5);
        assert!(matches!(node.mode, SplitMode::Batched(5)));
    }

    #[test]
    fn split_constructors() {
        let seq = SplitNode::sequential("items");
        assert!(matches!(seq.mode, SplitMode::Sequential));

        let par = SplitNode::parallel("items");
        assert!(matches!(par.mode, SplitMode::Parallel));
    }
}
