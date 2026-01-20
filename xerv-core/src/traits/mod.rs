//! Core traits for XERV components.
//!
//! This module defines the fundamental traits that all XERV components implement:
//! - `Node`: The basic unit of computation in a flow
//! - `Trigger`: Entry point for flow execution
//! - `Schema`: Type-safe data contracts for node I/O

mod context;
mod node;
mod pipeline;
mod schema;
mod trigger;

pub use context::{Context, PipelineCtx, TriggerController};
pub use node::{Node, NodeFactory, NodeFuture, NodeInfo, NodeOutput, Port, PortDirection};
pub use pipeline::{PipelineConfig, PipelineHook, PipelineSettings, PipelineState};
pub use schema::{FieldInfo, Schema, SchemaRegistry, TypeInfo};
pub use trigger::{
    Trigger, TriggerConfig, TriggerEvent, TriggerFactory, TriggerFuture, TriggerType,
};
