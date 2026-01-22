//! Flow definition - the top-level YAML document.

use super::metadata::FlowMetadata;
use super::validation::{FlowValidator, ValidationLimits, ValidationResult};
use super::{EdgeDefinition, FlowSettings, NodeDefinition, TriggerDefinition};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ts_rs::TS;

/// A complete flow definition from YAML.
///
/// This is the top-level structure representing a XERV flow document.
///
/// # Example
///
/// ```yaml
/// name: order_processing
/// version: "1.0"
/// description: Process incoming orders with fraud detection
///
/// triggers:
///   - id: api_webhook
///     type: webhook
///     params:
///       port: 8080
///       path: /orders
///
/// nodes:
///   validate:
///     type: std::json_parse
///     config:
///       strict: true
///
///   fraud_check:
///     type: std::switch
///     config:
///       condition:
///         type: greater_than
///         field: risk_score
///         value: 0.8
///
///   process_order:
///     type: plugins::order_processor
///
///   flag_fraud:
///     type: plugins::fraud_handler
///
/// edges:
///   - from: api_webhook
///     to: validate
///   - from: validate
///     to: fraud_check
///   - from: fraud_check.true
///     to: flag_fraud
///   - from: fraud_check.false
///     to: process_order
///
/// settings:
///   max_concurrent_executions: 100
///   execution_timeout_ms: 30000
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../web/src/bindings/")]
pub struct FlowDefinition {
    /// Flow name (required).
    pub name: String,

    /// Flow version (optional, defaults to "1.0").
    #[serde(default)]
    pub version: Option<String>,

    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,

    /// Visual metadata for the web UI (optional, ignored by executor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<FlowMetadata>,

    /// Triggers that start this flow.
    #[serde(default)]
    pub triggers: Vec<TriggerDefinition>,

    /// Nodes in the flow, keyed by node ID.
    #[serde(default)]
    pub nodes: HashMap<String, NodeDefinition>,

    /// Edges connecting nodes.
    #[serde(default)]
    pub edges: Vec<EdgeDefinition>,

    /// Runtime settings.
    #[serde(default)]
    pub settings: FlowSettings,
}

