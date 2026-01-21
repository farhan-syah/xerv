//! XervCluster Custom Resource Definition.
//!
//! Defines a XERV cluster deployment in Kubernetes.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// XervCluster is the Schema for the xervclusters API.
///
/// A XervCluster represents a deployed XERV workflow orchestration cluster.
/// The operator will create the necessary Kubernetes resources (StatefulSet/Deployment,
/// Service, ConfigMap, PVC) to run the cluster.
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "xerv.io",
    version = "v1",
    kind = "XervCluster",
    plural = "xervclusters",
    shortname = "xc",
    namespaced,
    status = "XervClusterStatus",
    printcolumn = r#"{"name":"Replicas", "type":"integer", "jsonPath":".spec.replicas"}"#,
    printcolumn = r#"{"name":"Backend", "type":"string", "jsonPath":".spec.dispatch.backend"}"#,
    printcolumn = r#"{"name":"Phase", "type":"string", "jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Age", "type":"date", "jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct XervClusterSpec {
    /// Number of replicas in the cluster.
    /// For Raft backend, should be an odd number (1, 3, 5).
    /// For Redis/NATS backend, can be any positive number.
    #[serde(default = "default_replicas")]
    pub replicas: i32,

    /// Dispatch backend configuration.
    #[serde(default)]
    pub dispatch: DispatchSpec,

    /// Storage configuration for arena and WAL.
    #[serde(default)]
    pub storage: StorageSpec,

    /// Resource requirements for each pod.
    #[serde(default)]
    pub resources: ResourceRequirements,

    /// Image to use for XERV pods.
    #[serde(default = "default_image")]
    pub image: String,

    /// Image pull policy.
    #[serde(default = "default_image_pull_policy")]
    pub image_pull_policy: String,

    /// Service account name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_account: Option<String>,

    /// Node selector for pod placement.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub node_selector: std::collections::BTreeMap<String, String>,

    /// Tolerations for pod scheduling.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tolerations: Vec<Toleration>,

    /// Enable Prometheus metrics endpoint.
    #[serde(default = "default_true")]
    pub metrics_enabled: bool,

    /// Port for the HTTP API.
    #[serde(default = "default_api_port")]
    pub api_port: i32,

    /// Port for gRPC (Raft communication).
    #[serde(default = "default_grpc_port")]
    pub grpc_port: i32,
}

fn default_replicas() -> i32 {
    1
}

fn default_image() -> String {
    "ghcr.io/ml-rust/xerv:latest".to_string()
}

fn default_image_pull_policy() -> String {
    "IfNotPresent".to_string()
}

fn default_true() -> bool {
    true
}

fn default_api_port() -> i32 {
    8080
}

fn default_grpc_port() -> i32 {
    5000
}

/// Dispatch backend specification.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DispatchSpec {
    /// Backend type: raft, redis, or nats.
    #[serde(default = "default_backend")]
    pub backend: String,

    /// Redis configuration (when backend is "redis").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redis: Option<RedisSpec>,

    /// NATS configuration (when backend is "nats").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nats: Option<NatsSpec>,
}

impl Default for DispatchSpec {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            redis: None,
            nats: None,
        }
    }
}

fn default_backend() -> String {
    "raft".to_string()
}

/// Redis backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RedisSpec {
    /// Redis connection URL or service name.
    pub url: String,

    /// Use Redis Streams for message delivery.
    #[serde(default = "default_true")]
    pub use_streams: bool,

    /// Connection pool size.
    #[serde(default = "default_pool_size")]
    pub pool_size: i32,

    /// Consumer group name.
    #[serde(default = "default_consumer_group")]
    pub consumer_group: String,
}

fn default_pool_size() -> i32 {
    10
}

fn default_consumer_group() -> String {
    "xerv-workers".to_string()
}

/// NATS backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NatsSpec {
    /// NATS connection URL or service name.
    pub url: String,

    /// Use JetStream for persistence.
    #[serde(default = "default_true")]
    pub use_jetstream: bool,

    /// Stream name for traces.
    #[serde(default = "default_stream_name")]
    pub stream_name: String,
}

fn default_stream_name() -> String {
    "XERV_TRACES".to_string()
}

