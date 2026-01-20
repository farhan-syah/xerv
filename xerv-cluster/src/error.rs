//! Error types for cluster operations.

use crate::types::ClusterNodeId;
use openraft::BasicNode;
use thiserror::Error;

/// Result type for cluster operations.
pub type ClusterResult<T> = std::result::Result<T, ClusterError>;

/// Type alias for OpenRaft Raft errors.
pub type RaftError<E = openraft::error::Infallible> = openraft::error::RaftError<ClusterNodeId, E>;

/// Type alias for OpenRaft RPC errors.
pub type RPCError<E = openraft::error::Infallible> =
    openraft::error::RPCError<ClusterNodeId, BasicNode, RaftError<E>>;

/// Type alias for client write errors.
pub type ClientWriteError = openraft::error::ClientWriteError<ClusterNodeId, BasicNode>;

/// Type alias for initialize errors.
pub type InitializeError = openraft::error::InitializeError<ClusterNodeId, BasicNode>;

/// Type alias for forward to leader errors.
pub type ForwardToLeader = openraft::error::ForwardToLeader<ClusterNodeId, BasicNode>;

/// Errors that can occur in cluster operations.
#[derive(Debug, Error)]
pub enum ClusterError {
    /// Raft consensus error.
    #[error("Raft error: {0}")]
    Raft(Box<RaftError>),

    /// Network/RPC error.
    #[error("Network error: {0}")]
    Network(#[from] tonic::Status),

    /// Transport error.
    #[error("Transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Storage error.
    #[error("Storage error: {0}")]
    Storage(String),

    /// Node not found in cluster.
    #[error("Node {0} not found in cluster")]
    NodeNotFound(u64),

    /// Not the leader - includes leader hint if known.
    #[error("Not the leader, leader is node {leader:?}")]
    NotLeader {
        /// The current leader if known.
        leader: Option<u64>,
    },

    /// Cluster not initialized.
    #[error("Cluster not initialized")]
    NotInitialized,

    /// Configuration error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// XERV core error.
    #[error("XERV error: {0}")]
    Xerv(#[from] xerv_core::error::XervError),
}

impl From<RaftError> for ClusterError {
    fn from(e: RaftError) -> Self {
        ClusterError::Raft(Box::new(e))
    }
}

impl From<serde_json::Error> for ClusterError {
    fn from(e: serde_json::Error) -> Self {
        ClusterError::Serialization(e.to_string())
    }
}
