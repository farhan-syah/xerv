//! Trait implementations for OpenRaft integration.

use crate::types::{
    ClusterLogId, ClusterSnapshot, ClusterSnapshotMeta, ClusterStorageError,
    ClusterStoredMembership, TypeConfig,
};
use openraft::storage::RaftStateMachine;
use openraft::{
    EntryPayload, RaftSnapshotBuilder, RaftTypeConfig, StorageIOError, StoredMembership,
};
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::ClusterStateMachine;
use super::apply::apply_command;
use super::state::ClusterState;
use super::types::{ClusterResponse, StoredSnapshot};

/// Implementation of OpenRaft's snapshot builder interface.
///
/// Builds a consistent snapshot of the current cluster state for
/// log compaction and state transfer to new nodes.
impl RaftSnapshotBuilder<TypeConfig> for Arc<ClusterStateMachine> {
    async fn build_snapshot(&mut self) -> Result<ClusterSnapshot, ClusterStorageError> {
        let state = self.state.read().await;

        let data =
            serde_json::to_vec(&*state).map_err(|e| StorageIOError::read_state_machine(&e))?;

        let last_applied_log = state.last_applied_log;
        let last_membership = state.last_membership.clone();

        // Lock snapshot before releasing state lock
        let mut current_snapshot = self.current_snapshot.write().await;
        drop(state);

        let snapshot_idx = self.snapshot_idx.fetch_add(1, Ordering::Relaxed) + 1;
        let snapshot_id = if let Some(last) = last_applied_log {
            format!("{}-{}-{}", last.leader_id, last.index, snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        let meta = ClusterSnapshotMeta {
            last_log_id: last_applied_log,
            last_membership,
            snapshot_id,
        };

        let snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        *current_snapshot = Some(snapshot);

        Ok(ClusterSnapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

/// Implementation of OpenRaft's state machine interface.
///
/// Applies committed log entries to the cluster state deterministically.
/// All nodes in the cluster apply the same sequence of commands, ensuring
/// consistent state across the cluster.
impl RaftStateMachine<TypeConfig> for Arc<ClusterStateMachine> {
    type SnapshotBuilder = Self;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<ClusterLogId>, ClusterStoredMembership), ClusterStorageError> {
        let state = self.state.read().await;
        Ok((state.last_applied_log, state.last_membership.clone()))
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<ClusterResponse>, ClusterStorageError>
    where
        I: IntoIterator<Item = openraft::Entry<TypeConfig>> + Send,
    {
        let mut responses = Vec::new();
        let mut state = self.state.write().await;

        for entry in entries {
            tracing::debug!(%entry.log_id, "applying to state machine");

            state.last_applied_log = Some(entry.log_id);

            match entry.payload {
                EntryPayload::Blank => {
                    responses.push(ClusterResponse::ok());
                }
                EntryPayload::Normal(cmd) => {
                    responses.push(apply_command(&mut state, cmd));
                }
                EntryPayload::Membership(membership) => {
                    state.last_membership = StoredMembership::new(Some(entry.log_id), membership);
                    responses.push(ClusterResponse::ok());
                }
            }
        }

        Ok(responses)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        Arc::clone(self)
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<<TypeConfig as RaftTypeConfig>::SnapshotData>, ClusterStorageError> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &ClusterSnapshotMeta,
        snapshot: Box<<TypeConfig as RaftTypeConfig>::SnapshotData>,
    ) -> Result<(), ClusterStorageError> {
        tracing::info!(
            snapshot_size = snapshot.get_ref().len(),
            "installing snapshot"
        );

        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        // Update state machine
        let new_state: ClusterState = serde_json::from_slice(&new_snapshot.data)
            .map_err(|e| StorageIOError::read_snapshot(Some(new_snapshot.meta.signature()), &e))?;

        let mut state = self.state.write().await;
        *state = new_state;
        state.last_applied_log = meta.last_log_id;
        state.last_membership = meta.last_membership.clone();

        // Lock snapshot before releasing state lock
        let mut current_snapshot = self.current_snapshot.write().await;
        drop(state);

        *current_snapshot = Some(new_snapshot);
        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<ClusterSnapshot>, ClusterStorageError> {
        match &*self.current_snapshot.read().await {
            Some(snapshot) => {
                let data = snapshot.data.clone();
                Ok(Some(ClusterSnapshot {
                    meta: snapshot.meta.clone(),
                    snapshot: Box::new(Cursor::new(data)),
                }))
            }
            None => Ok(None),
        }
    }
}
