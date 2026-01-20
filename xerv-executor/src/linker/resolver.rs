//! Selector resolver and linker.
//!
//! Compiles selector expressions to memory offsets at flow load time.
//! Supports both static (compile-time) and dynamic (runtime) resolution.

use super::parser::Selector;
use std::collections::{HashMap, HashSet};
use xerv_core::error::{Result, XervError};
use xerv_core::traits::{SchemaRegistry, TypeInfo};
use xerv_core::types::NodeId;

/// A resolved field with its memory offset.
#[derive(Debug, Clone)]
pub struct ResolvedField {
    /// The field name.
    pub name: String,
    /// The type name.
    pub type_name: String,
    /// Byte offset within the struct.
    pub offset: usize,
    /// Size in bytes.
    pub size: usize,
}

/// How a selector was resolved - statically or dynamically.
#[derive(Debug, Clone)]
pub enum SelectorResolution {
    /// Static resolution with known memory offsets.
    Static {
        /// Total byte offset from struct start.
        total_offset: usize,
        /// Final field type name.
        final_type: String,
        /// Final field size in bytes.
        final_size: usize,
        /// Resolved field chain.
        fields: Vec<ResolvedField>,
    },
    /// Dynamic resolution requiring runtime JSON path lookup.
    Dynamic {
        /// Field path to resolve at runtime (e.g., ["result", "score"]).
        field_path: Vec<String>,
        /// Expected type hint (if known).
        expected_type: Option<String>,
    },
}

impl SelectorResolution {
    /// Check if this is a static resolution.
    pub fn is_static(&self) -> bool {
        matches!(self, Self::Static { .. })
    }

    /// Check if this is a dynamic resolution.
    pub fn is_dynamic(&self) -> bool {
        matches!(self, Self::Dynamic { .. })
    }

    /// Get the total offset for static resolution.
    pub fn static_offset(&self) -> Option<usize> {
        match self {
            Self::Static { total_offset, .. } => Some(*total_offset),
            Self::Dynamic { .. } => None,
        }
    }

    /// Get the field path for dynamic resolution.
    pub fn dynamic_path(&self) -> Option<&[String]> {
        match self {
            Self::Static { .. } => None,
            Self::Dynamic { field_path, .. } => Some(field_path),
        }
    }
}

/// A compiled selector ready for execution.
#[derive(Debug, Clone)]
pub struct CompiledSelector {
    /// The original selector.
    pub selector: Selector,
    /// The node ID that produces this data.
    pub source_node: NodeId,
    /// The output port on the source node.
    pub source_port: String,
    /// How this selector was resolved.
    pub resolution: SelectorResolution,
}

impl CompiledSelector {
    /// Get the byte offset for this selector (static resolution only).
    pub fn offset(&self) -> Option<usize> {
        self.resolution.static_offset()
    }

    /// Get legacy total_offset field (for backwards compatibility).
    pub fn total_offset(&self) -> usize {
        self.resolution.static_offset().unwrap_or(0)
    }

    /// Get legacy fields (for backwards compatibility).
    pub fn fields(&self) -> &[ResolvedField] {
        match &self.resolution {
            SelectorResolution::Static { fields, .. } => fields,
            SelectorResolution::Dynamic { .. } => &[],
        }
    }

    /// Get legacy final_type (for backwards compatibility).
    pub fn final_type(&self) -> &str {
        match &self.resolution {
            SelectorResolution::Static { final_type, .. } => final_type,
            SelectorResolution::Dynamic { expected_type, .. } => {
                expected_type.as_deref().unwrap_or("dynamic")
            }
        }
    }

    /// Get legacy final_size (for backwards compatibility).
    pub fn final_size(&self) -> usize {
        match &self.resolution {
            SelectorResolution::Static { final_size, .. } => *final_size,
            SelectorResolution::Dynamic { .. } => 0,
        }
    }

    /// Check if this selector was resolved statically.
    pub fn is_static(&self) -> bool {
        self.resolution.is_static()
    }

    /// Check if this selector requires runtime resolution.
    pub fn is_dynamic(&self) -> bool {
        self.resolution.is_dynamic()
    }

    /// Check if this selector can be resolved to a primitive type.
    pub fn is_primitive(&self) -> bool {
        match &self.resolution {
            SelectorResolution::Static { final_type, .. } => matches!(
                final_type.as_str(),
                "f32"
                    | "f64"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "bool"
            ),
            SelectorResolution::Dynamic { .. } => false,
        }
    }
}

/// Node output schema information.
#[derive(Debug, Clone)]
pub struct NodeSchema {
    /// Node ID.
    pub node_id: NodeId,
    /// Node name (for error messages).
    pub node_name: String,
    /// Output port schemas (port -> type info).
    pub output_schemas: HashMap<String, TypeInfo>,
    /// Whether this node produces dynamic/schemaless output.
    pub is_dynamic: bool,
}

