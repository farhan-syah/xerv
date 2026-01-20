//! gRPC server for handling Raft RPC requests.

use crate::proto::raft_service_server::RaftService;
use crate::proto::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use crate::types::{ClusterNodeId, ClusterRaft, TypeConfig};
use openraft::raft::{
    AppendEntriesRequest as RaftAppendRequest, InstallSnapshotRequest as RaftSnapshotRequest,
    VoteRequest as RaftVoteRequest,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// gRPC server implementation for Raft RPC.
pub struct RaftServer {
    /// Reference to the Raft instance.
    raft: Arc<ClusterRaft>,
}

impl RaftServer {
    /// Create a new Raft server.
    pub fn new(raft: Arc<ClusterRaft>) -> Self {
        Self { raft }
    }
}

#[tonic::async_trait]
impl RaftService for RaftServer {
    async fn append_entries(
        &self,
        request: Request<AppendEntriesRequest>,
    ) -> Result<Response<AppendEntriesResponse>, Status> {
        let req: RaftAppendRequest<TypeConfig> = serde_json::from_slice(&request.into_inner().data)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let resp = self
            .raft
            .append_entries(req)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let data = serde_json::to_vec(&resp).map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(AppendEntriesResponse { data }))
    }

    async fn request_vote(
        &self,
        request: Request<VoteRequest>,
    ) -> Result<Response<VoteResponse>, Status> {
        let req: RaftVoteRequest<ClusterNodeId> =
            serde_json::from_slice(&request.into_inner().data)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let resp = self
            .raft
            .vote(req)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let data = serde_json::to_vec(&resp).map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(VoteResponse { data }))
    }

    async fn install_snapshot(
        &self,
        request: Request<InstallSnapshotRequest>,
    ) -> Result<Response<InstallSnapshotResponse>, Status> {
        let req: RaftSnapshotRequest<TypeConfig> =
            serde_json::from_slice(&request.into_inner().data)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let resp = self
            .raft
            .install_snapshot(req)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let data = serde_json::to_vec(&resp).map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(InstallSnapshotResponse { data }))
    }
}
