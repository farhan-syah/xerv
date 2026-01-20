//! Node trait and related types.

use super::context::Context;
use crate::error::Result;
use crate::types::RelPtr;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// Direction of a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortDirection {
    /// Input port.
    Input,
    /// Output port.
    Output,
}

/// A port on a node.
#[derive(Debug, Clone)]
pub struct Port {
    /// Port name (e.g., "in", "out", "error", "true", "false").
    pub name: String,
    /// Port direction.
    pub direction: PortDirection,
    /// Schema name for the data type.
    pub schema: String,
    /// Whether this port is required.
    pub required: bool,
    /// Description of the port.
    pub description: String,
}

impl Port {
    /// Create a standard input port.
    pub fn input(schema: impl Into<String>) -> Self {
        Self {
            name: "in".to_string(),
            direction: PortDirection::Input,
            schema: schema.into(),
            required: true,
            description: "Default input".to_string(),
        }
    }

    /// Create a standard output port.
    pub fn output(schema: impl Into<String>) -> Self {
        Self {
            name: "out".to_string(),
            direction: PortDirection::Output,
            schema: schema.into(),
            required: false,
            description: "Default output".to_string(),
        }
    }

    /// Create an error output port.
    pub fn error() -> Self {
        Self {
            name: "error".to_string(),
            direction: PortDirection::Output,
            schema: "Error@v1".to_string(),
            required: false,
            description: "Error output".to_string(),
        }
    }

    /// Create a named port.
    pub fn named(
        name: impl Into<String>,
        direction: PortDirection,
        schema: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            direction,
            schema: schema.into(),
            required: direction == PortDirection::Input,
            description: String::new(),
        }
    }

    /// Set the port as optional.
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    /// Set the port description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }
}

/// Metadata about a node type.
#[derive(Debug, Clone)]
pub struct NodeInfo {
    /// Fully qualified name (e.g., "std::switch", "plugins::fraud_model").
    pub name: String,
    /// Namespace (e.g., "std", "plugins").
    pub namespace: String,
    /// Short name (e.g., "switch", "fraud_model").
    pub short_name: String,
    /// Description of what the node does.
    pub description: String,
    /// Version of the node implementation.
    pub version: String,
    /// Input ports.
    pub inputs: Vec<Port>,
    /// Output ports.
    pub outputs: Vec<Port>,
    /// Whether this node has side effects.
    pub effectful: bool,
    /// Whether this node is deterministic (same input = same output).
    pub deterministic: bool,
}

impl NodeInfo {
    /// Create new node info.
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        let namespace = namespace.into();
        let short_name = name.into();
        let full_name = format!("{}::{}", namespace, short_name);

