//! Common test utilities for xerv-cluster tests.

use std::sync::atomic::{AtomicU16, Ordering};
use tempfile::TempDir;
use xerv_cluster::{ClusterConfig, ClusterNode};

/// Atomic counter for allocating unique ports.
static PORT_COUNTER: AtomicU16 = AtomicU16::new(15000);

/// Get a unique port for testing.
pub fn get_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Test context that holds temp directories and nodes.
#[allow(dead_code)]
pub struct TestCluster {
    /// Temp directories for each node (kept alive for the test duration).
    _temp_dirs: Vec<TempDir>,
    /// Cluster nodes.
    pub nodes: Vec<ClusterNode>,
    /// Node addresses.
    pub addresses: Vec<String>,
}

#[allow(dead_code)]
impl TestCluster {
    /// Create a new test cluster with the specified number of nodes.
    pub async fn new(node_count: usize) -> Self {
        let mut temp_dirs = Vec::with_capacity(node_count);
        let mut nodes = Vec::with_capacity(node_count);
        let mut addresses = Vec::with_capacity(node_count);

        // First, allocate ports and create configs
        let mut configs = Vec::with_capacity(node_count);
        for i in 0..node_count {
            let port = get_test_port();
            let addr = format!("127.0.0.1:{}", port);
            addresses.push(addr.clone());

            let temp_dir = TempDir::new().expect("Failed to create temp dir");
            let data_dir = temp_dir.path().to_path_buf();
            temp_dirs.push(temp_dir);

            configs.push((i as u64 + 1, addr, data_dir));
        }

        // Now create configs with peer information
        for (node_id, listen_addr, data_dir) in &configs {
            let mut builder = ClusterConfig::builder()
                .node_id(*node_id)
                .listen_addr(listen_addr.clone())
                .data_dir(data_dir.clone());

            // Add all other nodes as peers
            for (peer_id, peer_addr, _) in &configs {
                if peer_id != node_id {
                    builder = builder.peer(*peer_id, peer_addr.clone());
                }
            }

            let config = builder.build().expect("Invalid config");
            let node = ClusterNode::start(config)
                .await
                .expect("Failed to start node");
            nodes.push(node);
        }

        Self {
            _temp_dirs: temp_dirs,
            nodes,
            addresses,
        }
    }

    /// Initialize the cluster (call on first node).
    pub async fn initialize(&self) {
        self.nodes[0]
            .initialize()
            .await
            .expect("Failed to initialize cluster");
    }

    /// Add remaining nodes to the cluster.
    pub async fn add_all_nodes(&self) {
        for i in 1..self.nodes.len() {
            let node_id = (i + 1) as u64;
            self.nodes[0]
                .add_learner(node_id, self.addresses[i].clone())
                .await
                .expect("Failed to add learner");
        }

        // Change membership to include all nodes
        let all_ids: Vec<u64> = (1..=self.nodes.len() as u64).collect();
        self.nodes[0]
            .change_membership(all_ids)
            .await
            .expect("Failed to change membership");
    }

    /// Wait for a leader to be elected.
    pub async fn wait_for_leader(&self, timeout_ms: u64) -> Option<u64> {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout {
            for node in &self.nodes {
                if let Some(leader) = node.leader().await {
                    return Some(leader);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        None
    }

    /// Shutdown all nodes.
    pub async fn shutdown(&mut self) {
        for node in &mut self.nodes {
            let _ = node.shutdown().await;
        }
    }
}
