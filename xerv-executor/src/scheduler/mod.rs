//! Topological scheduler and execution engine.
//!
//! This module provides the core scheduling and execution infrastructure:
//! - [`FlowGraph`] - DAG representation of a flow
//! - [`Executor`] - Trace execution engine
//! - Topological sorting with cycle detection
//! - Support for declared loops via `std::loop` nodes

mod executor;
mod graph;

pub use executor::{ExecutionSignal, Executor, ExecutorConfig};
pub use graph::{Edge, FlowGraph, GraphNode};