impl NodeSchema {
    /// Create a new node schema.
    pub fn new(node_id: NodeId, node_name: impl Into<String>) -> Self {
        Self {
            node_id,
            node_name: node_name.into(),
            output_schemas: HashMap::new(),
            is_dynamic: false,
        }
    }

    /// Add an output port schema.
    pub fn with_output(mut self, port: impl Into<String>, schema: TypeInfo) -> Self {
        self.output_schemas.insert(port.into(), schema);
        self
    }

    /// Mark this node as producing dynamic/schemaless output.
    pub fn dynamic(mut self) -> Self {
        self.is_dynamic = true;
        self
    }
}

/// The linker resolves selectors to memory offsets.
pub struct Linker {
    /// Node schemas keyed by node name.
    node_schemas: HashMap<String, NodeSchema>,
    /// Pipeline config schema.
    pipeline_config: Option<TypeInfo>,
    /// Global schema registry.
    registry: SchemaRegistry,
    /// Node names marked as dynamic (produce schemaless output).
    dynamic_nodes: HashSet<String>,
}

impl Linker {
    /// Create a new linker.
    pub fn new() -> Self {
        Self {
            node_schemas: HashMap::new(),
            pipeline_config: None,
            registry: SchemaRegistry::new(),
            dynamic_nodes: HashSet::new(),
        }
    }

    /// Register a node's output schema.
    pub fn register_node(&mut self, name: impl Into<String>, schema: NodeSchema) {
        let name = name.into();
        if schema.is_dynamic {
            self.dynamic_nodes.insert(name.clone());
        }
        self.node_schemas.insert(name, schema);
    }

    /// Register a node as dynamic (produces schemaless output).
    pub fn register_dynamic_node(&mut self, name: impl Into<String>, node_id: NodeId) {
        let name = name.into();
        self.dynamic_nodes.insert(name.clone());
        let schema = NodeSchema::new(node_id, name.clone()).dynamic();
        self.node_schemas.insert(name, schema);
    }

    /// Check if a node is registered as dynamic.
    pub fn is_dynamic_node(&self, name: &str) -> bool {
        self.dynamic_nodes.contains(name)
    }

    /// Register the pipeline configuration schema.
    pub fn register_pipeline_config(&mut self, schema: TypeInfo) {
        self.pipeline_config = Some(schema.clone());
        self.registry.register(schema);
    }

    /// Register a schema in the global registry.
    pub fn register_schema(&mut self, schema: TypeInfo) {
        self.registry.register(schema);
    }

    /// Compile a selector to either static or dynamic resolution.
    ///
    /// For nodes with known schemas, attempts static resolution first.
    /// For dynamic/schemaless nodes, uses dynamic resolution.
    pub fn compile(&self, selector: &Selector) -> Result<CompiledSelector> {
        // Handle pipeline config selectors
        if selector.is_pipeline_config() {
            return self.compile_pipeline_config(selector);
        }

        // Try static resolution first
        match self.try_static_compile(selector) {
            Ok(compiled) => Ok(compiled),
            Err(e) => {
                // If the node is dynamic, fall back to dynamic resolution
                if self.is_dynamic_node(&selector.root) {
                    self.compile_dynamic(selector)
                } else {
                    // Not a dynamic node, propagate the error
                    Err(e)
                }
            }
        }
    }

    /// Attempt static compilation of a selector.
    fn try_static_compile(&self, selector: &Selector) -> Result<CompiledSelector> {
        // Look up the node schema
        let node_schema =
            self.node_schemas
                .get(&selector.root)
                .ok_or_else(|| XervError::SelectorResolution {
                    selector: selector.raw.clone(),
                    node_id: NodeId::new(0),
                    cause: format!("Unknown node: {}", selector.root),
                })?;

        // If the node is dynamic and has no schema, fail immediately
        if node_schema.is_dynamic && node_schema.output_schemas.is_empty() {
            return Err(XervError::SelectorResolution {
                selector: selector.raw.clone(),
                node_id: node_schema.node_id,
                cause: "Dynamic node - requires runtime resolution".to_string(),
            });
        }

        // Default to "out" port if no port specified
        let (port, field_path) = if selector.path.is_empty() {
            ("out".to_string(), Vec::new())
        } else {
            // Check if first path element is a port name
            let first = &selector.path[0];
            if node_schema.output_schemas.contains_key(first) {
                (first.clone(), selector.path[1..].to_vec())
            } else {
                ("out".to_string(), selector.path.clone())
            }
        };

        let schema =
            node_schema
                .output_schemas
                .get(&port)
                .ok_or_else(|| XervError::InvalidPort {
                    port: port.clone(),
                    node_id: node_schema.node_id,
                })?;

        // Resolve field chain
        let (fields, total_offset) = self.resolve_field_chain(schema, &field_path)?;

        let (final_type, final_size) = if let Some(last) = fields.last() {
            (last.type_name.clone(), last.size)
        } else {
            (schema.name.clone(), schema.size)
        };

        Ok(CompiledSelector {
            selector: selector.clone(),
            source_node: node_schema.node_id,
            source_port: port,
            resolution: SelectorResolution::Static {
                total_offset,
                final_type,
                final_size,
                fields,
            },
        })
    }

