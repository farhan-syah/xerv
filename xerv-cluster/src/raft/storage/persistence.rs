//! Persistence operations: vote, metadata, and migration.

use crate::types::{ClusterEntry, ClusterLogId, ClusterNodeId};
use openraft::Vote;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write as IoWrite};

use super::inner::LogStorageInner;
use super::segment::SegmentOps;

/// Persisted vote state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedVote {
    pub term: u64,
    pub node_id: Option<ClusterNodeId>,
    pub committed: bool,
}

/// Persisted log state metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedLogMeta {
    pub last_purged_log_id: Option<ClusterLogId>,
    pub committed: Option<ClusterLogId>,
}

/// Operations for persistence (vote, metadata, migration).
pub trait PersistenceOps {
    /// Load vote from disk.
    fn load_vote(&mut self) -> Result<(), std::io::Error>;

    /// Save vote to disk.
    fn save_vote_to_disk(&self, vote: &Vote<ClusterNodeId>) -> Result<(), std::io::Error>;

    /// Load metadata from disk.
    fn load_meta(&mut self) -> Result<(), std::io::Error>;

    /// Save metadata to disk.
    fn save_meta(&self) -> Result<(), std::io::Error>;

    /// Migrate from old single-file logs.json format.
    fn migrate_from_single_file(&mut self) -> Result<(), std::io::Error>;
}

impl PersistenceOps for LogStorageInner {
    fn load_vote(&mut self) -> Result<(), std::io::Error> {
        let vote_path = self.dir.join("vote.json");
        if !vote_path.exists() {
            return Ok(());
        }

        let file = File::open(&vote_path)?;
        let reader = BufReader::new(file);
        let persisted: PersistedVote = serde_json::from_reader(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut vote = Vote::new(persisted.term, persisted.node_id.unwrap_or(0));
        if persisted.committed {
            vote.commit();
        }

        self.vote = Some(vote);
        Ok(())
    }

    fn save_vote_to_disk(&self, vote: &Vote<ClusterNodeId>) -> Result<(), std::io::Error> {
        let vote_path = self.dir.join("vote.json");
        let temp_path = self.dir.join("vote.json.tmp");

        let persisted = PersistedVote {
            term: vote.leader_id().term,
            node_id: Some(vote.leader_id().node_id),
            committed: vote.is_committed(),
        };

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, &persisted)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writer.flush()?;
        writer.get_ref().sync_all()?;

        fs::rename(&temp_path, &vote_path)?;
        Ok(())
    }

    fn load_meta(&mut self) -> Result<(), std::io::Error> {
        let meta_path = self.dir.join("meta.json");
        if !meta_path.exists() {
            return Ok(());
        }

        let file = File::open(&meta_path)?;
        let reader = BufReader::new(file);
        let meta: PersistedLogMeta = serde_json::from_reader(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        self.last_purged_log_id = meta.last_purged_log_id;
        self.committed = meta.committed;
        Ok(())
    }

    fn save_meta(&self) -> Result<(), std::io::Error> {
        let meta_path = self.dir.join("meta.json");
        let temp_path = self.dir.join("meta.json.tmp");

        let meta = PersistedLogMeta {
            last_purged_log_id: self.last_purged_log_id,
            committed: self.committed,
        };

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, &meta)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writer.flush()?;
        writer.get_ref().sync_all()?;

        fs::rename(&temp_path, &meta_path)?;
        Ok(())
    }

    fn migrate_from_single_file(&mut self) -> Result<(), std::io::Error> {
        let old_logs_path = self.dir.join("logs.json");
        if !old_logs_path.exists() {
            return Ok(());
        }

        // Load old format
        let file = File::open(&old_logs_path)?;
        let reader = BufReader::new(file);
        let entries: Vec<ClusterEntry> = serde_json::from_reader(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        if entries.is_empty() {
            // Just remove the old file
            fs::remove_file(&old_logs_path)?;
            return Ok(());
        }

        // Write entries to in-memory map
        for entry in entries {
            self.logs.insert(entry.log_id.index, entry);
        }

        // Flush to segment files
        self.flush_all_to_segments()?;

        // Remove old file after successful migration
        fs::remove_file(&old_logs_path)?;

        Ok(())
    }
}
