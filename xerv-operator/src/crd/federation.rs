//! XervFederation Custom Resource Definition.
//!
//! Defines a federation of XERV clusters across multiple regions.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// XervFederation is the Schema for the xervfederations API.
///
/// A XervFederation represents a group of XERV clusters that work together
/// across multiple regions. The federation controller manages cluster membership,
/// pipeline distribution, and intelligent routing across the federated clusters.
///
/// # Example
///
/// ```yaml
/// apiVersion: xerv.io/v1
/// kind: XervFederation
/// metadata:
///   name: global
/// spec:
///   clusters:
///     - name: us-east
///       endpoint: https://xerv-us-east.example.com
///       region: us-east-1
///     - name: eu-west
///       endpoint: https://xerv-eu-west.example.com
///       region: eu-west-1
///   routing:
///     default: nearest
///     rules:
///       - pipeline: eu-orders
///         cluster: eu-west
/// ```
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "xerv.io",
    version = "v1",
    kind = "XervFederation",
    plural = "xervfederations",
    shortname = "xf",
    namespaced,
    status = "XervFederationStatus",
    printcolumn = r#"{"name":"Clusters", "type":"integer", "jsonPath":".status.clusterCount"}"#,
    printcolumn = r#"{"name":"Healthy", "type":"integer", "jsonPath":".status.healthyClusterCount"}"#,
    printcolumn = r#"{"name":"Phase", "type":"string", "jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Age", "type":"date", "jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct XervFederationSpec {
    /// Member clusters in this federation.
    pub clusters: Vec<FederationMember>,

    /// Routing configuration for the federation.
    #[serde(default)]
    pub routing: RoutingConfig,

    /// Health check configuration.
    #[serde(default)]
    pub health_check: HealthCheckConfig,

    /// Security configuration for cross-cluster communication.
    #[serde(default)]
    pub security: SecurityConfig,

    /// Sync configuration for pipeline distribution.
    #[serde(default)]
    pub sync: SyncConfig,
}

/// A member cluster in the federation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederationMember {
    /// Unique name for this cluster within the federation.
    pub name: String,

    /// API endpoint URL for the cluster.
    pub endpoint: String,

    /// Cloud region where this cluster is deployed.
    pub region: String,

    /// Optional zone within the region.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,

    /// Reference to a Kubernetes secret containing auth credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_secret: Option<String>,

    /// Weight for weighted routing (default: 100).
    #[serde(default = "default_weight")]
    pub weight: i32,

    /// Whether this cluster is enabled for routing.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Labels for affinity-based routing.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub labels: std::collections::BTreeMap<String, String>,
}

fn default_weight() -> i32 {
    100
}

fn default_true() -> bool {
    true
}

/// Routing configuration for the federation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutingConfig {
    /// Default routing strategy: nearest, round-robin, weighted, random.
    #[serde(default = "default_routing_strategy")]
    pub default_strategy: RoutingStrategy,

    /// Pipeline-specific routing rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RoutingRule>,

    /// Fallback cluster if primary is unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_cluster: Option<String>,

    /// Enable automatic failover to healthy clusters.
    #[serde(default = "default_true")]
    pub auto_failover: bool,
}

fn default_routing_strategy() -> RoutingStrategy {
    RoutingStrategy::Nearest
}

/// Routing strategy for the federation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RoutingStrategy {
    /// Route to the geographically nearest cluster.
    #[default]
    Nearest,
    /// Round-robin across all healthy clusters.
    RoundRobin,
    /// Weighted distribution based on cluster weights.
    Weighted,
    /// Random selection among healthy clusters.
    Random,
    /// Route based on label affinity.
    Affinity,
}

/// A routing rule for specific pipelines or patterns.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutingRule {
    /// Pipeline name or glob pattern to match.
    pub pipeline: String,

    /// Target cluster for this rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster: Option<String>,

    /// Strategy to use for this pipeline (overrides default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<RoutingStrategy>,

    /// Label selector for affinity-based routing.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub affinity_labels: std::collections::BTreeMap<String, String>,

    /// Priority of this rule (higher = more specific).
    #[serde(default)]
    pub priority: i32,
}

/// Health check configuration for member clusters.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckConfig {
    /// Interval between health checks in seconds.
    #[serde(default = "default_health_interval")]
    pub interval_seconds: i32,

    /// Timeout for health check requests in seconds.
    #[serde(default = "default_health_timeout")]
    pub timeout_seconds: i32,

    /// Number of consecutive failures before marking unhealthy.
    #[serde(default = "default_failure_threshold")]
    pub failure_threshold: i32,

    /// Number of consecutive successes before marking healthy.
    #[serde(default = "default_success_threshold")]
    pub success_threshold: i32,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            interval_seconds: default_health_interval(),
            timeout_seconds: default_health_timeout(),
            failure_threshold: default_failure_threshold(),
            success_threshold: default_success_threshold(),
        }
    }
}

fn default_health_interval() -> i32 {
    30
}

fn default_health_timeout() -> i32 {
    10
}

fn default_failure_threshold() -> i32 {
    3
}

fn default_success_threshold() -> i32 {
    1
}

/// Security configuration for cross-cluster communication.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecurityConfig {
    /// Enable mTLS for cluster-to-cluster communication.
    #[serde(default = "default_true")]
    pub mtls_enabled: bool,

    /// Reference to a secret containing CA certificate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_secret: Option<String>,

    /// Reference to a secret containing client certificate and key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_secret: Option<String>,

    /// Verify server certificates.
    #[serde(default = "default_true")]
    pub verify_server: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            mtls_enabled: true,
            ca_secret: None,
            cert_secret: None,
            verify_server: true,
        }
    }
}