impl FlowDefinition {
    /// Create a new flow definition.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: Some("1.0".to_string()),
            description: None,
            metadata: None,
            triggers: Vec::new(),
            nodes: HashMap::new(),
            edges: Vec::new(),
            settings: FlowSettings::default(),
        }
    }

    /// Parse a flow definition from YAML string.
    ///
    /// Note: This method does not validate size or depth limits.
    /// For secure parsing, use `from_yaml_with_limits` or `from_yaml_validated`.
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    /// Parse a flow definition from YAML string with security limits.
    ///
    /// This method validates:
    /// - Content size (default: 10MB max)
    /// - Nesting depth (default: 100 levels max)
    pub fn from_yaml_with_limits(
        yaml: &str,
        limits: &ValidationLimits,
    ) -> Result<Self, FlowLoadError> {
        // Validate content size BEFORE parsing (DoS protection)
        limits
            .validate_content_size(yaml)
            .map_err(|e| FlowLoadError::LimitExceeded { error: e })?;

        // Parse to intermediate value to check depth
        let value: serde_yaml::Value =
            serde_yaml::from_str(yaml).map_err(|e| FlowLoadError::ParseString { source: e })?;

        // Validate nesting depth (DoS protection against stack overflow)
        limits
            .validate_nesting_depth(&value)
            .map_err(|e| FlowLoadError::LimitExceeded { error: e })?;

        // Now deserialize to FlowDefinition
        serde_yaml::from_value(value).map_err(|e| FlowLoadError::ParseString { source: e })
    }

    /// Parse a flow definition from YAML file.
    pub fn from_file(path: &std::path::Path) -> Result<Self, FlowLoadError> {
        Self::from_file_with_limits(path, &ValidationLimits::default())
    }

    /// Parse a flow definition from YAML file with security limits.
    pub fn from_file_with_limits(
        path: &std::path::Path,
        limits: &ValidationLimits,
    ) -> Result<Self, FlowLoadError> {
        // Check file size before reading (early rejection)
        let metadata = std::fs::metadata(path).map_err(|e| FlowLoadError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;

        if metadata.len() as usize > limits.max_file_size {
            return Err(FlowLoadError::LimitExceeded {
                error: super::validation::ValidationError::new(
                    super::validation::ValidationErrorKind::LimitExceeded,
                    "flow",
                    format!(
                        "file size ({} bytes) exceeds maximum allowed ({} bytes)",
                        metadata.len(),
                        limits.max_file_size
                    ),
                ),
            });
        }

        let content = std::fs::read_to_string(path).map_err(|e| FlowLoadError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;

        Self::from_yaml_with_limits(&content, limits).map_err(|e| match e {
            FlowLoadError::ParseString { source } => FlowLoadError::Parse {
                path: path.to_path_buf(),
                source,
            },
            other => other,
        })
    }

    /// Serialize to YAML string.
    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    /// Validate the flow definition.
    pub fn validate(&self) -> ValidationResult {
        FlowValidator::new().validate(self)
    }

    /// Parse and validate in one step with default limits.
    pub fn from_yaml_validated(yaml: &str) -> Result<Self, FlowLoadError> {
        Self::from_yaml_validated_with_limits(yaml, &ValidationLimits::default())
    }

    /// Parse and validate in one step with custom limits.
    ///
    /// This is the recommended method for parsing untrusted YAML as it:
    /// 1. Validates content size before parsing
    /// 2. Validates nesting depth after initial parse
    /// 3. Validates node/edge/trigger counts
    /// 4. Validates semantic correctness
    pub fn from_yaml_validated_with_limits(
        yaml: &str,
        limits: &ValidationLimits,
    ) -> Result<Self, FlowLoadError> {
        // Parse with size and depth limits
        let flow = Self::from_yaml_with_limits(yaml, limits)?;

        // Validate with node/edge/trigger count limits
        FlowValidator::with_limits(limits.clone())
            .validate(&flow)
            .map_err(|errors| FlowLoadError::Validation { errors })?;

        Ok(flow)
    }

    /// Set version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Add a trigger.
    pub fn with_trigger(mut self, trigger: TriggerDefinition) -> Self {
        self.triggers.push(trigger);
        self
    }

    /// Add a node.
    pub fn with_node(mut self, id: impl Into<String>, node: NodeDefinition) -> Self {
        self.nodes.insert(id.into(), node);
        self
    }

    /// Add an edge.
    pub fn with_edge(mut self, edge: EdgeDefinition) -> Self {
        self.edges.push(edge);
        self
    }

    /// Set settings.
    pub fn with_settings(mut self, settings: FlowSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Get the effective version (defaults to "1.0").
    pub fn effective_version(&self) -> &str {
        self.version.as_deref().unwrap_or("1.0")
    }

    /// Get all node IDs.
    pub fn node_ids(&self) -> impl Iterator<Item = &str> {
        self.nodes.keys().map(|s| s.as_str())
    }

    /// Get all trigger IDs.
    pub fn trigger_ids(&self) -> impl Iterator<Item = &str> {
        self.triggers.iter().map(|t| t.id.as_str())
    }

    /// Get a node by ID.
    pub fn get_node(&self, id: &str) -> Option<&NodeDefinition> {
        self.nodes.get(id)
    }

    /// Get a trigger by ID.
    pub fn get_trigger(&self, id: &str) -> Option<&TriggerDefinition> {
        self.triggers.iter().find(|t| t.id == id)
    }

    /// Check if a trigger with the given ID exists.
    pub fn has_trigger(&self, id: &str) -> bool {
        self.triggers.iter().any(|t| t.id == id)
    }

    /// Check if a node with the given ID exists.
    pub fn has_node(&self, id: &str) -> bool {
        self.nodes.contains_key(id)
    }

    /// Get enabled triggers.
    pub fn enabled_triggers(&self) -> impl Iterator<Item = &TriggerDefinition> {
        self.triggers.iter().filter(|t| t.enabled)
    }

    /// Get enabled nodes.
    pub fn enabled_nodes(&self) -> impl Iterator<Item = (&str, &NodeDefinition)> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.enabled)
            .map(|(k, v)| (k.as_str(), v))
    }

    /// Find edges from a given node.
    pub fn edges_from(&self, node_id: &str) -> impl Iterator<Item = &EdgeDefinition> {
        self.edges.iter().filter(move |e| e.from_node() == node_id)
    }

    /// Find edges to a given node.
    pub fn edges_to(&self, node_id: &str) -> impl Iterator<Item = &EdgeDefinition> {
        self.edges.iter().filter(move |e| e.to_node() == node_id)
    }
}

