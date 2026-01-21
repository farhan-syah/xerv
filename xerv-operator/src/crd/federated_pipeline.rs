//! FederatedPipeline Custom Resource Definition.
//!
//! Defines a pipeline deployed across multiple clusters in a federation.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// FederatedPipeline is the Schema for the federatedpipelines API.
///
/// A FederatedPipeline represents a XERV pipeline that is deployed and synchronized
/// across multiple clusters in a federation. The federation controller ensures
/// the pipeline definition is consistent across all target clusters.
///
/// # Example
///
/// ```yaml
/// apiVersion: xerv.io/v1
/// kind: FederatedPipeline
/// metadata:
///   name: order-processing
/// spec:
///   federation: global
///   placement:
///     clusters:
///       - us-east
///       - eu-west
///     all: false
///   pipeline:
///     source:
///       configMap:
///         name: order-pipeline
///         key: pipeline.yaml
///     triggers:
///       - name: webhook
///         type: http
///         config:
///           path: /api/orders
/// ```
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "xerv.io",
    version = "v1",
    kind = "FederatedPipeline",
    plural = "federatedpipelines",
    shortname = "fp",
    namespaced,
    status = "FederatedPipelineStatus",
    printcolumn = r#"{"name":"Federation", "type":"string", "jsonPath":".spec.federation"}"#,
    printcolumn = r#"{"name":"Clusters", "type":"integer", "jsonPath":".status.deployedClusters"}"#,
    printcolumn = r#"{"name":"Synced", "type":"string", "jsonPath":".status.syncStatus"}"#,
    printcolumn = r#"{"name":"Age", "type":"date", "jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct FederatedPipelineSpec {
    /// Name of the XervFederation this pipeline belongs to.
    pub federation: String,

    /// Placement rules for where to deploy the pipeline.
    #[serde(default)]
    pub placement: PlacementConfig,

    /// The pipeline definition to deploy.
    pub pipeline: PipelineDefinition,

    /// Override settings per cluster.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<ClusterOverride>,

    /// Rollout strategy for updates.
    #[serde(default)]
    pub rollout: RolloutConfig,
}

/// Placement configuration for the federated pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementConfig {
    /// Deploy to all clusters in the federation.
    #[serde(default)]
    pub all: bool,

    /// Specific clusters to deploy to (if all is false).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<String>,

    /// Label selector to match clusters.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub cluster_selector: std::collections::BTreeMap<String, String>,

    /// Minimum number of clusters that must be healthy for deployment.
    #[serde(default = "default_min_clusters")]
    pub min_clusters: i32,
}

impl Default for PlacementConfig {
    fn default() -> Self {
        Self {
            all: true,
            clusters: vec![],
            cluster_selector: Default::default(),
            min_clusters: 1,
        }
    }
}

fn default_min_clusters() -> i32 {
    1
}

/// Pipeline definition to deploy.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineDefinition {
    /// Source of the pipeline definition.
    pub source: PipelineSource,

    /// Triggers to enable for this pipeline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<TriggerConfig>,

    /// Resource requirements for the pipeline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<PipelineResources>,

    /// Auto-scaling configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoscaling: Option<AutoscalingConfig>,
}

/// Source of the pipeline definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSource {
    /// Inline YAML pipeline definition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline: Option<String>,

    /// Reference to a ConfigMap containing the pipeline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_map: Option<ConfigMapRef>,

    /// Git repository source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<GitSource>,
}

/// Reference to a ConfigMap.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfigMapRef {
    /// Name of the ConfigMap.
    pub name: String,

    /// Key within the ConfigMap.
    #[serde(default = "default_config_key")]
    pub key: String,
}

fn default_config_key() -> String {
    "pipeline.yaml".to_string()
}

/// Git repository source for pipeline definitions.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GitSource {
    /// Git repository URL.
    pub url: String,

    /// Branch, tag, or commit to checkout.
    #[serde(default = "default_git_ref")]
    pub ref_: String,

    /// Path to the pipeline file within the repo.
    #[serde(default = "default_git_path")]
    pub path: String,

    /// Secret containing Git credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_secret: Option<String>,
}

