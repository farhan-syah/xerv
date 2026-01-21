//! Write-Ahead Log (WAL) for durability and crash recovery.
//!
//! The WAL records every state transition in the execution of a trace.
//! On crash, the WAL is replayed to recover the state of in-flight traces.
//!
//! # Deployment Modes and Durability
//!
//! XERV supports two deployment modes with different durability guarantees:
//!
//! ## Single-Node Mode (WAL)
//!
//! When running a single XERV instance, this WAL provides crash recovery:
//! - Records trace lifecycle events (start, node completion, errors)
//! - Enables recovery of in-flight traces after process restart
//! - Uses CRC32 checksums for corruption detection
//!
//! ## Clustered Mode (Raft)
//!
//! When running XERV in a cluster (`xerv-cluster`), durability is provided
//! by Raft's replicated log instead:
//! - Commands are replicated across the cluster before acknowledgment
//! - Raft's log provides durability and consistency guarantees
//! - The WAL is **NOT used** in clustered mode to avoid duplication
//!
//! This separation ensures:
//! - Single-node deployments don't need Raft overhead
//! - Clustered deployments don't have redundant durability layers
//!
//! # Record Format
//!
//! Each WAL record has the following structure:
//! ```text
//! ┌─────────┬────────┬───────┬──────────┬──────────┬─────────┐
//! │ Length  │ CRC32  │ Type  │ TraceId  │ NodeId   │ Payload │
//! │ (4 B)   │ (4 B)  │ (1 B) │ (16 B)   │ (4 B)    │ (var)   │
//! └─────────┴────────┴───────┴──────────┴──────────┴─────────┘
//! ```
//!
//! # Snapshots
//!
//! To avoid replaying the entire WAL on recovery, snapshots can be created
//! at regular intervals. A snapshot captures the state of all in-flight
//! traces at a specific WAL position, allowing recovery to start from the
//! snapshot instead of the beginning.

mod record;
mod snapshot;
mod snapshot_manager;
mod writer;

pub use record::{WalRecord, WalRecordType};
pub use snapshot::{
    SnapshotConfig, SnapshotNodeOutput, SnapshotTraceState, WalPosition, WalSnapshot,
};
pub use snapshot_manager::SnapshotManager;
pub use writer::{
    GroupCommitConfig, GroupCommitStats, NodeOutputLocation, TraceRecoveryState, Wal, WalConfig,
    WalReader,
};
