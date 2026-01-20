//! XERV Core Library
//!
//! This crate provides the foundational types, traits, and implementations
//! for the XERV orchestration platform.
//!
//! # Overview
//!
//! XERV (Executable Server) is a zero-copy, event-driven orchestration platform
//! that bridges visual automation with systems engineering.
//!
//! # Key Components
//!
//! - **Arena**: Memory-mapped append-only storage for zero-copy data sharing
//! - **WAL**: Write-Ahead Log for durability and crash recovery
//! - **Types**: Strongly-typed identifiers and pointers
//! - **Traits**: Core abstractions for nodes, triggers, and schemas
//!
//! # Example
//!
//! ```ignore
//! use xerv_core::prelude::*;
//!
//! // Create an arena for a trace
//! let trace_id = TraceId::new();
//! let config = ArenaConfig::default();
//! let arena = Arena::create(trace_id, &config)?;
//!
//! // Write data
//! let ptr = arena.write(&MyData { value: 42 })?;
//!
//! // Read data back (zero-copy)
//! let archived = arena.read(ptr)?;
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod arena;
pub mod auth;
pub mod error;
pub mod flow;
pub mod logging;
pub mod prelude;
pub mod schema;
// Testing module must be declared before traits because traits/context.rs uses testing providers
pub mod testing;
pub mod traits;
pub mod types;
pub mod value;
pub mod wal;

// Re-export key types at crate root for convenience
pub use arena::{Arena, ArenaConfig};
pub use error::{Result, XervError};
pub use flow::{EdgeDefinition, FlowDefinition, FlowSettings, NodeDefinition, TriggerDefinition};
pub use traits::{Node, NodeInfo, PipelineConfig, Schema, Trigger, TriggerType};
pub use types::{ArenaOffset, NodeId, RelPtr, TraceId};
pub use value::Value;
pub use wal::{Wal, WalConfig};
