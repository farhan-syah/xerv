//! WAL writer and reader implementation.

use super::record::{MIN_RECORD_SIZE, WalRecord, WalRecordType};
use super::snapshot::WalPosition;
use crate::error::{Result, XervError};
use crate::types::{NodeId, TraceId};
use byteorder::{LittleEndian, ReadBytesExt};
use fs2::FileExt;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Configuration for WAL creation.
#[derive(Debug, Clone)]
pub struct WalConfig {
    /// Directory for WAL files.
    pub directory: PathBuf,
    /// Maximum WAL file size before rotation.
    pub max_file_size: u64,
    /// Whether to sync after each write.
    pub sync_on_write: bool,
    /// Buffer size for writes.
    pub buffer_size: usize,
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("/tmp/xerv/wal"),
            max_file_size: 64 * 1024 * 1024, // 64 MB
            sync_on_write: true,
            buffer_size: 64 * 1024, // 64 KB
        }
    }
}

impl WalConfig {
    /// Create an in-memory WAL configuration (uses a temp directory).
    pub fn in_memory() -> Self {
        Self {
            directory: std::env::temp_dir().join(format!("xerv_wal_{}", uuid::Uuid::new_v4())),
            max_file_size: 64 * 1024 * 1024,
            sync_on_write: false,
            buffer_size: 64 * 1024,
        }
    }

    /// Set the WAL directory.
    pub fn with_directory(mut self, dir: impl Into<PathBuf>) -> Self {
        self.directory = dir.into();
        self
    }

    /// Set sync on write.
    pub fn with_sync(mut self, sync: bool) -> Self {
        self.sync_on_write = sync;
        self
    }
}

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
        })
    }

    /// Write a record to the WAL.
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

        if inner.config.sync_on_write {
            inner.file.flush().map_err(|e| XervError::WalWrite {
                trace_id: record.trace_id,
                cause: e.to_string(),
            })?;
            inner
                .file
                .get_ref()
                .sync_data()
                .map_err(|e| XervError::WalWrite {
                    trace_id: record.trace_id,
                    cause: e.to_string(),
                })?;
        }

        // Increment record count
        self.shared.record_count.fetch_add(1, Ordering::Relaxed);

        Ok(())
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
        WalReader {
            directory: inner.config.directory.clone(),
        }
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

/// WAL reader for recovery.
pub struct WalReader {
    directory: PathBuf,
}

impl WalReader {
    /// Create a new WAL reader.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    /// Read all records from all WAL files.
    pub fn read_all(&self) -> Result<Vec<WalRecord>> {
        self.read_from(WalPosition::default())
    }

