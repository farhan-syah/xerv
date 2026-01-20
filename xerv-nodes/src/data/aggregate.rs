//! Aggregate node (numeric aggregation).
//!
//! Aggregates numeric values from arrays or multiple fields.

use std::collections::HashMap;
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput, Port, PortDirection};
use xerv_core::types::RelPtr;
use xerv_core::value::Value;

/// Aggregation operation to perform.
#[derive(Debug, Clone, Copy, Default)]
pub enum AggregateOperation {
    /// Sum all values.
    #[default]
    Sum,
    /// Calculate the average.
    Average,
    /// Find the minimum value.
    Min,
    /// Find the maximum value.
    Max,
    /// Count the number of values.
    Count,
    /// Calculate the product of all values.
    Product,
}

impl AggregateOperation {
    /// Apply the aggregation operation to a list of values.
    fn apply(&self, values: &[f64]) -> f64 {
        if values.is_empty() {
            return match self {
                Self::Count => 0.0,
                Self::Sum | Self::Average | Self::Product => 0.0,
                Self::Min | Self::Max => f64::NAN,
            };
        }

        match self {
            Self::Sum => values.iter().sum(),
            Self::Average => values.iter().sum::<f64>() / values.len() as f64,
            Self::Min => values.iter().cloned().fold(f64::INFINITY, f64::min),
            Self::Max => values.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            Self::Count => values.len() as f64,
            Self::Product => values.iter().product(),
        }
    }
}

/// Aggregate node - numeric aggregation.
///
/// Performs aggregation operations on numeric values from:
/// - An array field (aggregates all elements)
/// - Multiple specified fields (aggregates their values)
///
/// # Ports
/// - Input: "in" - Source data with numeric values
/// - Output: "out" - Object with aggregation result
/// - Output: "error" - Emitted on errors
///
/// # Example Configuration
/// ```yaml
/// nodes:
///   calculate_total:
///     type: std::aggregate
///     config:
///       operation: sum
///       source: $.items[*].price    # Array field
///       output_field: total
///     inputs:
///       - from: cart.out -> in
///     outputs:
///       out: -> checkout.in
/// ```
#[derive(Debug)]
pub struct AggregateNode {
    /// The aggregation operation to perform.
    operation: AggregateOperation,
    /// Source: either an array field path or list of field paths.
    source: AggregateSource,
    /// Output field name for the result.
    output_field: String,
}

/// Source of values for aggregation.
#[derive(Debug, Clone)]
pub enum AggregateSource {
    /// Aggregate values from an array field.
    ArrayField {
        /// Path to the array field.
        array_path: String,
        /// Optional sub-field to extract from each element.
        value_field: Option<String>,
    },
    /// Aggregate values from multiple specific fields.
    Fields(Vec<String>),
}

impl AggregateNode {
    /// Create an aggregate node that operates on an array field.
    pub fn array(
        array_path: impl Into<String>,
        operation: AggregateOperation,
        output_field: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            source: AggregateSource::ArrayField {
                array_path: array_path.into(),
                value_field: None,
            },
            output_field: output_field.into(),
        }
    }

    /// Create an aggregate node that operates on an array of objects,
    /// extracting a specific field from each.
    pub fn array_field(
        array_path: impl Into<String>,
        value_field: impl Into<String>,
        operation: AggregateOperation,
        output_field: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            source: AggregateSource::ArrayField {
                array_path: array_path.into(),
                value_field: Some(value_field.into()),
            },
            output_field: output_field.into(),
        }
    }

    /// Create an aggregate node that operates on multiple fields.
    pub fn fields(
        fields: Vec<String>,
        operation: AggregateOperation,
        output_field: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            source: AggregateSource::Fields(fields),
            output_field: output_field.into(),
        }
    }

    /// Create a sum aggregation over an array.
    pub fn sum(array_path: impl Into<String>, output_field: impl Into<String>) -> Self {
        Self::array(array_path, AggregateOperation::Sum, output_field)
    }

    /// Create an average aggregation over an array.
    pub fn average(array_path: impl Into<String>, output_field: impl Into<String>) -> Self {
        Self::array(array_path, AggregateOperation::Average, output_field)
    }

    /// Create a count aggregation over an array.
    pub fn count(array_path: impl Into<String>, output_field: impl Into<String>) -> Self {
        Self::array(array_path, AggregateOperation::Count, output_field)
    }

    /// Extract numeric values from input based on source configuration.
    fn extract_values(&self, input: &Value) -> Vec<f64> {
        match &self.source {
            AggregateSource::ArrayField {
                array_path,
                value_field,
            } => {
                let Some(array_value) = input.get_field(array_path) else {
                    return Vec::new();
                };

                let Some(array) = array_value.inner().as_array() else {
                    return Vec::new();
                };

                array
                    .iter()
                    .filter_map(|item| {
                        if let Some(field) = value_field {
                            // Extract sub-field from each object
                            Value::from(item.clone())
                                .get_field(field)
                                .and_then(|v| v.as_f64())
                        } else {
                            // Use the element directly
                            item.as_f64()
                        }
                    })
                    .collect()
            }
            AggregateSource::Fields(fields) => fields
                .iter()
                .filter_map(|path| input.get_f64(path))
                .collect(),
        }
    }
}

