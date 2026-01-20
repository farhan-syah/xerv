//! Type definitions for OpenRaft integration.
//!
//! OpenRaft requires a type configuration that specifies all the concrete types
//! used in the Raft implementation. This module defines XERV's type configuration.

use crate::command::ClusterCommand;
use crate::state::ClusterResponse;
use openraft::BasicNode;
use std::io::Cursor;

/// Node ID type for the cluster.
///
/// Each node in the cluster has a unique 64-bit identifier.
pub type ClusterNodeId = u64;

// Use the declare_raft_types! macro to define the type configuration.
// This handles all the trait bounds and associated types correctly.
openraft::declare_raft_types!(
    /// OpenRaft type configuration for XERV.
    pub TypeConfig:
        D = ClusterCommand,
        R = ClusterResponse,
);

/// Type alias for Raft instance.
pub type ClusterRaft = openraft::Raft<TypeConfig>;

/// Type alias for log entry.
pub type ClusterEntry = openraft::Entry<TypeConfig>;

/// Type alias for vote.
pub type ClusterVote = openraft::Vote<ClusterNodeId>;

/// Type alias for log ID.
pub type ClusterLogId = openraft::LogId<ClusterNodeId>;

/// Type alias for membership.
pub type ClusterMembership = openraft::Membership<ClusterNodeId, BasicNode>;

/// Type alias for stored membership.
pub type ClusterStoredMembership = openraft::StoredMembership<ClusterNodeId, BasicNode>;

/// Type alias for snapshot metadata.
pub type ClusterSnapshotMeta = openraft::SnapshotMeta<ClusterNodeId, BasicNode>;

/// Type alias for snapshot.
pub type ClusterSnapshot = openraft::storage::Snapshot<TypeConfig>;

/// Type alias for log state.
pub type ClusterLogState = openraft::LogState<TypeConfig>;

/// Type alias for storage error.
pub type ClusterStorageError = openraft::StorageError<ClusterNodeId>;

/// Type alias for client write error.
pub type ClusterClientWriteError = openraft::error::ClientWriteError<ClusterNodeId, BasicNode>;

/// Type alias for forward-to-leader error.
pub type ClusterForwardToLeader = openraft::error::ForwardToLeader<ClusterNodeId, BasicNode>;

/// Type alias for raft error with client write error.
pub type ClusterRaftWriteError = openraft::error::RaftError<ClusterNodeId, ClusterClientWriteError>;

/// Information about the leader to forward requests to.
#[derive(Debug, Clone)]
pub struct LeaderInfo {
    /// The leader's node ID.
    pub leader_id: ClusterNodeId,
    /// The leader's address (may be empty if unknown).
    pub leader_addr: String,
}

/// Extract forward-to-leader info from a client write error.
///
/// Returns `Some(LeaderInfo)` if the error is a ForwardToLeader error with
/// a known leader ID, `None` otherwise.
pub fn extract_forward_to_leader(error: &ClusterRaftWriteError) -> Option<LeaderInfo> {
    use openraft::error::RaftError;

    match error {
        RaftError::APIError(ClusterClientWriteError::ForwardToLeader(forward)) => {
            // Only return if we have a leader ID
            forward.leader_id.map(|leader_id| LeaderInfo {
                leader_id,
                leader_addr: forward
                    .leader_node
                    .as_ref()
                    .map(|n| n.addr.clone())
                    .unwrap_or_default(),
            })
        }
        _ => None,
    }
}
