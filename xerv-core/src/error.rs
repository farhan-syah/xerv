//! Error types for XERV.
//!
//! This module provides strongly-typed errors with actionable context.
//! All errors include relevant identifiers (trace ID, node ID, etc.) to
//! aid in debugging and tracing.

use crate::types::{ArenaOffset, NodeId, TraceId};
use std::path::PathBuf;
use thiserror::Error;

/// The main error type for XERV operations.
#[derive(Error, Debug)]
pub enum XervError {
    // =========================================================================
    // Arena Errors (E001-E099)
    // =========================================================================
    /// Failed to create or open arena file.
    #[error("E001: Failed to create arena at {path}: {cause}")]
    ArenaCreate {
        /// The path where arena creation failed.
        path: PathBuf,
        /// Reason for the failure.
        cause: String,
    },

    /// Failed to memory-map the arena file.
    #[error("E002: Failed to mmap arena at {path}: {cause}")]
    ArenaMmap {
        /// The path of the arena file.
        path: PathBuf,
        /// Reason for the mmap failure.
        cause: String,
    },

    /// Arena write operation failed.
    #[error("E003: Arena write failed at offset {offset} for trace {trace_id}: {cause}")]
    ArenaWrite {
        /// The trace identifier.
        trace_id: TraceId,
        /// The offset where write was attempted.
        offset: ArenaOffset,
        /// Reason for the write failure.
        cause: String,
    },

    /// Arena read operation failed.
    #[error("E004: Arena read failed at offset {offset}: {cause}")]
    ArenaRead {
        /// The offset that could not be read.
        offset: ArenaOffset,
        /// Reason for the read failure.
        cause: String,
    },

