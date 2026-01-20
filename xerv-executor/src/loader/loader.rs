//! FlowLoader - loads and compiles YAML flow definitions.

use super::builder::{FlowBuilder, FlowMetadata};
use crate::scheduler::FlowGraph;
use std::path::Path;
use xerv_core::error::Result;
use xerv_core::flow::{FlowDefinition, FlowSettings};

/// Configuration for the flow loader.
#[derive(Debug, Clone)]
pub struct LoaderConfig {
    /// Whether to validate flows during loading.
    pub validate: bool,
    /// Whether to link selectors (resolve `${node.field}` expressions) during loading.
    pub link_selectors: bool,
    /// Whether to optimize the graph for execution.
    pub optimize: bool,
}

impl Default for LoaderConfig {
    fn default() -> Self {
        Self {
            validate: true,
            link_selectors: true,
            optimize: false,
        }
    }
}

impl LoaderConfig {
    /// Create a minimal config (no validation or linking).
    pub fn minimal() -> Self {
        Self {
            validate: false,
            link_selectors: false,
            optimize: false,
        }
    }

    /// Create a full config (validation + linking + optimization).
    pub fn full() -> Self {
        Self {
            validate: true,
            link_selectors: true,
            optimize: true,
        }
    }
}

/// A fully loaded and compiled flow.
#[derive(Debug)]
pub struct LoadedFlow {
    /// The compiled flow graph ready for execution.
    pub graph: FlowGraph,
    /// Flow metadata including name, version, and node mappings.
    pub metadata: FlowMetadata,
    /// Runtime settings such as concurrency limits and timeouts.
    pub settings: FlowSettings,
    /// The original flow definition before compilation.
    pub definition: FlowDefinition,
}

impl LoadedFlow {
    /// Get the flow name.
    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    /// Get the flow version.
    pub fn version(&self) -> &str {
        &self.metadata.version
    }

    /// Get the topological order of nodes.
    pub fn execution_order(&self) -> Result<Vec<xerv_core::types::NodeId>> {
        self.graph.topological_sort()
    }
}

/// Error during flow loading.
#[derive(Debug)]
pub enum LoaderError {
    /// I/O error reading file.
    Io {
        /// The file path that could not be read.
        path: std::path::PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// YAML parse error.
    Parse {
        /// The file path being parsed, if available.
        path: Option<std::path::PathBuf>,
        /// The underlying YAML parse error.
        source: serde_yaml::Error,
    },
    /// Validation error.
    Validation {
        /// The validation errors found in the flow definition.
        errors: Vec<xerv_core::flow::ValidationError>,
    },
    /// Build error.
    Build {
        /// The underlying error that occurred during flow graph building.
        source: xerv_core::error::XervError,
    },
}

impl std::fmt::Display for LoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "failed to read '{}': {}", path.display(), source)
            }
            Self::Parse {
                path: Some(path),
                source,
            } => {
                write!(f, "failed to parse '{}': {}", path.display(), source)
            }
            Self::Parse { path: None, source } => {
                write!(f, "failed to parse YAML: {}", source)
            }
            Self::Validation { errors } => {
                writeln!(f, "flow validation failed:")?;
                for error in errors {
                    writeln!(f, "  - {}", error)?;
                }
                Ok(())
            }
            Self::Build { source } => {
                write!(f, "failed to build flow graph: {}", source)
            }
        }
    }
}

impl std::error::Error for LoaderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::Validation { .. } => None,
            Self::Build { source } => Some(source),
        }
    }
}

/// Flow loader that handles parsing, validation, and compilation.
pub struct FlowLoader {
    /// The configuration controlling loader behavior.
    config: LoaderConfig,
}

impl FlowLoader {
    /// Create a new flow loader with default configuration.
    pub fn new() -> Self {
        Self {
            config: LoaderConfig::default(),
        }
    }

    /// Create a flow loader with custom configuration.
    pub fn with_config(config: LoaderConfig) -> Self {
        Self { config }
    }

