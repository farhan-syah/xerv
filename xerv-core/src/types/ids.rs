//! Strongly-typed identifiers for XERV entities.

use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use std::fmt;
use uuid::Uuid;

/// Unique identifier for a trace (single execution of a flow).
///
/// Each trace gets its own arena file and WAL entries.
/// The trace ID is stored as raw bytes internally for efficient serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(compare(PartialEq))]
#[repr(C)]
pub struct TraceId {
    /// UUID bytes in big-endian format.
    bytes: [u8; 16],
}

impl TraceId {
    /// Create a new random trace ID.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bytes: *Uuid::new_v4().as_bytes(),
        }
    }

    /// Create a trace ID from an existing UUID.
    #[must_use]
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self {
            bytes: *uuid.as_bytes(),
        }
    }

    /// Get the underlying UUID.
    #[must_use]
    pub fn as_uuid(&self) -> Uuid {
        Uuid::from_bytes(self.bytes)
    }

    /// Create a trace ID from a string (for testing/debugging).
    ///
    /// # Errors
    /// Returns `None` if the string is not a valid UUID.
    pub fn parse(s: &str) -> Option<Self> {
        Uuid::parse_str(s).ok().map(Self::from_uuid)
    }
}

impl Default for TraceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "trace_{}", self.as_uuid())
    }
}

impl SerdeSerialize for TraceId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.as_uuid().serialize(serializer)
    }
}

impl<'de> SerdeDeserialize<'de> for TraceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let uuid = Uuid::deserialize(deserializer)?;
        Ok(Self::from_uuid(uuid))
    }
}

/// Identifier for a node within a flow.
///
/// Node IDs are assigned at flow definition time and remain stable across executions.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
    SerdeSerialize,
    SerdeDeserialize,
)]
#[rkyv(compare(PartialEq))]
#[repr(C)]
pub struct NodeId(u32);

impl NodeId {
    /// Create a new node ID from a raw value.
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Get the raw ID value.
    #[must_use]
    pub const fn as_u32(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "node_{}", self.0)
    }
}

impl From<u32> for NodeId {
    fn from(id: u32) -> Self {
        Self(id)
    }
}

/// Identifier for a pipeline (flow definition).
///
/// Pipelines are versioned and can have multiple concurrent instances.
#[derive(Debug, Clone, PartialEq, Eq, Hash, SerdeSerialize, SerdeDeserialize)]
pub struct PipelineId {
    /// The pipeline name (e.g., "order_processing").
    pub name: String,
    /// The pipeline version.
    pub version: u32,
}

impl PipelineId {
    /// Create a new pipeline ID.
    #[must_use]
    pub fn new(name: impl Into<String>, version: u32) -> Self {
        Self {
            name: name.into(),
            version,
        }
    }

    /// Get the pipeline version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Get the pipeline name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for PipelineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@v{}", self.name, self.version)
    }
}

/// Identifier for a port on a node.
///
/// Ports are named connection points for inputs and outputs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PortId {
    /// The node this port belongs to.
    pub node: NodeId,
    /// The port name (e.g., "in", "out", "error", "true", "false").
    pub name: String,
}

impl PortId {
    /// Create a new port ID.
    #[must_use]
    pub fn new(node: NodeId, name: impl Into<String>) -> Self {
        Self {
            node,
            name: name.into(),
        }
    }

    /// Create the default input port for a node.
    #[must_use]
    pub fn input(node: NodeId) -> Self {
        Self::new(node, "in")
    }

    /// Create the default output port for a node.
    #[must_use]
    pub fn output(node: NodeId) -> Self {
        Self::new(node, "out")
    }

    /// Create the error output port for a node.
    #[must_use]
    pub fn error(node: NodeId) -> Self {
        Self::new(node, "error")
    }
}

impl fmt::Display for PortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.node, self.name)
    }
}

/// Parse a port reference string like "node_name.port_name".
impl std::str::FromStr for PortId {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(2, '.').collect();
        if parts.len() != 2 {
            return Err("Port ID must be in format 'node.port'");
        }
        // For now, we can't resolve node names to IDs without context
        // This is a placeholder implementation
        Err("Port ID parsing requires flow context")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_id_uniqueness() {
        let id1 = TraceId::new();
        let id2 = TraceId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn trace_id_display() {
        let id = TraceId::new();
        let display = format!("{}", id);
        assert!(display.starts_with("trace_"));
    }

    #[test]
    fn trace_id_roundtrip() {
        let id = TraceId::new();
        let uuid = id.as_uuid();
        let restored = TraceId::from_uuid(uuid);
        assert_eq!(id, restored);
    }

    #[test]
    fn node_id_creation() {
        let id = NodeId::new(42);
        assert_eq!(id.as_u32(), 42);
    }

    #[test]
    fn port_id_creation() {
        let node = NodeId::new(1);
        let port = PortId::new(node, "out");
        assert_eq!(port.node, node);
        assert_eq!(port.name, "out");
    }

    #[test]
    fn port_id_display() {
        let port = PortId::new(NodeId::new(5), "error");
        assert_eq!(format!("{}", port), "node_5.error");
    }

    #[test]
    fn pipeline_id_display() {
        let id = PipelineId::new("order_processing", 2);
        assert_eq!(format!("{}", id), "order_processing@v2");
    }
}
