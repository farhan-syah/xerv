//! Snapshot data structures.
//!
//! Note: Snapshot storage is now handled within `machine.rs` using `StoredSnapshot`.
//! This file is kept for backwards compatibility of the public API.

use crate::state::machine::ClusterState;

/// Snapshot of the cluster state.
///
/// This is an alias for ClusterState which is what gets serialized into snapshots.
pub type SnapshotData = ClusterState;
