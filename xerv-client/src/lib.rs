//! Type-safe Rust client for the XERV workflow orchestration API.
//!
//! This crate provides a high-level, ergonomic interface to the XERV server,
//! with full type safety through shared types from `xerv-core`.
//!
//! # Features
//!
//! - Type-safe API client with builder pattern
//! - Authentication support (Bearer token)
//! - Pipeline management (deploy, list, get, delete, lifecycle control)
//! - Trigger operations (list, fire, pause, resume)
//! - Trace inspection (list, get, query)
//! - Log querying and streaming
//! - Comprehensive error handling
//!
//! # Example
//!
//! ```no_run
//! use xerv_client::Client;
//! use serde_json::json;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a client
//! let client = Client::new("http://localhost:8080")?
//!     .with_api_key("my-secret-key");
//!
//! // Deploy a pipeline
//! let yaml = std::fs::read_to_string("my-pipeline.yaml")?;
//! let pipeline = client.deploy_pipeline(&yaml).await?;
//! println!("Deployed: {}", pipeline.pipeline_id);
//!
//! // Fire a trigger
//! let payload = json!({ "user_id": "123" });
//! let response = client.fire_trigger(&pipeline.pipeline_id, "webhook-1", &payload).await?;
//! println!("Started trace: {}", response.trace_id);
//!
//! // Get trace status
//! let trace = client.get_trace(response.trace_id).await?;
//! println!("Trace status: {:?}", trace.status);
//! # Ok(())
//! # }
//! ```
//!
//! # Authentication
//!
//! The client supports Bearer token authentication:
//!
//! ```no_run
//! # use xerv_client::Client;
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::new("http://localhost:8080")?
//!     .with_api_key(std::env::var("XERV_API_KEY")?);
//! # Ok(())
//! # }
//! ```
//!
//! # Error Handling
//!
//! All operations return `Result<T, ClientError>`:
//!
//! ```no_run
//! # use xerv_client::{Client, ClientError};
//! # async fn example() -> Result<(), ClientError> {
//! # let client = Client::new("http://localhost:8080")?;
//! match client.get_pipeline("my-pipeline").await {
//!     Ok(pipeline) => println!("Found: {}", pipeline.name),
//!     Err(ClientError::Api { status: 404, .. }) => println!("Pipeline not found"),
//!     Err(e) => println!("Error: {}", e),
//! }
//! # Ok(())
//! # }
//! ```

mod client;
mod error;
mod logs;
mod pipelines;
mod traces;
mod triggers;
mod types;

// Re-export the main types
pub use client::Client;
pub use error::{ClientError, Result};
pub use logs::LogStats;
pub use types::{
    HealthStatus, LogEntry, NodeId, PipelineInfo, PipelineStatus, TraceDetail, TraceId, TraceInfo,
    TraceStatus, TriggerInfo, TriggerResponse, ValidationIssue, ValidationReport,
};
