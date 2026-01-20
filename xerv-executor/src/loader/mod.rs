#![allow(clippy::module_inception)]

//! Flow loader for converting YAML definitions to executable FlowGraphs.
//!
//! This module provides the bridge between YAML flow definitions and the
//! runtime FlowGraph representation used by the scheduler.
//!
//! # Example
//!
//! ```ignore
//! use xerv_executor::loader::FlowLoader;
//!
//! // Load from file
//! let graph = FlowLoader::from_file("flows/order_processing.yaml")?;
//!
//! // Or load from string
//! let yaml = r#"
//! name: my_flow
//! triggers:
//!   - id: webhook
//!     type: webhook
//! nodes:
//!   process:
//!     type: std::log
//! edges:
//!   - from: webhook
//!     to: process
//! "#;
//! let graph = FlowLoader::from_yaml(yaml)?;
//! ```

mod builder;
mod loader;

pub use builder::FlowBuilder;
pub use loader::{FlowLoader, LoadedFlow, LoaderConfig, LoaderError};
