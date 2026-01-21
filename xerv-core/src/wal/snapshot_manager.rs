//! Snapshot manager for WAL snapshots.
//!
//! Handles creating, loading, and cleaning up WAL snapshots.

use super::snapshot::{
    SnapshotConfig, SnapshotNodeOutput, SnapshotTraceState, WalPosition, WalSnapshot,
};
use super::writer::{NodeOutputLocation, TraceRecoveryState};
use crate::error::{Result, XervError};
use crate::types::TraceId;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Snapshot file format version.
const SNAPSHOT_VERSION: u32 = 1;
/// Magic bytes for snapshot files.
const SNAPSHOT_MAGIC: &[u8; 4] = b"XSNP";

/// Manager for WAL snapshots.
pub struct SnapshotManager {
    /// Directory for snapshot files.
    directory: PathBuf,
    /// Snapshot configuration.
    config: SnapshotConfig,
    /// Current snapshot ID counter.
    next_id: AtomicU64,
    /// Records processed since last snapshot.
    records_since_snapshot: AtomicU64,
    /// Time of last snapshot.
    last_snapshot_time: parking_lot::Mutex<Instant>,
}

impl SnapshotManager {
    /// Create a new snapshot manager.
    pub fn new(wal_directory: impl Into<PathBuf>, config: SnapshotConfig) -> Result<Self> {
        let directory = wal_directory.into().join("snapshots");

        // Ensure directory exists
        fs::create_dir_all(&directory).map_err(|e| XervError::SnapshotCreation {
            cause: format!("Failed to create snapshot directory: {}", e),
        })?;

        // Find the next snapshot ID
        let next_id = Self::find_next_id(&directory)?;

        Ok(Self {
            directory,
            config,
            next_id: AtomicU64::new(next_id),
            records_since_snapshot: AtomicU64::new(0),
            last_snapshot_time: parking_lot::Mutex::new(Instant::now()),
        })
    }

