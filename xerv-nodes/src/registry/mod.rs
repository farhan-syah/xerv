//! Node metadata registry for API introspection.
//!
//! This module provides a trait-based system for nodes to declare their metadata,
//! which is used by the executor API to provide node information to frontends.

pub mod icons;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Node category for UI organization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeCategory {
    /// Flow control nodes (merge, switch, loop, wait)
    FlowControl,
    /// Data manipulation nodes (map, split, aggregate, concat)
    Data,
    /// Network operations (HTTP, WebSocket)
    Network,
    /// Logging and observability
    Logging,
}

/// Port type for inputs and outputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortType {
    /// Any type (no validation)
    Any,
    /// String type
    String,
    /// Number type
    Number,
    /// Boolean type
    Boolean,
    /// Array of any type
    Array,
    /// Object/map type
    Object,
}

/// Node port definition (input or output).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortDefinition {
    /// Port name (e.g., "input", "output", "true", "false")
    pub name: String,
    /// Port type
    #[serde(rename = "type")]
    pub port_type: PortType,
    /// Whether this port is required
    #[serde(default)]
    pub required: bool,
    /// Whether this port accepts/produces multiple values
    #[serde(default)]
    pub multiple: bool,
}

impl PortDefinition {
    /// Create a required single-value port.
    pub fn required(name: impl Into<String>, port_type: PortType) -> Self {
        Self {
            name: name.into(),
            port_type,
            required: true,
            multiple: false,
        }
    }

    /// Create an optional single-value port.
    pub fn optional(name: impl Into<String>, port_type: PortType) -> Self {
        Self {
            name: name.into(),
            port_type,
            required: false,
            multiple: false,
        }
    }

    /// Create a port that accepts/produces multiple values.
    pub fn multiple(name: impl Into<String>, port_type: PortType) -> Self {
        Self {
            name: name.into(),
            port_type,
            required: true,
            multiple: true,
        }
    }
}

/// Complete metadata for a node type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    /// Node type identifier (e.g., "std::merge")
    #[serde(rename = "type")]
    pub node_type: String,

    /// UI category for organization
    pub category: NodeCategory,

    /// Human-readable display name
    pub display_name: String,

    /// Detailed description of what the node does
    pub description: String,

    /// Emoji icon for UI representation
    pub icon: &'static str,

    /// Input port definitions
    pub inputs: Vec<PortDefinition>,

    /// Output port definitions
    pub outputs: Vec<PortDefinition>,

    /// JSON schema for node configuration
    pub config_schema: serde_json::Value,
}

/// Trait for nodes to provide their metadata.
///
/// This trait should be implemented by all standard library nodes to provide
/// introspection capabilities for the API and UI.
///
/// # Example
///
/// ```rust
/// use xerv_nodes::registry::{NodeMetadata, NodeInfo, NodeCategory, PortDefinition, PortType};
///
/// struct MyNode;
///
/// impl NodeMetadata for MyNode {
///     fn metadata() -> NodeInfo {
///         NodeInfo {
///             node_type: "std::my_node".to_string(),
///             category: NodeCategory::Data,
///             display_name: "My Node".to_string(),
///             description: "Does something useful".to_string(),
///             icon: "🔧",
///             inputs: vec![PortDefinition::required("input", PortType::Any)],
///             outputs: vec![PortDefinition::required("output", PortType::Any)],
///             config_schema: serde_json::json!({
///                 "type": "object",
///                 "properties": {}
///             }),
///         }
///     }
/// }
/// ```
pub trait NodeMetadata {
    /// Returns the complete metadata for this node type.
    fn metadata() -> NodeInfo;
}

/// Global node registry containing all standard library nodes.
pub struct NodeRegistry {
    nodes: HashMap<String, NodeInfo>,
}

impl NodeRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    /// Register a node type by calling its metadata() method.
    pub fn register<T: NodeMetadata>(&mut self) {
        let info = T::metadata();
        self.nodes.insert(info.node_type.clone(), info);
    }

    /// Get metadata for a specific node type.
    pub fn get(&self, node_type: &str) -> Option<&NodeInfo> {
        self.nodes.get(node_type)
    }

    /// Get all registered node metadata.
    pub fn all(&self) -> Vec<&NodeInfo> {
        self.nodes.values().collect()
    }

    /// Get all node metadata as JSON-serializable vector.
    pub fn all_json(&self) -> Vec<NodeInfo> {
        self.nodes.values().cloned().collect()
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Create and populate the standard library node registry.
///
/// This function registers all built-in nodes and returns a complete registry.
pub fn create_standard_registry() -> NodeRegistry {
    let mut registry = NodeRegistry::new();

    // Register flow control nodes
    registry.register::<crate::flow::MergeNode>();
    registry.register::<crate::flow::SwitchNode>();
    registry.register::<crate::flow::LoopNode>();
    registry.register::<crate::flow::WaitNode>();

    // Register data manipulation nodes
    registry.register::<crate::data::SplitNode>();
    registry.register::<crate::data::MapNode>();
    registry.register::<crate::data::ConcatNode>();
    registry.register::<crate::data::AggregateNode>();
    registry.register::<crate::data::JsonDynamicNode>();

    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_definition_builders() {
        let required = PortDefinition::required("input", PortType::String);
        assert_eq!(required.name, "input");
        assert_eq!(required.port_type, PortType::String);
        assert!(required.required);
        assert!(!required.multiple);

        let optional = PortDefinition::optional("output", PortType::Number);
        assert!(!optional.required);

        let multiple = PortDefinition::multiple("items", PortType::Array);
        assert!(multiple.multiple);
    }

    #[test]
    fn registry_operations() {
        let registry = create_standard_registry();

        // Should have all 9 standard nodes
        assert_eq!(registry.all().len(), 9);

        // Should be able to look up specific nodes
        assert!(registry.get("std::merge").is_some());
        assert!(registry.get("std::switch").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn node_info_serialization() {
        let info = NodeInfo {
            node_type: "test::node".to_string(),
            category: NodeCategory::Data,
            display_name: "Test".to_string(),
            description: "Test node".to_string(),
            icon: "🧪",
            inputs: vec![PortDefinition::required("in", PortType::Any)],
            outputs: vec![PortDefinition::required("out", PortType::Any)],
            config_schema: serde_json::json!({"type": "object"}),
        };

        // Should serialize to JSON correctly
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["type"], "test::node");
        assert_eq!(json["category"], "data");
        assert_eq!(json["icon"], "🧪");
    }
}