/// Sync configuration for pipeline distribution.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfig {
    /// Enable automatic pipeline sync across clusters.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Sync interval in seconds.
    #[serde(default = "default_sync_interval")]
    pub interval_seconds: i32,

    /// Conflict resolution strategy: source-wins, target-wins, newest-wins.
    #[serde(default)]
    pub conflict_resolution: ConflictResolution,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_seconds: default_sync_interval(),
            conflict_resolution: ConflictResolution::default(),
        }
    }
}

fn default_sync_interval() -> i32 {
    60
}

/// Conflict resolution strategy for pipeline sync.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictResolution {
    /// Source cluster (where change originated) wins.
    #[default]
    SourceWins,
    /// Target cluster (local) wins.
    TargetWins,
    /// Newest version based on timestamp wins.
    NewestWins,
}

/// XervFederation status.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct XervFederationStatus {
    /// Current phase of the federation.
    #[serde(default)]
    pub phase: FederationPhase,

    /// Total number of member clusters.
    #[serde(default)]
    pub cluster_count: i32,

    /// Number of healthy clusters.
    #[serde(default)]
    pub healthy_cluster_count: i32,

    /// Status of each member cluster.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cluster_statuses: Vec<ClusterMemberStatus>,

    /// Number of pipelines synced across the federation.
    #[serde(default)]
    pub synced_pipelines: i32,

    /// Last successful sync time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_time: Option<String>,

    /// Conditions representing the current state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<FederationCondition>,

    /// Last time the status was updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,

    /// Human-readable message about current state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Federation phase.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub enum FederationPhase {
    /// Federation is being created.
    #[default]
    Pending,
    /// Federation is initializing connections.
    Initializing,
    /// Federation is healthy and syncing.
    Healthy,
    /// Federation is partially healthy (some clusters down).
    Degraded,
    /// Federation has failed.
    Failed,
    /// Federation is being deleted.
    Terminating,
}

/// Status of a member cluster.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClusterMemberStatus {
    /// Cluster name.
    pub name: String,

    /// Whether the cluster is reachable.
    pub reachable: bool,

    /// Whether the cluster is healthy.
    pub healthy: bool,

    /// Last successful health check time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_health_check: Option<String>,

    /// Latency to the cluster in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<i64>,

    /// Number of pipelines on this cluster.
    #[serde(default)]
    pub pipeline_count: i32,

    /// Current load (active traces).
    #[serde(default)]
    pub active_traces: i32,

    /// Error message if unhealthy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Condition representing federation state.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederationCondition {
    /// Type of condition (Ready, Syncing, Degraded).
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
    fn default_federation_spec() {
        let spec = XervFederationSpec {
            clusters: vec![FederationMember {
                name: "us-east".to_string(),
                endpoint: "https://xerv-us-east.example.com".to_string(),
                region: "us-east-1".to_string(),
                zone: None,
                credentials_secret: None,
                weight: 100,
                enabled: true,
                labels: Default::default(),
            }],
            routing: RoutingConfig::default(),
            health_check: HealthCheckConfig::default(),
            security: SecurityConfig::default(),
            sync: SyncConfig::default(),
        };

        assert_eq!(spec.clusters.len(), 1);
        assert_eq!(spec.routing.default_strategy, RoutingStrategy::Nearest);
    }

    #[test]
    fn federation_serialization() {
        let spec = XervFederationSpec {
            clusters: vec![
                FederationMember {
                    name: "us-east".to_string(),
                    endpoint: "https://xerv-us-east.example.com".to_string(),
                    region: "us-east-1".to_string(),
                    zone: Some("us-east-1a".to_string()),
                    credentials_secret: Some("xerv-us-east-creds".to_string()),
                    weight: 100,
                    enabled: true,
                    labels: Default::default(),
                },
                FederationMember {
                    name: "eu-west".to_string(),
                    endpoint: "https://xerv-eu-west.example.com".to_string(),
                    region: "eu-west-1".to_string(),
                    zone: None,
                    credentials_secret: None,
                    weight: 50,
                    enabled: true,
                    labels: Default::default(),
                },
            ],
            routing: RoutingConfig {
                default_strategy: RoutingStrategy::Weighted,
                rules: vec![RoutingRule {
                    pipeline: "eu-orders".to_string(),
                    cluster: Some("eu-west".to_string()),
                    strategy: None,
                    affinity_labels: Default::default(),
                    priority: 10,
                }],
                fallback_cluster: Some("us-east".to_string()),
                auto_failover: true,
            },
            health_check: HealthCheckConfig::default(),
            security: SecurityConfig::default(),
            sync: SyncConfig::default(),
        };

        let json =
            serde_json::to_string(&spec).expect("Failed to serialize XervFederationSpec to JSON");
        assert!(json.contains("us-east"));
        assert!(json.contains("eu-west"));
        assert!(json.contains("weighted"));
        assert!(json.contains("eu-orders"));
    }

    #[test]
    fn routing_strategies() {
        assert_eq!(
            serde_json::to_string(&RoutingStrategy::Nearest)
                .expect("Failed to serialize Nearest strategy"),
            "\"nearest\""
        );
        assert_eq!(
            serde_json::to_string(&RoutingStrategy::RoundRobin)
                .expect("Failed to serialize RoundRobin strategy"),
            "\"round-robin\""
        );
        assert_eq!(
            serde_json::to_string(&RoutingStrategy::Weighted)
                .expect("Failed to serialize Weighted strategy"),
            "\"weighted\""
        );
    }

    #[test]
    fn federation_status_default() {
        let status = XervFederationStatus::default();
        assert_eq!(status.phase, FederationPhase::Pending);
        assert_eq!(status.cluster_count, 0);
        assert_eq!(status.healthy_cluster_count, 0);
    }
}
