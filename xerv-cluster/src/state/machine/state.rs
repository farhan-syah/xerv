//! Cluster state and serialization helpers.

use crate::types::{ClusterLogId, ClusterStoredMembership};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use xerv_core::types::{PipelineId, TraceId};

use super::types::{PipelineStateInfo, TraceStateInfo};

/// The cluster state that gets replicated.
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct ClusterState {
    /// Active traces: trace_id -> state.
    /// Uses custom serialization for JSON compatibility (HashMap keys must be strings in JSON).
    #[serde(
        serialize_with = "serialize_trace_map",
        deserialize_with = "deserialize_trace_map"
    )]
    pub traces: HashMap<TraceId, TraceStateInfo>,
    /// Deployed pipelines: pipeline_id -> state.
    #[serde(
        serialize_with = "serialize_pipeline_map",
        deserialize_with = "deserialize_pipeline_map"
    )]
    pub pipelines: HashMap<PipelineId, PipelineStateInfo>,
    /// Cluster nodes: cluster_node_id -> info.
    pub nodes: HashMap<u64, super::types::ClusterNodeInfo>,
    /// Last applied log ID.
    pub last_applied_log: Option<ClusterLogId>,
    /// Last membership configuration.
    pub last_membership: ClusterStoredMembership,
}

// Custom serialization for trace map (converts to Vec for JSON compatibility).
// Uses references to avoid cloning during serialization.
fn serialize_trace_map<S>(
    map: &HashMap<TraceId, TraceStateInfo>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(map.len()))?;
    for (k, v) in map {
        // Serialize as tuple without cloning by using references
        seq.serialize_element(&(*k, v))?;
    }
    seq.end()
}

fn deserialize_trace_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<TraceId, TraceStateInfo>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let entries: Vec<(TraceId, TraceStateInfo)> = serde::Deserialize::deserialize(deserializer)?;
    Ok(entries.into_iter().collect())
}

// Custom serialization for pipeline map.
// Uses references to avoid cloning during serialization.
fn serialize_pipeline_map<S>(
    map: &HashMap<PipelineId, PipelineStateInfo>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(map.len()))?;
    for (k, v) in map {
        // Serialize as tuple using references
        seq.serialize_element(&(k, v))?;
    }
    seq.end()
}

fn deserialize_pipeline_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<PipelineId, PipelineStateInfo>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let entries: Vec<(PipelineId, PipelineStateInfo)> =
        serde::Deserialize::deserialize(deserializer)?;
    Ok(entries.into_iter().collect())
}