    /// Load a flow from a YAML file.
    pub fn load_file(&self, path: &Path) -> std::result::Result<LoadedFlow, LoaderError> {
        let content = std::fs::read_to_string(path).map_err(|e| LoaderError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;

        let definition: FlowDefinition =
            serde_yaml::from_str(&content).map_err(|e| LoaderError::Parse {
                path: Some(path.to_path_buf()),
                source: e,
            })?;

        self.load_definition(definition)
    }

    /// Load a flow from a YAML string.
    pub fn load_yaml(&self, yaml: &str) -> std::result::Result<LoadedFlow, LoaderError> {
        let definition: FlowDefinition =
            serde_yaml::from_str(yaml).map_err(|e| LoaderError::Parse {
                path: None,
                source: e,
            })?;

        self.load_definition(definition)
    }

    /// Load from a pre-parsed FlowDefinition.
    pub fn load_definition(
        &self,
        definition: FlowDefinition,
    ) -> std::result::Result<LoadedFlow, LoaderError> {
        // Validate if configured
        if self.config.validate {
            if let Err(errors) = definition.validate() {
                return Err(LoaderError::Validation { errors });
            }
        }

        // Build the graph
        let builder = FlowBuilder::new();
        let graph = builder
            .build(&definition)
            .map_err(|e| LoaderError::Build { source: e })?;

        // Create metadata
        let node_ids = self.collect_node_ids(&definition);
        let metadata = FlowMetadata::from_definition(&definition, node_ids);
        let settings = definition.settings.clone();

        Ok(LoadedFlow {
            graph,
            metadata,
            settings,
            definition,
        })
    }

    /// Collect node ID mappings from the definition.
    fn collect_node_ids(
        &self,
        flow: &FlowDefinition,
    ) -> std::collections::HashMap<String, xerv_core::types::NodeId> {
        let mut ids = std::collections::HashMap::new();
        let mut next_id = 0u32;

        // Triggers first
        for trigger in &flow.triggers {
            if trigger.enabled {
                ids.insert(trigger.id.clone(), xerv_core::types::NodeId::new(next_id));
                next_id += 1;
            }
        }

        // Then nodes
        for (node_id, node) in &flow.nodes {
            if node.enabled {
                ids.insert(node_id.clone(), xerv_core::types::NodeId::new(next_id));
                next_id += 1;
            }
        }

        ids
    }

    // Static convenience methods

    /// Load a flow from file with default configuration.
    pub fn from_file(path: impl AsRef<Path>) -> std::result::Result<LoadedFlow, LoaderError> {
        Self::new().load_file(path.as_ref())
    }

    /// Load a flow from YAML string with default configuration.
    pub fn from_yaml(yaml: &str) -> std::result::Result<LoadedFlow, LoaderError> {
        Self::new().load_yaml(yaml)
    }

    /// Load a flow from YAML without validation (for testing).
    pub fn from_yaml_unchecked(yaml: &str) -> std::result::Result<LoadedFlow, LoaderError> {
        Self::with_config(LoaderConfig::minimal()).load_yaml(yaml)
    }
}

impl Default for FlowLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_FLOW: &str = r#"
name: test_flow
version: "1.0"

triggers:
  - id: webhook
    type: webhook
    params:
      port: 8080

nodes:
  process:
    type: std::log
    config:
      message: "Hello"

edges:
  - from: webhook
    to: process
"#;

    const COMPLEX_FLOW: &str = r#"
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
  validate:
    type: std::log
    config:
      message: "Validating order"

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
    to: validate
  - from: validate
    to: fraud_check
  - from: fraud_check.false
    to: process_order
  - from: fraud_check.true
    to: flag_fraud

settings:
  max_concurrent_executions: 50
  execution_timeout_ms: 30000
"#;

