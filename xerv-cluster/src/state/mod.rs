//! Raft state machine implementation.
//!
//! The state machine receives committed log entries (ClusterCommands) and
//! applies them to produce the cluster state. All nodes apply the same
//! commands in the same order, ensuring consistent state.

mod machine;
mod snapshot;

pub use machine::{ClusterResponse, ClusterStateMachine, TraceStatus};
pub use snapshot::SnapshotData;
