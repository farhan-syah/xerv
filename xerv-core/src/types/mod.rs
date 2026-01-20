//! Core types for XERV.
//!
//! This module provides fundamental types used throughout the XERV ecosystem:
//! - `TraceId`: Unique identifier for a trace (execution instance)
//! - `NodeId`: Identifier for a node within a flow
//! - `ArenaOffset`: Offset into the memory-mapped arena
//! - `RelPtr`: Relative pointer for zero-copy data access

mod ids;
mod pointer;

pub use ids::{NodeId, PipelineId, PortId, TraceId};
pub use pointer::{ArenaOffset, ArenaSlice, RelPtr};
