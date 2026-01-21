//! XervPipeline controller.
//!
//! Reconciles XervPipeline resources to deploy pipelines to XERV clusters.

use super::{ControllerContext, ReconcileAction};
use crate::crd::{PipelinePhase, PipelineSource, XervCluster, XervPipeline, XervPipelineStatus};
use crate::error::{OperatorError, OperatorResult};
use kube::api::{Patch, PatchParams};
use kube::{Api, ResourceExt};
use std::sync::Arc;

/// Controller for XervPipeline resources.
#[derive(Clone)]
pub struct PipelineController {
    ctx: Arc<ControllerContext>,
}

impl PipelineController {
    /// Create a new pipeline controller.
    pub fn new(ctx: Arc<ControllerContext>) -> Self {
        Self { ctx }
    }

    /// Reconcile a XervPipeline resource.
    ///
    /// This is the main reconciliation loop that:
    /// 1. Validates the pipeline spec
    /// 2. Ensures the target cluster exists and is ready
    /// 3. Loads the pipeline definition
    /// 4. Deploys the pipeline to the cluster
    /// 5. Configures triggers
    /// 6. Updates the pipeline status
    pub async fn reconcile(&self, pipeline: Arc<XervPipeline>) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        let namespace = pipeline
            .namespace()
            .ok_or_else(|| OperatorError::InvalidConfig("Pipeline must be namespaced".into()))?;

        tracing::info!(
            name = %name,
            namespace = %namespace,
            cluster = %pipeline.spec.cluster,
            "Reconciling XervPipeline"
        );

        // Get the API for this namespace
        let pipelines: Api<XervPipeline> = Api::namespaced(self.ctx.client.clone(), &namespace);

        // Check current phase
        let current_phase = pipeline
            .status
            .as_ref()
            .map(|s| &s.phase)
            .unwrap_or(&PipelinePhase::Pending);

        let action = match current_phase {
            PipelinePhase::Pending => {
                self.handle_pending(&pipeline, &pipelines, &namespace)
                    .await?
            }
            PipelinePhase::Validating => {
                self.handle_validating(&pipeline, &pipelines, &namespace)
                    .await?
            }
            PipelinePhase::Active => {
                self.handle_active(&pipeline, &pipelines, &namespace)
                    .await?
            }
            PipelinePhase::Paused => {
                self.handle_paused(&pipeline, &pipelines, &namespace)
                    .await?
            }
            PipelinePhase::Error => self.handle_error(&pipeline, &pipelines, &namespace).await?,
            PipelinePhase::Terminating => {
                self.handle_terminating(&pipeline, &pipelines, &namespace)
                    .await?
            }
        };

