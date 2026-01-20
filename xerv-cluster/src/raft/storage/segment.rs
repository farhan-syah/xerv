//! Segment file operations.
//!
//! Handles reading, writing, and managing segment files.

use crate::types::ClusterEntry;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write as IoWrite};
use std::path::{Path, PathBuf};

use super::inner::{ActiveSegment, LogStorageInner, SEGMENT_MAX_ENTRIES};

/// Operations for managing segment files.
pub trait SegmentOps {
    /// Generate segment filename from first index.
    fn segment_filename(first_index: u64) -> String;

    /// Parse first index from segment filename.
    fn parse_segment_filename(filename: &str) -> Option<u64>;

    /// Get all segment files sorted by first index.
    fn list_segments(&self) -> Result<Vec<(u64, PathBuf)>, std::io::Error>;

    /// Load all segment files into memory.
    fn load_segments(&mut self) -> Result<(), std::io::Error>;

    /// Load a single segment file into memory.
    fn load_segment_file(&mut self, first_index: u64, path: &Path) -> Result<(), std::io::Error>;

    /// Write a segment file (overwrites if exists).
    fn write_segment_file(
        &self,
        first_index: u64,
        entries: &[ClusterEntry],
    ) -> Result<(), std::io::Error>;

    /// Flush all in-memory logs to segment files (used during migration).
    fn flush_all_to_segments(&mut self) -> Result<(), std::io::Error>;
}

impl SegmentOps for LogStorageInner {
    fn segment_filename(first_index: u64) -> String {
        format!("seg_{:012}.log", first_index)
    }

    fn parse_segment_filename(filename: &str) -> Option<u64> {
        if filename.starts_with("seg_") && filename.ends_with(".log") {
            let index_str = &filename[4..16];
            index_str.parse().ok()
        } else {
            None
        }
    }

    fn list_segments(&self) -> Result<Vec<(u64, PathBuf)>, std::io::Error> {
        let mut segments = Vec::new();

        if !self.segments_dir.exists() {
            return Ok(segments);
        }

        for entry in fs::read_dir(&self.segments_dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if let Some(first_index) = Self::parse_segment_filename(filename) {
                    segments.push((first_index, path));
                }
            }
        }

        segments.sort_by_key(|(idx, _)| *idx);
        Ok(segments)
    }

    fn load_segments(&mut self) -> Result<(), std::io::Error> {
        let segments = self.list_segments()?;

        for (first_index, path) in &segments {
            self.load_segment_file(*first_index, path)?;
        }

        // Open the last segment as active if it exists and has room
        if let Some((first_index, path)) = segments.last() {
            let entry_count = self.logs.range(*first_index..).count();

            if entry_count < SEGMENT_MAX_ENTRIES {
                let file = OpenOptions::new().create(true).append(true).open(path)?;
                self.active_segment = Some(ActiveSegment {
                    first_index: *first_index,
                    entry_count,
                    writer: BufWriter::new(file),
                });
            }
        }

        Ok(())
    }

    fn load_segment_file(&mut self, _first_index: u64, path: &Path) -> Result<(), std::io::Error> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let line = line?;
            if line.is_empty() {
                continue;
            }

            let entry: ClusterEntry = serde_json::from_str(&line)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            self.logs.insert(entry.log_id.index, entry);
        }

        Ok(())
    }

    fn write_segment_file(
        &self,
        first_index: u64,
        entries: &[ClusterEntry],
    ) -> Result<(), std::io::Error> {
        let path = self.segments_dir.join(Self::segment_filename(first_index));
        let temp_path = self
            .segments_dir
            .join(format!("{}.tmp", Self::segment_filename(first_index)));

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)?;
        let mut writer = BufWriter::new(file);

        for entry in entries {
            let json = serde_json::to_string(entry)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            writeln!(writer, "{}", json)?;
        }

        writer.flush()?;
        writer.get_ref().sync_all()?;

        fs::rename(&temp_path, &path)?;
        Ok(())
    }

    fn flush_all_to_segments(&mut self) -> Result<(), std::io::Error> {
        // Close any active segment
        self.active_segment = None;

        // Group entries by segment
        let mut current_segment_entries: Vec<ClusterEntry> = Vec::new();
        let mut current_segment_first_index: Option<u64> = None;

        for (index, entry) in &self.logs {
            let segment_first = (*index / SEGMENT_MAX_ENTRIES as u64) * SEGMENT_MAX_ENTRIES as u64;

            if current_segment_first_index != Some(segment_first) {
                // Write previous segment if any
                if let Some(first_idx) = current_segment_first_index {
                    if !current_segment_entries.is_empty() {
                        self.write_segment_file(first_idx, &current_segment_entries)?;
                    }
                }
                current_segment_entries.clear();
                current_segment_first_index = Some(segment_first);
            }

            current_segment_entries.push(entry.clone());
        }

        // Write final segment
        if let Some(first_idx) = current_segment_first_index {
            if !current_segment_entries.is_empty() {
                self.write_segment_file(first_idx, &current_segment_entries)?;

                // Open as active segment if not full
                if current_segment_entries.len() < SEGMENT_MAX_ENTRIES {
                    let path = self.segments_dir.join(Self::segment_filename(first_idx));
                    let file = OpenOptions::new().create(true).append(true).open(&path)?;
                    self.active_segment = Some(ActiveSegment {
                        first_index: first_idx,
                        entry_count: current_segment_entries.len(),
                        writer: BufWriter::new(file),
                    });
                }
            }
        }

        Ok(())
    }
}
