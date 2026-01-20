//! WAL writer implementation.

use super::config::{GroupCommitConfig, WalConfig};
use super::group_commit::{GroupCommitState, GroupCommitStats};
use super::reader::WalReader;
use crate::error::{Result, XervError};
use crate::types::TraceId;
use crate::wal::record::WalRecord;
use crate::wal::snapshot::WalPosition;
use fs2::FileExt;
use parking_lot::Mutex;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Internal state of the WAL writer.
struct WalInner {
    /// Current WAL file.
    file: BufWriter<File>,
    /// Path to current file.
    path: PathBuf,
    /// Current file size.
    file_size: u64,
    /// Configuration.
    config: WalConfig,
    /// File sequence number.
    sequence: u64,
}

/// Shared state for the WAL (accessed without locking inner).
struct WalShared {
    /// Total records written across all files.
    record_count: AtomicU64,
}

/// Write-Ahead Log for durability.
pub struct Wal {
    inner: Arc<Mutex<WalInner>>,
    shared: Arc<WalShared>,
    group_commit: Option<(GroupCommitConfig, Arc<GroupCommitState>)>,
}

impl Wal {
    /// Create or open a WAL.
    pub fn open(config: WalConfig) -> Result<Self> {
        // Ensure directory exists
        std::fs::create_dir_all(&config.directory).map_err(|e| XervError::WalWrite {
            trace_id: TraceId::new(),
            cause: format!("Failed to create WAL directory: {}", e),
        })?;

        // Find or create WAL file
        let (path, sequence) = find_or_create_wal_file(&config.directory)?;

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| XervError::WalWrite {
                trace_id: TraceId::new(),
                cause: format!("Failed to open WAL file: {}", e),
            })?;

        // Lock the file
        file.try_lock_exclusive().map_err(|e| XervError::WalWrite {
            trace_id: TraceId::new(),
            cause: format!("Failed to lock WAL file: {}", e),
        })?;

