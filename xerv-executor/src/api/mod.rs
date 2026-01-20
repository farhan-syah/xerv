//! REST API for XERV pipeline management.
//!
//! This module provides an HTTP API for:
//! - Pipeline lifecycle management (deploy, start, pause, stop, drain)
//! - Trigger control (list, fire, pause, resume)
//! - Trace inspection (list, get details)
//! - Health and status endpoints
//!
//! # Architecture
//!
//! The API uses pure Hyper 1.x for HTTP handling, matching existing XERV patterns.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                      ApiServer                           │
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
//! │  │   router     │──│   handlers   │──│    state     │  │
//! │  └──────────────┘  └──────────────┘  └──────────────┘  │
//! │         │                  │                 │          │
//! │         ▼                  ▼                 ▼          │
//! │  ┌──────────────────────────────────────────────────┐  │
//! │  │              PipelineController                   │  │
//! │  └──────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use xerv_executor::api::{ApiServer, ServerConfig};
//! use xerv_executor::pipeline::PipelineController;
//! use xerv_executor::listener::ListenerPool;
//! use std::sync::Arc;
//!
//! let controller = Arc::new(PipelineController::new());
//! let listener_pool = Arc::new(ListenerPool::new());
//! let config = ServerConfig::new("0.0.0.0", 8080);
//!
//! let mut server = ApiServer::new(config, controller, listener_pool);
//! server.run().await?;
//! ```

mod error;
pub mod handlers;
mod request;
mod response;
mod router;
mod server;
mod state;

pub use error::ApiError;
pub use server::{ApiServer, ServerConfig};
pub use state::{AppState, TraceHistory, TraceRecord, TraceStatus};
