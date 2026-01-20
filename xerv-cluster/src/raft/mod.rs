//! Raft integration module.
//!
//! This module provides the ClusterNode which ties together:
//! - Log storage (RaftLogStorage)
//! - State machine (ClusterStateMachine)
//! - Network (RaftNetwork)
//! - The Raft instance itself

mod node;
mod storage;

pub use node::ClusterNode;
pub use storage::LogStorage;