    /// Compile a selector for dynamic (runtime) resolution.
    fn compile_dynamic(&self, selector: &Selector) -> Result<CompiledSelector> {
        let node_schema = self.node_schemas.get(&selector.root);
        let node_id = node_schema
            .map(|s| s.node_id)
            .unwrap_or_else(|| NodeId::new(0));

        // Extract port and field path
        let (port, field_path) = if selector.path.is_empty() {
            ("out".to_string(), Vec::new())
        } else if let Some(schema) = node_schema {
            let first = &selector.path[0];
            if schema.output_schemas.contains_key(first) {
                (first.clone(), selector.path[1..].to_vec())
            } else {
                ("out".to_string(), selector.path.clone())
            }
        } else {
            ("out".to_string(), selector.path.clone())
        };

        Ok(CompiledSelector {
            selector: selector.clone(),
            source_node: node_id,
            source_port: port,
            resolution: SelectorResolution::Dynamic {
                field_path,
                expected_type: None,
            },
        })
    }

    /// Compile a pipeline config selector.
    fn compile_pipeline_config(&self, selector: &Selector) -> Result<CompiledSelector> {
        let schema =
            self.pipeline_config
                .as_ref()
                .ok_or_else(|| XervError::SelectorResolution {
                    selector: selector.raw.clone(),
                    node_id: NodeId::new(0),
                    cause: "Pipeline config not registered".to_string(),
                })?;

        // Skip "config" prefix
        let field_path: Vec<String> = selector.path.iter().skip(1).cloned().collect();

        let (fields, total_offset) = self.resolve_field_chain(schema, &field_path)?;

        let (final_type, final_size) = if let Some(last) = fields.last() {
            (last.type_name.clone(), last.size)
        } else {
            (schema.name.clone(), schema.size)
        };

        Ok(CompiledSelector {
            selector: selector.clone(),
            source_node: NodeId::new(0), // Pipeline config is at node 0
            source_port: "__config__".to_string(),
            resolution: SelectorResolution::Static {
                total_offset,
                final_type,
                final_size,
                fields,
            },
        })
    }

    /// Resolve a field chain to get offsets.
    fn resolve_field_chain(
        &self,
        root_schema: &TypeInfo,
        field_path: &[String],
    ) -> Result<(Vec<ResolvedField>, usize)> {
        if field_path.is_empty() {
            return Ok((Vec::new(), 0));
        }

        let mut fields = Vec::new();
        let mut current_schema = root_schema;
        let mut total_offset = 0;

        for field_name in field_path {
            let field = current_schema.get_field(field_name).ok_or_else(|| {
                XervError::SelectorTargetNotFound {
                    field: field_name.clone(),
                    node_id: NodeId::new(0),
                }
            })?;

            let resolved = ResolvedField {
                name: field.name.clone(),
                type_name: field.type_name.clone(),
                offset: field.offset,
                size: field.size,
            };

            total_offset += field.offset;
            fields.push(resolved);

            // Try to look up nested schema for the next iteration
            if let Some(nested) = self.registry.get(&field.type_name) {
                current_schema = Box::leak(Box::new(nested));
            }
        }

        Ok((fields, total_offset))
    }

    /// Compile all selectors in a string template.
    pub fn compile_template(&self, template: &str) -> Result<Vec<CompiledSelector>> {
        use super::parser::SelectorParser;

        let selectors = SelectorParser::parse_all(template)?;
        selectors.iter().map(|s| self.compile(s)).collect()
    }
}

