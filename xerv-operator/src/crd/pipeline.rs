//! XervPipeline Custom Resource Definition.
//!
//! Defines a pipeline deployment to a XERV cluster.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// XervPipeline is the Schema for the xervpipelines API.
///
/// A XervPipeline represents a workflow pipeline deployed to a XervCluster.
/// The operator will load the pipeline definition and configure triggers.
#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "xerv.io",
    version = "v1",
    kind = "XervPipeline",
    plural = "xervpipelines",
    shortname = "xp",
    namespaced,
    status = "XervPipelineStatus",
    printcolumn = r#"{"name":"Cluster", "type":"string", "jsonPath":".spec.cluster"}"#,
    printcolumn = r#"{"name":"Phase", "type":"string", "jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Triggers", "type":"integer", "jsonPath":".status.activeTriggers"}"#,
    printcolumn = r#"{"name":"Age", "type":"date", "jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct XervPipelineSpec {
    /// Name of the XervCluster to deploy to.
    pub cluster: String,

    /// Pipeline definition source.
    pub source: PipelineSource,

    /// Trigger configurations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<TriggerSpec>,

    /// Whether the pipeline is paused.
    #[serde(default)]
    pub paused: bool,

    /// Environment variables for the pipeline.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub env: std::collections::BTreeMap<String, String>,

    /// Secrets to inject (from Kubernetes secrets).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretRef>,
}

/// Pipeline definition source.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PipelineSource {
    /// Load from a ConfigMap.
    ConfigMap {
        /// ConfigMap name.
        name: String,
        /// Key in the ConfigMap containing the pipeline YAML.
        #[serde(default = "default_config_key")]
        key: String,
    },

    /// Load from a Git repository.
    Git(GitSource),

    /// Inline pipeline definition.
    Inline {
        /// Pipeline YAML content.
        content: String,
    },
}

fn default_config_key() -> String {
    "pipeline.yaml".to_string()
}

/// Git source for pipeline definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GitSource {
    /// Git repository URL.
    pub repo: String,

    /// Branch, tag, or commit to use.
    #[serde(default = "default_branch")]
    pub ref_name: String,

    /// Path to the pipeline file in the repository.
    pub path: String,

    /// Secret containing Git credentials (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_secret: Option<String>,

    /// Interval for checking updates.
    #[serde(default = "default_sync_interval")]
    pub sync_interval: String,
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_sync_interval() -> String {
    "5m".to_string()
}

/// Trigger configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TriggerSpec {
    /// Trigger type.
    #[serde(rename = "type")]
    pub trigger_type: TriggerType,

    /// Trigger name (must be unique within the pipeline).
    pub name: String,

    /// Webhook configuration (when type is webhook).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook: Option<WebhookTrigger>,

    /// Cron configuration (when type is cron).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron: Option<CronTrigger>,

    /// Manual trigger configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual: Option<ManualTrigger>,
}

/// Trigger type.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TriggerType {
    /// HTTP webhook trigger.
    Webhook,
    /// Cron schedule trigger.
    Cron,
    /// Manual trigger (API call).
    Manual,
}

/// Webhook trigger configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookTrigger {
    /// Path for the webhook endpoint.
    pub path: String,

    /// HTTP methods to accept.
    #[serde(default = "default_methods")]
    pub methods: Vec<String>,

    /// Authentication type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<WebhookAuth>,
}

fn default_methods() -> Vec<String> {
    vec!["POST".to_string()]
}

/// Webhook authentication configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WebhookAuth {
    /// Auth type (bearer, basic, hmac).
    #[serde(rename = "type")]
    pub auth_type: String,

    /// Secret containing the credentials.
    pub secret: String,

    /// Key in the secret containing the credential.
    #[serde(default = "default_secret_key")]
    pub key: String,
}

fn default_secret_key() -> String {
    "token".to_string()
}

/// Cron trigger configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CronTrigger {
    /// Cron schedule expression.
    pub schedule: String,

    /// Timezone for the schedule.
    #[serde(default = "default_timezone")]
    pub timezone: String,

    /// Whether to catch up missed runs.
    #[serde(default)]
    pub catch_up: bool,

    /// Concurrency policy (Allow, Forbid, Replace).
    #[serde(default = "default_concurrency_policy")]
    pub concurrency_policy: String,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

fn default_concurrency_policy() -> String {
    "Forbid".to_string()
}

/// Manual trigger configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManualTrigger {
    /// Description of the manual trigger.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Required parameters for the trigger.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<TriggerParameter>,
}