fn default_git_ref() -> String {
    "main".to_string()
}

fn default_git_path() -> String {
    "pipeline.yaml".to_string()
}

/// Trigger configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TriggerConfig {
    /// Trigger name.
    pub name: String,

    /// Trigger type: http, cron, manual.
    #[serde(rename = "type")]
    pub trigger_type: String,

    /// Trigger-specific configuration.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub config: std::collections::BTreeMap<String, String>,

    /// Whether this trigger is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Resource requirements for the pipeline.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineResources {
    /// CPU request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,

    /// Memory request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,

    /// Maximum concurrent traces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent_traces: Option<i32>,
}

/// Autoscaling configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutoscalingConfig {
    /// Enable autoscaling.
    #[serde(default)]
    pub enabled: bool,

    /// Minimum replicas.
    #[serde(default = "default_min_replicas")]
    pub min_replicas: i32,

    /// Maximum replicas.
    #[serde(default = "default_max_replicas")]
    pub max_replicas: i32,

    /// Target queue depth per replica.
    #[serde(default = "default_target_queue")]
    pub target_queue_depth: i32,
}

fn default_min_replicas() -> i32 {
    1
}

fn default_max_replicas() -> i32 {
    10
}

fn default_target_queue() -> i32 {
    100
}

/// Per-cluster override settings.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClusterOverride {
    /// Cluster name to apply overrides to.
    pub cluster: String,

    /// Override resource requirements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<PipelineResources>,

    /// Override autoscaling configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoscaling: Option<AutoscalingConfig>,

    /// Override trigger configuration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<TriggerOverride>,

    /// Additional environment variables.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub env: std::collections::BTreeMap<String, String>,

    /// Disable the pipeline on this cluster.
    #[serde(default)]
    pub disabled: bool,
}

/// Override for a specific trigger.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TriggerOverride {
    /// Trigger name to override.
    pub name: String,

    /// Whether this trigger is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    /// Override configuration values.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub config: std::collections::BTreeMap<String, String>,
}

/// Rollout strategy for updates.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RolloutConfig {
    /// Rollout strategy: all-at-once, rolling, canary.
    #[serde(default)]
    pub strategy: RolloutStrategy,

    /// Maximum clusters to update simultaneously (for rolling).
    #[serde(default = "default_max_surge")]
    pub max_surge: i32,

    /// Pause between cluster updates in seconds (for rolling).
    #[serde(default)]
    pub pause_seconds: i32,

    /// Automatically rollback on failure.
    #[serde(default = "default_true")]
    pub auto_rollback: bool,
}

impl Default for RolloutConfig {
    fn default() -> Self {
        Self {
            strategy: RolloutStrategy::default(),
            max_surge: default_max_surge(),
            pause_seconds: 0,
            auto_rollback: true,
        }
    }
}

fn default_max_surge() -> i32 {
    1
}

/// Rollout strategy.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RolloutStrategy {
    /// Update all clusters simultaneously.
    #[default]
    AllAtOnce,
    /// Update clusters one at a time.
    Rolling,
    /// Canary deployment (one cluster first, then rest).
    Canary,
}

/// FederatedPipeline status.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederatedPipelineStatus {
    /// Current sync status: Synced, Syncing, OutOfSync, Failed.
    #[serde(default)]
    pub sync_status: SyncStatus,

    /// Number of clusters where pipeline is deployed.
    #[serde(default)]
    pub deployed_clusters: i32,

    /// Total target clusters.
    #[serde(default)]
    pub target_clusters: i32,

    /// Status per cluster.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cluster_statuses: Vec<ClusterPipelineStatus>,

    /// Current generation being deployed.
    #[serde(default)]
    pub observed_generation: i64,

    /// Last successful sync time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_time: Option<String>,

    /// Last time the status was updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,

    /// Conditions representing the current state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<FederatedPipelineCondition>,

    /// Human-readable message about current state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Sync status.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub enum SyncStatus {
    /// Pipeline is being created.
    #[default]
    Pending,
    /// Pipeline is being synced to clusters.
    Syncing,
    /// Pipeline is synced across all target clusters.
    Synced,
    /// Pipeline is out of sync (some clusters differ).
    OutOfSync,
    /// Pipeline sync has failed.
    Failed,
}