    /// Arena capacity exceeded.
    #[error(
        "E005: Arena capacity exceeded: requested {requested} bytes, available {available} bytes"
    )]
    ArenaCapacity {
        /// Number of bytes requested.
        requested: u64,
        /// Number of bytes available.
        available: u64,
    },

    /// Invalid arena offset.
    #[error("E006: Invalid arena offset {offset}: {cause}")]
    ArenaInvalidOffset {
        /// The invalid offset.
        offset: ArenaOffset,
        /// Reason why the offset is invalid.
        cause: String,
    },

    /// Arena corruption detected.
    #[error("E007: Arena corruption detected at offset {offset}: {cause}")]
    ArenaCorruption {
        /// The offset where corruption was detected.
        offset: ArenaOffset,
        /// Description of the corruption.
        cause: String,
    },

    // =========================================================================
    // Selector/Linker Errors (E100-E199)
    // =========================================================================
    /// Failed to resolve a selector expression.
    #[error("E101: Selector resolution failed for '{selector}' in node {node_id}: {cause}")]
    SelectorResolution {
        /// The selector expression that failed to resolve.
        selector: String,
        /// The node where resolution was attempted.
        node_id: NodeId,
        /// Reason for the resolution failure.
        cause: String,
    },

    /// Non-deterministic layout detected (unstable offsets).
    #[error("E102: Non-deterministic layout for type '{type_name}': {cause}")]
    NonDeterministicLayout {
        /// The type with non-deterministic layout.
        type_name: String,
        /// Description of the layout issue.
        cause: String,
    },

    /// Invalid selector syntax.
    #[error("E103: Invalid selector syntax '{selector}': {cause}")]
    SelectorSyntax {
        /// The selector with invalid syntax.
        selector: String,
        /// Description of the syntax error.
        cause: String,
    },

    /// Selector target not found.
    #[error("E104: Selector target '{field}' not found in output of node {node_id}")]
    SelectorTargetNotFound {
        /// The field that was not found.
        field: String,
        /// The node whose output was queried.
        node_id: NodeId,
    },

    /// Type mismatch in selector.
    #[error("E105: Type mismatch for selector '{selector}': expected {expected}, got {actual}")]
    SelectorTypeMismatch {
        /// The selector expression with type mismatch.
        selector: String,
        /// The expected type.
        expected: String,
        /// The actual type found.
        actual: String,
    },

    // =========================================================================
    // Schema Evolution Errors (E110-E119)
    // =========================================================================
    /// Schema version not found.
    #[error("E110: Schema version '{schema}@{version}' not found")]
    SchemaVersionNotFound {
        /// The schema name.
        schema: String,
        /// The requested version.
        version: String,
    },

    /// Schema incompatible with no migration.
    #[error("E111: Schema incompatible: {from} → {to}, no migration available")]
    SchemaIncompatible {
        /// The source schema version.
        from: String,
        /// The target schema version.
        to: String,
        /// Reason why schemas are incompatible.
        cause: String,
    },

    /// Migration failed.
    #[error("E112: Migration failed from {from} to {to}: {cause}")]
    MigrationFailed {
        /// The source schema version.
        from: String,
        /// The target schema version.
        to: String,
        /// Reason for the migration failure.
        cause: String,
    },

    /// Breaking schema change without migration.
    #[error("E113: Breaking change in {schema}: {change}")]
    BreakingSchemaChange {
        /// The affected schema.
        schema: String,
        /// Description of the breaking change.
        change: String,
    },

    /// Migration path too long.
    #[error("E114: Migration path from {from} to {to} exceeds max hops ({max_hops})")]
    MigrationPathTooLong {
        /// The source schema version.
        from: String,
        /// The target schema version.
        to: String,
        /// The maximum allowed number of migration hops.
        max_hops: u32,
    },

    // =========================================================================
    // Runtime/Dependency Errors (E200-E299)
    // =========================================================================
    /// Missing binary dependency.
    #[error("E201: Missing binary '{binary}': {cause}")]
    MissingBinary {
        /// The name of the missing binary.
        binary: String,
        /// Reason why the binary is missing.
        cause: String,
    },

    /// Runtime version mismatch.
    #[error("E202: Runtime mismatch for '{runtime}': required {required}, found {found}")]
    RuntimeMismatch {
        /// The runtime with version mismatch.
        runtime: String,
        /// The required version.
        required: String,
        /// The found version.
        found: String,
    },

    /// Container image unavailable.
    #[error("E203: Container image unavailable: {image}")]
    ContainerImageUnavailable {
        /// The container image reference.
        image: String,
    },

    /// Unsupported execution profile.
    #[error("E204: Unsupported execution profile '{profile}' on this host")]
    UnsupportedProfile {
        /// The requested execution profile.
        profile: String,
    },

    // =========================================================================
    // Node Execution Errors (E300-E399)
    // =========================================================================
    /// Node execution failed.
    #[error("E301: Node {node_id} execution failed in trace {trace_id}: {cause}")]
    NodeExecution {
        /// The node that failed to execute.
        node_id: NodeId,
        /// The trace in which execution occurred.
        trace_id: TraceId,
        /// Reason for the execution failure.
        cause: String,
    },

    /// Node timeout.
    #[error("E302: Node {node_id} timed out after {timeout_ms}ms in trace {trace_id}")]
    NodeTimeout {
        /// The node that timed out.
        node_id: NodeId,
        /// The trace in which timeout occurred.
        trace_id: TraceId,
        /// Timeout duration in milliseconds.
        timeout_ms: u64,
    },

    /// Node panic.
    #[error("E303: Node {node_id} panicked in trace {trace_id}: {message}")]
    NodePanic {
        /// The node that panicked.
        node_id: NodeId,
        /// The trace in which panic occurred.
        trace_id: TraceId,
        /// The panic message.
        message: String,
    },

    /// Invalid node configuration.
    #[error("E304: Invalid configuration for node {node_id}: {cause}")]
    NodeConfig {
        /// The node with invalid configuration.
        node_id: NodeId,
        /// Description of the configuration error.
        cause: String,
    },

    /// Node not found.
    #[error("E305: Node '{node_name}' not found in flow")]
    NodeNotFound {
        /// The name of the node that was not found.
        node_name: String,
    },

    // =========================================================================
    // Flow/Topology Errors (E400-E499)
    // =========================================================================
    /// No compatible version for migration.
    #[error("E401: No compatible version found for trace {trace_id}: {cause}")]
    NoCompatibleVersion {
        /// The trace for which no compatible version was found.
        trace_id: TraceId,
        /// Reason why no compatible version is available.
        cause: String,
    },

    /// Invalid flow topology.
    #[error("E402: Invalid flow topology: {cause}")]
    InvalidTopology {
        /// Description of the topology issue.
        cause: String,
    },

    /// Cycle detected without loop controller.
    #[error("E403: Uncontrolled cycle detected involving nodes: {nodes:?}")]
    UncontrolledCycle {
        /// The nodes involved in the cycle.
        nodes: Vec<NodeId>,
    },

    /// Invalid port reference.
    #[error("E404: Invalid port '{port}' on node {node_id}")]
    InvalidPort {
        /// The port name that is invalid.
        port: String,
        /// The node with the invalid port.
        node_id: NodeId,
    },

    /// Missing required edge.
    #[error("E405: Missing required edge from {from_node}.{from_port} to {to_node}.{to_port}")]
    MissingEdge {
        /// The source node.
        from_node: NodeId,
        /// The source port.
        from_port: String,
        /// The destination node.
        to_node: NodeId,
        /// The destination port.
        to_port: String,
    },

    /// Invalid edge (references non-existent node or port).
    #[error("E406: Invalid edge from {from_node}.{from_port} to {to_node}.{to_port}")]
    InvalidEdge {
        /// The source node.
        from_node: NodeId,
        /// The source port.
        from_port: String,
        /// The destination node.
        to_node: NodeId,
        /// The destination port.
        to_port: String,
    },

    // =========================================================================
    // Pipeline Errors (E500-E599)
    // =========================================================================
    /// Pipeline not found.
    #[error("E501: Pipeline '{pipeline_id}' not found")]
    PipelineNotFound {
        /// The pipeline identifier that was not found.
        pipeline_id: String,
    },

    /// Pipeline already exists.
    #[error("E502: Pipeline '{pipeline_id}' already exists")]
    PipelineExists {
        /// The pipeline identifier that already exists.
        pipeline_id: String,
    },

    /// Pipeline concurrency limit reached.
    #[error("E503: Pipeline '{pipeline_id}' concurrency limit reached: {current}/{max}")]
    ConcurrencyLimit {
        /// The pipeline identifier.
        pipeline_id: String,
        /// Current number of concurrent executions.
        current: u32,
        /// Maximum allowed concurrent executions.
        max: u32,
    },

    /// Pipeline circuit breaker triggered.
    #[error("E504: Pipeline '{pipeline_id}' circuit breaker open: error rate {error_rate:.1}%")]
    CircuitBreakerOpen {
        /// The pipeline identifier.
        pipeline_id: String,
        /// The error rate that triggered the circuit breaker.
        error_rate: f64,
    },

    /// Pipeline drain timeout.
    #[error("E505: Pipeline '{pipeline_id}' drain timeout: {pending_traces} traces still pending")]
    DrainTimeout {
        /// The pipeline identifier.
        pipeline_id: String,
        /// Number of traces still pending.
        pending_traces: u32,
    },

    // =========================================================================
    // WAL Errors (E600-E699)
    // =========================================================================
    /// WAL write failed.
    #[error("E601: WAL write failed for trace {trace_id}: {cause}")]
    WalWrite {
        /// The trace being written to the WAL.
        trace_id: TraceId,
        /// Reason for the write failure.
        cause: String,
    },

    /// WAL read failed.
    #[error("E602: WAL read failed: {cause}")]
    WalRead {
        /// Reason for the read failure.
        cause: String,
    },

    /// WAL corruption detected.
    #[error("E603: WAL corruption detected at position {position}: {cause}")]
    WalCorruption {
        /// The position in the WAL where corruption was detected.
        position: u64,
        /// Description of the corruption.
        cause: String,
    },

    /// WAL replay failed.
    #[error("E604: WAL replay failed for trace {trace_id}: {cause}")]
    WalReplay {
        /// The trace being replayed from the WAL.
        trace_id: TraceId,
        /// Reason for the replay failure.
        cause: String,
    },

    // =========================================================================
    // WASM Errors (E700-E799)
    // =========================================================================
    /// WASM module loading failed.
    #[error("E701: Failed to load WASM module '{module}': {cause}")]
    WasmLoad {
        /// The WASM module that failed to load.
        module: String,
        /// Reason for the load failure.
        cause: String,
    },

    /// WASM execution failed.
    #[error("E702: WASM execution failed in node {node_id}: {cause}")]
    WasmExecution {
        /// The node running WASM that failed.
        node_id: NodeId,
        /// Reason for the execution failure.
        cause: String,
    },

    /// WASM memory allocation failed.
    #[error("E703: WASM memory allocation failed: requested {requested} bytes")]
    WasmMemoryAlloc {
        /// Number of bytes requested for allocation.
        requested: u64,
    },

    /// WASM host function error.
    #[error("E704: WASM host function '{function}' failed: {cause}")]
    WasmHostFunction {
        /// The host function name that failed.
        function: String,
        /// Reason for the function failure.
        cause: String,
    },

    // =========================================================================
    // Configuration Errors (E800-E899)
    // =========================================================================
    /// YAML parsing failed.
    #[error("E801: Failed to parse YAML at {path}: {cause}")]
    YamlParse {
        /// The path to the YAML file.
        path: PathBuf,
        /// Reason for the parse failure.
        cause: String,
    },

    /// Invalid configuration value.
    #[error("E802: Invalid configuration '{field}': {cause}")]
    ConfigValue {
        /// The configuration field with invalid value.
        field: String,
        /// Description of why the value is invalid.
        cause: String,
    },

    /// Schema validation failed.
    #[error("E803: Schema validation failed for '{schema}': {cause}")]
    SchemaValidation {
        /// The schema being validated.
        schema: String,
        /// Reason for the validation failure.
        cause: String,
    },

    /// Serialization/deserialization error.
    #[error("E804: Serialization error: {0}")]
    Serialization(
        /// The serialization error message.
        String,
    ),

    // =========================================================================
    // I/O Errors (E900-E999)
    // =========================================================================
    /// File I/O error.
    #[error("E901: I/O error at {path}: {cause}")]
    Io {
        /// The path where the I/O error occurred.
        path: PathBuf,
        /// Description of the I/O error.
        cause: String,
    },

    /// Network error.
    #[error("E902: Network error: {cause}")]
    Network {
        /// Description of the network error.
        cause: String,
    },

    // =========================================================================
    // Authentication/Authorization Errors (E1000-E1099)
    // =========================================================================
    /// Authentication failed.
    #[error("E1001: Authentication failed: {cause}")]
    AuthenticationFailed {
        /// Reason for the authentication failure.
        cause: String,
    },

    /// Authorization denied.
    #[error(
        "E1002: Authorization denied for '{identity}': required scope '{required_scope}' - {cause}"
    )]
    AuthorizationDenied {
        /// The identity that was denied.
        identity: String,
        /// The required scope.
        required_scope: String,
        /// Reason for the denial.
        cause: String,
    },

    // =========================================================================
    // Suspension Errors (E1100-E1199)
    // =========================================================================
    /// Hook not found.
    #[error("E1101: Hook '{hook_id}' not found")]
    HookNotFound {
        /// The hook ID that was not found.
        hook_id: String,
    },

    /// Suspension failed.
    #[error("E1102: Suspension failed for trace {trace_id}: {cause}")]
    SuspensionFailed {
        /// The trace being suspended.
        trace_id: TraceId,
        /// Reason for the suspension failure.
        cause: String,
    },

    // =========================================================================
    // Compaction Errors (E1200-E1299)
    // =========================================================================
    /// Compaction failed.
    #[error("E1201: Arena compaction failed: {cause}")]
    CompactionFailed {
        /// Reason for the compaction failure.
        cause: String,
    },

    // =========================================================================
    // Snapshot Errors (E1300-E1399)
    // =========================================================================
    /// Snapshot creation failed.
    #[error("E1301: WAL snapshot creation failed: {cause}")]
    SnapshotCreation {
        /// Reason for the snapshot failure.
        cause: String,
    },

    /// Snapshot load failed.
    #[error("E1302: WAL snapshot load failed: {cause}")]
    SnapshotLoad {
        /// Reason for the load failure.
        cause: String,
    },
}