    /// Read records starting from a specific WAL position.
    ///
    /// This is used for snapshot-based recovery to skip records
    /// that were already captured in the snapshot.
    pub fn read_from(&self, start: WalPosition) -> Result<Vec<WalRecord>> {
        let mut records = Vec::new();
        let mut files: Vec<(u64, PathBuf)> = Vec::new();

        // Collect all WAL files with their sequence numbers
        if let Ok(entries) = std::fs::read_dir(&self.directory) {
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
                            files.push((seq, path));
                        }
                    }
                }
            }
        }

        // Sort by sequence number
        files.sort_by_key(|(seq, _)| *seq);

        // Read files, skipping those before start position
        for (seq, path) in files {
            if seq < start.file_sequence {
                continue;
            }

            let start_offset = if seq == start.file_sequence {
                start.offset
            } else {
                0
            };

            records.extend(self.read_file_from(&path, start_offset)?);
        }

        Ok(records)
    }

    /// Read records from a single WAL file starting from a specific offset.
    fn read_file_from(&self, path: &Path, start_offset: u64) -> Result<Vec<WalRecord>> {
        let file = File::open(path).map_err(|e| XervError::WalRead {
            cause: format!("Failed to open {}: {}", path.display(), e),
        })?;

        let mut reader = BufReader::new(file);
        let mut records = Vec::new();

        // Seek to start offset if specified
        if start_offset > 0 {
            reader
                .seek(SeekFrom::Start(start_offset))
                .map_err(|e| XervError::WalRead {
                    cause: format!("Failed to seek to offset {}: {}", start_offset, e),
                })?;
        }

        loop {
            // Try to read record length
            let length = match reader.read_u32::<LittleEndian>() {
                Ok(len) => len as usize,
                Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => {
                    return Err(XervError::WalRead {
                        cause: format!("Failed to read record length: {}", e),
                    });
                }
            };

            if length < MIN_RECORD_SIZE {
                return Err(XervError::WalCorruption {
                    position: reader.stream_position().unwrap_or(0),
                    cause: format!("Invalid record length: {}", length),
                });
            }

            // Seek back to include length in the record
            reader
                .seek(SeekFrom::Current(-4))
                .map_err(|e| XervError::WalRead {
                    cause: format!("Seek failed: {}", e),
                })?;

            // Read full record
            let mut buf = vec![0u8; length];
            reader
                .read_exact(&mut buf)
                .map_err(|e| XervError::WalRead {
                    cause: format!("Failed to read record: {}", e),
                })?;

            match WalRecord::from_bytes(&buf) {
                Ok(record) => records.push(record),
                Err(e) => {
                    // Log corruption but continue reading
                    tracing::warn!("Corrupted WAL record at {}: {}", path.display(), e);
                }
            }
        }

        Ok(records)
    }

    /// Get the state of in-flight traces from WAL records.
    ///
    /// Returns traces that started but did not complete or fail.
    pub fn get_incomplete_traces(&self) -> Result<HashMap<TraceId, TraceRecoveryState>> {
        self.get_incomplete_traces_from(WalPosition::default(), HashMap::new())
    }

    /// Get incomplete traces, starting from a snapshot.
    ///
    /// This method allows recovery to skip records that were already
    /// captured in the snapshot, significantly speeding up recovery
    /// for WALs with many records.
    pub fn get_incomplete_traces_from(
        &self,
        start_position: WalPosition,
        initial_traces: HashMap<TraceId, TraceRecoveryState>,
    ) -> Result<HashMap<TraceId, TraceRecoveryState>> {
        let records = self.read_from(start_position)?;
        let mut traces = initial_traces;

        for record in records {
            Self::apply_record_to_traces(&mut traces, &record);
        }

        Ok(traces)
    }

    /// Apply a single WAL record to the trace state map.
    fn apply_record_to_traces(
        traces: &mut HashMap<TraceId, TraceRecoveryState>,
        record: &WalRecord,
    ) {
        match record.record_type {
            WalRecordType::TraceStart => {
                traces.insert(
                    record.trace_id,
                    TraceRecoveryState {
                        trace_id: record.trace_id,
                        last_completed_node: None,
                        suspended_at: None,
                        started_nodes: Vec::new(),
                        completed_nodes: HashMap::new(),
                    },
                );
            }
            WalRecordType::NodeStart => {
                if let Some(state) = traces.get_mut(&record.trace_id) {
                    state.started_nodes.push(record.node_id);
                }
            }
            WalRecordType::NodeDone => {
                if let Some(state) = traces.get_mut(&record.trace_id) {
                    state.last_completed_node = Some(record.node_id);
                    state.started_nodes.retain(|&n| n != record.node_id);
                    // Store the output location for recovery
                    state.completed_nodes.insert(
                        record.node_id,
                        NodeOutputLocation {
                            offset: record.output_offset,
                            size: record.output_size,
                            schema_hash: record.schema_hash,
                        },
                    );
                }
            }
            WalRecordType::TraceComplete | WalRecordType::TraceFailed => {
                traces.remove(&record.trace_id);
            }
            WalRecordType::TraceSuspended => {
                if let Some(state) = traces.get_mut(&record.trace_id) {
                    state.suspended_at = Some(record.node_id);
                }
            }
            WalRecordType::TraceResumed => {
                if let Some(state) = traces.get_mut(&record.trace_id) {
                    state.suspended_at = None;
                }
            }
            _ => {}
        }
    }
}

/// State needed to recover an incomplete trace.
#[derive(Debug, Clone)]
pub struct TraceRecoveryState {
    /// The trace identifier.
    pub trace_id: TraceId,
    /// The last node that completed successfully (if any).
    pub last_completed_node: Option<NodeId>,
    /// Node where the trace is currently suspended (if any).
    pub suspended_at: Option<NodeId>,
    /// Nodes that have started execution but did not complete.
    pub started_nodes: Vec<NodeId>,
    /// Map of completed nodes to their output locations in the arena.
    pub completed_nodes: HashMap<NodeId, NodeOutputLocation>,
}

/// Location of a node's output in the arena.
#[derive(Debug, Clone, Copy)]
pub struct NodeOutputLocation {
    /// The arena offset where the node's output data is stored.
    pub offset: crate::types::ArenaOffset,
    /// Size of the output data in bytes.
    pub size: u32,
    /// Schema hash used to verify the output data type.
    pub schema_hash: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ArenaOffset;
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
}
