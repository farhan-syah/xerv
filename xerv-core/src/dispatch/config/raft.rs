//! Raft-based dispatch configuration.

use serde::{Deserialize, Serialize};

/// Configuration for Raft-based dispatch.
///
/// Uses the Raft consensus algorithm for distributed, consistent trace dispatch
/// without external dependencies. Best for edge, air-gapped, and simple production
/// deployments.
///
/// # Example
///
/// ```
/// use xerv_core::dispatch::{RaftConfig, RaftPeer};
///
/// let config = RaftConfig::builder()
///     .node_id(1)
///     .listen_addr("0.0.0.0:5000")
///     .peer(2, "10.0.0.2:5000")
///     .peer(3, "10.0.0.3:5000")
///     .build();
///
/// assert_eq!(config.node_id, 1);
/// assert_eq!(config.peers.len(), 2);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftConfig {
    /// This node's ID in the cluster.
    pub node_id: u64,

    /// Address to listen for Raft RPC.
    pub listen_addr: String,

    /// Peer addresses (node_id -> address).
    #[serde(default)]
    pub peers: Vec<RaftPeer>,

    /// Data directory for Raft log persistence.
    #[serde(default = "default_raft_data_dir")]
    pub data_dir: String,

    /// Election timeout in milliseconds.
    #[serde(default = "default_election_timeout")]
    pub election_timeout_ms: u64,

    /// Heartbeat interval in milliseconds.
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_ms: u64,

    /// Maximum entries per append request.
    #[serde(default = "default_max_entries_per_request")]
    pub max_entries_per_request: u64,

    /// Snapshot interval (number of log entries).
    #[serde(default = "default_snapshot_interval")]
    pub snapshot_interval: u64,
}

/// A Raft peer in the cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftPeer {
    /// Peer's node ID.
    pub node_id: u64,
    /// Peer's address (host:port).
    pub address: String,
}

impl RaftPeer {
    /// Create a new peer.
    pub fn new(node_id: u64, address: impl Into<String>) -> Self {
        Self {
            node_id,
            address: address.into(),
        }
    }
}

impl Default for RaftConfig {
    fn default() -> Self {
        Self {
            node_id: 1,
            listen_addr: "127.0.0.1:5000".to_string(),
            peers: Vec::new(),
            data_dir: default_raft_data_dir(),
            election_timeout_ms: default_election_timeout(),
            heartbeat_interval_ms: default_heartbeat_interval(),
            max_entries_per_request: default_max_entries_per_request(),
            snapshot_interval: default_snapshot_interval(),
        }
    }
}

impl RaftConfig {
    /// Create a new Raft config with the given node ID.
    pub fn new(node_id: u64) -> Self {
        Self {
            node_id,
            ..Default::default()
        }
    }

    /// Create a builder for Raft config.
    pub fn builder() -> RaftConfigBuilder {
        RaftConfigBuilder::default()
    }
}

/// Builder for RaftConfig.
///
/// Provides a fluent API for constructing Raft configurations.
#[derive(Default)]
pub struct RaftConfigBuilder {
    config: RaftConfig,
}

impl RaftConfigBuilder {
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

    /// Add a peer.
    pub fn peer(mut self, node_id: u64, address: impl Into<String>) -> Self {
        self.config.peers.push(RaftPeer::new(node_id, address));
        self
    }

    /// Set multiple peers.
    pub fn peers(mut self, peers: Vec<(u64, String)>) -> Self {
        self.config.peers = peers
            .into_iter()
            .map(|(id, addr)| RaftPeer::new(id, addr))
            .collect();
        self
    }

    /// Set the data directory.
    pub fn data_dir(mut self, dir: impl Into<String>) -> Self {
        self.config.data_dir = dir.into();
        self
    }

    /// Set the election timeout in milliseconds.
    pub fn election_timeout_ms(mut self, ms: u64) -> Self {
        self.config.election_timeout_ms = ms;
        self
    }

    /// Set the heartbeat interval in milliseconds.
    pub fn heartbeat_interval_ms(mut self, ms: u64) -> Self {
        self.config.heartbeat_interval_ms = ms;
        self
    }

    /// Build the config.
    pub fn build(self) -> RaftConfig {
        self.config
    }
}

fn default_raft_data_dir() -> String {
    "/var/lib/xerv/raft".to_string()
}

fn default_election_timeout() -> u64 {
    1000
}

fn default_heartbeat_interval() -> u64 {
    100
}

fn default_max_entries_per_request() -> u64 {
    64
}

fn default_snapshot_interval() -> u64 {
    10_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let config = RaftConfig::default();
        assert_eq!(config.node_id, 1);
        assert_eq!(config.listen_addr, "127.0.0.1:5000");
        assert!(config.peers.is_empty());
    }

    #[test]
    fn builder_pattern() {
        let config = RaftConfig::builder()
            .node_id(1)
            .listen_addr("0.0.0.0:5000")
            .peer(2, "10.0.0.2:5000")
            .peer(3, "10.0.0.3:5000")
            .build();

        assert_eq!(config.node_id, 1);
        assert_eq!(config.peers.len(), 2);
        assert_eq!(config.peers[0].node_id, 2);
        assert_eq!(config.peers[1].address, "10.0.0.3:5000");
    }

    #[test]
    fn new_with_node_id() {
        let config = RaftConfig::new(42);
        assert_eq!(config.node_id, 42);
    }
}
