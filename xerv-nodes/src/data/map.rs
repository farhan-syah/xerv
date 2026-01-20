//! Map node (field transformation).
//!
//! Transforms data by renaming, extracting, or restructuring fields.
//! Designed for zero-copy operations where possible.

use std::collections::HashMap;
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput, Port, PortDirection};
use xerv_core::types::RelPtr;
use xerv_core::value::Value;

/// A single field mapping operation.
#[derive(Debug, Clone)]
pub struct FieldMapping {
    /// Source field path (e.g., "user.name" or "$.data.id").
    pub from: String,
    /// Target field name in the output.
    pub to: String,
    /// Optional default value if source is missing.
    pub default: Option<Value>,
}

impl FieldMapping {
    /// Create a field mapping.
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            default: None,
        }
    }

    /// Create a mapping that renames a field (same name transformation).
    pub fn rename(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self::new(from, to)
    }

    /// Create a mapping that extracts a nested field to a top-level field.
    pub fn extract(nested_path: impl Into<String>, output_name: impl Into<String>) -> Self {
        Self::new(nested_path, output_name)
    }

    /// Set a default value if the source field is missing.
    pub fn with_default(mut self, default: Value) -> Self {
        self.default = Some(default);
        self
    }
}

/// Map node - field transformation.
///
/// Creates a new data structure by mapping fields from the input
/// to new field names in the output. Supports:
/// - Field renaming
/// - Nested field extraction
/// - Default values for missing fields
///
/// # Ports
/// - Input: "in" - Source data
/// - Output: "out" - Transformed data
/// - Output: "error" - Emitted on errors
///
/// # Example Configuration
/// ```yaml
/// nodes:
///   transform_user:
///     type: std::map
///     config:
///       mappings:
///         - from: $.user.full_name
///           to: name
///         - from: $.user.email_address
///           to: email
///         - from: $.metadata.created_at
///           to: timestamp
///           default: "unknown"
///     inputs:
///       - from: fetch_user.out -> in
///     outputs:
///       out: -> process_user.in
/// ```
#[derive(Debug)]
pub struct MapNode {
    /// Field mappings to apply.
    mappings: Vec<FieldMapping>,
    /// Whether to include unmapped fields in output.
    passthrough: bool,
}

impl MapNode {
    /// Create a map node with the given field mappings.
    pub fn new(mappings: Vec<FieldMapping>) -> Self {
        Self {
            mappings,
            passthrough: false,
        }
    }

    /// Create an empty map node.
    pub fn empty() -> Self {
        Self {
            mappings: Vec::new(),
            passthrough: false,
        }
    }

    /// Add a field mapping.
    pub fn with_mapping(mut self, mapping: FieldMapping) -> Self {
        self.mappings.push(mapping);
        self
    }

    /// Add a simple rename mapping.
    pub fn rename(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.mappings.push(FieldMapping::rename(from, to));
        self
    }

    /// Enable passthrough of unmapped fields.
    pub fn with_passthrough(mut self) -> Self {
        self.passthrough = true;
        self
    }

    /// Apply mappings to create the output value.
    fn apply_mappings(&self, input: &Value) -> Value {
        let mut output = serde_json::Map::new();

        // If passthrough is enabled, start with all input fields
        if self.passthrough {
            if let Some(obj) = input.inner().as_object() {
                for (key, value) in obj {
                    output.insert(key.clone(), value.clone());
                }
            }
        }

        // Apply explicit mappings (these override passthrough fields)
        for mapping in &self.mappings {
            let value = input
                .get_field(&mapping.from)
                .map(|v| v.into_inner())
                .or_else(|| mapping.default.as_ref().map(|d| d.inner().clone()));

            if let Some(v) = value {
                // Handle nested output paths (e.g., "user.name")
                self.set_nested_field(&mut output, &mapping.to, v);
            }
        }

        Value::from(serde_json::Value::Object(output))
    }

    /// Set a potentially nested field in the output.
    fn set_nested_field(
        &self,
        output: &mut serde_json::Map<String, serde_json::Value>,
        path: &str,
        value: serde_json::Value,
    ) {
        let parts: Vec<&str> = path.split('.').collect();
        Self::set_path_value(output, &parts, value);
    }

