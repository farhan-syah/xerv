//! WAL snapshot types for faster recovery.
//!
//! Snapshots capture the state of all in-flight traces at a specific
//! WAL position, allowing recovery to skip replaying records before
//! the snapshot.

use crate::types::{ArenaOffset, NodeId, TraceId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Configuration for WAL snapshotting.
#[derive(Debug, Clone)]
pub struct SnapshotConfig {
    /// Create a snapshot every N records.
    pub record_interval: u64,
    /// Create a snapshot after this duration.
    pub time_interval: Duration,
    /// Number of snapshots to retain.
    pub retention_count: usize,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            record_interval: 10_000,
            time_interval: Duration::from_secs(300), // 5 minutes
            retention_count: 3,
        }
    }
}

impl SnapshotConfig {
    /// Create a config that creates snapshots frequently (for testing).
    pub fn frequent() -> Self {
        Self {
            record_interval: 100,
            time_interval: Duration::from_secs(10),
            retention_count: 3,
        }
    }

    /// Set the record interval.
    pub fn with_record_interval(mut self, interval: u64) -> Self {
        self.record_interval = interval;
        self
    }

    /// Set the time interval.
    pub fn with_time_interval(mut self, interval: Duration) -> Self {
        self.time_interval = interval;
        self
    }

    /// Set the retention count.
    pub fn with_retention_count(mut self, count: usize) -> Self {
        self.retention_count = count;
        self
    }
}

/// Position within the WAL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WalPosition {
    /// WAL file sequence number.
    pub file_sequence: u64,
    /// Byte offset within the file.
    pub offset: u64,
}

impl WalPosition {
    /// Create a new WAL position.
    pub fn new(file_sequence: u64, offset: u64) -> Self {
        Self {
            file_sequence,
            offset,
        }
    }

    /// Check if this position is before another.
    pub fn is_before(&self, other: &WalPosition) -> bool {
        self.file_sequence < other.file_sequence
            || (self.file_sequence == other.file_sequence && self.offset < other.offset)
    }
}

/// State of a trace in a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotTraceState {
    /// The trace identifier.
    pub trace_id: TraceId,
    /// The last node that completed successfully.
    pub last_completed_node: Option<NodeId>,
    /// Node where the trace is suspended (if any).
    pub suspended_at: Option<NodeId>,
    /// Nodes that have started but not completed.
    pub started_nodes: Vec<NodeId>,
    /// Map of completed nodes to their output locations.
    pub completed_nodes: HashMap<NodeId, SnapshotNodeOutput>,
}

impl SnapshotTraceState {
    /// Create a new trace state for a snapshot.
    pub fn new(trace_id: TraceId) -> Self {
        Self {
            trace_id,
            last_completed_node: None,
            suspended_at: None,
            started_nodes: Vec::new(),
            completed_nodes: HashMap::new(),
        }
    }
}

/// Node output location stored in a snapshot.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SnapshotNodeOutput {
    /// Arena offset where output is stored.
    pub offset: ArenaOffset,
    /// Size of the output in bytes.
    pub size: u32,
    /// Schema hash for type verification.
    pub schema_hash: u64,
}

/// A WAL snapshot containing trace states at a specific position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalSnapshot {
    /// Unique snapshot identifier.
    pub id: u64,
    /// WAL position when this snapshot was created.
    pub wal_position: WalPosition,
    /// States of all in-flight traces at snapshot time.
    pub trace_states: HashMap<TraceId, SnapshotTraceState>,
    /// Timestamp when the snapshot was created.
    pub created_at: u64,
    /// Number of records processed before this snapshot.
    pub record_count: u64,
}

impl WalSnapshot {
    /// Create a new snapshot.
    pub fn new(
        id: u64,
        wal_position: WalPosition,
        trace_states: HashMap<TraceId, SnapshotTraceState>,
        record_count: u64,
    ) -> Self {
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self {
            id,
            wal_position,
            trace_states,
            created_at,
            record_count,
        }
    }

    /// Get the number of traces in this snapshot.
    pub fn trace_count(&self) -> usize {
        self.trace_states.len()
    }

    /// Check if this snapshot is empty (no in-flight traces).
    pub fn is_empty(&self) -> bool {
        self.trace_states.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wal_position_ordering() {
        let pos1 = WalPosition::new(1, 100);
        let pos2 = WalPosition::new(1, 200);
        let pos3 = WalPosition::new(2, 0);

        assert!(pos1.is_before(&pos2));
        assert!(pos2.is_before(&pos3));
        assert!(pos1.is_before(&pos3));
        assert!(!pos2.is_before(&pos1));
    }

    #[test]
    fn snapshot_creation() {
        let mut states = HashMap::new();
        let trace_id = TraceId::new();
        states.insert(trace_id, SnapshotTraceState::new(trace_id));

        let snapshot = WalSnapshot::new(1, WalPosition::new(0, 1024), states, 1000);

        assert_eq!(snapshot.id, 1);
        assert_eq!(snapshot.wal_position.offset, 1024);
        assert_eq!(snapshot.trace_count(), 1);
        assert_eq!(snapshot.record_count, 1000);
    }

    #[test]
    fn snapshot_config_builder() {
        let config = SnapshotConfig::default()
            .with_record_interval(5000)
            .with_time_interval(Duration::from_secs(60))
            .with_retention_count(5);

        assert_eq!(config.record_interval, 5000);
        assert_eq!(config.time_interval, Duration::from_secs(60));
        assert_eq!(config.retention_count, 5);
    }
}