/// Error loading a flow definition.
///
/// This enum represents all possible errors that can occur when loading,
/// parsing, or validating a flow definition from YAML.
#[derive(Debug)]
pub enum FlowLoadError {
    /// I/O error reading file.
    Io {
        /// Path to the file that couldn't be read.
        path: std::path::PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// YAML parse error from file.
    Parse {
        /// Path to the file that couldn't be parsed.
        path: std::path::PathBuf,
        /// The underlying YAML parse error.
        source: serde_yaml::Error,
    },
    /// YAML parse error from string input.
    ParseString {
        /// The underlying YAML parse error.
        source: serde_yaml::Error,
    },
    /// Flow validation failed with one or more errors.
    Validation {
        /// List of validation errors found in the flow.
        errors: Vec<super::validation::ValidationError>,
    },
    /// Validation limit exceeded (size, depth, or count).
    ///
    /// This error occurs when the flow exceeds configured security limits,
    /// such as maximum file size, nesting depth, or node count.
    LimitExceeded {
        /// The specific limit that was exceeded.
        error: super::validation::ValidationError,
    },
}

impl std::fmt::Display for FlowLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(
                    f,
                    "failed to read flow file '{}': {}",
                    path.display(),
                    source
                )
            }
            Self::Parse { path, source } => {
                write!(
                    f,
                    "failed to parse flow file '{}': {}",
                    path.display(),
                    source
                )
            }
            Self::ParseString { source } => {
                write!(f, "failed to parse YAML: {}", source)
            }
            Self::Validation { errors } => {
                writeln!(f, "flow validation failed with {} error(s):", errors.len())?;
                for error in errors {
                    writeln!(f, "  - {}", error)?;
                }
                Ok(())
            }
            Self::LimitExceeded { error } => {
                write!(f, "flow validation limit exceeded: {}", error)
            }
        }
    }
}

