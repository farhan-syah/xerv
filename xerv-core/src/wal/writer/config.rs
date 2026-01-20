//! WAL configuration.

use std::path::PathBuf;

/// Configuration for WAL creation.
#[derive(Debug, Clone)]
pub struct WalConfig {
    /// Directory for WAL files.
    pub directory: PathBuf,
    /// Maximum WAL file size before rotation.
    pub max_file_size: u64,
    /// Whether to sync after each write.
    ///
    /// When true, fsync is called after every write (slow but safest).
    /// When false, writes are buffered and synced according to group_commit settings.
    pub sync_on_write: bool,
    /// Buffer size for writes.
    pub buffer_size: usize,
    /// Group commit configuration for batching fsyncs.
    ///
    /// When enabled, instead of syncing each write individually, the WAL
    /// will batch writes and sync them together, significantly improving throughput.
    pub group_commit: Option<GroupCommitConfig>,
}

/// Configuration for WAL group commit.
///
/// Group commit batches multiple WAL writes together before calling fsync,
/// which dramatically improves write throughput while maintaining durability
/// guarantees. This is especially important on systems where fsync latency
/// is high (e.g., cloud storage, spinning disks).
///
/// # Trade-offs
///
/// - **Higher throughput**: Fewer fsyncs means less I/O overhead
/// - **Slightly higher latency**: Individual writes may wait for batch
/// - **Batched durability**: Writes become durable together
///
/// # Example
///
/// ```ignore
/// let config = WalConfig::default()
///     .with_group_commit(GroupCommitConfig {
///         max_delay_ms: 10,    // Sync at least every 10ms
///         max_batch_size: 100, // Or after 100 records
///     });
/// ```
#[derive(Debug, Clone, Copy)]
pub struct GroupCommitConfig {
    /// Maximum delay in milliseconds before forcing a sync.
    ///
    /// The WAL will sync at least this often, even if max_batch_size
    /// hasn't been reached. Lower values provide better latency guarantees
    /// but may reduce batching efficiency.
    ///
    /// Typical values: 5-20ms
    pub max_delay_ms: u64,
    /// Maximum number of records to batch before forcing a sync.
    ///
    /// When this many records have been written since the last sync,
    /// a sync is triggered immediately (without waiting for max_delay).
    ///
    /// Typical values: 50-200 records
    pub max_batch_size: usize,
}

impl Default for GroupCommitConfig {
    fn default() -> Self {
        Self {
            max_delay_ms: 10,    // 10ms default
            max_batch_size: 100, // 100 records default
        }
    }
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("/tmp/xerv/wal"),
            max_file_size: 64 * 1024 * 1024, // 64 MB
            sync_on_write: true,
            buffer_size: 64 * 1024, // 64 KB
            group_commit: None,     // Disabled by default for backwards compatibility
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
            group_commit: None,
        }
    }

    /// Create a high-performance configuration using group commit.
    ///
    /// This configuration batches fsyncs for higher throughput while
    /// maintaining durability guarantees.
    pub fn high_performance(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            max_file_size: 64 * 1024 * 1024,
            sync_on_write: false, // Disable per-write sync
            buffer_size: 64 * 1024,
            group_commit: Some(GroupCommitConfig::default()),
        }
    }

    /// Set the WAL directory.
    pub fn with_directory(mut self, dir: impl Into<PathBuf>) -> Self {
        self.directory = dir.into();
        self
    }

    /// Set sync on write.
    ///
    /// When sync_on_write is true, group_commit is ignored.
    pub fn with_sync(mut self, sync: bool) -> Self {
        self.sync_on_write = sync;
        self
    }

    /// Enable group commit with the given configuration.
    ///
    /// This automatically disables sync_on_write.
    pub fn with_group_commit(mut self, config: GroupCommitConfig) -> Self {
        self.sync_on_write = false;
        self.group_commit = Some(config);
        self
    }
}