impl Node for AggregateNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("std", "aggregate")
            .with_description("Aggregate numeric values (sum, avg, min, max, count)")
            .with_inputs(vec![Port::input("Any")])
            .with_outputs(vec![
                Port::named("out", PortDirection::Output, "Any")
                    .with_description("Object with aggregation result"),
                Port::error(),
            ])
    }

    fn execute<'a>(&'a self, ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a> {
        Box::pin(async move {
            let input = inputs.get("in").copied().unwrap_or_else(RelPtr::null);

            // Read and parse input data
            let value = if input.is_null() {
                Value::null()
            } else {
                match ctx.read_bytes(input) {
                    Ok(bytes) => Value::from_bytes(&bytes).unwrap_or_else(|_| Value::null()),
                    Err(_) => Value::null(),
                }
            };

            // Extract values and compute aggregate
            let values = self.extract_values(&value);
            let result = self.operation.apply(&values);

            tracing::debug!(
                operation = ?self.operation,
                value_count = values.len(),
                result = result,
                output_field = %self.output_field,
                "Aggregate: computed result"
            );

            // Create output object
            let output = Value::from(serde_json::json!({
                &self.output_field: result
            }));

            // Write output to arena
            let output_bytes = match output.to_bytes() {
                Ok(bytes) => bytes,
                Err(e) => {
                    return Ok(NodeOutput::error_with_message(format!(
                        "Failed to serialize output: {}",
                        e
                    )));
                }
            };

            let output_ptr = match ctx.write_bytes(&output_bytes) {
                Ok(ptr) => ptr,
                Err(e) => {
                    return Ok(NodeOutput::error_with_message(format!(
                        "Failed to write output: {}",
                        e
                    )));
                }
            };

            Ok(NodeOutput::out(output_ptr))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn aggregate_node_info() {
        let node = AggregateNode::sum("values", "total");
        let info = node.info();

        assert_eq!(info.name, "std::aggregate");
        assert_eq!(info.inputs.len(), 1);
        assert_eq!(info.outputs.len(), 2);
    }

    #[test]
    fn aggregate_sum_array() {
        let node = AggregateNode::sum("numbers", "total");
        let input = Value::from(json!({"numbers": [1, 2, 3, 4, 5]}));
        let values = node.extract_values(&input);
        let result = node.operation.apply(&values);

        assert_eq!(result, 15.0);
    }

    #[test]
    fn aggregate_average_array() {
        let node = AggregateNode::average("scores", "avg");
        let input = Value::from(json!({"scores": [10, 20, 30]}));
        let values = node.extract_values(&input);
        let result = node.operation.apply(&values);

        assert_eq!(result, 20.0);
    }

    #[test]
    fn aggregate_min_max() {
        let input = Value::from(json!({"values": [5, 2, 8, 1, 9]}));

        let min_node = AggregateNode::array("values", AggregateOperation::Min, "min");
        let min_values = min_node.extract_values(&input);
        assert_eq!(min_node.operation.apply(&min_values), 1.0);

        let max_node = AggregateNode::array("values", AggregateOperation::Max, "max");
        let max_values = max_node.extract_values(&input);
        assert_eq!(max_node.operation.apply(&max_values), 9.0);
    }

    #[test]
    fn aggregate_count() {
        let node = AggregateNode::count("items", "count");
        let input = Value::from(json!({"items": [1, 2, 3, 4, 5, 6]}));
        let values = node.extract_values(&input);
        let result = node.operation.apply(&values);

        assert_eq!(result, 6.0);
    }

    #[test]
    fn aggregate_array_of_objects() {
        let node = AggregateNode::array_field("items", "price", AggregateOperation::Sum, "total");

        let input = Value::from(json!({
            "items": [
                {"name": "A", "price": 10.0},
                {"name": "B", "price": 20.0},
                {"name": "C", "price": 30.0}
            ]
        }));

        let values = node.extract_values(&input);
        let result = node.operation.apply(&values);

        assert_eq!(result, 60.0);
    }

    #[test]
    fn aggregate_multiple_fields() {
        let node = AggregateNode::fields(
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            AggregateOperation::Sum,
            "sum",
        );

        let input = Value::from(json!({"a": 10, "b": 20, "c": 30}));
        let values = node.extract_values(&input);
        let result = node.operation.apply(&values);

        assert_eq!(result, 60.0);
    }

    #[test]
    fn aggregate_empty_array() {
        let node = AggregateNode::sum("empty", "total");
        let input = Value::from(json!({"empty": []}));
        let values = node.extract_values(&input);
        let result = node.operation.apply(&values);

        assert_eq!(result, 0.0);
    }

    #[test]
    fn aggregate_missing_field() {
        let node = AggregateNode::sum("missing", "total");
        let input = Value::from(json!({"other": [1, 2, 3]}));
        let values = node.extract_values(&input);
        let result = node.operation.apply(&values);

        assert_eq!(result, 0.0);
    }

    #[test]
    fn aggregate_product() {
        let node = AggregateNode::array("factors", AggregateOperation::Product, "product");
        let input = Value::from(json!({"factors": [2, 3, 4]}));
        let values = node.extract_values(&input);
        let result = node.operation.apply(&values);

        assert_eq!(result, 24.0);
    }
}