impl Default for Linker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xerv_core::traits::FieldInfo;

    fn create_test_schema() -> TypeInfo {
        TypeInfo::new("FraudResult", 1)
            .with_hash(0x12345678)
            .with_size(16)
            .with_fields(vec![
                FieldInfo::new("score", "f32").with_offset(0).with_size(4),
                FieldInfo::new("risk_level", "u8")
                    .with_offset(4)
                    .with_size(1),
                FieldInfo::new("flags", "u32").with_offset(8).with_size(4),
            ])
            .stable()
    }

    #[test]
    fn compile_simple_selector() {
        let mut linker = Linker::new();

        let schema = create_test_schema();
        let node_schema = NodeSchema::new(NodeId::new(1), "fraud_check").with_output("out", schema);

        linker.register_node("fraud_check", node_schema);

        let selector = Selector::new("fraud_check", vec!["score".to_string()]);
        let compiled = linker.compile(&selector).unwrap();

        assert_eq!(compiled.source_node, NodeId::new(1));
        assert!(compiled.is_static());
        assert_eq!(compiled.total_offset(), 0);
        assert_eq!(compiled.final_type(), "f32");
        assert_eq!(compiled.final_size(), 4);
    }

    #[test]
    fn compile_pipeline_config_selector() {
        let mut linker = Linker::new();

        let config_schema = TypeInfo::new("ECommerceConfig", 1)
            .with_size(24)
            .with_fields(vec![
                FieldInfo::new("min_order_value", "f64")
                    .with_offset(0)
                    .with_size(8),
                FieldInfo::new("max_retries", "u8")
                    .with_offset(8)
                    .with_size(1),
            ])
            .stable();

        linker.register_pipeline_config(config_schema);

        let selector = Selector::new(
            "pipeline",
            vec!["config".to_string(), "min_order_value".to_string()],
        );
        let compiled = linker.compile(&selector).unwrap();

        assert_eq!(compiled.source_port, "__config__");
        assert!(compiled.is_static());
        assert_eq!(compiled.total_offset(), 0);
        assert_eq!(compiled.final_type(), "f64");
    }

    #[test]
    fn unknown_node_error() {
        let linker = Linker::new();

        let selector = Selector::new("unknown_node", vec!["field".to_string()]);
        let result = linker.compile(&selector);

        assert!(matches!(result, Err(XervError::SelectorResolution { .. })));
    }

    #[test]
    fn unknown_field_error() {
        let mut linker = Linker::new();

        let schema = create_test_schema();
        let node_schema = NodeSchema::new(NodeId::new(1), "fraud_check").with_output("out", schema);

        linker.register_node("fraud_check", node_schema);

        let selector = Selector::new("fraud_check", vec!["unknown_field".to_string()]);
        let result = linker.compile(&selector);

        assert!(matches!(
            result,
            Err(XervError::SelectorTargetNotFound { .. })
        ));
    }

    #[test]
    fn compile_dynamic_node_selector() {
        let mut linker = Linker::new();

        // Register a dynamic node (e.g., JsonDynamicNode)
        linker.register_dynamic_node("json_processor", NodeId::new(2));

        let selector = Selector::new(
            "json_processor",
            vec!["result".to_string(), "score".to_string()],
        );
        let compiled = linker.compile(&selector).unwrap();

        assert!(compiled.is_dynamic());
        assert_eq!(compiled.source_node, NodeId::new(2));
        assert_eq!(
            compiled.resolution.dynamic_path(),
            Some(&["result".to_string(), "score".to_string()][..])
        );
    }

    #[test]
    fn static_node_unknown_field_not_dynamic() {
        let mut linker = Linker::new();

        // Register a static node with schema
        let schema = create_test_schema();
        let node_schema = NodeSchema::new(NodeId::new(1), "fraud_check").with_output("out", schema);
        linker.register_node("fraud_check", node_schema);

        // Try to access unknown field - should fail, not fall back to dynamic
        let selector = Selector::new("fraud_check", vec!["unknown_field".to_string()]);
        let result = linker.compile(&selector);

        assert!(result.is_err());
    }

    #[test]
    fn dynamic_node_with_partial_schema() {
        let mut linker = Linker::new();

        // Register a dynamic node that also has some known schema
        let schema = TypeInfo::new("PartialOutput", 1)
            .with_size(8)
            .with_fields(vec![
                FieldInfo::new("status", "u32").with_offset(0).with_size(4),
            ])
            .stable();

        let node_schema = NodeSchema::new(NodeId::new(3), "hybrid_processor")
            .with_output("out", schema)
            .dynamic();

        linker.register_node("hybrid_processor", node_schema);

        // Known field - should resolve statically
        let known_selector = Selector::new("hybrid_processor", vec!["status".to_string()]);
        let known_compiled = linker.compile(&known_selector).unwrap();
        assert!(known_compiled.is_static());

        // Unknown field - should fall back to dynamic
        let unknown_selector = Selector::new(
            "hybrid_processor",
            vec!["dynamic_field".to_string(), "nested".to_string()],
        );
        let unknown_compiled = linker.compile(&unknown_selector).unwrap();
        assert!(unknown_compiled.is_dynamic());
    }
}
