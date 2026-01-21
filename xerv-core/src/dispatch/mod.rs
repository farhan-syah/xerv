//! Pluggable dispatch backends for trace execution.
//!
//! This module provides an abstraction over how traces are queued and distributed
//! to workers. XERV supports multiple dispatch backends:
//!
//! - **Raft** (default): Built-in consensus, zero external dependencies
//! - **Redis**: High-throughput, requires external Redis
//! - **NATS**: Cloud-native streaming, requires external NATS
//!
//! # Choosing a Backend
//!
//! | Backend | Throughput | Dependencies | Use Case |
//! |---------|------------|--------------|----------|
//! | Raft    | ~5k/s      | None         | Edge, air-gapped, simple ops |
//! | Redis   | ~50k+/s    | Redis        | Cloud, K8s, high-throughput |
//! | NATS    | ~30k+/s    | NATS         | Multi-region, event-driven |
//!
//! # Example
//!
//! ```ignore
//! use xerv_core::dispatch::{DispatchBackend, DispatchConfig};
//!
//! // Create a backend from config
//! let config = DispatchConfig::default(); // Uses Raft
//! let backend = config.build().await?;
//!
//! // Enqueue a trace
//! let trace_id = backend.enqueue(request).await?;
//!
//! // Dequeue (for workers)
//! if let Some(request) = backend.dequeue().await? {
//!     // Process the trace
//!     backend.ack(request.trace_id).await?;
//! }
//! ```

mod config;
mod request;
mod traits;

#[cfg(feature = "dispatch-raft")]
pub mod raft;

#[cfg(feature = "dispatch-redis")]
pub mod redis;

#[cfg(feature = "dispatch-nats")]
pub mod nats;

// Re-export public API
pub use config::{
    DispatchConfig, MemoryConfig, NatsConfig, NatsCredentials, RaftConfig, RaftConfigBuilder,
    RaftPeer, RedisConfig,
};
pub use request::{TraceRequest, TraceRequestBuilder};
pub use traits::{DispatchBackend, DispatchError, DispatchResult};

#[cfg(feature = "dispatch-raft")]
pub use raft::{RaftClusterProvider, RaftDispatch};

#[cfg(feature = "dispatch-redis")]
pub use redis::RedisDispatch;

#[cfg(feature = "dispatch-nats")]
pub use nats::NatsDispatch;

/// Default in-memory dispatch for testing and single-node deployments.
pub mod memory;
pub use memory::MemoryDispatch;