    /// Recursively set a value at a path.
    fn set_path_value(
        map: &mut serde_json::Map<String, serde_json::Value>,
        parts: &[&str],
        value: serde_json::Value,
    ) {
        if parts.is_empty() {
            return;
        }

        let key = parts[0].to_string();

        if parts.len() == 1 {
            map.insert(key, value);
        } else {
            let entry = map
                .entry(key)
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

            if let serde_json::Value::Object(obj) = entry {
                Self::set_path_value(obj, &parts[1..], value);
            } else {
                // Overwrite non-object with object
                let mut new_obj = serde_json::Map::new();
                Self::set_path_value(&mut new_obj, &parts[1..], value);
                *entry = serde_json::Value::Object(new_obj);
            }
        }
    }
}

impl Node for MapNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("std", "map")
            .with_description("Transform data by mapping fields to new names")
            .with_inputs(vec![Port::input("Any")])
            .with_outputs(vec![
                Port::named("out", PortDirection::Output, "Any")
                    .with_description("Transformed data"),
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
                    Ok(bytes) => Value::from_bytes(&bytes).unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "Failed to parse input, using null");
                        Value::null()
                    }),
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to read input, using null");
                        Value::null()
                    }
                }
            };

            // Apply mappings
            let output = self.apply_mappings(&value);

            tracing::debug!(
                mappings = self.mappings.len(),
                passthrough = self.passthrough,
                "Map: applied field transformations"
            );

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
    fn map_node_info() {
        let node = MapNode::empty();
        let info = node.info();

        assert_eq!(info.name, "std::map");
        assert_eq!(info.inputs.len(), 1);
        assert_eq!(info.outputs.len(), 2);
    }

    #[test]
    fn map_simple_rename() {
        let node = MapNode::empty().rename("old_name", "new_name");

        let input = Value::from(json!({"old_name": "value"}));
        let output = node.apply_mappings(&input);

        assert_eq!(output.get_string("new_name"), Some("value".to_string()));
        assert!(output.get_field("old_name").is_none());
    }

    #[test]
    fn map_nested_extraction() {
        let node = MapNode::empty()
            .rename("user.name", "name")
            .rename("user.email", "email");

        let input = Value::from(json!({
            "user": {
                "name": "Alice",
                "email": "alice@example.com"
            }
        }));
        let output = node.apply_mappings(&input);

        assert_eq!(output.get_string("name"), Some("Alice".to_string()));
        assert_eq!(
            output.get_string("email"),
            Some("alice@example.com".to_string())
        );
    }

    #[test]
    fn map_with_default() {
        let node = MapNode::new(vec![
            FieldMapping::new("name", "name").with_default(Value::string("Unknown")),
            FieldMapping::new("missing", "value").with_default(Value::string("default")),
        ]);

        let input = Value::from(json!({"name": "Bob"}));
        let output = node.apply_mappings(&input);

        assert_eq!(output.get_string("name"), Some("Bob".to_string()));
        assert_eq!(output.get_string("value"), Some("default".to_string()));
    }

    #[test]
    fn map_with_passthrough() {
        let node = MapNode::empty().with_passthrough().rename("a", "renamed_a");

        let input = Value::from(json!({"a": 1, "b": 2, "c": 3}));
        let output = node.apply_mappings(&input);

        // a is renamed, b and c pass through
        assert_eq!(output.get_f64("renamed_a"), Some(1.0));
        assert_eq!(output.get_f64("b"), Some(2.0));
        assert_eq!(output.get_f64("c"), Some(3.0));
    }

    #[test]
    fn map_nested_output() {
        let node = MapNode::empty()
            .rename("name", "user.display_name")
            .rename("id", "user.id");

        let input = Value::from(json!({"name": "Test", "id": 42}));
        let output = node.apply_mappings(&input);

        assert_eq!(
            output.get_string("user.display_name"),
            Some("Test".to_string())
        );
        assert_eq!(output.get_f64("user.id"), Some(42.0));
    }

    #[test]
    fn map_empty_input() {
        let node = MapNode::empty().rename("a", "b");

        let input = Value::null();
        let output = node.apply_mappings(&input);

        assert!(output.get_field("b").is_none());
    }
}
