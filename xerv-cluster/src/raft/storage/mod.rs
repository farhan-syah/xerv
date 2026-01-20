//! Raft log storage implementation with segmented log files.
//!
//! This implements OpenRaft's RaftLogStorage trait using segmented log files
//! for efficient append and purge operations:
//!
//! - **Append**: O(1) - writes to active segment file
//! - **Purge**: O(1) - deletes old segment files
//! - **Truncate**: O(segment_size) - rewrites current segment only
//!
//! ## Storage Layout
//!
//! ```text
//! raft_logs/
//! ├── meta.json           # Metadata (last_purged_log_id, committed)
//! ├── vote.json           # Vote state
//! └── segments/
//!     ├── seg_000000000000.log  # Entries 0-999 (closed)
//!     ├── seg_000000001000.log  # Entries 1000-1999 (closed)
//!     └── seg_000000002000.log  # Entries 2000+ (active)
//! ```
//!
//! Each segment file uses newline-delimited JSON (NDJSON) format for
//! human-readability and easy debugging.

mod entries;
mod inner;
mod persistence;
mod segment;

use crate::types::{ClusterEntry, ClusterLogId, ClusterNodeId, ClusterStorageError, TypeConfig};
use entries::EntryOps;
use inner::LogStorageInner;
use openraft::storage::{LogFlushed, RaftLogStorage};
use openraft::{LogState, RaftLogReader, StorageIOError, Vote};
use persistence::PersistenceOps;
use std::fmt::Debug;
use std::fs;
use std::ops::RangeBounds;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Persistent storage for Raft logs and vote state using segmented files.
///
/// This storage implementation provides:
/// - O(1) append operations by writing to the active segment
/// - O(1) purge operations by deleting old segment files
/// - Crash recovery through atomic file operations
/// - Automatic migration from old single-file format
#[derive(Clone)]
pub struct LogStorage {
    inner: Arc<RwLock<LogStorageInner>>,
}

impl LogStorage {
    /// Create or open log storage in the given directory.
    ///
    /// If the directory contains old-format `logs.json`, it will be
    /// automatically migrated to the segmented format.
    pub fn open(dir: PathBuf) -> Result<Self, std::io::Error> {
        fs::create_dir_all(&dir)?;

        let segments_dir = dir.join("segments");
        fs::create_dir_all(&segments_dir)?;

        let mut inner = LogStorageInner::new(dir, segments_dir);
        inner.initialize()?;

        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
        })
    }
}

/// Implementation of OpenRaft's log reader interface.
impl RaftLogReader<TypeConfig> for LogStorage {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug>(
        &mut self,
        range: RB,
    ) -> Result<Vec<ClusterEntry>, ClusterStorageError> {
        let inner = self.inner.read().await;
        let entries: Vec<ClusterEntry> = inner.logs.range(range).map(|(_, e)| e.clone()).collect();
        Ok(entries)
    }
}

/// Implementation of OpenRaft's log storage interface.
impl RaftLogStorage<TypeConfig> for LogStorage {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, ClusterStorageError> {
        let inner = self.inner.read().await;

        let last_log_id = inner.logs.values().last().map(|e| e.log_id);
        let last_purged_log_id = inner.last_purged_log_id;

        // If no logs but we have a purged log id, use that
        let last_log_id = last_log_id.or(last_purged_log_id);

        Ok(LogState {
            last_purged_log_id,
            last_log_id,
        })
    }