    /// Find the next snapshot ID by examining existing files.
    fn find_next_id(directory: &Path) -> Result<u64> {
        let mut max_id = 0u64;

        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                if name_str.starts_with("snapshot_") && name_str.ends_with(".snap") {
                    if let Some(id_str) = name_str
                        .strip_prefix("snapshot_")
                        .and_then(|s| s.strip_suffix(".snap"))
                    {
                        if let Ok(id) = u64::from_str_radix(id_str, 16) {
                            max_id = max_id.max(id);
                        }
                    }
                }
            }
        }

        Ok(max_id + 1)
    }

    /// Check if a snapshot should be created.
    pub fn should_snapshot(&self) -> bool {
        let records = self.records_since_snapshot.load(Ordering::Relaxed);
        if records >= self.config.record_interval {
            return true;
        }

        let elapsed = self.last_snapshot_time.lock().elapsed();
        elapsed >= self.config.time_interval
    }

    /// Notify that a record was written.
    pub fn record_written(&self) {
        self.records_since_snapshot.fetch_add(1, Ordering::Relaxed);
    }

    /// Reset the snapshot counters after creating a snapshot.
    fn reset_counters(&self) {
        self.records_since_snapshot.store(0, Ordering::Relaxed);
        *self.last_snapshot_time.lock() = Instant::now();
    }

    /// Create a snapshot from the current trace states.
    pub fn create_snapshot(
        &self,
        wal_position: WalPosition,
        trace_states: &HashMap<TraceId, TraceRecoveryState>,
        record_count: u64,
    ) -> Result<WalSnapshot> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        // Convert TraceRecoveryState to SnapshotTraceState
        let snapshot_states: HashMap<TraceId, SnapshotTraceState> = trace_states
            .iter()
            .map(|(tid, state)| {
                let completed_nodes: HashMap<_, _> = state
                    .completed_nodes
                    .iter()
                    .map(|(nid, loc)| {
                        (
                            *nid,
                            SnapshotNodeOutput {
                                offset: loc.offset,
                                size: loc.size,
                                schema_hash: loc.schema_hash,
                            },
                        )
                    })
                    .collect();

                let snap_state = SnapshotTraceState {
                    trace_id: *tid,
                    last_completed_node: state.last_completed_node,
                    suspended_at: state.suspended_at,
                    started_nodes: state.started_nodes.clone(),
                    completed_nodes,
                };

                (*tid, snap_state)
            })
            .collect();

        let snapshot = WalSnapshot::new(id, wal_position, snapshot_states, record_count);

        // Write to disk
        self.write_snapshot(&snapshot)?;

        // Cleanup old snapshots
        self.cleanup_old_snapshots()?;

        // Reset counters
        self.reset_counters();

        tracing::info!(
            snapshot_id = id,
            position = ?wal_position,
            traces = snapshot.trace_count(),
            records = record_count,
            "Created WAL snapshot"
        );

        Ok(snapshot)
    }

    /// Write a snapshot to disk.
    fn write_snapshot(&self, snapshot: &WalSnapshot) -> Result<()> {
        let path = self.snapshot_path(snapshot.id);
        let temp_path = path.with_extension("snap.tmp");

        let file = File::create(&temp_path).map_err(|e| XervError::SnapshotCreation {
            cause: format!("Failed to create snapshot file: {}", e),
        })?;

        let mut writer = BufWriter::new(file);

        // Write header
        writer
            .write_all(SNAPSHOT_MAGIC)
            .map_err(|e| XervError::SnapshotCreation {
                cause: format!("Failed to write magic: {}", e),
            })?;
        writer
            .write_all(&SNAPSHOT_VERSION.to_le_bytes())
            .map_err(|e| XervError::SnapshotCreation {
                cause: format!("Failed to write version: {}", e),
            })?;

        // Serialize snapshot data
        let data = serde_json::to_vec(snapshot).map_err(|e| XervError::SnapshotCreation {
            cause: format!("Failed to serialize snapshot: {}", e),
        })?;

        // Write length and data
        let len = data.len() as u64;
        writer
            .write_all(&len.to_le_bytes())
            .map_err(|e| XervError::SnapshotCreation {
                cause: format!("Failed to write length: {}", e),
            })?;
        writer
            .write_all(&data)
            .map_err(|e| XervError::SnapshotCreation {
                cause: format!("Failed to write data: {}", e),
            })?;

        writer.flush().map_err(|e| XervError::SnapshotCreation {
            cause: format!("Failed to flush snapshot: {}", e),
        })?;

        // Sync to disk
        writer
            .get_ref()
            .sync_all()
            .map_err(|e| XervError::SnapshotCreation {
                cause: format!("Failed to sync snapshot: {}", e),
            })?;

        // Atomic rename
        fs::rename(&temp_path, &path).map_err(|e| XervError::SnapshotCreation {
            cause: format!("Failed to rename snapshot: {}", e),
        })?;

        Ok(())
    }

    /// Load the latest snapshot.
    pub fn load_latest(&self) -> Result<Option<WalSnapshot>> {
        let mut snapshots: Vec<u64> = Vec::new();

        if let Ok(entries) = fs::read_dir(&self.directory) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                if name_str.starts_with("snapshot_") && name_str.ends_with(".snap") {
                    if let Some(id_str) = name_str
                        .strip_prefix("snapshot_")
                        .and_then(|s| s.strip_suffix(".snap"))
                    {
                        if let Ok(id) = u64::from_str_radix(id_str, 16) {
                            snapshots.push(id);
                        }
                    }
                }
            }
        }

        if snapshots.is_empty() {
            return Ok(None);
        }

        // Load the most recent valid snapshot
        snapshots.sort_by(|a, b| b.cmp(a)); // Descending order

        for id in snapshots {
            match self.load_snapshot(id) {
                Ok(snapshot) => return Ok(Some(snapshot)),
                Err(e) => {
                    tracing::warn!(snapshot_id = id, error = %e, "Failed to load snapshot, trying older one");
                }
            }
        }

        Ok(None)
    }

    /// Load a specific snapshot.
    fn load_snapshot(&self, id: u64) -> Result<WalSnapshot> {
        let path = self.snapshot_path(id);

        let file = File::open(&path).map_err(|e| XervError::SnapshotLoad {
            cause: format!("Failed to open snapshot {}: {}", id, e),
        })?;

        let mut reader = BufReader::new(file);

        // Read and verify header
        let mut magic = [0u8; 4];
        reader
            .read_exact(&mut magic)
            .map_err(|e| XervError::SnapshotLoad {
                cause: format!("Failed to read magic: {}", e),
            })?;

        if &magic != SNAPSHOT_MAGIC {
            return Err(XervError::SnapshotLoad {
                cause: "Invalid snapshot magic".to_string(),
            });
        }

        let mut version_bytes = [0u8; 4];
        reader
            .read_exact(&mut version_bytes)
            .map_err(|e| XervError::SnapshotLoad {
                cause: format!("Failed to read version: {}", e),
            })?;

        let version = u32::from_le_bytes(version_bytes);
        if version != SNAPSHOT_VERSION {
            return Err(XervError::SnapshotLoad {
                cause: format!("Unsupported snapshot version: {}", version),
            });
        }

        // Read length
        let mut len_bytes = [0u8; 8];
        reader
            .read_exact(&mut len_bytes)
            .map_err(|e| XervError::SnapshotLoad {
                cause: format!("Failed to read length: {}", e),
            })?;

        let len = u64::from_le_bytes(len_bytes) as usize;

        // Read data
        let mut data = vec![0u8; len];
        reader
            .read_exact(&mut data)
            .map_err(|e| XervError::SnapshotLoad {
                cause: format!("Failed to read data: {}", e),
            })?;

        // Deserialize
        let snapshot: WalSnapshot =
            serde_json::from_slice(&data).map_err(|e| XervError::SnapshotLoad {
                cause: format!("Failed to deserialize snapshot: {}", e),
            })?;

        Ok(snapshot)
    }

    /// Cleanup old snapshots beyond retention count.
    fn cleanup_old_snapshots(&self) -> Result<()> {
        let mut snapshots: Vec<u64> = Vec::new();

        if let Ok(entries) = fs::read_dir(&self.directory) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                if name_str.starts_with("snapshot_") && name_str.ends_with(".snap") {
                    if let Some(id_str) = name_str
                        .strip_prefix("snapshot_")
                        .and_then(|s| s.strip_suffix(".snap"))
                    {
                        if let Ok(id) = u64::from_str_radix(id_str, 16) {
                            snapshots.push(id);
                        }
                    }
                }
            }
        }

        if snapshots.len() <= self.config.retention_count {
            return Ok(());
        }

        // Sort ascending and remove oldest
        snapshots.sort();
        let to_remove = snapshots.len() - self.config.retention_count;

        for id in snapshots.iter().take(to_remove) {
            let path = self.snapshot_path(*id);
            if let Err(e) = fs::remove_file(&path) {
                tracing::warn!(snapshot_id = id, error = %e, "Failed to remove old snapshot");
            } else {
                tracing::debug!(snapshot_id = id, "Removed old snapshot");
            }
        }

        Ok(())
    }

    /// Get the path for a snapshot file.
    fn snapshot_path(&self, id: u64) -> PathBuf {
        self.directory.join(format!("snapshot_{:016x}.snap", id))
    }

    /// Convert a snapshot to trace recovery states.
    pub fn snapshot_to_recovery_states(
        snapshot: &WalSnapshot,
    ) -> HashMap<TraceId, TraceRecoveryState> {
        snapshot
            .trace_states
            .iter()
            .map(|(tid, state)| {
                let completed_nodes: HashMap<_, _> = state
                    .completed_nodes
                    .iter()
                    .map(|(nid, out)| {
                        (
                            *nid,
                            NodeOutputLocation {
                                offset: out.offset,
                                size: out.size,
                                schema_hash: out.schema_hash,
                            },
                        )
                    })
                    .collect();

                let recovery_state = TraceRecoveryState {
                    trace_id: *tid,
                    last_completed_node: state.last_completed_node,
                    suspended_at: state.suspended_at,
                    started_nodes: state.started_nodes.clone(),
                    completed_nodes,
                    suspension_metadata: None,
                };

                (*tid, recovery_state)
            })
            .collect()
    }

    /// Get the directory for this manager.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Get the config.
    pub fn config(&self) -> &SnapshotConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ArenaOffset, NodeId};
    use tempfile::tempdir;

    #[test]
    fn snapshot_manager_create_and_load() {
        let dir = tempdir().unwrap();
        let manager = SnapshotManager::new(dir.path(), SnapshotConfig::default()).unwrap();

        // Create some trace states
        let mut trace_states = HashMap::new();
        let trace_id = TraceId::new();
        let node_id = NodeId::new(1);

        let mut state = TraceRecoveryState {
            trace_id,
            last_completed_node: Some(node_id),
            suspended_at: None,
            started_nodes: vec![],
            completed_nodes: HashMap::new(),
            suspension_metadata: None,
        };
        state.completed_nodes.insert(
            node_id,
            NodeOutputLocation {
                offset: ArenaOffset::new(0x100),
                size: 64,
                schema_hash: 12345,
            },
        );
        trace_states.insert(trace_id, state);

        // Create snapshot
        let position = WalPosition::new(1, 1024);
        let snapshot = manager
            .create_snapshot(position, &trace_states, 500)
            .unwrap();

        assert_eq!(snapshot.wal_position, position);
        assert_eq!(snapshot.trace_count(), 1);
        assert_eq!(snapshot.record_count, 500);

        // Load it back
        let loaded = manager
            .load_latest()
            .unwrap()
            .expect("snapshot should exist");

        assert_eq!(loaded.id, snapshot.id);
        assert_eq!(loaded.wal_position, position);
        assert_eq!(loaded.trace_count(), 1);
    }

    #[test]
    fn snapshot_retention() {
        let dir = tempdir().unwrap();
        let config = SnapshotConfig::default().with_retention_count(2);
        let manager = SnapshotManager::new(dir.path(), config).unwrap();

        let trace_states = HashMap::new();

        // Create 4 snapshots
        for i in 0..4 {
            let position = WalPosition::new(0, i * 1000);
            manager
                .create_snapshot(position, &trace_states, i * 100)
                .unwrap();
        }

        // Count remaining snapshots
        let mut count = 0;
        for entry in fs::read_dir(manager.directory()).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name().to_string_lossy().ends_with(".snap") {
                count += 1;
            }
        }

        assert_eq!(count, 2);
    }

    #[test]
    fn should_snapshot_by_records() {
        let dir = tempdir().unwrap();
        let config = SnapshotConfig::default().with_record_interval(10);
        let manager = SnapshotManager::new(dir.path(), config).unwrap();

        assert!(!manager.should_snapshot());

        for _ in 0..10 {
            manager.record_written();
        }

        assert!(manager.should_snapshot());
    }
}
