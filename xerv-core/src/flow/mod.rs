//! Flow definition types for YAML deserialization.
//!
//! This module provides strongly-typed structures for parsing YAML flow definitions:
//!
//! - [`FlowDefinition`] - The top-level flow document
//! - [`NodeDefinition`] - Individual node configuration
//! - [`EdgeDefinition`] - Connection between nodes
//! - [`TriggerDefinition`] - Trigger configuration
//!
//! # Example YAML
//!
//! ```yaml
//! name: order_processing
//! version: "1.0"
//! description: Process incoming orders
//!
//! triggers:
//!   - id: webhook_in
//!     type: webhook
//!     params:
//!       port: 8080
//!       path: /orders
//!
//! nodes:
//!   fraud_check:
//!     type: std::switch
//!     config:
//!       condition:
//!         type: greater_than
//!         field: risk_score
//!         value: 0.8
//!
//!   high_risk:
//!     type: std::log
//!     config:
//!       message: "High risk order detected"
//!
//! edges:
//!   - from: webhook_in
//!     to: fraud_check
//!   - from: fraud_check.true
//!     to: high_risk
//!
//! settings:
//!   max_concurrent_executions: 100
//!   execution_timeout_ms: 60000
//! ```

mod definition;
mod edge;
mod node;
mod settings;
mod trigger;
mod validation;

pub use definition::FlowDefinition;
pub use edge::EdgeDefinition;
pub use node::NodeDefinition;
pub use settings::FlowSettings;
pub use trigger::TriggerDefinition;
pub use validation::{ValidationError, ValidationResult};
