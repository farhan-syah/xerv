//! JsonDynamic node (schemaless JSON handling).
//!
//! Handles unstructured or volatile JSON payloads without strict schemas.
//! Enables late binding and runtime field resolution.

use crate::registry::{NodeCategory, NodeMetadata, PortDefinition, PortType, icons};
use std::collections::HashMap;
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput, Port, PortDirection};
use xerv_core::types::RelPtr;
use xerv_core::value::Value;

/// Operation to perform on dynamic JSON.
#[derive(Debug, Clone, Default)]
pub enum JsonOperation {
    /// Pass through the entire input unchanged.
    #[default]
    Passthrough,
    /// Extract specific fields into a new object.
    Extract(Vec<String>),
    /// Merge multiple input objects into one.
    Merge,
    /// Set a field to a specific value.
    Set { path: String, value: Value },
    /// Delete a field from the input.
    Delete(String),
    /// Flatten nested objects to a single level.
    Flatten { separator: String },
}

/// JsonDynamic node - schemaless JSON handling.
///
/// Handles dynamic/volatile JSON payloads that don't conform to strict schemas.
/// Useful for:
/// - Accepting webhook payloads from external services
/// - Working with APIs that change frequently
/// - Late binding before converting to strict types
///
/// # Ports
/// - Input: "in" - Any JSON input
/// - Output: "out" - Processed JSON output
/// - Output: "error" - Emitted on errors
///
/// # Example Configuration
/// ```yaml
/// nodes:
///   handle_webhook:
///     type: std::json_dynamic
///     config:
///       operation: extract
///       fields:
///         - $.data.user.id
///         - $.data.user.email
///         - $.metadata.timestamp
///     inputs:
///       - from: webhook.out -> in
///     outputs:
///       out: -> validate.in
/// ```
#[derive(Debug)]
pub struct JsonDynamicNode {
    /// Operation to perform.
    operation: JsonOperation,
    /// Validation rules (optional).
    required_fields: Vec<String>,
}

impl JsonDynamicNode {
    /// Create a passthrough node (no transformation).
    pub fn passthrough() -> Self {
        Self {
            operation: JsonOperation::Passthrough,
            required_fields: Vec::new(),
        }
    }

    /// Create a node that extracts specific fields.
    pub fn extract(fields: Vec<String>) -> Self {
        Self {
            operation: JsonOperation::Extract(fields),
            required_fields: Vec::new(),
        }
    }

    /// Create a node that merges multiple inputs.
    pub fn merge() -> Self {
        Self {
            operation: JsonOperation::Merge,
            required_fields: Vec::new(),
        }
    }

    /// Create a node that sets a specific field.
    pub fn set(path: impl Into<String>, value: Value) -> Self {
        Self {
            operation: JsonOperation::Set {
                path: path.into(),
                value,
            },
            required_fields: Vec::new(),
        }
    }

    /// Create a node that deletes a field.
    pub fn delete(path: impl Into<String>) -> Self {
        Self {
            operation: JsonOperation::Delete(path.into()),
            required_fields: Vec::new(),
        }
    }

    /// Create a node that flattens nested objects.
    pub fn flatten(separator: impl Into<String>) -> Self {
        Self {
            operation: JsonOperation::Flatten {
                separator: separator.into(),
            },
            required_fields: Vec::new(),
        }
    }

    /// Add required field validation.
    pub fn with_required(mut self, fields: Vec<String>) -> Self {
        self.required_fields = fields;
        self
    }