impl std::error::Error for FlowLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::ParseString { source } => Some(source),
            Self::Validation { .. } => None,
            Self::LimitExceeded { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_complete_flow() {
        let yaml = r#"
name: order_processing
version: "2.0"
description: Process orders with fraud detection

triggers:
  - id: api_webhook
    type: webhook
    params:
      port: 8080
      path: /orders

nodes:
  fraud_check:
    type: std::switch
    config:
      condition:
        type: greater_than
        field: risk_score
        value: 0.8

  process_order:
    type: std::log
    config:
      message: "Processing order"

  flag_fraud:
    type: std::log
    config:
      message: "Fraud detected"

edges:
  - from: api_webhook
    to: fraud_check
  - from: fraud_check.false
    to: process_order
  - from: fraud_check.true
    to: flag_fraud

settings:
  max_concurrent_executions: 50
  execution_timeout_ms: 30000
"#;

        let flow = FlowDefinition::from_yaml(yaml).unwrap();

        assert_eq!(flow.name, "order_processing");
        assert_eq!(flow.version, Some("2.0".to_string()));
        assert_eq!(
            flow.description,
            Some("Process orders with fraud detection".to_string())
        );

        assert_eq!(flow.triggers.len(), 1);
        assert_eq!(flow.triggers[0].id, "api_webhook");

        assert_eq!(flow.nodes.len(), 3);
        assert!(flow.has_node("fraud_check"));
        assert!(flow.has_node("process_order"));
        assert!(flow.has_node("flag_fraud"));

        assert_eq!(flow.edges.len(), 3);

        assert_eq!(flow.settings.max_concurrent_executions, 50);
        assert_eq!(flow.settings.execution_timeout_ms, 30000);
    }

    #[test]
    fn parse_minimal_flow() {
        let yaml = r#"
name: simple
"#;
        let flow = FlowDefinition::from_yaml(yaml).unwrap();
        assert_eq!(flow.name, "simple");
        assert!(flow.triggers.is_empty());
        assert!(flow.nodes.is_empty());
        assert!(flow.edges.is_empty());
    }

    #[test]
    fn flow_builder() {
        let flow = FlowDefinition::new("test_flow")
            .with_version("1.0.0")
            .with_description("A test flow")
            .with_trigger(TriggerDefinition::new("webhook", "webhook"))
            .with_node("log", NodeDefinition::new("std::log"))
            .with_edge(EdgeDefinition::new("webhook", "log"));

        assert_eq!(flow.name, "test_flow");
        assert_eq!(flow.triggers.len(), 1);
        assert_eq!(flow.nodes.len(), 1);
        assert_eq!(flow.edges.len(), 1);
    }

    #[test]
    fn validate_and_parse() {
        let yaml = r#"
name: validated_flow
triggers:
  - id: webhook
    type: webhook
nodes:
  log:
    type: std::log
edges:
  - from: webhook
    to: log
"#;

        let result = FlowDefinition::from_yaml_validated(yaml);
        assert!(result.is_ok());
    }

    #[test]
    fn validation_errors() {
        let yaml = r#"
name: ""
triggers:
  - id: test
    type: invalid_trigger_type
"#;

        let result = FlowDefinition::from_yaml_validated(yaml);
        assert!(result.is_err());

        if let Err(FlowLoadError::Validation { errors }) = result {
            assert!(!errors.is_empty());
        } else {
            panic!("Expected validation error");
        }
    }

    #[test]
    fn to_yaml_roundtrip() {
        let flow = FlowDefinition::new("roundtrip_test")
            .with_trigger(TriggerDefinition::new("webhook", "webhook"))
            .with_node("log", NodeDefinition::new("std::log"));

        let yaml = flow.to_yaml().unwrap();
        let parsed = FlowDefinition::from_yaml(&yaml).unwrap();

        assert_eq!(parsed.name, "roundtrip_test");
        assert_eq!(parsed.triggers.len(), 1);
        assert_eq!(parsed.nodes.len(), 1);
    }

    #[test]
    fn query_methods() {
        let flow = FlowDefinition::new("query_test")
            .with_trigger(TriggerDefinition::new("t1", "webhook"))
            .with_node("n1", NodeDefinition::new("std::log"))
            .with_node("n2", NodeDefinition::new("std::switch"))
            .with_edge(EdgeDefinition::new("t1", "n1"))
            .with_edge(EdgeDefinition::new("n1", "n2"));

        assert!(flow.has_trigger("t1"));
        assert!(!flow.has_trigger("nonexistent"));

        assert!(flow.has_node("n1"));
        assert!(!flow.has_node("nonexistent"));

        let edges_from_t1: Vec<_> = flow.edges_from("t1").collect();
        assert_eq!(edges_from_t1.len(), 1);

        let edges_to_n2: Vec<_> = flow.edges_to("n2").collect();
        assert_eq!(edges_to_n2.len(), 1);
    }
}