impl XervError {
    /// Get the error code (e.g., "E001").
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::ArenaCreate { .. } => "E001",
            Self::ArenaMmap { .. } => "E002",
            Self::ArenaWrite { .. } => "E003",
            Self::ArenaRead { .. } => "E004",
            Self::ArenaCapacity { .. } => "E005",
            Self::ArenaInvalidOffset { .. } => "E006",
            Self::ArenaCorruption { .. } => "E007",
            Self::SelectorResolution { .. } => "E101",
            Self::NonDeterministicLayout { .. } => "E102",
            Self::SelectorSyntax { .. } => "E103",
            Self::SelectorTargetNotFound { .. } => "E104",
            Self::SelectorTypeMismatch { .. } => "E105",
            Self::SchemaVersionNotFound { .. } => "E110",
            Self::SchemaIncompatible { .. } => "E111",
            Self::MigrationFailed { .. } => "E112",
            Self::BreakingSchemaChange { .. } => "E113",
            Self::MigrationPathTooLong { .. } => "E114",
            Self::MissingBinary { .. } => "E201",
            Self::RuntimeMismatch { .. } => "E202",
            Self::ContainerImageUnavailable { .. } => "E203",
            Self::UnsupportedProfile { .. } => "E204",
            Self::NodeExecution { .. } => "E301",
            Self::NodeTimeout { .. } => "E302",
            Self::NodePanic { .. } => "E303",
            Self::NodeConfig { .. } => "E304",
            Self::NodeNotFound { .. } => "E305",
            Self::NoCompatibleVersion { .. } => "E401",
            Self::InvalidTopology { .. } => "E402",
            Self::UncontrolledCycle { .. } => "E403",
            Self::InvalidPort { .. } => "E404",
            Self::MissingEdge { .. } => "E405",
            Self::PipelineNotFound { .. } => "E501",
            Self::PipelineExists { .. } => "E502",
            Self::ConcurrencyLimit { .. } => "E503",
            Self::CircuitBreakerOpen { .. } => "E504",
            Self::DrainTimeout { .. } => "E505",
            Self::WalWrite { .. } => "E601",
            Self::WalRead { .. } => "E602",
            Self::WalCorruption { .. } => "E603",
            Self::WalReplay { .. } => "E604",
            Self::WasmLoad { .. } => "E701",
            Self::WasmExecution { .. } => "E702",
            Self::WasmMemoryAlloc { .. } => "E703",
            Self::WasmHostFunction { .. } => "E704",
            Self::YamlParse { .. } => "E801",
            Self::ConfigValue { .. } => "E802",
            Self::SchemaValidation { .. } => "E803",
            Self::Serialization(_) => "E804",
            Self::Io { .. } => "E901",
            Self::Network { .. } => "E902",
            Self::InvalidEdge { .. } => "E406",
            Self::AuthenticationFailed { .. } => "E1001",
            Self::AuthorizationDenied { .. } => "E1002",
            Self::HookNotFound { .. } => "E1101",
            Self::SuspensionFailed { .. } => "E1102",
            Self::CompactionFailed { .. } => "E1201",
            Self::SnapshotCreation { .. } => "E1301",
            Self::SnapshotLoad { .. } => "E1302",
        }
    }

    /// Check if this error is retriable.
    #[must_use]
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            Self::ArenaWrite { .. }
                | Self::NodeTimeout { .. }
                | Self::WalWrite { .. }
                | Self::Network { .. }
        )
    }

    /// Check if this error is a configuration/validation error.
    #[must_use]
    pub fn is_config_error(&self) -> bool {
        matches!(
            self,
            Self::SelectorResolution { .. }
                | Self::NonDeterministicLayout { .. }
                | Self::SelectorSyntax { .. }
                | Self::SelectorTargetNotFound { .. }
                | Self::SelectorTypeMismatch { .. }
                | Self::SchemaVersionNotFound { .. }
                | Self::BreakingSchemaChange { .. }
                | Self::InvalidTopology { .. }
                | Self::UncontrolledCycle { .. }
                | Self::InvalidPort { .. }
                | Self::MissingEdge { .. }
                | Self::YamlParse { .. }
                | Self::ConfigValue { .. }
                | Self::SchemaValidation { .. }
        )
    }

    /// Check if this error is a schema evolution error.
    #[must_use]
    pub fn is_schema_error(&self) -> bool {
        matches!(
            self,
            Self::SchemaVersionNotFound { .. }
                | Self::SchemaIncompatible { .. }
                | Self::MigrationFailed { .. }
                | Self::BreakingSchemaChange { .. }
                | Self::MigrationPathTooLong { .. }
                | Self::SchemaValidation { .. }
        )
    }
}

