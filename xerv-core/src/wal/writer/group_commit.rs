//! Group commit state tracking.

use super::config::GroupCommitConfig;
use std::sync::atomic::{AtomicU64, Ordering};

/// Group commit state tracking.
pub(super) struct GroupCommitState {
    /// Number of writes since last sync.
    pending_writes: AtomicU64,
    /// Timestamp of last sync (Unix millis).
    last_sync_time: AtomicU64,
}

impl GroupCommitState {
    pub fn new() -> Self {
        Self {
            pending_writes: AtomicU64::new(0),
            last_sync_time: AtomicU64::new(current_time_millis()),
        }
    }

    /// Check if we should sync based on group commit config.
    pub fn should_sync(&self, config: &GroupCommitConfig) -> bool {
        let pending = self.pending_writes.load(Ordering::Acquire);
        let last_sync = self.last_sync_time.load(Ordering::Acquire);
        let now = current_time_millis();

        // Sync if we have enough pending writes
        if pending >= config.max_batch_size as u64 {
            return true;
        }

        // Sync if enough time has passed since last sync
        if now.saturating_sub(last_sync) >= config.max_delay_ms {
            return true;
        }

        false
    }

    /// Record a write.
    pub fn record_write(&self) {
        self.pending_writes.fetch_add(1, Ordering::Release);
    }

    /// Reset after sync.
    pub fn reset_after_sync(&self) {
        self.pending_writes.store(0, Ordering::Release);
        self.last_sync_time
            .store(current_time_millis(), Ordering::Release);
    }

    /// Get the number of pending writes.
    pub fn pending_writes(&self) -> u64 {
        self.pending_writes.load(Ordering::Relaxed)
    }

    /// Get the last sync time in milliseconds.
    pub fn last_sync_time(&self) -> u64 {
        self.last_sync_time.load(Ordering::Relaxed)
    }
}

/// Get current time in milliseconds.
pub(super) fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Statistics for group commit monitoring.
#[derive(Debug, Clone, Copy)]
pub struct GroupCommitStats {
    /// Number of writes pending sync.
    pub pending_writes: u64,
    /// Unix timestamp (ms) of last sync.
    pub last_sync_time_ms: u64,
    /// Configured max delay before forced sync.
    pub config_max_delay_ms: u64,
    /// Configured max batch size before forced sync.
    pub config_max_batch_size: usize,
}
