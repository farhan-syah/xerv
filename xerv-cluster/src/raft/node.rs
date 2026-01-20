//! ClusterNode - the main entry point for cluster operations.

use crate::command::ClusterCommand;
use crate::config::ClusterConfig;
use crate::error::{ClusterError, ClusterResult};
use crate::network::{NetworkClient, RaftServer};
use crate::proto::ExecuteRequest;
use crate::proto::cluster_service_server::ClusterServiceServer;
use crate::proto::raft_service_server::RaftServiceServer;
use crate::raft::storage::LogStorage;
use crate::service::ClusterServiceImpl;
use crate::state::{ClusterResponse, ClusterStateMachine};
use crate::types::{ClusterNodeId, ClusterRaft, extract_forward_to_leader};
use openraft::{BasicNode, Config, Raft};
use parking_lot::RwLock;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::oneshot;
use tonic::transport::{Channel, Server};

/// A node in the XERV cluster.
///
/// This is the main entry point for interacting with the cluster.
/// It manages the Raft instance, network server, and provides
/// methods for executing commands through consensus.
pub struct ClusterNode {
    /// This node's ID.
    node_id: ClusterNodeId,
    /// The Raft instance.
    raft: Arc<ClusterRaft>,
    /// The state machine (for read-only queries).
    state_machine: Arc<ClusterStateMachine>,
    /// Cached connections to peer nodes for leader forwarding.
    peer_connections: Arc<RwLock<HashMap<ClusterNodeId, Channel>>>,
    /// Peer addresses from config.
    peer_addresses: HashMap<ClusterNodeId, String>,
    /// Shutdown signal sender.
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl ClusterNode {
    /// Start a new cluster node.
    pub async fn start(config: ClusterConfig) -> ClusterResult<Self> {
        config.validate().map_err(ClusterError::Config)?;

        let node_id = config.node_id;
        let peer_addresses = config.peers.clone();

        // Create storage
        let log_storage =
            LogStorage::open(config.data_dir.join("raft")).map_err(ClusterError::Io)?;

        // Create state machine
        let state_machine = Arc::new(ClusterStateMachine::new());

        // Create network client
        let network = NetworkClient::new();

        // Create Raft config
        let raft_config = Config {
            cluster_name: "xerv-cluster".to_string(),
            election_timeout_min: config.raft.election_timeout_ms.0,
            election_timeout_max: config.raft.election_timeout_ms.1,
            heartbeat_interval: config.raft.heartbeat_interval_ms,
            max_payload_entries: config.raft.max_entries_per_append,
            snapshot_policy: openraft::SnapshotPolicy::LogsSinceLast(
                config.snapshot.snapshot_threshold,
            ),
            ..Default::default()
        };

        let raft_config = Arc::new(
            raft_config
                .validate()
                .map_err(|e| ClusterError::Config(e.to_string()))?,
        );

        // Create Raft instance
        let raft = Raft::new(
            node_id,
            raft_config,
            network,
            log_storage,
            state_machine.clone(),
        )
        .await
        .map_err(|e| ClusterError::Storage(format!("Failed to create Raft: {:?}", e)))?;

        let raft = Arc::new(raft);

        // Start gRPC server with both Raft and Cluster services
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let raft_server = RaftServer::new(Arc::clone(&raft));
        let cluster_server = ClusterServiceImpl::new(node_id, Arc::clone(&raft));
        let addr = config
            .listen_addr
            .parse()
            .map_err(|e: std::net::AddrParseError| ClusterError::Config(e.to_string()))?;

        tokio::spawn(async move {
            let _ = Server::builder()
                .add_service(RaftServiceServer::new(raft_server))
                .add_service(ClusterServiceServer::new(cluster_server))
                .serve_with_shutdown(addr, async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        tracing::info!(node_id, addr = %config.listen_addr, "Cluster node started");

        Ok(Self {
            node_id,
            raft,
            state_machine,
            peer_connections: Arc::new(RwLock::new(HashMap::new())),
            peer_addresses,
            shutdown_tx: Some(shutdown_tx),
        })
    }

    /// Initialize a new cluster (call on first node only).
    ///
    /// This should be called once when bootstrapping a new cluster.
    /// It makes this node the initial leader with itself as the only member.
    pub async fn initialize(&self) -> ClusterResult<()> {
        let mut members = BTreeMap::new();
        members.insert(
            self.node_id,
            BasicNode {
                addr: String::new(), // Self doesn't need address
            },
        );

        self.raft
            .initialize(members)
            .await
            .map_err(to_cluster_error)?;

        tracing::info!(node_id = self.node_id, "Cluster initialized");
        Ok(())
    }

    /// Add a new node to the cluster.
    ///
    /// This must be called on the current leader.
    pub async fn add_learner(&self, node_id: ClusterNodeId, addr: String) -> ClusterResult<()> {
        let node = BasicNode { addr };
        self.raft
            .add_learner(node_id, node, true)
            .await
            .map_err(to_cluster_error)?;

        tracing::info!(node_id, "Added learner to cluster");
        Ok(())
    }

    /// Promote learners to voters.
    ///
    /// After adding nodes as learners, call this to make them voting members.
    pub async fn change_membership(
        &self,
        members: impl IntoIterator<Item = ClusterNodeId>,
    ) -> ClusterResult<()> {
        let member_set: std::collections::BTreeSet<_> = members.into_iter().collect();
        self.raft
            .change_membership(member_set, false)
            .await
            .map_err(to_cluster_error)?;

        tracing::info!("Membership changed");
        Ok(())
    }

    /// Execute a command through Raft consensus.
    ///
    /// This sends the command to the leader (or this node if it's the leader),
    /// waits for it to be committed and applied, and returns the response.
    /// If this node is not the leader, the command is automatically forwarded.
    pub async fn execute(&self, cmd: ClusterCommand) -> ClusterResult<ClusterResponse> {
        // Try local execution first
        match self.raft.client_write(cmd.clone()).await {
            Ok(resp) => Ok(resp.data),
            Err(e) => {
                // Check if we need to forward to leader using proper enum matching
                if let Some(leader_info) = extract_forward_to_leader(&e) {
                    if leader_info.leader_id != self.node_id {
                        return self.forward_to_leader(leader_info.leader_id, cmd).await;
                    }
                }
                Err(to_cluster_error(e))
            }
        }
    }

    /// Forward a command to the leader node.
    async fn forward_to_leader(
        &self,
        leader_id: ClusterNodeId,
        cmd: ClusterCommand,
    ) -> ClusterResult<ClusterResponse> {
        // Get leader address from membership or peer addresses
        let leader_addr = self.get_leader_address(leader_id).await?;

        tracing::debug!(leader_id, %leader_addr, "forwarding command to leader");

        // Forward via ClusterService on the leader
        self.execute_via_cluster_service(leader_id, &leader_addr, cmd)
            .await
    }

    /// Execute a command via the ClusterService on a remote node.
    async fn execute_via_cluster_service(
        &self,
        leader_id: ClusterNodeId,
        leader_addr: &str,
        cmd: ClusterCommand,
    ) -> ClusterResult<ClusterResponse> {
        use crate::proto::cluster_service_client::ClusterServiceClient;

        let channel = self
            .get_or_create_connection(leader_id, leader_addr)
            .await?;
        let mut client = ClusterServiceClient::new(channel);

        let command_data = serde_json::to_vec(&cmd)?;
        let request = tonic::Request::new(ExecuteRequest {
            command: command_data,
        });

        let response = client.execute(request).await?;
        let resp = response.into_inner();

        if resp.success {
            let data: ClusterResponse = if resp.result.is_empty() {
                ClusterResponse::ok()
            } else {
                serde_json::from_slice(&resp.result)?
            };
            Ok(data)
        } else {
            Ok(ClusterResponse::err(resp.error))
        }
    }

    /// Get the address of a leader node.
    async fn get_leader_address(&self, leader_id: ClusterNodeId) -> ClusterResult<String> {
        // First check peer addresses from config
        if let Some(addr) = self.peer_addresses.get(&leader_id) {
            return Ok(addr.clone());
        }

        // Then check membership config from Raft metrics
        let metrics = self.raft.metrics().borrow().clone();
        for (node_id, node) in metrics.membership_config.nodes() {
            if *node_id == leader_id && !node.addr.is_empty() {
                return Ok(node.addr.clone());
            }
        }

        Err(ClusterError::NodeNotFound(leader_id))
    }

    /// Get or create a gRPC connection to a peer node.
    async fn get_or_create_connection(
        &self,
        node_id: ClusterNodeId,
        addr: &str,
    ) -> ClusterResult<Channel> {
        // Check cache first
        {
            let connections = self.peer_connections.read();
            if let Some(channel) = connections.get(&node_id) {
                return Ok(channel.clone());
            }
        }

        // Create new connection
        let endpoint = format!("http://{}", addr);
        let channel = Channel::from_shared(endpoint)
            .map_err(|e| ClusterError::Config(e.to_string()))?
            .connect()
            .await?;

        // Cache it
        {
            let mut connections = self.peer_connections.write();
            connections.insert(node_id, channel.clone());
        }

        Ok(channel)
    }

    /// Get the current leader's node ID.
    pub async fn leader(&self) -> Option<ClusterNodeId> {
        self.raft.current_leader().await
    }

    /// Check if this node is the leader.
    pub async fn is_leader(&self) -> bool {
        self.raft.current_leader().await == Some(self.node_id)
    }

    /// Get this node's ID.
    pub fn node_id(&self) -> ClusterNodeId {
        self.node_id
    }

    /// Get a reference to the state machine for read-only queries.
    ///
    /// Note: For strong consistency, use `execute()` to route reads through
    /// the leader. This method provides eventual consistency reads.
    pub fn state_machine(&self) -> &ClusterStateMachine {
        &self.state_machine
    }

    /// Get cluster metrics.
    pub fn metrics(&self) -> openraft::RaftMetrics<ClusterNodeId, BasicNode> {
        self.raft.metrics().borrow().clone()
    }

    /// Trigger a snapshot.
    pub async fn trigger_snapshot(&self) -> ClusterResult<()> {
        self.raft
            .trigger()
            .snapshot()
            .await
            .map_err(|e| ClusterError::Storage(format!("{:?}", e)))?;
        Ok(())
    }

    /// Shutdown the node gracefully.
    pub async fn shutdown(&mut self) -> ClusterResult<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        self.raft
            .shutdown()
            .await
            .map_err(|e| ClusterError::Storage(format!("Shutdown error: {:?}", e)))?;

        tracing::info!(node_id = self.node_id, "Cluster node shutdown");
        Ok(())
    }
}

impl Drop for ClusterNode {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Convert any Raft error to ClusterError.
fn to_cluster_error<E: std::fmt::Debug>(
    e: openraft::error::RaftError<ClusterNodeId, E>,
) -> ClusterError {
    ClusterError::Storage(format!("{:?}", e))
}
