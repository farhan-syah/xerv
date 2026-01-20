//! Suspension system for WaitNode and human-in-the-loop workflows.
//!
//! This module provides:
//! - `SuspensionRequest` - A request from a node to suspend trace execution
//! - `SuspensionStore` - Trait for storing suspended trace state
//! - `MemorySuspensionStore` - In-memory implementation for testing
//! - `ResumeDecision` - Decision made when resuming a suspended trace
//! - `TimeoutProcessor` - Background task for handling suspension timeouts

mod request;
mod store;
mod timeout;

pub use request::{ResumeDecision, SuspendedTraceState, SuspensionRequest, TimeoutAction};
pub use store::{MemorySuspensionStore, SuspensionStore};
pub use timeout::TimeoutProcessor;
