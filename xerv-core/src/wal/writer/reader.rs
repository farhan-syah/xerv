//! WAL reader for recovery.

use crate::error::{Result, XervError};
use crate::types::{ArenaOffset, NodeId, TraceId};
use crate::wal::record::{MIN_RECORD_SIZE, WalRecord, WalRecordType};
use crate::wal::snapshot::WalPosition;
use byteorder::{LittleEndian, ReadBytesExt};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

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
    pub offset: ArenaOffset,
    /// Size of the output data in bytes.
    pub size: u32,
    /// Schema hash used to verify the output data type.
    pub schema_hash: u64,
}