        Ok(action)
    }

    /// Handle pending phase - start validation.
    async fn handle_pending(
        &self,
        pipeline: &XervPipeline,
        api: &Api<XervPipeline>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::info!(name = %name, "Pipeline is pending, starting validation");

        // Check if target cluster exists
        let clusters: Api<XervCluster> = Api::namespaced(self.ctx.client.clone(), namespace);
        let cluster =
            clusters
                .get(&pipeline.spec.cluster)
                .await
                .map_err(|_| OperatorError::NotFound {
                    kind: "XervCluster".into(),
                    name: pipeline.spec.cluster.clone(),
                    namespace: namespace.into(),
                })?;

        // Check if cluster is ready
        let cluster_ready = cluster
            .status
            .as_ref()
            .map(|s| s.phase == crate::crd::ClusterPhase::Running)
            .unwrap_or(false);

        if !cluster_ready {
            return Err(OperatorError::ClusterNotReady {
                name: pipeline.spec.cluster.clone(),
                reason: "Cluster is not in Running phase".into(),
            });
        }

        // Update status to Validating
        self.update_status(
            api,
            &name,
            XervPipelineStatus {
                phase: PipelinePhase::Validating,
                message: Some("Validating pipeline definition".into()),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
        )
        .await?;

        Ok(ReconcileAction::requeue_short())
    }

    /// Handle validating phase - validate and load pipeline.
    async fn handle_validating(
        &self,
        pipeline: &XervPipeline,
        api: &Api<XervPipeline>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::info!(name = %name, "Validating pipeline definition");

        // Load pipeline definition based on source
        let pipeline_yaml = match &pipeline.spec.source {
            PipelineSource::ConfigMap { name, key } => {
                self.load_from_configmap(namespace, name, key).await?
            }
            PipelineSource::Git(git) => self.load_from_git(git).await?,
            PipelineSource::Inline { content } => content.clone(),
        };

        // Validate pipeline YAML structure
        self.validate_pipeline_yaml(&pipeline_yaml)?;

        // Deploy pipeline to cluster via API
        let cluster_endpoint = self
            .get_cluster_endpoint(namespace, &pipeline.spec.cluster)
            .await?;
        self.deploy_to_cluster(&cluster_endpoint, &name, &pipeline_yaml)
            .await?;

        // Configure triggers if specified
        for trigger in &pipeline.spec.triggers {
            self.configure_trigger(&cluster_endpoint, &name, trigger)
                .await?;
        }

        // Update status to Active
        let active_triggers = pipeline.spec.triggers.len() as i32;
        self.update_status(
            api,
            &name,
            XervPipelineStatus {
                phase: if pipeline.spec.paused {
                    PipelinePhase::Paused
                } else {
                    PipelinePhase::Active
                },
                active_triggers,
                message: Some("Pipeline deployed successfully".into()),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                observed_generation: pipeline.metadata.generation,
                ..Default::default()
            },
        )
        .await?;

        tracing::info!(
            name = %name,
            triggers = active_triggers,
            "Pipeline deployed and active"
        );
        Ok(ReconcileAction::requeue_long())
    }

    /// Handle active phase - monitor pipeline health.
    async fn handle_active(
        &self,
        pipeline: &XervPipeline,
        api: &Api<XervPipeline>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::debug!(name = %name, "Pipeline is active, monitoring");

        // Check if paused flag changed
        if pipeline.spec.paused {
            let cluster_endpoint = self
                .get_cluster_endpoint(namespace, &pipeline.spec.cluster)
                .await?;
            self.pause_pipeline_in_cluster(&cluster_endpoint, &name)
                .await?;

            self.update_status(
                api,
                &name,
                XervPipelineStatus {
                    phase: PipelinePhase::Paused,
                    message: Some("Pipeline paused by user".into()),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    ..pipeline.status.clone().unwrap_or_default()
                },
            )
            .await?;
            return Ok(ReconcileAction::requeue_short());
        }

        // Check for pipeline definition updates (generation changed)
        let observed_gen = pipeline.status.as_ref().and_then(|s| s.observed_generation);
        let current_gen = pipeline.metadata.generation;

        if observed_gen != current_gen {
            tracing::info!(
                name = %name,
                observed = ?observed_gen,
                current = ?current_gen,
                "Pipeline spec changed, redeploying"
            );

            // Reload and redeploy the pipeline
            let pipeline_yaml = match &pipeline.spec.source {
                PipelineSource::ConfigMap { name, key } => {
                    self.load_from_configmap(namespace, name, key).await?
                }
                PipelineSource::Git(git) => self.load_from_git(git).await?,
                PipelineSource::Inline { content } => content.clone(),
            };

            self.validate_pipeline_yaml(&pipeline_yaml)?;

            let cluster_endpoint = self
                .get_cluster_endpoint(namespace, &pipeline.spec.cluster)
                .await?;
            self.deploy_to_cluster(&cluster_endpoint, &name, &pipeline_yaml)
                .await?;

            self.update_status(
                api,
                &name,
                XervPipelineStatus {
                    phase: PipelinePhase::Active,
                    message: Some("Pipeline updated".into()),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    observed_generation: current_gen,
                    ..pipeline.status.clone().unwrap_or_default()
                },
            )
            .await?;
        }

        // Update metrics from cluster
        if let Ok(cluster_endpoint) = self
            .get_cluster_endpoint(namespace, &pipeline.spec.cluster)
            .await
        {
            if let Ok(metrics) = self.fetch_pipeline_metrics(&cluster_endpoint, &name).await {
                self.update_status(
                    api,
                    &name,
                    XervPipelineStatus {
                        traces_completed: metrics.traces_completed,
                        traces_failed: metrics.traces_failed,
                        last_updated: Some(chrono::Utc::now().to_rfc3339()),
                        ..pipeline.status.clone().unwrap_or_default()
                    },
                )
                .await?;
            }
        }

        Ok(ReconcileAction::requeue_long())
    }

    /// Handle paused phase - wait for unpause.
    async fn handle_paused(
        &self,
        pipeline: &XervPipeline,
        api: &Api<XervPipeline>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::debug!(name = %name, "Pipeline is paused");

        // Check if unpaused
        if !pipeline.spec.paused {
            let cluster_endpoint = self
                .get_cluster_endpoint(namespace, &pipeline.spec.cluster)
                .await?;
            self.resume_pipeline_in_cluster(&cluster_endpoint, &name)
                .await?;

            self.update_status(
                api,
                &name,
                XervPipelineStatus {
                    phase: PipelinePhase::Active,
                    message: Some("Pipeline resumed".into()),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    ..pipeline.status.clone().unwrap_or_default()
                },
            )
            .await?;
            return Ok(ReconcileAction::requeue_short());
        }

        Ok(ReconcileAction::requeue_long())
    }

    /// Handle error phase - attempt recovery.
    async fn handle_error(
        &self,
        pipeline: &XervPipeline,
        api: &Api<XervPipeline>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        let error_count = pipeline
            .status
            .as_ref()
            .and_then(|s| s.error_count)
            .unwrap_or(0);

        tracing::warn!(
            name = %name,
            error_count = error_count,
            "Pipeline is in error state, attempting recovery"
        );

        // Exponential backoff: don't retry too frequently
        if error_count > 5 {
            tracing::error!(
                name = %name,
                "Pipeline has failed too many times, manual intervention required"
            );
            return Ok(ReconcileAction::requeue_long());
        }

        // Attempt to redeploy
        let cluster_endpoint = match self
            .get_cluster_endpoint(namespace, &pipeline.spec.cluster)
            .await
        {
            Ok(ep) => ep,
            Err(e) => {
                tracing::warn!(error = %e, "Cannot reach cluster, will retry");
                self.update_status(
                    api,
                    &name,
                    XervPipelineStatus {
                        error_count: Some(error_count + 1),
                        message: Some(format!(
                            "Recovery attempt {} failed: {}",
                            error_count + 1,
                            e
                        )),
                        last_updated: Some(chrono::Utc::now().to_rfc3339()),
                        ..pipeline.status.clone().unwrap_or_default()
                    },
                )
                .await?;
                return Ok(ReconcileAction::requeue_medium());
            }
        };

        // Try to reload and redeploy
        let pipeline_yaml = match &pipeline.spec.source {
            PipelineSource::ConfigMap { name, key } => {
                self.load_from_configmap(namespace, name, key).await?
            }
            PipelineSource::Git(git) => self.load_from_git(git).await?,
            PipelineSource::Inline { content } => content.clone(),
        };

        match self
            .deploy_to_cluster(&cluster_endpoint, &name, &pipeline_yaml)
            .await
        {
            Ok(()) => {
                tracing::info!(name = %name, "Recovery successful, pipeline redeployed");
                self.update_status(
                    api,
                    &name,
                    XervPipelineStatus {
                        phase: PipelinePhase::Active,
                        message: Some("Recovered from error".into()),
                        error_count: Some(0),
                        last_updated: Some(chrono::Utc::now().to_rfc3339()),
                        ..Default::default()
                    },
                )
                .await?;
                Ok(ReconcileAction::requeue_short())
            }
            Err(e) => {
                self.update_status(
                    api,
                    &name,
                    XervPipelineStatus {
                        error_count: Some(error_count + 1),
                        message: Some(format!(
                            "Recovery attempt {} failed: {}",
                            error_count + 1,
                            e
                        )),
                        last_updated: Some(chrono::Utc::now().to_rfc3339()),
                        ..pipeline.status.clone().unwrap_or_default()
                    },
                )
                .await?;
                Ok(ReconcileAction::requeue_medium())
            }
        }
    }

    /// Handle terminating phase - cleanup.
    async fn handle_terminating(
        &self,
        pipeline: &XervPipeline,
        _api: &Api<XervPipeline>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::info!(name = %name, "Pipeline is terminating, cleaning up");

        // Get cluster endpoint (if available)
        if let Ok(cluster_endpoint) = self
            .get_cluster_endpoint(namespace, &pipeline.spec.cluster)
            .await
        {
            // Remove pipeline from cluster
            if let Err(e) = self
                .remove_pipeline_from_cluster(&cluster_endpoint, &name)
                .await
            {
                tracing::warn!(error = %e, "Failed to remove pipeline from cluster");
            }

            // Clean up triggers
            for trigger in &pipeline.spec.triggers {
                if let Err(e) = self.remove_trigger(&cluster_endpoint, &name, trigger).await {
                    tracing::warn!(
                        trigger = %trigger.name,
                        error = %e,
                        "Failed to remove trigger"
                    );
                }
            }
        }

        tracing::info!(name = %name, "Pipeline cleanup complete");
        Ok(ReconcileAction::Done)
    }

    /// Validate pipeline YAML structure.
    fn validate_pipeline_yaml(&self, yaml: &str) -> OperatorResult<()> {
        // Parse YAML to check structure
        let doc: serde_yaml::Value = serde_yaml::from_str(yaml)
            .map_err(|e| OperatorError::ValidationError(format!("Invalid YAML: {}", e)))?;

        // Check required fields
        let obj = doc.as_mapping().ok_or_else(|| {
            OperatorError::ValidationError("Pipeline YAML must be a mapping".into())
        })?;

        if !obj.contains_key(serde_yaml::Value::String("name".to_string())) {
            return Err(OperatorError::ValidationError(
                "Pipeline must have a 'name' field".into(),
            ));
        }

        if !obj.contains_key(serde_yaml::Value::String("nodes".to_string())) {
            return Err(OperatorError::ValidationError(
                "Pipeline must have a 'nodes' field".into(),
            ));
        }

        tracing::debug!("Pipeline YAML validation passed");
        Ok(())
    }

    /// Get the endpoint URL for a cluster.
    async fn get_cluster_endpoint(
        &self,
        namespace: &str,
        cluster_name: &str,
    ) -> OperatorResult<String> {
        let clusters: Api<XervCluster> = Api::namespaced(self.ctx.client.clone(), namespace);
        let cluster = clusters
            .get(cluster_name)
            .await
            .map_err(|_| OperatorError::NotFound {
                kind: "XervCluster".into(),
                name: cluster_name.into(),
                namespace: namespace.into(),
            })?;

        cluster
            .status
            .as_ref()
            .and_then(|s| s.endpoint.clone())
            .ok_or_else(|| OperatorError::ClusterNotReady {
                name: cluster_name.into(),
                reason: "Cluster endpoint not available".into(),
            })
    }

    /// Deploy pipeline to cluster via REST API.
    async fn deploy_to_cluster(
        &self,
        endpoint: &str,
        pipeline_name: &str,
        yaml: &str,
    ) -> OperatorResult<()> {
        let url = format!("{}/api/v1/pipelines", endpoint);

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("Content-Type", "application/x-yaml")
            .body(yaml.to_string())
            .send()
            .await
            .map_err(|e| OperatorError::ApiError(format!("Failed to deploy pipeline: {}", e)))?;

        if resp.status().is_success() || resp.status().as_u16() == 409 {
            // 409 Conflict means pipeline already exists, which is OK for updates
            tracing::debug!(pipeline = %pipeline_name, "Pipeline deployed to cluster");
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(OperatorError::ApiError(format!(
                "Deploy failed ({}): {}",
                status, body
            )))
        }
    }

    /// Remove pipeline from cluster via REST API.
    async fn remove_pipeline_from_cluster(
        &self,
        endpoint: &str,
        pipeline_name: &str,
    ) -> OperatorResult<()> {
        let url = format!("{}/api/v1/pipelines/{}", endpoint, pipeline_name);

        let client = reqwest::Client::new();
        let resp =
            client.delete(&url).send().await.map_err(|e| {
                OperatorError::ApiError(format!("Failed to remove pipeline: {}", e))
            })?;

        if resp.status().is_success() || resp.status().as_u16() == 404 {
            tracing::debug!(pipeline = %pipeline_name, "Pipeline removed from cluster");
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(OperatorError::ApiError(format!(
                "Remove failed ({}): {}",
                status, body
            )))
        }
    }

    /// Pause pipeline in cluster.
    async fn pause_pipeline_in_cluster(
        &self,
        endpoint: &str,
        pipeline_name: &str,
    ) -> OperatorResult<()> {
        let url = format!("{}/api/v1/pipelines/{}/pause", endpoint, pipeline_name);

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .send()
            .await
            .map_err(|e| OperatorError::ApiError(format!("Failed to pause pipeline: {}", e)))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(OperatorError::ApiError("Pause failed".into()))
        }
    }

    /// Resume pipeline in cluster.
    async fn resume_pipeline_in_cluster(
        &self,
        endpoint: &str,
        pipeline_name: &str,
    ) -> OperatorResult<()> {
        let url = format!("{}/api/v1/pipelines/{}/resume", endpoint, pipeline_name);

        let client = reqwest::Client::new();
        let resp =
            client.post(&url).send().await.map_err(|e| {
                OperatorError::ApiError(format!("Failed to resume pipeline: {}", e))
            })?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(OperatorError::ApiError("Resume failed".into()))
        }
    }

    /// Configure a trigger in the cluster.
    async fn configure_trigger(
        &self,
        endpoint: &str,
        pipeline_name: &str,
        trigger: &crate::crd::PipelineTrigger,
    ) -> OperatorResult<()> {
        let url = format!("{}/api/v1/pipelines/{}/triggers", endpoint, pipeline_name);

        let client = reqwest::Client::new();
        let resp =
            client.post(&url).json(&trigger).send().await.map_err(|e| {
                OperatorError::ApiError(format!("Failed to configure trigger: {}", e))
            })?;

        if resp.status().is_success() || resp.status().as_u16() == 409 {
            tracing::debug!(
                pipeline = %pipeline_name,
                trigger = %trigger.name,
                "Trigger configured"
            );
            Ok(())
        } else {
            Err(OperatorError::ApiError(
                "Trigger configuration failed".into(),
            ))
        }
    }

    /// Remove a trigger from the cluster.
    async fn remove_trigger(
        &self,
        endpoint: &str,
        pipeline_name: &str,
        trigger: &crate::crd::PipelineTrigger,
    ) -> OperatorResult<()> {
        let url = format!(
            "{}/api/v1/pipelines/{}/triggers/{}",
            endpoint, pipeline_name, trigger.name
        );

        let client = reqwest::Client::new();
        let resp = client
            .delete(&url)
            .send()
            .await
            .map_err(|e| OperatorError::ApiError(format!("Failed to remove trigger: {}", e)))?;

        if resp.status().is_success() || resp.status().as_u16() == 404 {
            Ok(())
        } else {
            Err(OperatorError::ApiError("Trigger removal failed".into()))
        }
    }

    /// Fetch pipeline metrics from cluster.
    async fn fetch_pipeline_metrics(
        &self,
        endpoint: &str,
        pipeline_name: &str,
    ) -> OperatorResult<PipelineMetrics> {
        let url = format!("{}/api/v1/pipelines/{}/metrics", endpoint, pipeline_name);

        let client = reqwest::Client::new();
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| OperatorError::ApiError(format!("Failed to fetch metrics: {}", e)))?;

        if resp.status().is_success() {
            resp.json::<PipelineMetrics>()
                .await
                .map_err(|e| OperatorError::ApiError(format!("Failed to parse metrics: {}", e)))
        } else {
            Err(OperatorError::ApiError("Metrics fetch failed".into()))
        }
    }

    /// Load pipeline definition from a ConfigMap.
    async fn load_from_configmap(
        &self,
        namespace: &str,
        name: &str,
        key: &str,
    ) -> OperatorResult<String> {
        use k8s_openapi::api::core::v1::ConfigMap;

        let configmaps: Api<ConfigMap> = Api::namespaced(self.ctx.client.clone(), namespace);
        let cm = configmaps
            .get(name)
            .await
            .map_err(|_| OperatorError::NotFound {
                kind: "ConfigMap".into(),
                name: name.into(),
                namespace: namespace.into(),
            })?;

        cm.data
            .and_then(|data| data.get(key).cloned())
            .ok_or_else(|| {
                OperatorError::InvalidConfig(format!("Key '{}' not found in ConfigMap", key))
            })
    }

    /// Load pipeline definition from a Git repository.
    async fn load_from_git(&self, git: &crate::crd::GitSource) -> OperatorResult<String> {
        use std::process::Command;

        // Create temporary directory for clone
        let temp_dir = std::env::temp_dir().join(format!("xerv-git-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir)
            .map_err(|e| OperatorError::IoError(format!("Failed to create temp dir: {}", e)))?;

        // Build git clone command
        let mut cmd = Command::new("git");
        cmd.arg("clone")
            .arg("--depth")
            .arg("1")
            .arg("--single-branch");

        if !git.ref_name.is_empty() {
            cmd.arg("--branch").arg(&git.ref_name);
        }

        cmd.arg(&git.repo).arg(&temp_dir);

        // Execute clone
        let output = cmd
            .output()
            .map_err(|e| OperatorError::IoError(format!("Git clone failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Clean up temp dir
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Err(OperatorError::IoError(format!(
                "Git clone failed: {}",
                stderr
            )));
        }

        // Read the pipeline file
        let file_path = temp_dir.join(&git.path);
        let content = std::fs::read_to_string(&file_path).map_err(|e| {
            let _ = std::fs::remove_dir_all(&temp_dir);
            OperatorError::IoError(format!("Failed to read file '{}': {}", git.path, e))
        })?;

        // Clean up temp dir
        let _ = std::fs::remove_dir_all(&temp_dir);

        tracing::info!(
            repository = %git.repo,
            path = %git.path,
            "Loaded pipeline from Git"
        );

        Ok(content)
    }

    /// Update the pipeline status.
    async fn update_status(
        &self,
        api: &Api<XervPipeline>,
        name: &str,
        status: XervPipelineStatus,
    ) -> OperatorResult<()> {
        let patch = serde_json::json!({
            "status": status
        });

        api.patch_status(name, &PatchParams::default(), &Patch::Merge(&patch))
            .await?;

        Ok(())
    }
}

/// Pipeline metrics from cluster.
#[derive(Debug, serde::Deserialize)]
struct PipelineMetrics {
    #[serde(default)]
    traces_completed: i64,
    #[serde(default)]
    traces_failed: i64,
}

/// Handle errors during reconciliation.
pub fn error_policy(
    _pipeline: Arc<XervPipeline>,
    error: &OperatorError,
    _ctx: Arc<ControllerContext>,
) -> kube::runtime::controller::Action {
    tracing::error!(error = %error, "Reconciliation error");
    // Requeue after error with backoff
    kube::runtime::controller::Action::requeue(std::time::Duration::from_secs(30))
}
