//! Raft state machine for XERV cluster state.
//!
//! The state machine is the core of the cluster's replicated state. It:
//! - Receives committed log entries (commands)
//! - Applies them deterministically to produce consistent state
//! - Supports snapshots for log compaction and state transfer
//!
//! ## Module Structure
//!
//! - `types`: Response types, trace/pipeline state info
//! - `state`: The replicated ClusterState and serialization
//! - `apply`: Command application logic
//! - `traits`: OpenRaft trait implementations

mod apply;
mod state;
mod traits;
mod types;

pub use state::ClusterState;
pub use types::{ClusterResponse, StoredSnapshot, TraceStatus};

use std::sync::atomic::AtomicU64;
use tokio::sync::RwLock;

/// The Raft state machine.
///
/// This struct holds the replicated cluster state and provides
/// thread-safe access for reads and applies.
#[derive(Debug, Default)]
pub struct ClusterStateMachine {
    /// The replicated state.
    state: RwLock<ClusterState>,
    /// Snapshot index counter.
    snapshot_idx: AtomicU64,
    /// Current snapshot.
    current_snapshot: RwLock<Option<StoredSnapshot>>,
}

impl ClusterStateMachine {
    /// Create a new state machine.
    pub fn new() -> Self {
        Self {
            state: RwLock::new(ClusterState::default()),
            snapshot_idx: AtomicU64::new(0),
            current_snapshot: RwLock::new(None),
        }
    }

    /// Get a read-only view of the current state.
    ///
    /// Note: For strong consistency, route reads through the leader
    /// using Raft linearizable reads. This method provides eventual
    /// consistency reads from the local replica.
    pub async fn state(&self) -> tokio::sync::RwLockReadGuard<'_, ClusterState> {
        self.state.read().await
    }
}
