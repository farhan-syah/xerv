//! Internal storage state types.

use crate::types::{ClusterEntry, ClusterLogId, ClusterNodeId};
use openraft::Vote;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::BufWriter;
use std::path::PathBuf;

use super::persistence::PersistenceOps;
use super::segment::SegmentOps;

/// Maximum entries per segment before rotating to a new segment.
pub const SEGMENT_MAX_ENTRIES: usize = 1000;

/// Inner storage state.
pub struct LogStorageInner {
    /// Directory for storage files.
    pub dir: PathBuf,
    /// Segments subdirectory.
    pub segments_dir: PathBuf,
    /// In-memory log entries (for fast access).
    pub logs: BTreeMap<u64, ClusterEntry>,
    /// Last purged log ID.
    pub last_purged_log_id: Option<ClusterLogId>,
    /// Committed log ID.
    pub committed: Option<ClusterLogId>,
    /// Current vote state.
    pub vote: Option<Vote<ClusterNodeId>>,
    /// Active segment state.
    pub active_segment: Option<ActiveSegment>,
}

/// State of the currently active (writable) segment.
pub struct ActiveSegment {
    /// First log index in this segment.
    pub first_index: u64,
    /// Number of entries in this segment.
    pub entry_count: usize,
    /// File handle for appending.
    pub writer: BufWriter<File>,
}

impl LogStorageInner {
    /// Create a new inner storage state.
    pub fn new(dir: PathBuf, segments_dir: PathBuf) -> Self {
        Self {
            dir,
            segments_dir,
            logs: BTreeMap::new(),
            last_purged_log_id: None,
            committed: None,
            vote: None,
            active_segment: None,
        }
    }

    /// Initialize storage by loading existing data.
    pub fn initialize(&mut self) -> Result<(), std::io::Error> {
        self.load_segments()?;
        self.load_vote()?;
        self.load_meta()?;
        self.migrate_from_single_file()?;
        Ok(())
    }

    /// Start a new segment file.
    pub fn start_new_segment(&mut self, first_index: u64) -> Result<(), std::io::Error> {
        // Align to segment boundary
        let aligned_first = (first_index / SEGMENT_MAX_ENTRIES as u64) * SEGMENT_MAX_ENTRIES as u64;

        let path = self
            .segments_dir
            .join(Self::segment_filename(aligned_first));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        self.active_segment = Some(ActiveSegment {
            first_index: aligned_first,
            entry_count: 0,
            writer: BufWriter::new(file),
        });

        Ok(())
    }

    /// Check if a segment is the currently active segment.
    pub fn is_active_segment(&self, first_index: u64) -> bool {
        self.active_segment
            .as_ref()
            .map(|s| s.first_index == first_index)
            .unwrap_or(false)
    }
}
