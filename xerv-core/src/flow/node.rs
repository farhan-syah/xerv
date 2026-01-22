//! Node definition from YAML.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ts_rs::TS;

/// A node definition from YAML.
///
/// # Example
///
/// ```yaml
/// nodes:
///   fraud_check:
///     type: std::switch
///     description: Route based on risk score
///     config:
///       condition:
///         type: greater_than
///         field: risk_score
///         value: 0.8
///
///   log_result:
///     type: std::log
///     config:
///       message: "Processing complete"
///       level: info
///
///   aggregate_stats:
///     type: std::aggregate
///     config:
///       operation: sum
///       field: amount
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../web/src/bindings/")]
pub struct NodeDefinition {
    /// Node type (e.g., "std::switch", "std::merge", "plugins::fraud_model").
    #[serde(rename = "type")]
    pub node_type: String,

    /// Node-specific configuration.
    #[serde(default)]
    #[ts(skip)]
    pub config: serde_yaml::Value,

    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,

    /// Whether the node is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Retry configuration.
    #[serde(default)]
    pub retry: Option<RetryConfig>,

    /// Timeout override for this node (milliseconds).
    #[serde(default)]
    pub timeout_ms: Option<u64>,

    /// Custom labels for this node.
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

fn default_enabled() -> bool {
    true
}

/// Retry configuration for a node.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../web/src/bindings/")]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    #[serde(default = "default_max_retries")]
    pub max_attempts: u32,

    /// Initial delay between retries in milliseconds.
    #[serde(default = "default_retry_delay_ms")]
    pub initial_delay_ms: u64,

    /// Maximum delay between retries in milliseconds.
    #[serde(default = "default_max_delay_ms")]
    pub max_delay_ms: u64,

    /// Multiplier for exponential backoff.
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,

    /// Whether to add jitter to delay.
    #[serde(default = "default_jitter")]
    pub jitter: bool,
}

fn default_max_retries() -> u32 {
    3
}
fn default_retry_delay_ms() -> u64 {
    1000
}
fn default_max_delay_ms() -> u64 {
    30_000
}
fn default_backoff_multiplier() -> f64 {
    2.0
}
fn default_jitter() -> bool {
    true
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_max_retries(),
            initial_delay_ms: default_retry_delay_ms(),
            max_delay_ms: default_max_delay_ms(),
            backoff_multiplier: default_backoff_multiplier(),
            jitter: default_jitter(),
        }
    }
}

impl NodeDefinition {
    /// Create a new node definition.
    pub fn new(node_type: impl Into<String>) -> Self {
        Self {
            node_type: node_type.into(),
            config: serde_yaml::Value::Null,
            description: None,
            enabled: true,
            retry: None,
            timeout_ms: None,
            labels: HashMap::new(),
        }
    }

    /// Set configuration.
    pub fn with_config(mut self, config: serde_yaml::Value) -> Self {
        self.config = config;
        self
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set retry configuration.
    pub fn with_retry(mut self, retry: RetryConfig) -> Self {
        self.retry = Some(retry);
        self
    }

    /// Set timeout override.
    pub fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// Add a label.
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }

    /// Disable the node.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Check if this is a standard library node.
    pub fn is_std(&self) -> bool {
        self.node_type.starts_with("std::")
    }

    /// Check if this is a trigger node.
    pub fn is_trigger(&self) -> bool {
        self.node_type.starts_with("trigger::")
    }

    /// Get a string config value.
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(|v| v.as_str())
    }

    /// Get an integer config value.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.config.get(key).and_then(|v| v.as_i64())
    }

    /// Get a boolean config value.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.config.get(key).and_then(|v| v.as_bool())
    }

    /// Get a nested config value.
    pub fn get_nested(&self, path: &[&str]) -> Option<&serde_yaml::Value> {
        let mut current = &self.config;
        for key in path {
            current = current.get(key)?;
        }
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_simple_node() {
        let yaml = r#"
type: std::log
config:
  message: "Hello"
  level: info
"#;
        let node: NodeDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(node.node_type, "std::log");
        assert_eq!(node.get_string("message"), Some("Hello"));
        assert!(node.enabled);
        assert!(node.is_std());
    }

    #[test]
    fn deserialize_node_with_retry() {
        let yaml = r#"
type: plugins::http_call
config:
  url: "https://api.example.com"
retry:
  max_attempts: 5
  initial_delay_ms: 500
"#;
        let node: NodeDefinition = serde_yaml::from_str(yaml).unwrap();
        assert!(node.retry.is_some());
        let retry = node.retry.unwrap();
        assert_eq!(retry.max_attempts, 5);
        assert_eq!(retry.initial_delay_ms, 500);
    }

    #[test]
    fn deserialize_node_with_labels() {
        let yaml = r#"
type: std::switch
labels:
  team: payments
  tier: critical
"#;
        let node: NodeDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(node.labels.get("team"), Some(&"payments".to_string()));
        assert_eq!(node.labels.get("tier"), Some(&"critical".to_string()));
    }

    #[test]
    fn node_builder() {
        let node = NodeDefinition::new("std::merge")
            .with_description("Wait for all inputs")
            .with_timeout_ms(5000)
            .with_label("category", "flow-control");

        assert_eq!(node.node_type, "std::merge");
        assert_eq!(node.description, Some("Wait for all inputs".to_string()));
        assert_eq!(node.timeout_ms, Some(5000));
        assert!(node.is_std());
    }

    #[test]
    fn nested_config_access() {
        let yaml = r#"
type: std::switch
config:
  condition:
    type: greater_than
    field: amount
    value: 100
"#;
        let node: NodeDefinition = serde_yaml::from_str(yaml).unwrap();
        let condition_type = node
            .get_nested(&["condition", "type"])
            .and_then(|v| v.as_str());
        assert_eq!(condition_type, Some("greater_than"));
    }

    #[test]
    fn disabled_node() {
        let yaml = r#"
type: std::log
enabled: false
"#;
        let node: NodeDefinition = serde_yaml::from_str(yaml).unwrap();
        assert!(!node.enabled);
    }
}