/// Result type alias using `XervError`.
pub type Result<T> = std::result::Result<T, XervError>;

/// Extension trait for adding context to errors.
pub trait ResultExt<T> {
    /// Add trace context to an error.
    fn with_trace(self, trace_id: TraceId) -> Result<T>;

    /// Add node context to an error.
    fn with_node(self, node_id: NodeId) -> Result<T>;
}

impl<T, E: std::fmt::Display> ResultExt<T> for std::result::Result<T, E> {
    fn with_trace(self, trace_id: TraceId) -> Result<T> {
        self.map_err(|e| XervError::NodeExecution {
            node_id: NodeId::new(0),
            trace_id,
            cause: e.to_string(),
        })
    }

    fn with_node(self, node_id: NodeId) -> Result<T> {
        self.map_err(|e| XervError::NodeConfig {
            node_id,
            cause: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_correct() {
        let err = XervError::ArenaCreate {
            path: PathBuf::from("/tmp/test"),
            cause: "test".to_string(),
        };
        assert_eq!(err.code(), "E001");

        let err = XervError::SelectorResolution {
            selector: "${test}".to_string(),
            node_id: NodeId::new(1),
            cause: "not found".to_string(),
        };
        assert_eq!(err.code(), "E101");
    }

    #[test]
    fn error_display() {
        let err = XervError::NodeTimeout {
            node_id: NodeId::new(5),
            trace_id: TraceId::new(),
            timeout_ms: 5000,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("E302"));
        assert!(msg.contains("node_5"));
        assert!(msg.contains("5000ms"));
    }

    #[test]
    fn retriable_errors() {
        assert!(
            XervError::Network {
                cause: "timeout".to_string()
            }
            .is_retriable()
        );

        assert!(
            !XervError::InvalidTopology {
                cause: "cycle".to_string()
            }
            .is_retriable()
        );
    }

    #[test]
    fn config_errors() {
        assert!(
            XervError::YamlParse {
                path: PathBuf::from("test.yaml"),
                cause: "syntax".to_string()
            }
            .is_config_error()
        );

        assert!(
            !XervError::NodeExecution {
                node_id: NodeId::new(1),
                trace_id: TraceId::new(),
                cause: "failed".to_string()
            }
            .is_config_error()
        );
    }
}
