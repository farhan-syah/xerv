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
///
/// Nodes return one of three variants:
/// - `Complete`: Normal completion with data on a specific output port
/// - `Error`: Execution error with an optional message
/// - `Suspend`: Request to suspend the trace for human-in-the-loop approval
#[derive(Debug)]
pub enum NodeOutput {
    /// Node completed successfully with output data.
    Complete {
        /// The output port that was activated (e.g., "out", "true", "false").
        port: String,
        /// Pointer to the output data in the arena.
        data: RelPtr<()>,
        /// Schema hash of the output data.
        schema_hash: u64,
    },

    /// Node execution resulted in an error.
    Error {
        /// Error message describing what went wrong.
        message: String,
        /// Optional pointer to error data in the arena.
        data: Option<RelPtr<()>>,
    },

    /// Node requests trace suspension (human-in-the-loop).
    ///
    /// When a node returns this variant, the executor will:
    /// 1. Flush the arena to disk
    /// 2. Store the trace state in the suspension store
    /// 3. Remove the trace from active memory
    /// 4. Wait for an external resume signal via the API
    Suspend {
        /// The suspension request with hook ID and timeout settings.
        request: crate::suspension::SuspensionRequest,
        /// Pointer to pending data that should be preserved.
        pending_data: RelPtr<()>,
    },
}

impl NodeOutput {
    /// Create a new node output on a specific port.
    pub fn new<T>(port: impl Into<String>, data: RelPtr<T>) -> Self {
        Self::Complete {
            port: port.into(),
            data: RelPtr::new(data.offset(), data.size()),
            schema_hash: 0,
        }
    }

    /// Create output on the default "out" port.
    pub fn out<T>(data: RelPtr<T>) -> Self {
        Self::new("out", data)
    }

    /// Create output on the "error" port with arena data.
    pub fn error<T>(data: RelPtr<T>) -> Self {
        Self::Error {
            message: String::new(),
            data: Some(RelPtr::new(data.offset(), data.size())),
        }
    }

    /// Create output on the "error" port with a message string.
    ///
    /// Use this when you have an error message but no arena data to write.
    /// The error message will be available for logging and debugging.
    pub fn error_with_message(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            data: None,
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

    /// Create a suspension request.
    ///
    /// The executor will pause this trace and wait for an external
    /// resume signal via the API.
    pub fn suspend<T>(
        request: crate::suspension::SuspensionRequest,
        pending_data: RelPtr<T>,
    ) -> Self {
        Self::Suspend {
            request,
            pending_data: RelPtr::new(pending_data.offset(), pending_data.size()),
        }
    }

    /// Set the schema hash (only applies to Complete variant).
    pub fn with_schema_hash(mut self, hash: u64) -> Self {
        if let Self::Complete { schema_hash, .. } = &mut self {
            *schema_hash = hash;
        }
        self
    }

    /// Check if this output is a suspension request.
    pub fn is_suspended(&self) -> bool {
        matches!(self, Self::Suspend { .. })
    }

    /// Check if this output is an error.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }

    /// Check if this output is a successful completion.
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete { .. })
    }

    /// Check if this output has an error message.
    pub fn has_error_message(&self) -> bool {
        matches!(self, Self::Error { message, .. } if !message.is_empty())
    }

    /// Get the error message, if any.
    pub fn get_error_message(&self) -> Option<&str> {
        match self {
            Self::Error { message, .. } if !message.is_empty() => Some(message),
            _ => None,
        }
    }

    /// Get the output port name.
    ///
    /// Returns the port for Complete variants, "error" for Error variants,
    /// and None for Suspend variants.
    pub fn port(&self) -> Option<&str> {
        match self {
            Self::Complete { port, .. } => Some(port),
            Self::Error { .. } => Some("error"),
            Self::Suspend { .. } => None,
        }
    }

    /// Check if this output matches a specific port name.
    ///
    /// Returns `true` if this output has a port that matches the expected port.
    /// Useful for checking if an edge should be activated.
    pub fn matches_port(&self, expected_port: &str) -> bool {
        self.port().map_or(false, |p| p == expected_port)
    }

    /// Get the arena location (offset and size) of the output data.
    ///
    /// Returns `(offset, size)` tuple for use in WAL records and crash recovery.
    /// Returns `(ArenaOffset::NULL, 0)` if the data pointer is null or for errors
    /// without data.
    pub fn arena_location(&self) -> (crate::types::ArenaOffset, u32) {
        match self {
            Self::Complete { data, .. } => (data.offset(), data.size()),
            Self::Error {
                data: Some(ptr), ..
            } => (ptr.offset(), ptr.size()),
            Self::Error { data: None, .. } => (crate::types::ArenaOffset::NULL, 0),
            Self::Suspend { pending_data, .. } => (pending_data.offset(), pending_data.size()),
        }
    }

    /// Get the schema hash (only for Complete variant).
    pub fn schema_hash(&self) -> u64 {
        match self {
            Self::Complete { schema_hash, .. } => *schema_hash,
            _ => 0,
        }
    }

    /// Get the data pointer (for Complete and some Error variants).
    pub fn data(&self) -> RelPtr<()> {
        match self {
            Self::Complete { data, .. } => *data,
            Self::Error {
                data: Some(ptr), ..
            } => *ptr,
            Self::Error { data: None, .. } => RelPtr::null(),
            Self::Suspend { pending_data, .. } => *pending_data,
        }
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