        let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);

        // Extract group commit config before moving config into inner
        let group_commit = config
            .group_commit
            .map(|gc| (gc, Arc::new(GroupCommitState::new())));

        let inner = WalInner {
            file: BufWriter::with_capacity(config.buffer_size, file),
            path,
            file_size,
            config,
            sequence,
        };

        let shared = WalShared {
            record_count: AtomicU64::new(0),
        };

        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
            shared: Arc::new(shared),
            group_commit,
        })
    }

    /// Write a record to the WAL.
    ///
    /// Depending on configuration:
    /// - `sync_on_write: true`: Syncs immediately after write (safest, slowest)
    /// - `group_commit: Some(...)`: Batches syncs for higher throughput
    /// - Neither: Buffers writes, sync only on explicit flush() or rotation
    pub fn write(&self, record: &WalRecord) -> Result<()> {
        let mut inner = self.inner.lock();

        let bytes = record.to_bytes().map_err(|e| XervError::WalWrite {
            trace_id: record.trace_id,
            cause: e.to_string(),
        })?;

        // Check if we need to rotate
        if inner.file_size + bytes.len() as u64 > inner.config.max_file_size {
            self.rotate_locked(&mut inner)?;
        }

        inner
            .file
            .write_all(&bytes)
            .map_err(|e| XervError::WalWrite {
                trace_id: record.trace_id,
                cause: e.to_string(),
            })?;

        inner.file_size += bytes.len() as u64;

        // Determine sync strategy
        if inner.config.sync_on_write {
            // Traditional sync-on-every-write mode
            Self::sync_file(&mut inner.file, record.trace_id)?;
        } else if let Some((ref gc_config, ref gc_state)) = self.group_commit {
            // Group commit mode: batch syncs for better performance
            gc_state.record_write();

            if gc_state.should_sync(gc_config) {
                Self::sync_file(&mut inner.file, record.trace_id)?;
                gc_state.reset_after_sync();
            }
        }
        // else: no sync, writes are buffered until explicit flush() or rotation

        // Increment record count
        self.shared.record_count.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Internal helper to sync file.
    fn sync_file(file: &mut BufWriter<File>, trace_id: TraceId) -> Result<()> {
        file.flush().map_err(|e| XervError::WalWrite {
            trace_id,
            cause: e.to_string(),
        })?;
        file.get_ref().sync_data().map_err(|e| XervError::WalWrite {
            trace_id,
            cause: e.to_string(),
        })
    }

    /// Force a sync if using group commit mode.
    ///
    /// This is useful when you need to ensure durability before proceeding,
    /// even if the group commit thresholds haven't been reached.
    ///
    /// When not using group commit, this is equivalent to flush().
    pub fn sync(&self) -> Result<()> {
        let mut inner = self.inner.lock();
        Self::sync_file(&mut inner.file, TraceId::new())?;

        // Reset group commit state if applicable
        if let Some((_, ref gc_state)) = self.group_commit {
            gc_state.reset_after_sync();
        }

        Ok(())
    }

    /// Get group commit statistics for monitoring.
    ///
    /// Returns None if group commit is not enabled.
    pub fn group_commit_stats(&self) -> Option<GroupCommitStats> {
        self.group_commit
            .as_ref()
            .map(|(config, state)| GroupCommitStats {
                pending_writes: state.pending_writes(),
                last_sync_time_ms: state.last_sync_time(),
                config_max_delay_ms: config.max_delay_ms,
                config_max_batch_size: config.max_batch_size,
            })
    }

    /// Flush pending writes.
    pub fn flush(&self) -> Result<()> {
        let mut inner = self.inner.lock();
        inner.file.flush().map_err(|e| XervError::WalWrite {
            trace_id: TraceId::new(),
            cause: e.to_string(),
        })?;
        inner
            .file
            .get_ref()
            .sync_data()
            .map_err(|e| XervError::WalWrite {
                trace_id: TraceId::new(),
                cause: e.to_string(),
            })
    }

    /// Rotate to a new WAL file.
    fn rotate_locked(&self, inner: &mut WalInner) -> Result<()> {
        // Flush current file
        inner.file.flush().map_err(|e| XervError::WalWrite {
            trace_id: TraceId::new(),
            cause: e.to_string(),
        })?;

        // Create new file
        inner.sequence += 1;
        let new_path = inner
            .config
            .directory
            .join(format!("wal_{:016x}.log", inner.sequence));

        let new_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&new_path)
            .map_err(|e| XervError::WalWrite {
                trace_id: TraceId::new(),
                cause: format!("Failed to create new WAL file: {}", e),
            })?;

        new_file
            .try_lock_exclusive()
            .map_err(|e| XervError::WalWrite {
                trace_id: TraceId::new(),
                cause: format!("Failed to lock new WAL file: {}", e),
            })?;

        // Unlock old file
        let _ = fs2::FileExt::unlock(inner.file.get_ref());

        inner.file = BufWriter::with_capacity(inner.config.buffer_size, new_file);
        inner.path = new_path;
        inner.file_size = 0;

        Ok(())
    }

    /// Get the current WAL file path.
    pub fn path(&self) -> PathBuf {
        self.inner.lock().path.clone()
    }

    /// Create a reader for recovery.
    pub fn reader(&self) -> WalReader {
        let inner = self.inner.lock();
        WalReader::new(inner.config.directory.clone())
    }

    /// Get the current WAL position.
    pub fn get_current_position(&self) -> WalPosition {
        let inner = self.inner.lock();
        WalPosition {
            file_sequence: inner.sequence,
            offset: inner.file_size,
        }
    }

    /// Get the total number of records written.
    pub fn get_record_count(&self) -> u64 {
        self.shared.record_count.load(Ordering::Relaxed)
    }

    /// Get the WAL directory.
    pub fn directory(&self) -> PathBuf {
        self.inner.lock().config.directory.clone()
    }

    /// Purge old WAL segments, keeping only the most recent ones.
    ///
    /// # Arguments
    /// * `keep_count` - Number of recent segments to keep (including current)
    ///
    /// # Returns
    /// Number of segments deleted
    pub fn purge_old_segments(&self, keep_count: usize) -> Result<usize> {
        let inner = self.inner.lock();
        let current_sequence = inner.sequence;
        let directory = inner.config.directory.clone();
        drop(inner);

        let mut deleted = 0;

        if let Ok(entries) = std::fs::read_dir(&directory) {
            let mut segments: Vec<(u64, PathBuf)> = Vec::new();

            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                if name_str.starts_with("wal_") && name_str.ends_with(".log") {
                    if let Some(seq_str) = name_str
                        .strip_prefix("wal_")
                        .and_then(|s| s.strip_suffix(".log"))
                    {
                        if let Ok(seq) = u64::from_str_radix(seq_str, 16) {
                            segments.push((seq, path));
                        }
                    }
                }
            }

            // Sort by sequence (oldest first)
            segments.sort_by_key(|(seq, _)| *seq);

            // Delete all but the last keep_count segments (never delete current)
            let to_delete = segments.len().saturating_sub(keep_count);
            for (seq, path) in segments.into_iter().take(to_delete) {
                // Never delete the current segment
                if seq == current_sequence {
                    continue;
                }
                if std::fs::remove_file(&path).is_ok() {
                    deleted += 1;
                }
            }
        }

        Ok(deleted)
    }
}

impl Drop for Wal {
    fn drop(&mut self) {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            let inner = inner.get_mut();
            let _ = inner.file.flush();
            let _ = fs2::FileExt::unlock(inner.file.get_ref());
        }
    }
}

/// Find the latest WAL file or create a new one.
fn find_or_create_wal_file(directory: &Path) -> Result<(PathBuf, u64)> {
    let mut max_sequence = 0u64;

    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if name_str.starts_with("wal_") && name_str.ends_with(".log") {
                if let Some(seq_str) = name_str
                    .strip_prefix("wal_")
                    .and_then(|s| s.strip_suffix(".log"))
                {
                    if let Ok(seq) = u64::from_str_radix(seq_str, 16) {
                        max_sequence = max_sequence.max(seq);
                    }
                }
            }
        }
    }

    // Use the latest file if it exists and is not too large, otherwise create new
    let path = directory.join(format!("wal_{:016x}.log", max_sequence));

    if path.exists() {
        if let Ok(meta) = std::fs::metadata(&path) {
            // If file is already large, create a new one
            if meta.len() > 32 * 1024 * 1024 {
                let new_seq = max_sequence + 1;
                let new_path = directory.join(format!("wal_{:016x}.log", new_seq));
                return Ok((new_path, new_seq));
            }
        }
    }

    // If max_sequence is 0, this creates wal_0000000000000000.log
    Ok((path, max_sequence))
}