/// Storage configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StorageSpec {
    /// Storage class name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,

    /// Storage size (e.g., "10Gi").
    #[serde(default = "default_storage_size")]
    pub size: String,

    /// Access mode (ReadWriteOnce, ReadWriteMany).
    #[serde(default = "default_access_mode")]
    pub access_mode: String,
}

impl Default for StorageSpec {
    fn default() -> Self {
        Self {
            class: None,
            size: default_storage_size(),
            access_mode: default_access_mode(),
        }
    }
}

fn default_storage_size() -> String {
    "10Gi".to_string()
}

fn default_access_mode() -> String {
    "ReadWriteOnce".to_string()
}

/// Resource requirements.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRequirements {
    /// Resource requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requests: Option<ResourceSpec>,

    /// Resource limits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<ResourceSpec>,
}

/// Resource specification (CPU and memory).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSpec {
    /// CPU (e.g., "1", "500m").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,

    /// Memory (e.g., "1Gi", "512Mi").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

/// Kubernetes toleration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Toleration {
    /// Taint key to match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,

    /// Operator (Equal or Exists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,

    /// Taint value to match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    /// Effect (NoSchedule, PreferNoSchedule, NoExecute).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,

    /// Toleration seconds for NoExecute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toleration_seconds: Option<i64>,
}

/// XervCluster status.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct XervClusterStatus {
    /// Current phase of the cluster.
    #[serde(default)]
    pub phase: ClusterPhase,

    /// Number of ready replicas.
    #[serde(default)]
    pub ready_replicas: i32,

    /// Number of available replicas.
    #[serde(default)]
    pub available_replicas: i32,

    /// Current leader node (for Raft backend).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leader: Option<String>,

    /// Cluster endpoint URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Conditions representing the current state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<ClusterCondition>,

    /// Last time the status was updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,

    /// Human-readable message about current state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Cluster phase.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub enum ClusterPhase {
    /// Cluster is being created.
    #[default]
    Pending,
    /// Cluster is starting up.
    Initializing,
    /// Cluster is running and healthy.
    Running,
    /// Cluster is being updated.
    Updating,
    /// Cluster is in a degraded state.
    Degraded,
    /// Cluster has failed.
    Failed,
    /// Cluster is being deleted.
    Terminating,
}

/// Condition representing cluster state.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClusterCondition {
    /// Type of condition (Ready, Available, Progressing).
    #[serde(rename = "type")]
    pub condition_type: String,

    /// Status of the condition (True, False, Unknown).
    pub status: String,

    /// Last time the condition transitioned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_transition_time: Option<String>,

    /// Reason for the condition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Human-readable message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cluster_spec() {
        let spec = XervClusterSpec {
            replicas: 3,
            dispatch: DispatchSpec::default(),
            storage: StorageSpec::default(),
            resources: ResourceRequirements::default(),
            image: default_image(),
            image_pull_policy: default_image_pull_policy(),
            service_account: None,
            node_selector: Default::default(),
            tolerations: vec![],
            metrics_enabled: true,
            api_port: 8080,
            grpc_port: 5000,
        };

        assert_eq!(spec.replicas, 3);
        assert_eq!(spec.dispatch.backend, "raft");
    }

    #[test]
    fn cluster_serialization() {
        let spec = XervClusterSpec {
            replicas: 3,
            dispatch: DispatchSpec {
                backend: "redis".to_string(),
                redis: Some(RedisSpec {
                    url: "redis://redis:6379".to_string(),
                    use_streams: true,
                    pool_size: 10,
                    consumer_group: "xerv-workers".to_string(),
                }),
                nats: None,
            },
            storage: StorageSpec::default(),
            resources: ResourceRequirements::default(),
            image: default_image(),
            image_pull_policy: default_image_pull_policy(),
            service_account: None,
            node_selector: Default::default(),
            tolerations: vec![],
            metrics_enabled: true,
            api_port: 8080,
            grpc_port: 5000,
        };

        let json =
            serde_json::to_string(&spec).expect("Failed to serialize XervClusterSpec to JSON");
        assert!(json.contains("redis"));
        assert!(json.contains("redis://redis:6379"));
    }
}
