//! ClusterService gRPC implementation for external clients.

use crate::command::ClusterCommand;
use crate::proto::cluster_service_server::ClusterService;
use crate::proto::{
    AddNodeRequest, AddNodeResponse, ExecuteRequest, ExecuteResponse, NodeInfo, RemoveNodeRequest,
    RemoveNodeResponse, StatusRequest, StatusResponse,
};
use crate::types::{ClusterNodeId, ClusterRaft, extract_forward_to_leader};
use openraft::BasicNode;
use std::collections::BTreeSet;
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// gRPC service implementation for external cluster clients.
///
/// This service handles:
/// - Command execution with automatic leader forwarding
/// - Cluster status queries
/// - Membership changes (add/remove nodes)
pub struct ClusterServiceImpl {
    /// This node's ID.
    node_id: ClusterNodeId,
    /// Reference to the Raft instance.
    raft: Arc<ClusterRaft>,
}

impl ClusterServiceImpl {
    /// Create a new cluster service.
    pub fn new(node_id: ClusterNodeId, raft: Arc<ClusterRaft>) -> Self {
        Self { node_id, raft }
    }

    /// Get the current leader's node ID and address if known.
    async fn get_leader_info(&self) -> Option<(ClusterNodeId, String)> {
        let metrics = self.raft.metrics().borrow().clone();

        let leader_id = metrics.current_leader?;

        // Find the leader's address from membership nodes
        for (node_id, node) in metrics.membership_config.nodes() {
            if *node_id == leader_id {
                return Some((leader_id, node.addr.clone()));
            }
        }

        Some((leader_id, String::new()))
    }
}

#[tonic::async_trait]
impl ClusterService for ClusterServiceImpl {
    async fn execute(
        &self,
        request: Request<ExecuteRequest>,
    ) -> Result<Response<ExecuteResponse>, Status> {
        let req = request.into_inner();

        // Deserialize the command
        let cmd: ClusterCommand = serde_json::from_slice(&req.command)
            .map_err(|e| Status::invalid_argument(format!("Invalid command: {}", e)))?;

        tracing::debug!(command = %cmd.name(), "executing cluster command");

        // Try to execute through Raft
        match self.raft.client_write(cmd).await {
            Ok(resp) => {
                let result = serde_json::to_vec(&resp.data)
                    .map_err(|e| Status::internal(format!("Serialization error: {}", e)))?;

                Ok(Response::new(ExecuteResponse {
                    success: resp.data.success,
                    error: resp.data.error.unwrap_or_default(),
                    result,
                }))
            }
            Err(e) => {
                // Check if we need to forward to leader using proper enum matching
                if let Some(leader_info) = extract_forward_to_leader(&e) {
                    let addr = if leader_info.leader_addr.is_empty() {
                        // Fall back to getting address from membership
                        self.get_leader_info()
                            .await
                            .map(|(_, a)| a)
                            .unwrap_or_default()
                    } else {
                        leader_info.leader_addr
                    };

                    return Ok(Response::new(ExecuteResponse {
                        success: false,
                        error: format!(
                            "Not the leader. Leader is node {} at {}",
                            leader_info.leader_id, addr
                        ),
                        result: Vec::new(),
                    }));
                }

                Ok(Response::new(ExecuteResponse {
                    success: false,
                    error: format!("Raft error: {:?}", e),
                    result: Vec::new(),
                }))
            }
        }
    }

    async fn get_status(
        &self,
        _request: Request<StatusRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        let metrics = self.raft.metrics().borrow().clone();

        let state = match metrics.state {
            openraft::ServerState::Leader => "leader",
            openraft::ServerState::Follower => "follower",
            openraft::ServerState::Candidate => "candidate",
            openraft::ServerState::Learner => "learner",
            openraft::ServerState::Shutdown => "shutdown",
        };

        let leader_id = metrics.current_leader.unwrap_or(0);
        let term = metrics.current_term;
        let last_applied = metrics.last_applied.map(|log_id| log_id.index).unwrap_or(0);

        // Build member list from membership config
        let mut members = Vec::new();
        for (node_id, node) in metrics.membership_config.nodes() {
            members.push(NodeInfo {
                node_id: *node_id,
                address: node.addr.clone(),
                is_leader: *node_id == leader_id,
            });
        }

        Ok(Response::new(StatusResponse {
            leader_id,
            term,
            node_id: self.node_id,
            state: state.to_string(),
            commit_index: last_applied, // Use last_applied as commit proxy
            last_applied,
            members,
        }))
    }

    async fn add_node(
        &self,
        request: Request<AddNodeRequest>,
    ) -> Result<Response<AddNodeResponse>, Status> {
        let req = request.into_inner();

        tracing::info!(node_id = req.node_id, addr = %req.address, "adding node to cluster");

        // First add as learner
        let node = BasicNode {
            addr: req.address.clone(),
        };

        if let Err(e) = self.raft.add_learner(req.node_id, node, true).await {
            return Ok(Response::new(AddNodeResponse {
                success: false,
                error: format!("Failed to add learner: {:?}", e),
            }));
        }

        // Get current voters and add the new node
        let metrics = self.raft.metrics().borrow().clone();
        let mut voters: BTreeSet<ClusterNodeId> = BTreeSet::new();

        for node_id in metrics.membership_config.voter_ids() {
            voters.insert(node_id);
        }
        voters.insert(req.node_id);

        // Change membership to include new node as voter
        if let Err(e) = self.raft.change_membership(voters, false).await {
            return Ok(Response::new(AddNodeResponse {
                success: false,
                error: format!("Failed to change membership: {:?}", e),
            }));
        }

        tracing::info!(node_id = req.node_id, "node added successfully");

        Ok(Response::new(AddNodeResponse {
            success: true,
            error: String::new(),
        }))
    }

    async fn remove_node(
        &self,
        request: Request<RemoveNodeRequest>,
    ) -> Result<Response<RemoveNodeResponse>, Status> {
        let req = request.into_inner();

        tracing::info!(node_id = req.node_id, "removing node from cluster");

        // Get current voters and remove the specified node
        let metrics = self.raft.metrics().borrow().clone();
        let mut voters: BTreeSet<ClusterNodeId> = BTreeSet::new();

        for node_id in metrics.membership_config.voter_ids() {
            if node_id != req.node_id {
                voters.insert(node_id);
            }
        }

        if voters.is_empty() {
            return Ok(Response::new(RemoveNodeResponse {
                success: false,
                error: "Cannot remove last node from cluster".to_string(),
            }));
        }

        // Change membership to exclude the node
        if let Err(e) = self.raft.change_membership(voters, false).await {
            return Ok(Response::new(RemoveNodeResponse {
                success: false,
                error: format!("Failed to change membership: {:?}", e),
            }));
        }

        tracing::info!(node_id = req.node_id, "node removed successfully");

        Ok(Response::new(RemoveNodeResponse {
            success: true,
            error: String::new(),
        }))
    }
}