/// Trigger parameter definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TriggerParameter {
    /// Parameter name.
    pub name: String,

    /// Parameter type (string, number, boolean, object).
    #[serde(rename = "type", default = "default_param_type")]
    pub param_type: String,

    /// Whether the parameter is required.
    #[serde(default)]
    pub required: bool,

    /// Default value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_param_type() -> String {
    "string".to_string()
}

/// Reference to a Kubernetes secret.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    /// Secret name.
    pub name: String,

    /// Key in the secret.
    pub key: String,

    /// Environment variable name to inject as.
    pub env_var: String,
}

/// XervPipeline status.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct XervPipelineStatus {
    /// Current phase of the pipeline.
    #[serde(default)]
    pub phase: PipelinePhase,

    /// Number of active triggers.
    #[serde(default)]
    pub active_triggers: i32,

    /// Total traces executed.
    #[serde(default)]
    pub total_traces: i64,

    /// Successful traces (alias: traces_completed).
    #[serde(default)]
    pub successful_traces: i64,

    /// Alias for successful_traces for backward compatibility.
    #[serde(default)]
    pub traces_completed: i64,

    /// Failed traces (alias: traces_failed).
    #[serde(default)]
    pub failed_traces: i64,

    /// Alias for failed_traces for backward compatibility.
    #[serde(default)]
    pub traces_failed: i64,

    /// Error count for retry tracking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_count: Option<i32>,

    /// Last successful trace time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_time: Option<String>,

    /// Last failed trace time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure_time: Option<String>,

    /// Conditions representing the current state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<PipelineCondition>,

    /// Last time the status was updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,

    /// Human-readable message about current state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Observed generation for change detection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
}

/// Pipeline phase.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub enum PipelinePhase {
    /// Pipeline is being deployed.
    #[default]
    Pending,
    /// Pipeline is being validated.
    Validating,
    /// Pipeline is active and receiving triggers.
    Active,
    /// Pipeline is paused.
    Paused,
    /// Pipeline has errors.
    Error,
    /// Pipeline is being deleted.
    Terminating,
}

/// Condition representing pipeline state.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineCondition {
    /// Type of condition (Ready, Valid, Synced).
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
    fn pipeline_with_webhook_trigger() {
        let spec = XervPipelineSpec {
            cluster: "production".to_string(),
            source: PipelineSource::ConfigMap {
                name: "order-flow".to_string(),
                key: "pipeline.yaml".to_string(),
            },
            triggers: vec![TriggerSpec {
                trigger_type: TriggerType::Webhook,
                name: "orders".to_string(),
                webhook: Some(WebhookTrigger {
                    path: "/orders".to_string(),
                    methods: vec!["POST".to_string()],
                    auth: None,
                }),
                cron: None,
                manual: None,
            }],
            paused: false,
            env: Default::default(),
            secrets: vec![],
        };

        assert_eq!(spec.cluster, "production");
        assert_eq!(spec.triggers.len(), 1);
        assert_eq!(spec.triggers[0].trigger_type, TriggerType::Webhook);
    }

    #[test]
    fn pipeline_with_git_source() {
        let spec = XervPipelineSpec {
            cluster: "production".to_string(),
            source: PipelineSource::Git(GitSource {
                repo: "https://github.com/org/flows".to_string(),
                ref_name: "main".to_string(),
                path: "pipelines/orders.yaml".to_string(),
                credentials_secret: Some("git-credentials".to_string()),
                sync_interval: "5m".to_string(),
            }),
            triggers: vec![],
            paused: false,
            env: Default::default(),
            secrets: vec![],
        };

        let json = serde_json::to_string(&spec)
            .expect("Failed to serialize XervPipelineSpec with git source to JSON");
        assert!(json.contains("github.com"));
    }

    #[test]
    fn pipeline_serialization() {
        let spec = XervPipelineSpec {
            cluster: "test".to_string(),
            source: PipelineSource::Inline {
                content: "version: 1\nnodes: []".to_string(),
            },
            triggers: vec![TriggerSpec {
                trigger_type: TriggerType::Cron,
                name: "hourly".to_string(),
                webhook: None,
                cron: Some(CronTrigger {
                    schedule: "0 * * * *".to_string(),
                    timezone: "UTC".to_string(),
                    catch_up: false,
                    concurrency_policy: "Forbid".to_string(),
                }),
                manual: None,
            }],
            paused: false,
            env: Default::default(),
            secrets: vec![],
        };

        let json =
            serde_json::to_string(&spec).expect("Failed to serialize XervPipelineSpec to JSON");
        assert!(json.contains("cron"));
        assert!(json.contains("0 * * * *"));

        let parsed: XervPipelineSpec =
            serde_json::from_str(&json).expect("Failed to parse XervPipelineSpec from JSON");
        assert_eq!(parsed.triggers[0].trigger_type, TriggerType::Cron);
    }
}
