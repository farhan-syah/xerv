//! Write-Ahead Log (WAL) for durability and crash recovery.
//!
//! The WAL records every state transition in the execution of a trace.
//! On crash, the WAL is replayed to recover the state of in-flight traces.
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
pub use writer::{NodeOutputLocation, TraceRecoveryState, Wal, WalConfig, WalReader};
