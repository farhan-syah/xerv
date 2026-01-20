//! WAL writer and reader implementation.
//!
//! This module provides the core WAL functionality:
//! - [`WalConfig`] - Configuration for WAL creation
//! - [`GroupCommitConfig`] - Configuration for batching fsyncs
//! - [`Wal`] - The main WAL writer
//! - [`WalReader`] - Reader for WAL recovery

mod config;
mod group_commit;
mod reader;
mod wal;

pub use config::{GroupCommitConfig, WalConfig};
pub use group_commit::GroupCommitStats;
pub use reader::{NodeOutputLocation, TraceRecoveryState, WalReader};
pub use wal::Wal;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ArenaOffset, NodeId, TraceId};
    use crate::wal::record::{WalRecord, WalRecordType};
    use tempfile::tempdir;

    #[test]
    fn wal_write_and_read() {
        let dir = tempdir().unwrap();
        let config = WalConfig::default()
            .with_directory(dir.path())
            .with_sync(false);

        let wal = Wal::open(config).unwrap();

        let trace_id = TraceId::new();
        let node_id = NodeId::new(1);

        // Write some records
        wal.write(&WalRecord::trace_start(trace_id)).unwrap();
        wal.write(&WalRecord::node_start(trace_id, node_id))
            .unwrap();
        wal.write(&WalRecord::node_done(
            trace_id,
            node_id,
            ArenaOffset::new(0x100),
            64,
            0,
        ))
        .unwrap();
        wal.write(&WalRecord::trace_complete(trace_id)).unwrap();
        wal.flush().unwrap();

        // Read back
        let reader = wal.reader();
        let records = reader.read_all().unwrap();

        assert_eq!(records.len(), 4);
        assert_eq!(records[0].record_type, WalRecordType::TraceStart);
        assert_eq!(records[1].record_type, WalRecordType::NodeStart);
        assert_eq!(records[2].record_type, WalRecordType::NodeDone);
        assert_eq!(records[3].record_type, WalRecordType::TraceComplete);
    }

    #[test]
    fn wal_incomplete_trace_detection() {
        let dir = tempdir().unwrap();
        let config = WalConfig::default()
            .with_directory(dir.path())
            .with_sync(false);

        let wal = Wal::open(config).unwrap();

        let trace1 = TraceId::new();
        let trace2 = TraceId::new();
        let node_id = NodeId::new(1);

        // Trace 1: complete
        wal.write(&WalRecord::trace_start(trace1)).unwrap();
        wal.write(&WalRecord::node_done(
            trace1,
            node_id,
            ArenaOffset::NULL,
            0,
            0,
        ))
        .unwrap();
        wal.write(&WalRecord::trace_complete(trace1)).unwrap();

        // Trace 2: incomplete (crashed during node execution)
        wal.write(&WalRecord::trace_start(trace2)).unwrap();
        wal.write(&WalRecord::node_start(trace2, node_id)).unwrap();
        // No NodeDone or TraceComplete

        wal.flush().unwrap();

        let reader = wal.reader();
        let incomplete = reader.get_incomplete_traces().unwrap();

        assert!(!incomplete.contains_key(&trace1));
        assert!(incomplete.contains_key(&trace2));

        let state = incomplete.get(&trace2).unwrap();
        assert!(state.started_nodes.contains(&node_id));
    }

    // ========== Group Commit Tests ==========

    #[test]
    fn wal_group_commit_config() {
        let gc = GroupCommitConfig {
            max_delay_ms: 20,
            max_batch_size: 50,
        };

        let dir = tempdir().unwrap();
        let config = WalConfig::default()
            .with_directory(dir.path())
            .with_group_commit(gc);

        // with_group_commit should disable sync_on_write
        assert!(!config.sync_on_write);
        assert!(config.group_commit.is_some());

        let gc = config.group_commit.unwrap();
        assert_eq!(gc.max_delay_ms, 20);
        assert_eq!(gc.max_batch_size, 50);
    }

    #[test]
    fn wal_group_commit_stats() {
        let dir = tempdir().unwrap();
        let config = WalConfig::default()
            .with_directory(dir.path())
            .with_group_commit(GroupCommitConfig::default());

        let wal = Wal::open(config).unwrap();

        // Initially no pending writes
        let stats = wal.group_commit_stats().unwrap();
        assert_eq!(stats.pending_writes, 0);

        // Write some records (below batch threshold)
        let trace_id = TraceId::new();
        for _ in 0..5 {
            wal.write(&WalRecord::trace_start(trace_id)).unwrap();
        }

        // Should have pending writes
        let stats = wal.group_commit_stats().unwrap();
        assert!(stats.pending_writes > 0);
    }

    #[test]
    fn wal_group_commit_syncs_on_batch_size() {
        let dir = tempdir().unwrap();
        let config = WalConfig::default()
            .with_directory(dir.path())
            .with_group_commit(GroupCommitConfig {
                max_delay_ms: 60000, // Very long delay
                max_batch_size: 5,   // But small batch size
            });

        let wal = Wal::open(config).unwrap();

        let trace_id = TraceId::new();

        // Write 5 records (should trigger sync at 5)
        for _ in 0..5 {
            wal.write(&WalRecord::trace_start(trace_id)).unwrap();
        }

        // After reaching batch size, pending should be reset to 0
        let stats = wal.group_commit_stats().unwrap();
        assert_eq!(stats.pending_writes, 0);
    }

    #[test]
    fn wal_high_performance_config() {
        let dir = tempdir().unwrap();
        let config = WalConfig::high_performance(dir.path());

        // Should have group commit enabled
        assert!(!config.sync_on_write);
        assert!(config.group_commit.is_some());
    }

    #[test]
    fn wal_explicit_sync() {
        let dir = tempdir().unwrap();
        let config = WalConfig::default()
            .with_directory(dir.path())
            .with_group_commit(GroupCommitConfig {
                max_delay_ms: 60000,
                max_batch_size: 100,
            });

        let wal = Wal::open(config).unwrap();

        let trace_id = TraceId::new();
        wal.write(&WalRecord::trace_start(trace_id)).unwrap();

        // Should have pending writes
        let stats = wal.group_commit_stats().unwrap();
        assert_eq!(stats.pending_writes, 1);

        // Explicit sync
        wal.sync().unwrap();

        // Should reset pending
        let stats = wal.group_commit_stats().unwrap();
        assert_eq!(stats.pending_writes, 0);
    }

    #[test]
    fn wal_without_group_commit_returns_no_stats() {
        let dir = tempdir().unwrap();
        let config = WalConfig::default()
            .with_directory(dir.path())
            .with_sync(false);

        let wal = Wal::open(config).unwrap();

        // Should return None when group commit is not enabled
        assert!(wal.group_commit_stats().is_none());
    }
}
