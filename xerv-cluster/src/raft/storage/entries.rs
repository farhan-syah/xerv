//! Log entry operations: append, purge, truncate.

use crate::types::{ClusterEntry, ClusterLogId};
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write as IoWrite};

use super::inner::{ActiveSegment, LogStorageInner, SEGMENT_MAX_ENTRIES};
use super::persistence::PersistenceOps;
use super::segment::SegmentOps;

/// Operations for managing log entries.
pub trait EntryOps {
    /// Append entries to the active segment, rotating if needed.
    fn append_entries(&mut self, entries: Vec<ClusterEntry>) -> Result<(), std::io::Error>;

    /// Purge entries up to and including log_id by deleting old segments.
    fn purge_entries(&mut self, log_id: ClusterLogId) -> Result<(), std::io::Error>;

    /// Truncate entries at and after log_id.
    fn truncate_entries(&mut self, log_id: ClusterLogId) -> Result<(), std::io::Error>;
}

impl EntryOps for LogStorageInner {
    fn append_entries(&mut self, entries: Vec<ClusterEntry>) -> Result<(), std::io::Error> {
        if entries.is_empty() {
            return Ok(());
        }

        for entry in entries {
            // Ensure we have an active segment
            if self.active_segment.is_none() {
                self.start_new_segment(entry.log_id.index)?;
            }

            let active = self
                .active_segment
                .as_mut()
                .expect("active segment must exist after start_new_segment");

            // Check if we need to rotate
            if active.entry_count >= SEGMENT_MAX_ENTRIES {
                // Flush and close current segment
                active.writer.flush()?;
                active.writer.get_ref().sync_all()?;

                // Start new segment
                let new_first_index = entry.log_id.index;
                self.start_new_segment(new_first_index)?;
            }

            // Append to active segment
            let active = self
                .active_segment
                .as_mut()
                .expect("active segment must exist after rotation check");
            let json = serde_json::to_string(&entry)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            writeln!(active.writer, "{}", json)?;
            active.entry_count += 1;

            // Also add to in-memory map
            self.logs.insert(entry.log_id.index, entry);
        }

        // Flush and fsync
        if let Some(active) = &mut self.active_segment {
            active.writer.flush()?;
            active.writer.get_ref().sync_all()?;
        }

        Ok(())
    }

    fn purge_entries(&mut self, log_id: ClusterLogId) -> Result<(), std::io::Error> {
        let purge_index = log_id.index;

        // Update metadata first
        self.last_purged_log_id = Some(log_id);

        // Remove from in-memory map
        let keys_to_remove: Vec<u64> = self.logs.range(..=purge_index).map(|(k, _)| *k).collect();

        for key in keys_to_remove {
            self.logs.remove(&key);
        }

        // Delete segment files that are fully purged
        let segments = self.list_segments()?;
        for (first_index, path) in segments {
            // Calculate the last index in this segment
            let last_index_in_segment = first_index + SEGMENT_MAX_ENTRIES as u64 - 1;

            if last_index_in_segment <= purge_index {
                // All entries in this segment are purged, delete the file
                if !self.is_active_segment(first_index) {
                    fs::remove_file(&path)?;
                } else {
                    // Close active segment and delete it
                    self.active_segment = None;
                    fs::remove_file(&path)?;
                }
            }
        }

        self.save_meta()?;
        Ok(())
    }

    fn truncate_entries(&mut self, log_id: ClusterLogId) -> Result<(), std::io::Error> {
        let truncate_index = log_id.index;

        // Remove from in-memory map
        let keys_to_remove: Vec<u64> = self.logs.range(truncate_index..).map(|(k, _)| *k).collect();

        for key in keys_to_remove {
            self.logs.remove(&key);
        }

        // Find which segment contains the truncate point
        let segments = self.list_segments()?;

        for (first_index, path) in segments {
            if first_index >= truncate_index {
                // Delete entire segment
                if self.is_active_segment(first_index) {
                    self.active_segment = None;
                }
                fs::remove_file(&path)?;
            } else if first_index + SEGMENT_MAX_ENTRIES as u64 > truncate_index {
                // This segment contains the truncate point, rewrite it
                let entries: Vec<ClusterEntry> = self
                    .logs
                    .range(first_index..truncate_index)
                    .map(|(_, e)| e.clone())
                    .collect();

                // Close active segment if it's this one
                if self.is_active_segment(first_index) {
                    self.active_segment = None;
                }

                if entries.is_empty() {
                    // Just delete the segment
                    fs::remove_file(&path)?;
                } else {
                    // Rewrite with remaining entries
                    self.write_segment_file(first_index, &entries)?;

                    // Reopen as active segment
                    let file = OpenOptions::new().create(true).append(true).open(&path)?;
                    self.active_segment = Some(ActiveSegment {
                        first_index,
                        entry_count: entries.len(),
                        writer: BufWriter::new(file),
                    });
                }
            }
        }

        Ok(())
    }
}