    #[test]
    fn load_simple_flow() {
        let loaded = FlowLoader::from_yaml(SIMPLE_FLOW).unwrap();

        assert_eq!(loaded.name(), "test_flow");
        assert_eq!(loaded.version(), "1.0");
        assert_eq!(loaded.graph.entry_points().len(), 1);
    }

    #[test]
    fn load_complex_flow() {
        let loaded = FlowLoader::from_yaml(COMPLEX_FLOW).unwrap();

        assert_eq!(loaded.name(), "order_processing");
        assert_eq!(loaded.version(), "2.0");
        assert_eq!(loaded.settings.max_concurrent_executions, 50);
        assert_eq!(loaded.settings.execution_timeout_ms, 30000);

        // Should have 5 nodes total
        assert_eq!(loaded.graph.node_ids().count(), 5);

        // Execution order should work
        let order = loaded.execution_order().unwrap();
        assert_eq!(order.len(), 5);
    }

    #[test]
    fn metadata_node_lookup() {
        let loaded = FlowLoader::from_yaml(COMPLEX_FLOW).unwrap();

        // Should be able to look up nodes by name
        assert!(loaded.metadata.get_node_id("api_webhook").is_some());
        assert!(loaded.metadata.get_node_id("fraud_check").is_some());
        assert!(loaded.metadata.get_node_id("nonexistent").is_none());

        // Should be able to get node definitions
        assert!(loaded.metadata.get_node("fraud_check").is_some());
        assert_eq!(
            loaded.metadata.get_node("fraud_check").unwrap().node_type,
            "std::switch"
        );
    }

    #[test]
    fn validation_error() {
        let invalid_yaml = r#"
name: ""
triggers:
  - id: test
    type: invalid_type
"#;

        let result = FlowLoader::from_yaml(invalid_yaml);
        assert!(result.is_err());

        if let Err(LoaderError::Validation { errors }) = result {
            assert!(!errors.is_empty());
        } else {
            panic!("Expected validation error");
        }
    }

    #[test]
    fn skip_validation() {
        let invalid_yaml = r#"
name: ""
triggers:
  - id: test
    type: invalid_type
"#;

        // Should succeed with validation disabled
        let result = FlowLoader::from_yaml_unchecked(invalid_yaml);
        // May still fail during build, but not during validation
        // In this case, it should fail because there's no edge, which is ok
        assert!(result.is_ok() || matches!(result, Err(LoaderError::Build { .. })));
    }

    #[test]
    fn parse_error() {
        let invalid_yaml = "name: [unclosed bracket";

        let result = FlowLoader::from_yaml(invalid_yaml);
        assert!(matches!(result, Err(LoaderError::Parse { .. })));
    }

    #[test]
    fn with_disabled_nodes() {
        let yaml = r#"
name: test
triggers:
  - id: webhook
    type: webhook
nodes:
  enabled_node:
    type: std::log
  disabled_node:
    type: std::log
    enabled: false
edges:
  - from: webhook
    to: enabled_node
"#;

        let loaded = FlowLoader::from_yaml(yaml).unwrap();

        // Should only have 2 nodes (webhook + enabled_node)
        assert_eq!(loaded.graph.node_ids().count(), 2);

        // disabled_node should not have an ID
        assert!(loaded.metadata.get_node_id("disabled_node").is_none());
    }

    #[test]
    fn with_loop_back_edges() {
        let yaml = r#"
name: loop_test
triggers:
  - id: webhook
    type: webhook
nodes:
  loop_ctrl:
    type: std::loop
    config:
      max_iterations: 10
  process:
    type: std::log
edges:
  - from: webhook
    to: loop_ctrl
  - from: loop_ctrl.continue
    to: process
  - from: process
    to: loop_ctrl
    loop_back: true
"#;

        let loaded = FlowLoader::from_yaml(yaml).unwrap();

        // Should be able to topologically sort (back-edge excluded)
        let order = loaded.execution_order().unwrap();
        assert_eq!(order.len(), 3);
    }
}
