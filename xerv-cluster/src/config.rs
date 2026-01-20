//! Cluster configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Configuration for a cluster node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// This node's unique ID in the cluster (1-based).
    pub node_id: u64,

    /// Address this node listens on for Raft RPC (e.g., "0.0.0.0:5000").
    pub listen_addr: String,

    /// Address advertised to other nodes (e.g., "192.168.1.10:5000").
    /// If not set, uses listen_addr.
    pub advertise_addr: Option<String>,

    /// Peer nodes in the cluster: node_id -> address.
    pub peers: HashMap<u64, String>,

    /// Directory for Raft log storage.
    pub data_dir: PathBuf,

    /// Raft timing configuration.
    pub raft: RaftConfig,

    /// Snapshot configuration.
    pub snapshot: SnapshotConfig,
}

/// Raft timing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftConfig {
    /// Election timeout range (min, max) in milliseconds.
    /// A random value in this range is chosen for each election.
    /// Should be >> heartbeat_interval to avoid spurious elections.
    pub election_timeout_ms: (u64, u64),

    /// Heartbeat interval in milliseconds.
    /// Leader sends heartbeats at this interval to maintain authority.
    pub heartbeat_interval_ms: u64,

    /// Maximum entries per AppendEntries RPC.
    pub max_entries_per_append: u64,

    /// Maximum size of AppendEntries payload in bytes.
    pub max_append_payload_bytes: u64,
}

/// Snapshot configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotConfig {
    /// Create snapshot after this many log entries.
    pub snapshot_threshold: u64,

    /// Maximum number of log entries to keep after snapshot.
    pub max_log_entries: u64,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            node_id: 1,
            listen_addr: "127.0.0.1:5000".to_string(),
            advertise_addr: None,
            peers: HashMap::new(),
            data_dir: PathBuf::from("./xerv-cluster-data"),
            raft: RaftConfig::default(),
            snapshot: SnapshotConfig::default(),
        }
    }
}

impl Default for RaftConfig {
    fn default() -> Self {
        Self {
            // Election timeout: 150-300ms (standard Raft recommendation)
            election_timeout_ms: (150, 300),
            // Heartbeat: 50ms (should be << election timeout)
            heartbeat_interval_ms: 50,
            // Up to 100 entries per batch
            max_entries_per_append: 100,
            // 1MB max payload
            max_append_payload_bytes: 1024 * 1024,
        }
    }
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            // Snapshot every 10000 entries
            snapshot_threshold: 10_000,
            // Keep up to 1000 entries after snapshot for catch-up
            max_log_entries: 1_000,
        }
    }
}

impl ClusterConfig {
    /// Create a new configuration builder.
    pub fn builder() -> ClusterConfigBuilder {
        ClusterConfigBuilder::default()
    }

    /// Get the advertised address (falls back to listen_addr).
    pub fn advertise_addr(&self) -> &str {
        self.advertise_addr.as_deref().unwrap_or(&self.listen_addr)
    }

    /// Get the election timeout as a Duration range.
    pub fn election_timeout(&self) -> (Duration, Duration) {
        (
            Duration::from_millis(self.raft.election_timeout_ms.0),
            Duration::from_millis(self.raft.election_timeout_ms.1),
        )
    }

    /// Get the heartbeat interval as a Duration.
    pub fn heartbeat_interval(&self) -> Duration {
        Duration::from_millis(self.raft.heartbeat_interval_ms)
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.node_id == 0 {
            return Err("node_id must be > 0".to_string());
        }

        if self.listen_addr.is_empty() {
            return Err("listen_addr is required".to_string());
        }

        // Heartbeat should be much less than election timeout
        let (min_election, _) = self.raft.election_timeout_ms;
        if self.raft.heartbeat_interval_ms >= min_election / 2 {
            return Err(format!(
                "heartbeat_interval_ms ({}) should be << election_timeout_ms ({})",
                self.raft.heartbeat_interval_ms, min_election
            ));
        }

        Ok(())
    }
}

/// Builder for ClusterConfig.
#[derive(Debug, Default)]
pub struct ClusterConfigBuilder {
    config: ClusterConfig,
}

impl ClusterConfigBuilder {
    /// Set the node ID.
    pub fn node_id(mut self, id: u64) -> Self {
        self.config.node_id = id;
        self
    }

    /// Set the listen address.
    pub fn listen_addr(mut self, addr: impl Into<String>) -> Self {
        self.config.listen_addr = addr.into();
        self
    }

    /// Set the advertise address.
    pub fn advertise_addr(mut self, addr: impl Into<String>) -> Self {
        self.config.advertise_addr = Some(addr.into());
        self
    }

    /// Add a peer node.
    pub fn peer(mut self, node_id: u64, addr: impl Into<String>) -> Self {
        self.config.peers.insert(node_id, addr.into());
        self
    }

    /// Set all peers at once.
    pub fn peers(mut self, peers: impl IntoIterator<Item = (u64, String)>) -> Self {
        self.config.peers = peers.into_iter().collect();
        self
    }

    /// Set the data directory.
    pub fn data_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.data_dir = path.into();
        self
    }

    /// Set election timeout range in milliseconds.
    pub fn election_timeout_ms(mut self, min: u64, max: u64) -> Self {
        self.config.raft.election_timeout_ms = (min, max);
        self
    }

    /// Set heartbeat interval in milliseconds.
    pub fn heartbeat_interval_ms(mut self, ms: u64) -> Self {
        self.config.raft.heartbeat_interval_ms = ms;
        self
    }

    /// Set snapshot threshold.
    pub fn snapshot_threshold(mut self, entries: u64) -> Self {
        self.config.snapshot.snapshot_threshold = entries;
        self
    }

    /// Build the configuration.
    pub fn build(self) -> Result<ClusterConfig, String> {
        self.config.validate()?;
        Ok(self.config)
    }
}