    /// Process the input according to the configured operation.
    fn process(&self, input: &Value) -> Result<Value, String> {
        // Validate required fields
        for field in &self.required_fields {
            if input.get_field(field).is_none() {
                return Err(format!("Required field '{}' is missing", field));
            }
        }

        match &self.operation {
            JsonOperation::Passthrough => Ok(input.clone()),

            JsonOperation::Extract(fields) => {
                let mut output = serde_json::Map::new();
                for field in fields {
                    if let Some(value) = input.get_field(field) {
                        // Use the last part of the path as the output key
                        let key = field.split('.').next_back().unwrap_or(field);
                        output.insert(key.to_string(), value.into_inner());
                    }
                }
                Ok(Value::from(serde_json::Value::Object(output)))
            }

            JsonOperation::Merge => {
                // For merge, the input should be an array of objects to merge
                if let Some(arr) = input.inner().as_array() {
                    let mut merged = serde_json::Map::new();
                    for item in arr {
                        if let Some(obj) = item.as_object() {
                            for (k, v) in obj {
                                merged.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    Ok(Value::from(serde_json::Value::Object(merged)))
                } else if let Some(obj) = input.inner().as_object() {
                    // Single object, just pass through
                    Ok(Value::from(serde_json::Value::Object(obj.clone())))
                } else {
                    Err("Merge requires object or array of objects".to_string())
                }
            }

            JsonOperation::Set { path, value } => {
                let mut output = input.inner().clone();
                self.set_path(&mut output, path, value.inner().clone());
                Ok(Value::from(output))
            }

            JsonOperation::Delete(path) => {
                let mut output = input.inner().clone();
                self.delete_path(&mut output, path);
                Ok(Value::from(output))
            }

            JsonOperation::Flatten { separator } => {
                let mut flattened = serde_json::Map::new();
                self.flatten_recursive(input.inner(), String::new(), separator, &mut flattened);
                Ok(Value::from(serde_json::Value::Object(flattened)))
            }
        }
    }

    /// Set a value at a path in a JSON structure.
    fn set_path(&self, obj: &mut serde_json::Value, path: &str, value: serde_json::Value) {
        let parts: Vec<&str> = path.split('.').collect();
        Self::set_path_recursive(obj, &parts, value);
    }

    /// Recursively set a value at a path.
    fn set_path_recursive(obj: &mut serde_json::Value, parts: &[&str], value: serde_json::Value) {
        if parts.is_empty() {
            return;
        }

        // Ensure obj is an object
        if !obj.is_object() {
            *obj = serde_json::Value::Object(serde_json::Map::new());
        }

        let obj_map = obj.as_object_mut().unwrap();
        let key = parts[0].to_string();

        if parts.len() == 1 {
            obj_map.insert(key, value);
        } else {
            let entry = obj_map
                .entry(key)
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            Self::set_path_recursive(entry, &parts[1..], value);
        }
    }

    /// Delete a value at a path in a JSON structure.
    fn delete_path(&self, obj: &mut serde_json::Value, path: &str) {
        let parts: Vec<&str> = path.split('.').collect();
        Self::delete_path_recursive(obj, &parts);
    }

    /// Recursively delete a value at a path.
    fn delete_path_recursive(obj: &mut serde_json::Value, parts: &[&str]) {
        if parts.is_empty() {
            return;
        }

        let Some(obj_map) = obj.as_object_mut() else {
            return;
        };

        if parts.len() == 1 {
            obj_map.remove(parts[0]);
        } else if let Some(next) = obj_map.get_mut(parts[0]) {
            Self::delete_path_recursive(next, &parts[1..]);
        }
    }

    /// Recursively flatten a JSON structure.
    fn flatten_recursive(
        &self,
        value: &serde_json::Value,
        prefix: String,
        separator: &str,
        output: &mut serde_json::Map<String, serde_json::Value>,
    ) {
        match value {
            serde_json::Value::Object(obj) => {
                for (k, v) in obj {
                    let new_prefix = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{}{}{}", prefix, separator, k)
                    };
                    self.flatten_recursive(v, new_prefix, separator, output);
                }
            }
            serde_json::Value::Array(arr) => {
                for (i, v) in arr.iter().enumerate() {
                    let new_prefix = if prefix.is_empty() {
                        format!("{}", i)
                    } else {
                        format!("{}{}{}", prefix, separator, i)
                    };
                    self.flatten_recursive(v, new_prefix, separator, output);
                }
            }
            _ => {
                output.insert(prefix, value.clone());
            }
        }
    }
}

impl Node for JsonDynamicNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("std", "json_dynamic")
            .with_description("Handle schemaless/dynamic JSON payloads")
            .with_inputs(vec![Port::input("Any")])
            .with_outputs(vec![
                Port::named("out", PortDirection::Output, "Any").with_description("Processed JSON"),
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

            // Process the input
            let output = match self.process(&value) {
                Ok(result) => result,
                Err(e) => {
                    return Ok(NodeOutput::error_with_message(e));
                }
            };

            tracing::debug!(
                operation = ?self.operation,
                "JsonDynamic: processed input"
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

impl NodeMetadata for JsonDynamicNode {
    fn metadata() -> crate::registry::NodeInfo {
        crate::registry::NodeInfo {
            node_type: "std::json_dynamic".to_string(),
            category: NodeCategory::Data,
            display_name: "JSON Dynamic".to_string(),
            description: "Handle schemaless/dynamic JSON payloads without strict schemas. Enables late binding and runtime field resolution for volatile JSON structures.".to_string(),
            icon: icons::ICON_JSON_DYNAMIC,
            inputs: vec![PortDefinition::required("in", PortType::Any)],
            outputs: vec![
                PortDefinition::required("out", PortType::Any),
                PortDefinition::required("error", PortType::Any),
            ],
            config_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["passthrough", "extract", "merge", "set", "delete", "flatten"],
                        "default": "passthrough",
                        "description": "JSON operation to perform"
                    },
                    "fields": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Fields to extract (for extract operation)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Field path (for set or delete operations)"
                    },
                    "value": {
                        "description": "Value to set (for set operation)"
                    },
                    "separator": {
                        "type": "string",
                        "default": "_",
                        "description": "Separator for flattening nested objects"
                    },
                    "required_fields": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Fields that must be present in input"
                    }
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_dynamic_node_info() {
        let node = JsonDynamicNode::passthrough();
        let info = node.info();

        assert_eq!(info.name, "std::json_dynamic");
        assert_eq!(info.inputs.len(), 1);
        assert_eq!(info.outputs.len(), 2);
    }

    #[test]
    fn json_dynamic_passthrough() {
        let node = JsonDynamicNode::passthrough();
        let input = Value::from(json!({"a": 1, "b": 2}));
        let result = node.process(&input).unwrap();

        assert_eq!(result.get_f64("a"), Some(1.0));
        assert_eq!(result.get_f64("b"), Some(2.0));
    }

    #[test]
    fn json_dynamic_extract() {
        let node =
            JsonDynamicNode::extract(vec!["user.name".to_string(), "user.email".to_string()]);

        let input = Value::from(json!({
            "user": {"name": "Alice", "email": "alice@example.com", "age": 30},
            "metadata": {"source": "api"}
        }));

        let result = node.process(&input).unwrap();

        assert_eq!(result.get_string("name"), Some("Alice".to_string()));
        assert_eq!(
            result.get_string("email"),
            Some("alice@example.com".to_string())
        );
        assert!(result.get_field("age").is_none());
        assert!(result.get_field("metadata").is_none());
    }

    #[test]
    fn json_dynamic_set() {
        let node = JsonDynamicNode::set("new_field", Value::string("new_value"));

        let input = Value::from(json!({"existing": "data"}));
        let result = node.process(&input).unwrap();

        assert_eq!(result.get_string("existing"), Some("data".to_string()));
        assert_eq!(
            result.get_string("new_field"),
            Some("new_value".to_string())
        );
    }

    #[test]
    fn json_dynamic_delete() {
        let node = JsonDynamicNode::delete("remove_me");

        let input = Value::from(json!({"keep": 1, "remove_me": 2}));
        let result = node.process(&input).unwrap();

        assert_eq!(result.get_f64("keep"), Some(1.0));
        assert!(result.get_field("remove_me").is_none());
    }

    #[test]
    fn json_dynamic_flatten() {
        let node = JsonDynamicNode::flatten("_");

        let input = Value::from(json!({
            "user": {
                "name": "Bob",
                "address": {
                    "city": "NYC"
                }
            }
        }));

        let result = node.process(&input).unwrap();

        assert_eq!(result.get_string("user_name"), Some("Bob".to_string()));
        assert_eq!(
            result.get_string("user_address_city"),
            Some("NYC".to_string())
        );
    }

    #[test]
    fn json_dynamic_merge() {
        let node = JsonDynamicNode::merge();

        let input = Value::from(json!([
            {"a": 1, "b": 2},
            {"c": 3, "b": 4}
        ]));

        let result = node.process(&input).unwrap();

        assert_eq!(result.get_f64("a"), Some(1.0));
        assert_eq!(result.get_f64("b"), Some(4.0)); // Second value wins
        assert_eq!(result.get_f64("c"), Some(3.0));
    }

    #[test]
    fn json_dynamic_required_field_validation() {
        let node = JsonDynamicNode::passthrough().with_required(vec!["required_field".to_string()]);

        let input = Value::from(json!({"other": "data"}));
        let result = node.process(&input);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("required_field"));
    }

    #[test]
    fn json_dynamic_set_nested() {
        let node = JsonDynamicNode::set("a.b.c", Value::string("deep"));

        let input = Value::from(json!({}));
        let result = node.process(&input).unwrap();

        assert_eq!(result.get_string("a.b.c"), Some("deep".to_string()));
    }

    #[test]
    fn json_dynamic_delete_nested() {
        let node = JsonDynamicNode::delete("a.b");

        let input = Value::from(json!({"a": {"b": 1, "c": 2}}));
        let result = node.process(&input).unwrap();

        assert!(result.get_field("a.b").is_none());
        assert_eq!(result.get_f64("a.c"), Some(2.0));
    }
}