        Self {
            name: full_name,
            namespace,
            short_name,
            description: String::new(),
            version: "1.0.0".to_string(),
            inputs: vec![Port::input("Any")],
            outputs: vec![Port::output("Any"), Port::error()],
            effectful: false,
            deterministic: true,
        }
    }

    /// Set the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set the version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Set input ports.
    pub fn with_inputs(mut self, inputs: Vec<Port>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Set output ports.
    pub fn with_outputs(mut self, outputs: Vec<Port>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Mark as effectful (has side effects).
    pub fn effectful(mut self) -> Self {
        self.effectful = true;
        self
    }

    /// Mark as non-deterministic.
    pub fn non_deterministic(mut self) -> Self {
        self.deterministic = false;
        self
    }

    /// Get an input port by name.
    pub fn get_input(&self, name: &str) -> Option<&Port> {
        self.inputs.iter().find(|p| p.name == name)
    }

    /// Get an output port by name.
    pub fn get_output(&self, name: &str) -> Option<&Port> {
        self.outputs.iter().find(|p| p.name == name)
    }
}

/// Output from a node execution.
#[derive(Debug)]
pub struct NodeOutput {
    /// The output port that was activated.
    pub port: String,
    /// Pointer to the output data in the arena.
    pub data: RelPtr<()>,
    /// Schema hash of the output data.
    pub schema_hash: u64,
    /// Optional error message (for error outputs without arena data).
    pub error_message: Option<String>,
}

impl NodeOutput {
    /// Create a new node output.
    pub fn new<T>(port: impl Into<String>, data: RelPtr<T>) -> Self {
        Self {
            port: port.into(),
            data: RelPtr::new(data.offset(), data.size()),
            schema_hash: 0,
            error_message: None,
        }
    }

    /// Create output on the default "out" port.
    pub fn out<T>(data: RelPtr<T>) -> Self {
        Self::new("out", data)
    }

    /// Create output on the "error" port with arena data.
    pub fn error<T>(data: RelPtr<T>) -> Self {
        Self::new("error", data)
    }

    /// Create output on the "error" port with a message string.
    ///
    /// Use this when you have an error message but no arena data to write.
    /// The error message will be available for logging and debugging.
    pub fn error_with_message(message: impl Into<String>) -> Self {
        Self {
            port: "error".to_string(),
            data: RelPtr::null(),
            schema_hash: 0,
            error_message: Some(message.into()),
        }
    }

    /// Create output on the "true" port (for switch nodes).
    pub fn on_true<T>(data: RelPtr<T>) -> Self {
        Self::new("true", data)
    }

    /// Create output on the "false" port (for switch nodes).
    pub fn on_false<T>(data: RelPtr<T>) -> Self {
        Self::new("false", data)
    }

    /// Set the schema hash.
    pub fn with_schema_hash(mut self, hash: u64) -> Self {
        self.schema_hash = hash;
        self
    }

    /// Check if this output has an error message.
    pub fn has_error_message(&self) -> bool {
        self.error_message.is_some()
    }

    /// Get the error message, if any.
    pub fn get_error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// Get the arena location (offset and size) of the output data.
    ///
    /// Returns `(offset, size)` tuple for use in WAL records and crash recovery.
    /// Returns `(ArenaOffset::NULL, 0)` if the data pointer is null.
    pub fn arena_location(&self) -> (crate::types::ArenaOffset, u32) {
        (self.data.offset(), self.data.size())
    }
}

/// A boxed future for async node execution.
pub type NodeFuture<'a> = Pin<Box<dyn Future<Output = Result<NodeOutput>> + Send + 'a>>;

/// The core trait for all XERV nodes.
///
/// Nodes are the basic units of computation in a flow. Each node:
/// - Receives input from upstream nodes via the arena
/// - Performs some computation
/// - Writes output to the arena
/// - Returns which output port to activate
///
/// # Example
///
/// ```ignore
/// use xerv_core::prelude::*;
///
/// struct MyNode {
///     config: MyConfig,
/// }
///
/// impl Node for MyNode {
///     fn info(&self) -> NodeInfo {
///         NodeInfo::new("custom", "my_node")
///             .with_description("My custom node")
///     }
///
///     fn execute<'a>(&'a self, ctx: Context, input: RelPtr<()>) -> NodeFuture<'a> {
///         Box::pin(async move {
///             // Read input
///             let data = ctx.read::<MyInput>(input.cast())?;
///
///             // Process
///             let output = MyOutput { value: data.value * 2 };
///
///             // Write output
///             let ptr = ctx.write(&output)?;
///             Ok(NodeOutput::out(ptr))
///         })
///     }
/// }
/// ```
pub trait Node: Send + Sync {
    /// Get metadata about this node.
    fn info(&self) -> NodeInfo;

    /// Execute the node.
    ///
    /// # Parameters
    /// - `ctx`: Execution context with access to arena and logging
    /// - `inputs`: Map of input port names to data pointers
    ///
    /// # Returns
    /// The output port and data pointer to activate.
    fn execute<'a>(&'a self, ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a>;

    /// Called when the node is being shut down.
    fn shutdown(&self) {}

    /// Get the schema hash for this node's output type.
    fn output_schema_hash(&self) -> u64 {
        0
    }
}

/// A node factory that creates node instances from configuration.
pub trait NodeFactory: Send + Sync {
    /// Get the node type name this factory creates.
    fn node_type(&self) -> &str;

    /// Create a new node instance from YAML configuration.
    fn create(&self, config: &serde_yaml::Value) -> Result<Box<dyn Node>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_creation() {
        let input = Port::input("OrderInput@v1");
        assert_eq!(input.name, "in");
        assert_eq!(input.direction, PortDirection::Input);
        assert!(input.required);

        let output = Port::output("OrderOutput@v1").optional();
        assert_eq!(output.name, "out");
        assert!(!output.required);
    }

    #[test]
    fn node_info_creation() {
        let info = NodeInfo::new("std", "switch")
            .with_description("Conditional branching")
            .with_inputs(vec![Port::input("Any")])
            .with_outputs(vec![
                Port::named("true", PortDirection::Output, "Any"),
                Port::named("false", PortDirection::Output, "Any"),
                Port::error(),
            ]);

        assert_eq!(info.name, "std::switch");
        assert_eq!(info.namespace, "std");
        assert_eq!(info.short_name, "switch");
        assert_eq!(info.inputs.len(), 1);
        assert_eq!(info.outputs.len(), 3);
    }
}
