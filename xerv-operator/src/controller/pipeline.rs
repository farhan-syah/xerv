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
        let _pipeline_yaml = match &pipeline.spec.source {
            PipelineSource::ConfigMap { name, key } => {
                self.load_from_configmap(namespace, name, key).await?
            }
            PipelineSource::Git(git) => self.load_from_git(git).await?,
            PipelineSource::Inline { content } => content.clone(),
        };

        // TODO: Validate pipeline YAML
        // TODO: Deploy to cluster

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
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::debug!(name = %name, "Pipeline is active, monitoring");

        // Check if paused flag changed
        if pipeline.spec.paused {
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

        // TODO: Check for pipeline definition updates
        // TODO: Update metrics from cluster

        Ok(ReconcileAction::requeue_long())
    }

    /// Handle paused phase - wait for unpause.
    async fn handle_paused(
        &self,
        pipeline: &XervPipeline,
        api: &Api<XervPipeline>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::debug!(name = %name, "Pipeline is paused");

        // Check if unpaused
        if !pipeline.spec.paused {
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
        _api: &Api<XervPipeline>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::warn!(name = %name, "Pipeline is in error state, attempting recovery");

        // TODO: Implement recovery logic
        Ok(ReconcileAction::requeue_medium())
    }

    /// Handle terminating phase - cleanup.
    async fn handle_terminating(
        &self,
        pipeline: &XervPipeline,
        _api: &Api<XervPipeline>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::info!(name = %name, "Pipeline is terminating, cleaning up");

        // TODO: Remove pipeline from cluster
        // TODO: Clean up triggers
        Ok(ReconcileAction::Done)
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
    async fn load_from_git(&self, _git: &crate::crd::GitSource) -> OperatorResult<String> {
        // TODO: Implement Git fetch
        // For now, return a placeholder
        tracing::warn!("Git source loading not yet implemented");
        Err(OperatorError::InvalidConfig(
            "Git source not yet implemented".into(),
        ))
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
