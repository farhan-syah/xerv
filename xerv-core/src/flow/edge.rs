//! Edge definition from YAML.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An edge definition connecting nodes in a flow.
///
/// Edges can be specified in several formats:
///
/// # Simple format (default ports)
/// ```yaml
/// edges:
///   - from: node_a
///     to: node_b
/// ```
///
/// # With explicit ports
/// ```yaml
/// edges:
///   - from: switch_node.true
///     to: handler_node.in
///   - from: switch_node.false
///     to: error_handler.in
/// ```
///
/// # With conditions
/// ```yaml
/// edges:
///   - from: node_a.out
///     to: node_b.in
///     condition: "${node_a.status} == 'success'"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../web/src/bindings/")]
pub struct EdgeDefinition {
    /// Source node and optional port (format: "node_id" or "node_id.port").
    pub from: String,

    /// Target node and optional port (format: "node_id" or "node_id.port").
    pub to: String,

    /// Optional condition for this edge (selector expression).
    #[serde(default)]
    pub condition: Option<String>,

    /// Whether this edge represents a loop back-edge.
    #[serde(default)]
    pub loop_back: bool,

    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
}

impl EdgeDefinition {
    /// Create a new edge definition.
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            condition: None,
            loop_back: false,
            description: None,
        }
    }

    /// Set a condition.
    pub fn with_condition(mut self, condition: impl Into<String>) -> Self {
        self.condition = Some(condition.into());
        self
    }

    /// Mark as a loop back-edge.
    pub fn as_loop_back(mut self) -> Self {
        self.loop_back = true;
        self
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Parse the source into (node_id, port).
    ///
    /// Returns (node_id, port) where port defaults to "out" if not specified.
    pub fn parse_from(&self) -> (&str, &str) {
        parse_node_port(&self.from, "out")
    }

    /// Parse the target into (node_id, port).
    ///
    /// Returns (node_id, port) where port defaults to "in" if not specified.
    pub fn parse_to(&self) -> (&str, &str) {
        parse_node_port(&self.to, "in")
    }

    /// Get the source node ID.
    pub fn from_node(&self) -> &str {
        self.parse_from().0
    }

    /// Get the source port.
    pub fn from_port(&self) -> &str {
        self.parse_from().1
    }

    /// Get the target node ID.
    pub fn to_node(&self) -> &str {
        self.parse_to().0
    }

    /// Get the target port.
    pub fn to_port(&self) -> &str {
        self.parse_to().1
    }
}

/// Parse a "node.port" or "node" string into (node, port).
fn parse_node_port<'a>(s: &'a str, default_port: &'static str) -> (&'a str, &'a str) {
    if let Some(dot_pos) = s.rfind('.') {
        // Check if what's after the dot looks like a port name
        let after_dot = &s[dot_pos + 1..];
        // Port names are typically short identifiers like "in", "out", "true", "false", "error"
        // If it looks like a selector or contains special chars, don't split
        if !after_dot.is_empty()
            && !after_dot.contains('$')
            && !after_dot.contains('{')
            && after_dot.len() < 20
            && after_dot.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            return (&s[..dot_pos], after_dot);
        }
    }
    (s, default_port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_simple_edge() {
        let yaml = r#"
from: node_a
to: node_b
"#;
        let edge: EdgeDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(edge.from, "node_a");
        assert_eq!(edge.to, "node_b");
        assert_eq!(edge.from_node(), "node_a");
        assert_eq!(edge.from_port(), "out");
        assert_eq!(edge.to_node(), "node_b");
        assert_eq!(edge.to_port(), "in");
    }

    #[test]
    fn deserialize_edge_with_ports() {
        let yaml = r#"
from: switch.true
to: handler.input
"#;
        let edge: EdgeDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(edge.from_node(), "switch");
        assert_eq!(edge.from_port(), "true");
        assert_eq!(edge.to_node(), "handler");
        assert_eq!(edge.to_port(), "input");
    }

    #[test]
    fn deserialize_edge_with_condition() {
        let yaml = r#"
from: validator.out
to: processor.in
condition: "${validator.is_valid} == true"
"#;
        let edge: EdgeDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            edge.condition,
            Some("${validator.is_valid} == true".to_string())
        );
    }

    #[test]
    fn deserialize_loop_back_edge() {
        let yaml = r#"
from: processor.out
to: loop_controller.in
loop_back: true
description: "Return to loop controller"
"#;
        let edge: EdgeDefinition = serde_yaml::from_str(yaml).unwrap();
        assert!(edge.loop_back);
        assert_eq!(
            edge.description,
            Some("Return to loop controller".to_string())
        );
    }

    #[test]
    fn edge_builder() {
        let edge = EdgeDefinition::new("source.out", "target.in")
            .with_condition("${source.success}")
            .with_description("Main flow");

        assert_eq!(edge.from_node(), "source");
        assert_eq!(edge.from_port(), "out");
        assert_eq!(edge.condition, Some("${source.success}".to_string()));
    }

    #[test]
    fn parse_node_without_port() {
        let edge = EdgeDefinition::new("simple_node", "another_node");
        assert_eq!(edge.from_node(), "simple_node");
        assert_eq!(edge.from_port(), "out");
        assert_eq!(edge.to_node(), "another_node");
        assert_eq!(edge.to_port(), "in");
    }

    #[test]
    fn parse_special_ports() {
        // Switch node with boolean ports
        let edge1 = EdgeDefinition::new("switch.true", "handler");
        assert_eq!(edge1.from_node(), "switch");
        assert_eq!(edge1.from_port(), "true");

        let edge2 = EdgeDefinition::new("switch.false", "error_handler");
        assert_eq!(edge2.from_node(), "switch");
        assert_eq!(edge2.from_port(), "false");

        // Error port
        let edge3 = EdgeDefinition::new("processor.error", "error_log");
        assert_eq!(edge3.from_node(), "processor");
        assert_eq!(edge3.from_port(), "error");
    }
}
