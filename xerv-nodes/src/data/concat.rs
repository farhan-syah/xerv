//! Concat node (string combination).
//!
//! Combines multiple string fields or values into a single string.

use crate::registry::{NodeCategory, NodeMetadata, PortDefinition, PortType, icons};
use std::collections::HashMap;
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput, Port, PortDirection};
use xerv_core::types::RelPtr;
use xerv_core::value::Value;

/// Concat node - string combination.
///
/// Combines multiple fields or literal values into a single string output.
/// Supports field references, literal strings, and separators.
///
/// # Ports
/// - Input: "in" - Source data containing fields to concatenate
/// - Output: "out" - Object with the concatenated string
/// - Output: "error" - Emitted on errors
///
/// # Example Configuration
/// ```yaml
/// nodes:
///   build_greeting:
///     type: std::concat
///     config:
///       parts:
///         - "Hello, "
///         - $.user.first_name
///         - " "
///         - $.user.last_name
///         - "!"
///       output_field: greeting
///     inputs:
///       - from: get_user.out -> in
///     outputs:
///       out: -> send_message.in
/// ```
#[derive(Debug)]
pub struct ConcatNode {
    /// Parts to concatenate (field paths or literal strings).
    parts: Vec<ConcatPart>,
    /// Separator between parts (empty by default).
    separator: String,
    /// Output field name for the result.
    output_field: String,
}

/// A part in a concatenation operation.
#[derive(Debug, Clone)]
pub enum ConcatPart {
    /// A literal string value.
    Literal(String),
    /// A field path to extract from input.
    Field(String),
}

impl ConcatNode {
    /// Create a concat node with the given parts and output field.
    pub fn new(parts: Vec<ConcatPart>, output_field: impl Into<String>) -> Self {
        Self {
            parts,
            separator: String::new(),
            output_field: output_field.into(),
        }
    }

    /// Create a concat node that joins fields.
    pub fn fields(fields: &[&str], output_field: impl Into<String>) -> Self {
        let parts = fields
            .iter()
            .map(|f| ConcatPart::Field(f.to_string()))
            .collect();
        Self::new(parts, output_field)
    }

    /// Set the separator between parts.
    pub fn with_separator(mut self, separator: impl Into<String>) -> Self {
        self.separator = separator.into();
        self
    }

    /// Add a literal string part.
    pub fn add_literal(mut self, value: impl Into<String>) -> Self {
        self.parts.push(ConcatPart::Literal(value.into()));
        self
    }

    /// Add a field reference part.
    pub fn add_field(mut self, path: impl Into<String>) -> Self {
        self.parts.push(ConcatPart::Field(path.into()));
        self
    }

    /// Build the concatenated string from input.
    fn build_string(&self, input: &Value) -> String {
        let mut result = Vec::new();

        for part in &self.parts {
            match part {
                ConcatPart::Literal(s) => {
                    result.push(s.clone());
                }
                ConcatPart::Field(path) => {
                    if let Some(value) = input.get_field(path) {
                        if let Some(s) = value.as_string() {
                            result.push(s);
                        }
                    }
                }
            }
        }

        result.join(&self.separator)
    }
}

impl Node for ConcatNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("std", "concat")
            .with_description("Combine multiple string fields or values into one")
            .with_inputs(vec![Port::input("Any")])
            .with_outputs(vec![
                Port::named("out", PortDirection::Output, "Any")
                    .with_description("Object with concatenated string"),
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

            // Build the concatenated string
            let result = self.build_string(&value);

            tracing::debug!(
                parts = self.parts.len(),
                result_len = result.len(),
                output_field = %self.output_field,
                "Concat: built string"
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

impl NodeMetadata for ConcatNode {
    fn metadata() -> crate::registry::NodeInfo {
        crate::registry::NodeInfo {
            node_type: "std::concat".to_string(),
            category: NodeCategory::Data,
            display_name: "Concat".to_string(),
            description: "Combine multiple string fields or literal values into a single string. Supports field references, literal strings, and separators between parts.".to_string(),
            icon: icons::ICON_CONCAT,
            inputs: vec![PortDefinition::required("in", PortType::Any)],
            outputs: vec![
                PortDefinition::required("out", PortType::Any),
                PortDefinition::required("error", PortType::Any),
            ],
            config_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "parts": {
                        "type": "array",
                        "description": "Parts to concatenate (field paths or literal strings)",
                        "items": {
                            "oneOf": [
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {"const": "literal"},
                                        "value": {"type": "string"}
                                    }
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": {"const": "field"},
                                        "path": {"type": "string"}
                                    }
                                }
                            ]
                        }
                    },
                    "separator": {
                        "type": "string",
                        "default": "",
                        "description": "Separator between parts"
                    },
                    "output_field": {
                        "type": "string",
                        "description": "Output field name for the concatenated result"
                    }
                },
                "required": ["parts", "output_field"]
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn concat_node_info() {
        let node = ConcatNode::new(vec![], "result");
        let info = node.info();

        assert_eq!(info.name, "std::concat");
        assert_eq!(info.inputs.len(), 1);
        assert_eq!(info.outputs.len(), 2);
    }

    #[test]
    fn concat_literals() {
        let node = ConcatNode::new(
            vec![
                ConcatPart::Literal("Hello".to_string()),
                ConcatPart::Literal("World".to_string()),
            ],
            "result",
        )
        .with_separator(" ");

        let input = Value::null();
        let result = node.build_string(&input);

        assert_eq!(result, "Hello World");
    }

    #[test]
    fn concat_fields() {
        let node = ConcatNode::fields(&["first", "last"], "full_name").with_separator(" ");

        let input = Value::from(json!({"first": "John", "last": "Doe"}));
        let result = node.build_string(&input);

        assert_eq!(result, "John Doe");
    }

    #[test]
    fn concat_mixed() {
        let node = ConcatNode::new(vec![], "greeting")
            .add_literal("Hello, ")
            .add_field("name")
            .add_literal("!");

        let input = Value::from(json!({"name": "Alice"}));
        let result = node.build_string(&input);

        assert_eq!(result, "Hello, Alice!");
    }

    #[test]
    fn concat_nested_fields() {
        let node = ConcatNode::fields(&["user.first", "user.last"], "name").with_separator(" ");

        let input = Value::from(json!({
            "user": {"first": "Jane", "last": "Smith"}
        }));
        let result = node.build_string(&input);

        assert_eq!(result, "Jane Smith");
    }

    #[test]
    fn concat_missing_field() {
        let node = ConcatNode::fields(&["a", "b", "c"], "result").with_separator("-");

        let input = Value::from(json!({"a": "X", "c": "Z"}));
        let result = node.build_string(&input);

        // Missing field "b" is skipped
        assert_eq!(result, "X-Z");
    }

    #[test]
    fn concat_no_separator() {
        let node = ConcatNode::fields(&["a", "b"], "result");

        let input = Value::from(json!({"a": "foo", "b": "bar"}));
        let result = node.build_string(&input);

        assert_eq!(result, "foobar");
    }
}
