//! Network client implementing OpenRaft's RaftNetwork trait.

use crate::error::{ClusterError, RPCError};
use crate::proto::raft_service_client::RaftServiceClient;
use crate::proto::{AppendEntriesRequest, InstallSnapshotRequest, VoteRequest};
use crate::types::{ClusterNodeId, TypeConfig};
use openraft::BasicNode;
use openraft::error::{InstallSnapshotError, NetworkError, Unreachable};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest as RaftAppendRequest, AppendEntriesResponse as RaftAppendResponse,
    InstallSnapshotRequest as RaftSnapshotRequest, InstallSnapshotResponse as RaftSnapshotResponse,
    VoteRequest as RaftVoteRequest, VoteResponse as RaftVoteResponse,
};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::transport::Channel;

/// Factory for creating network connections to other nodes.
#[derive(Clone)]
pub struct NetworkClient {
    /// Cached connections to other nodes.
    connections: Arc<RwLock<HashMap<ClusterNodeId, Channel>>>,
}

impl NetworkClient {
    /// Create a new network client.
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create a connection to a node.
    async fn get_connection(
        &self,
        target: ClusterNodeId,
        addr: &str,
    ) -> Result<Channel, ClusterError> {
        // Check cache first
        {
            let connections = self.connections.read();
            if let Some(channel) = connections.get(&target) {
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
            let mut connections = self.connections.write();
            connections.insert(target, channel.clone());
        }

        Ok(channel)
    }
}

impl Default for NetworkClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RaftNetworkFactory<TypeConfig> for NetworkClient {
    type Network = NetworkConnection;

    async fn new_client(&mut self, target: ClusterNodeId, node: &BasicNode) -> Self::Network {
        NetworkConnection {
            target,
            addr: node.addr.clone(),
            client: self.clone(),
        }
    }
}

/// A connection to a specific node.
pub struct NetworkConnection {
    /// Target node ID.
    target: ClusterNodeId,
    /// Target address.
    addr: String,
    /// Reference to the client factory.
    client: NetworkClient,
}

impl RaftNetwork<TypeConfig> for NetworkConnection {
    async fn append_entries(
        &mut self,
        req: RaftAppendRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<RaftAppendResponse<ClusterNodeId>, RPCError> {
        let channel = self
            .client
            .get_connection(self.target, &self.addr)
            .await
            .map_err(|e| to_network_error(&e))?;

        let mut client = RaftServiceClient::new(channel);

        let data = serde_json::to_vec(&req)
            .map_err(|e| to_network_error(&ClusterError::Serialization(e.to_string())))?;

        let response = client
            .append_entries(AppendEntriesRequest { data })
            .await
            .map_err(|e| to_unreachable_error(&e))?;

        let resp: RaftAppendResponse<ClusterNodeId> =
            serde_json::from_slice(&response.into_inner().data)
                .map_err(|e| to_network_error(&ClusterError::Serialization(e.to_string())))?;

        Ok(resp)
    }

    async fn install_snapshot(
        &mut self,
        req: RaftSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<RaftSnapshotResponse<ClusterNodeId>, RPCError<InstallSnapshotError>> {
        let channel = self
            .client
            .get_connection(self.target, &self.addr)
            .await
            .map_err(|e| to_network_error_snapshot(&e))?;

        let mut client = RaftServiceClient::new(channel);

        let data = serde_json::to_vec(&req)
            .map_err(|e| to_network_error_snapshot(&ClusterError::Serialization(e.to_string())))?;

        let response = client
            .install_snapshot(InstallSnapshotRequest { data })
            .await
            .map_err(|e| to_unreachable_error_snapshot(&e))?;

        let resp: RaftSnapshotResponse<ClusterNodeId> =
            serde_json::from_slice(&response.into_inner().data).map_err(|e| {
                to_network_error_snapshot(&ClusterError::Serialization(e.to_string()))
            })?;

        Ok(resp)
    }

    async fn vote(
        &mut self,
        req: RaftVoteRequest<ClusterNodeId>,
        _option: RPCOption,
    ) -> Result<RaftVoteResponse<ClusterNodeId>, RPCError> {
        let channel = self
            .client
            .get_connection(self.target, &self.addr)
            .await
            .map_err(|e| to_network_error(&e))?;

        let mut client = RaftServiceClient::new(channel);

        let data = serde_json::to_vec(&req)
            .map_err(|e| to_network_error(&ClusterError::Serialization(e.to_string())))?;

        let response = client
            .request_vote(VoteRequest { data })
            .await
            .map_err(|e| to_unreachable_error(&e))?;

        let resp: RaftVoteResponse<ClusterNodeId> =
            serde_json::from_slice(&response.into_inner().data)
                .map_err(|e| to_network_error(&ClusterError::Serialization(e.to_string())))?;

        Ok(resp)
    }
}

/// Helper to create NetworkError from ClusterError.
fn to_network_error(err: &(impl std::error::Error + 'static)) -> RPCError {
    openraft::error::RPCError::Network(NetworkError::new(err))
}

/// Helper to create Unreachable error from tonic Status.
fn to_unreachable_error(err: &tonic::Status) -> RPCError {
    openraft::error::RPCError::Unreachable(Unreachable::new(err))
}

/// Helper to create NetworkError from ClusterError for snapshot operations.
fn to_network_error_snapshot(
    err: &(impl std::error::Error + 'static),
) -> RPCError<InstallSnapshotError> {
    openraft::error::RPCError::Network(NetworkError::new(err))
}

/// Helper to create Unreachable error from tonic Status for snapshot operations.
fn to_unreachable_error_snapshot(err: &tonic::Status) -> RPCError<InstallSnapshotError> {
    openraft::error::RPCError::Unreachable(Unreachable::new(err))
}