    async fn save_committed(
        &mut self,
        committed: Option<ClusterLogId>,
    ) -> Result<(), ClusterStorageError> {
        let mut inner = self.inner.write().await;
        inner.committed = committed;
        inner
            .save_meta()
            .map_err(|e| StorageIOError::write_logs(&e))?;
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<ClusterLogId>, ClusterStorageError> {
        let inner = self.inner.read().await;
        Ok(inner.committed)
    }

    async fn save_vote(&mut self, vote: &Vote<ClusterNodeId>) -> Result<(), ClusterStorageError> {
        let mut inner = self.inner.write().await;
        inner
            .save_vote_to_disk(vote)
            .map_err(|e| StorageIOError::write_vote(&e))?;
        inner.vote = Some(*vote);
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<ClusterNodeId>>, ClusterStorageError> {
        let inner = self.inner.read().await;
        Ok(inner.vote)
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<TypeConfig>,
    ) -> Result<(), ClusterStorageError>
    where
        I: IntoIterator<Item = ClusterEntry>,
    {
        let mut inner = self.inner.write().await;

        let entries: Vec<ClusterEntry> = entries.into_iter().collect();
        inner
            .append_entries(entries)
            .map_err(|e| StorageIOError::write_logs(&e))?;

        callback.log_io_completed(Ok(()));
        Ok(())
    }

    async fn truncate(&mut self, log_id: ClusterLogId) -> Result<(), ClusterStorageError> {
        let mut inner = self.inner.write().await;
        inner
            .truncate_entries(log_id)
            .map_err(|e| StorageIOError::write_logs(&e))?;
        Ok(())
    }

    async fn purge(&mut self, log_id: ClusterLogId) -> Result<(), ClusterStorageError> {
        let mut inner = self.inner.write().await;
        inner
            .purge_entries(log_id)
            .map_err(|e| StorageIOError::write_logs(&e))?;
        Ok(())
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openraft::CommittedLeaderId;
    use std::fs::File;
    use tempfile::TempDir;

    /// Helper to create a log ID for testing.
    fn test_log_id(term: u64, index: u64) -> ClusterLogId {
        ClusterLogId::new(CommittedLeaderId::new(term, 0), index)
    }

    /// Helper to create test entries.
    fn test_entries(term: u64, range: std::ops::RangeInclusive<u64>) -> Vec<ClusterEntry> {
        range
            .map(|i| ClusterEntry {
                log_id: test_log_id(term, i),
                payload: openraft::EntryPayload::Blank,
            })
            .collect()
    }

    /// Helper to append entries directly (bypasses callback).
    async fn append_test_entries(storage: &LogStorage, entries: Vec<ClusterEntry>) {
        let mut inner = storage.inner.write().await;
        inner
            .append_entries(entries)
            .expect("append should succeed");
    }

    #[tokio::test]
    async fn test_basic_operations() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let storage = LogStorage::open(temp_dir.path().to_path_buf()).expect("open storage");

        let entries = test_entries(1, 1..=10);
        append_test_entries(&storage, entries).await;

        let mut storage = storage;
        let read_entries = storage
            .try_get_log_entries(1..=10)
            .await
            .expect("read entries");
        assert_eq!(read_entries.len(), 10);
    }

    #[tokio::test]
    async fn test_segment_rotation() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let storage = LogStorage::open(temp_dir.path().to_path_buf()).expect("open storage");

        let entries = test_entries(1, 1..=1500);
        append_test_entries(&storage, entries).await;

        let mut storage = storage;
        let read_entries = storage
            .try_get_log_entries(1..=1500)
            .await
            .expect("read entries");
        assert_eq!(read_entries.len(), 1500);

        let segments_dir = temp_dir.path().join("segments");
        let segment_count = fs::read_dir(&segments_dir)
            .expect("read segments dir")
            .count();
        assert!(
            segment_count >= 2,
            "Expected at least 2 segments, got {}",
            segment_count
        );
    }

    #[tokio::test]
    async fn test_purge() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let storage = LogStorage::open(temp_dir.path().to_path_buf()).expect("open storage");

        let entries = test_entries(1, 1..=2500);
        append_test_entries(&storage, entries).await;

        let mut storage = storage;
        storage.purge(test_log_id(1, 1500)).await.expect("purge");

        let read_entries = storage
            .try_get_log_entries(1..=1500)
            .await
            .expect("read entries");
        assert!(read_entries.is_empty(), "Purged entries should be gone");

        let read_entries = storage
            .try_get_log_entries(1501..=2500)
            .await
            .expect("read entries");
        assert_eq!(read_entries.len(), 1000);
    }

    #[tokio::test]
    async fn test_truncate() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let storage = LogStorage::open(temp_dir.path().to_path_buf()).expect("open storage");

        let entries = test_entries(1, 1..=100);
        append_test_entries(&storage, entries).await;

        let mut storage = storage;
        storage
            .truncate(test_log_id(1, 50))
            .await
            .expect("truncate");

        let read_entries = storage
            .try_get_log_entries(1..50)
            .await
            .expect("read entries");
        assert_eq!(read_entries.len(), 49);

        let read_entries = storage
            .try_get_log_entries(50..=100)
            .await
            .expect("read entries");
        assert!(read_entries.is_empty());
    }

    #[tokio::test]
    async fn test_persistence() {
        let temp_dir = TempDir::new().expect("create temp dir");

        {
            let storage = LogStorage::open(temp_dir.path().to_path_buf()).expect("open storage");
            let entries = test_entries(1, 1..=100);
            append_test_entries(&storage, entries).await;
        }

        {
            let mut storage =
                LogStorage::open(temp_dir.path().to_path_buf()).expect("reopen storage");
            let read_entries = storage
                .try_get_log_entries(1..=100)
                .await
                .expect("read entries");
            assert_eq!(read_entries.len(), 100);
        }
    }

    #[tokio::test]
    async fn test_vote_persistence() {
        let temp_dir = TempDir::new().expect("create temp dir");

        {
            let mut storage =
                LogStorage::open(temp_dir.path().to_path_buf()).expect("open storage");
            let vote = Vote::new(5, 3);
            storage.save_vote(&vote).await.expect("save vote");
        }

        {
            let mut storage =
                LogStorage::open(temp_dir.path().to_path_buf()).expect("reopen storage");
            let vote = storage.read_vote().await.expect("read vote");
            assert!(vote.is_some());
            let vote = vote.expect("vote should exist");
            assert_eq!(vote.leader_id().term, 5);
            assert_eq!(vote.leader_id().node_id, 3);
        }
    }

    #[tokio::test]
    async fn test_migration_from_old_format() {
        let temp_dir = TempDir::new().expect("create temp dir");

        // Create old-format logs.json file
        let entries = test_entries(1, 1..=50);
        let old_logs_path = temp_dir.path().join("logs.json");
        let file = File::create(&old_logs_path).expect("create old logs file");
        serde_json::to_writer(file, &entries).expect("write old logs");

        // Open storage (should migrate)
        let mut storage = LogStorage::open(temp_dir.path().to_path_buf()).expect("open storage");

        // Verify old file is gone
        assert!(
            !old_logs_path.exists(),
            "Old logs.json should be deleted after migration"
        );

        // Verify entries are accessible
        let read_entries = storage
            .try_get_log_entries(1..=50)
            .await
            .expect("read entries");
        assert_eq!(read_entries.len(), 50);

        // Verify segments were created
        let segments_dir = temp_dir.path().join("segments");
        assert!(segments_dir.exists(), "Segments directory should exist");
    }
}
