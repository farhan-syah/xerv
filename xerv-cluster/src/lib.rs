//! Distributed clustering and consensus for XERV.
//!
//! This crate provides multi-node high-availability for XERV using the Raft
//! consensus algorithm via OpenRaft.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    XERV Cluster                             │
//! │                                                             │
//! │  ┌─────────────┐   ┌─────────────┐   ┌─────────────┐       │
//! │  │   Node 1    │   │   Node 2    │   │   Node 3    │       │
//! │  │  (Leader)   │   │ (Follower)  │   │ (Follower)  │       │
//! │  │             │   │             │   │             │       │
//! │  │ ┌─────────┐ │   │ ┌─────────┐ │   │ ┌─────────┐ │       │
//! │  │ │  Raft   │◄┼───┼─┤  Raft   │◄┼───┼─┤  Raft   │ │       │
//! │  │ │ Engine  │ │   │ │ Engine  │ │   │ │ Engine  │ │       │
//! │  │ └────┬────┘ │   │ └────┬────┘ │   │ └────┬────┘ │       │
//! │  │      │      │   │      │      │   │      │      │       │
//! │  │ ┌────▼────┐ │   │ ┌────▼────┐ │   │ ┌────▼────┐ │       │
//! │  │ │  State  │ │   │ │  State  │ │   │ │  State  │ │       │
//! │  │ │ Machine │ │   │ │ Machine │ │   │ │ Machine │ │       │
//! │  │ └─────────┘ │   │ └─────────┘ │   │ └─────────┘ │       │
//! │  └─────────────┘   └─────────────┘   └─────────────┘       │
//! │                                                             │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use xerv_cluster::{ClusterConfig, ClusterNode};
//!
//! // Configure the cluster
//! let config = ClusterConfig::builder()
//!     .node_id(1)
//!     .listen_addr("127.0.0.1:5000")
//!     .peers(vec![
//!         (2, "127.0.0.1:5001".to_string()),
//!         (3, "127.0.0.1:5002".to_string()),
//!     ])
//!     .build();
//!
//! // Start the node
//! let node = ClusterNode::start(config).await?;
//!
//! // Execute commands through the cluster (automatically routed to leader)
//! node.execute(ClusterCommand::StartTrace { ... }).await?;
//! ```

pub mod command;
pub mod config;
pub mod error;
pub mod network;
pub mod raft;
pub mod service;
pub mod state;
pub mod types;

// Re-export main types
pub use command::ClusterCommand;
pub use config::ClusterConfig;
pub use error::{ClusterError, ClusterResult};
pub use raft::ClusterNode;
pub use service::ClusterServiceImpl;
pub use state::{ClusterResponse, ClusterStateMachine, TraceStatus};
pub use types::{ClusterNodeId, TypeConfig};

// Generated protobuf code
pub mod proto {
    tonic::include_proto!("xerv.raft");
}