/// Status of the pipeline on a specific cluster.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClusterPipelineStatus {
    /// Cluster name.
    pub cluster: String,

    /// Whether the pipeline is deployed.
    pub deployed: bool,

    /// Whether the pipeline is in sync.
    pub synced: bool,

    /// Current pipeline version/generation.
    pub generation: i64,

    /// Pipeline status on this cluster.
    pub status: String,

    /// Number of active traces on this cluster.
    #[serde(default)]
    pub active_traces: i32,

    /// Last deployment time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_deployed: Option<String>,

    /// Error message if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Condition representing federated pipeline state.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederatedPipelineCondition {
    /// Type of condition (Ready, Synced, Progressing).
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
    fn default_federated_pipeline_spec() {
        let spec = FederatedPipelineSpec {
            federation: "global".to_string(),
            placement: PlacementConfig::default(),
            pipeline: PipelineDefinition {
                source: PipelineSource {
                    inline: Some("id: test\nnodes: []".to_string()),
                    config_map: None,
                    git: None,
                },
                triggers: vec![],
                resources: None,
                autoscaling: None,
            },
            overrides: vec![],
            rollout: RolloutConfig::default(),
        };

        assert_eq!(spec.federation, "global");
        assert!(spec.placement.all);
    }

    #[test]
    fn federated_pipeline_serialization() {
        let spec = FederatedPipelineSpec {
            federation: "global".to_string(),
            placement: PlacementConfig {
                all: false,
                clusters: vec!["us-east".to_string(), "eu-west".to_string()],
                cluster_selector: Default::default(),
                min_clusters: 2,
            },
            pipeline: PipelineDefinition {
                source: PipelineSource {
                    inline: None,
                    config_map: Some(ConfigMapRef {
                        name: "order-pipeline".to_string(),
                        key: "pipeline.yaml".to_string(),
                    }),
                    git: None,
                },
                triggers: vec![TriggerConfig {
                    name: "webhook".to_string(),
                    trigger_type: "http".to_string(),
                    config: [("path".to_string(), "/api/orders".to_string())]
                        .into_iter()
                        .collect(),
                    enabled: true,
                }],
                resources: None,
                autoscaling: None,
            },
            overrides: vec![ClusterOverride {
                cluster: "eu-west".to_string(),
                resources: None,
                autoscaling: None,
                triggers: vec![],
                env: [("REGION".to_string(), "eu".to_string())]
                    .into_iter()
                    .collect(),
                disabled: false,
            }],
            rollout: RolloutConfig {
                strategy: RolloutStrategy::Rolling,
                max_surge: 1,
                pause_seconds: 30,
                auto_rollback: true,
            },
        };

        let json = serde_json::to_string(&spec)
            .expect("Failed to serialize FederatedPipelineSpec to JSON");
        assert!(json.contains("global"));
        assert!(json.contains("us-east"));
        assert!(json.contains("eu-west"));
        assert!(json.contains("order-pipeline"));
        assert!(json.contains("rolling"));
    }

    #[test]
    fn rollout_strategies() {
        assert_eq!(
            serde_json::to_string(&RolloutStrategy::AllAtOnce)
                .expect("Failed to serialize AllAtOnce strategy"),
            "\"all-at-once\""
        );
        assert_eq!(
            serde_json::to_string(&RolloutStrategy::Rolling)
                .expect("Failed to serialize Rolling strategy"),
            "\"rolling\""
        );
        assert_eq!(
            serde_json::to_string(&RolloutStrategy::Canary)
                .expect("Failed to serialize Canary strategy"),
            "\"canary\""
        );
    }

    #[test]
    fn sync_status_default() {
        let status = FederatedPipelineStatus::default();
        assert_eq!(status.sync_status, SyncStatus::Pending);
        assert_eq!(status.deployed_clusters, 0);
    }
}
