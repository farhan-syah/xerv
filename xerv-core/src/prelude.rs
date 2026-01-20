//! Prelude for convenient imports.
//!
//! This module re-exports the most commonly used types and traits.
//!
//! # Example
//!
//! ```ignore
//! use xerv_core::prelude::*;
//! ```

// Core types
pub use crate::types::{ArenaOffset, ArenaSlice, NodeId, PipelineId, PortId, RelPtr, TraceId};

// Error handling
pub use crate::error::{Result, ResultExt, XervError};

// Arena
pub use crate::arena::{Arena, ArenaConfig, ArenaHeader, ArenaReader, ArenaWriter};

// WAL
pub use crate::wal::{Wal, WalConfig, WalReader, WalRecord, WalRecordType};

// Traits
pub use crate::traits::{
    Context, FieldInfo, Node, NodeFactory, NodeFuture, NodeInfo, NodeOutput, PipelineConfig,
    PipelineCtx, PipelineHook, PipelineSettings, PipelineState, Port, PortDirection, Schema,
    SchemaRegistry, Trigger, TriggerConfig, TriggerController, TriggerEvent, TriggerFactory,
    TriggerFuture, TriggerType, TypeInfo,
};

// Flow definitions
pub use crate::flow::{
    EdgeDefinition, FlowDefinition, FlowSettings, NodeDefinition, TriggerDefinition,
    ValidationError, ValidationResult,
};

// Schema evolution
pub use crate::schema::{
    ChangeKind, CompatibilityMatrix, CompatibilityReport, DefaultProvider, DefaultRegistry,
    FieldDefault, MapDefaultProvider, Migration, MigrationFn, MigrationRegistry, SchemaChange,
    SchemaVersion, VersionRange, VersionedSchemaRegistry,
};

// Logging
pub use crate::logging::{
    BufferedCollector, LogCategory, LogCollector, LogContext, LogEvent, LogEventBuilder, LogFilter,
    LogFilterBuilder, LogLevel,
};

// Re-export rkyv for node implementations
pub use rkyv::{Archive, Deserialize, Serialize};
