//! XervCluster controller.
//!
//! Reconciles XervCluster resources to create and manage XERV cluster deployments.

use super::{ControllerContext, ReconcileAction};
use crate::crd::{ClusterPhase, XervCluster, XervClusterStatus};
use crate::error::{OperatorError, OperatorResult};
use crate::resources;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{ConfigMap, Service};
use kube::api::{Patch, PatchParams, PostParams};
use kube::{Api, ResourceExt};
use std::sync::Arc;

/// Controller for XervCluster resources.
#[derive(Clone)]
pub struct ClusterController {
    ctx: Arc<ControllerContext>,
}

impl ClusterController {
    /// Create a new cluster controller.
    pub fn new(ctx: Arc<ControllerContext>) -> Self {
        Self { ctx }
    }

    /// Reconcile a XervCluster resource.
    ///
    /// This is the main reconciliation loop that:
    /// 1. Validates the cluster spec
    /// 2. Creates/updates the StatefulSet or Deployment
    /// 3. Creates/updates the Service
    /// 4. Creates/updates the ConfigMap
    /// 5. Updates the cluster status
    pub async fn reconcile(&self, cluster: Arc<XervCluster>) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        let namespace = cluster
            .namespace()
            .ok_or_else(|| OperatorError::InvalidConfig("Cluster must be namespaced".into()))?;

        tracing::info!(
            name = %name,
            namespace = %namespace,
            replicas = cluster.spec.replicas,
            backend = %cluster.spec.dispatch.backend,
            "Reconciling XervCluster"
        );

        // Get the API for this namespace
        let clusters: Api<XervCluster> = Api::namespaced(self.ctx.client.clone(), &namespace);

        // Check current phase
        let current_phase = cluster
            .status
            .as_ref()
            .map(|s| &s.phase)
            .unwrap_or(&ClusterPhase::Pending);

        let action = match current_phase {
            ClusterPhase::Pending => self.handle_pending(&cluster, &clusters, &namespace).await?,
            ClusterPhase::Initializing => {
                self.handle_initializing(&cluster, &clusters, &namespace)
                    .await?
            }
            ClusterPhase::Running => self.handle_running(&cluster, &clusters, &namespace).await?,
            ClusterPhase::Updating => {
                self.handle_updating(&cluster, &clusters, &namespace)
                    .await?
            }
            ClusterPhase::Degraded => {
                self.handle_degraded(&cluster, &clusters, &namespace)
                    .await?
            }
            ClusterPhase::Failed => self.handle_failed(&cluster, &clusters, &namespace).await?,
            ClusterPhase::Terminating => {
                self.handle_terminating(&cluster, &clusters, &namespace)
                    .await?
            }
        };

        Ok(action)
    }

    /// Handle pending phase - validate and create resources.
    async fn handle_pending(
        &self,
        cluster: &XervCluster,
        api: &Api<XervCluster>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        tracing::info!(name = %name, "Cluster is pending, validating spec");

        // Validate the spec
        self.validate_spec(&cluster.spec)?;

        // Update status to Initializing
        self.update_status(
            api,
            &name,
            XervClusterStatus {
                phase: ClusterPhase::Initializing,
                message: Some("Creating cluster resources".into()),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
        )
        .await?;

        // Create ConfigMap first (needed by pods)
        self.ensure_configmap(cluster, namespace).await?;

        // Create Services
        self.ensure_headless_service(cluster, namespace).await?;
        self.ensure_api_service(cluster, namespace).await?;

        // Create workload (StatefulSet for Raft, Deployment for Redis/NATS)
        if cluster.spec.dispatch.backend == "raft" {
            self.ensure_statefulset(cluster, namespace).await?;
        } else {
            self.ensure_deployment(cluster, namespace).await?;
        }

        tracing::info!(name = %name, "Cluster resources created, waiting for pods");
        Ok(ReconcileAction::requeue_short())
    }

    /// Ensure ConfigMap exists for the cluster.
    async fn ensure_configmap(&self, cluster: &XervCluster, namespace: &str) -> OperatorResult<()> {
        let cm = resources::build_configmap(cluster, namespace);
        let cm_name = cm.metadata.name.clone().unwrap_or_default();
        let configmaps: Api<ConfigMap> = Api::namespaced(self.ctx.client.clone(), namespace);

        match configmaps.get(&cm_name).await {
            Ok(_existing) => {
                // Update existing ConfigMap
                tracing::debug!(name = %cm_name, "Updating existing ConfigMap");
                configmaps
                    .patch(
                        &cm_name,
                        &PatchParams::apply("xerv-operator"),
                        &Patch::Apply(&cm),
                    )
                    .await?;
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {
                // Create new ConfigMap
                tracing::info!(name = %cm_name, "Creating ConfigMap");
                configmaps.create(&PostParams::default(), &cm).await?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    /// Ensure headless Service exists for the cluster.
    async fn ensure_headless_service(
        &self,
        cluster: &XervCluster,
        namespace: &str,
    ) -> OperatorResult<()> {
        let svc = resources::build_headless_service(cluster, namespace);
        let svc_name = svc.metadata.name.clone().unwrap_or_default();
        let services: Api<Service> = Api::namespaced(self.ctx.client.clone(), namespace);

        match services.get(&svc_name).await {
            Ok(_existing) => {
                tracing::debug!(name = %svc_name, "Headless Service already exists");
                // Services are generally immutable for key fields, skip update
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {
                tracing::info!(name = %svc_name, "Creating headless Service");
                services.create(&PostParams::default(), &svc).await?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    /// Ensure API Service exists for the cluster.
    async fn ensure_api_service(
        &self,
        cluster: &XervCluster,
        namespace: &str,
    ) -> OperatorResult<()> {
        let svc = resources::build_api_service(cluster, namespace);
        let svc_name = svc.metadata.name.clone().unwrap_or_default();
        let services: Api<Service> = Api::namespaced(self.ctx.client.clone(), namespace);

        match services.get(&svc_name).await {
            Ok(_existing) => {
                tracing::debug!(name = %svc_name, "API Service already exists");
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {
                tracing::info!(name = %svc_name, "Creating API Service");
                services.create(&PostParams::default(), &svc).await?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    /// Ensure StatefulSet exists for Raft-based cluster.
    async fn ensure_statefulset(
        &self,
        cluster: &XervCluster,
        namespace: &str,
    ) -> OperatorResult<()> {
        let sts = resources::build_statefulset(cluster, namespace);
        let sts_name = sts.metadata.name.clone().unwrap_or_default();
        let statefulsets: Api<StatefulSet> = Api::namespaced(self.ctx.client.clone(), namespace);

        match statefulsets.get(&sts_name).await {
            Ok(_existing) => {
                tracing::debug!(name = %sts_name, "Updating existing StatefulSet");
                statefulsets
                    .patch(
                        &sts_name,
                        &PatchParams::apply("xerv-operator"),
                        &Patch::Apply(&sts),
                    )
                    .await?;
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {
                tracing::info!(name = %sts_name, "Creating StatefulSet");
                statefulsets.create(&PostParams::default(), &sts).await?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    /// Ensure Deployment exists for Redis/NATS-based cluster.
    async fn ensure_deployment(
        &self,
        cluster: &XervCluster,
        namespace: &str,
    ) -> OperatorResult<()> {
        let deploy = resources::build_deployment(cluster, namespace);
        let deploy_name = deploy.metadata.name.clone().unwrap_or_default();
        let deployments: Api<Deployment> = Api::namespaced(self.ctx.client.clone(), namespace);

        match deployments.get(&deploy_name).await {
            Ok(_existing) => {
                tracing::debug!(name = %deploy_name, "Updating existing Deployment");
                deployments
                    .patch(
                        &deploy_name,
                        &PatchParams::apply("xerv-operator"),
                        &Patch::Apply(&deploy),
                    )
                    .await?;
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {
                tracing::info!(name = %deploy_name, "Creating Deployment");
                deployments.create(&PostParams::default(), &deploy).await?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    /// Handle initializing phase - wait for pods to be ready.
    async fn handle_initializing(
        &self,
        cluster: &XervCluster,
        api: &Api<XervCluster>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        tracing::info!(name = %name, "Cluster is initializing, checking pod status");

        // Check workload status based on backend
        let (ready_replicas, available_replicas) = if cluster.spec.dispatch.backend == "raft" {
            self.get_statefulset_status(&name, namespace).await?
        } else {
            self.get_deployment_status(&name, namespace).await?
        };

        if ready_replicas >= cluster.spec.replicas {
            // Determine endpoint URL
            let endpoint = format!(
                "http://{}.{}.svc.cluster.local:{}",
                name, namespace, cluster.spec.api_port
            );

            self.update_status(
                api,
                &name,
                XervClusterStatus {
                    phase: ClusterPhase::Running,
                    ready_replicas,
                    available_replicas,
                    endpoint: Some(endpoint),
                    message: Some("Cluster is running".into()),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    ..Default::default()
                },
            )
            .await?;

            tracing::info!(name = %name, ready = ready_replicas, "Cluster is now running");
            Ok(ReconcileAction::requeue_long())
        } else {
            tracing::info!(
                name = %name,
                ready = ready_replicas,
                desired = cluster.spec.replicas,
                "Waiting for pods to be ready"
            );
            Ok(ReconcileAction::requeue_short())
        }
    }

    /// Get StatefulSet status (ready and available replicas).
    async fn get_statefulset_status(
        &self,
        name: &str,
        namespace: &str,
    ) -> OperatorResult<(i32, i32)> {
        let statefulsets: Api<StatefulSet> = Api::namespaced(self.ctx.client.clone(), namespace);

        match statefulsets.get(name).await {
            Ok(sts) => {
                let status = sts.status.unwrap_or_default();
                let ready = status.ready_replicas.unwrap_or(0);
                let available = status.available_replicas.unwrap_or(0);
                Ok((ready, available))
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {
                // StatefulSet not found yet, return 0
                Ok((0, 0))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Get Deployment status (ready and available replicas).
    async fn get_deployment_status(
        &self,
        name: &str,
        namespace: &str,
    ) -> OperatorResult<(i32, i32)> {
        let deployments: Api<Deployment> = Api::namespaced(self.ctx.client.clone(), namespace);

        match deployments.get(name).await {
            Ok(deploy) => {
                let status = deploy.status.unwrap_or_default();
                let ready = status.ready_replicas.unwrap_or(0);
                let available = status.available_replicas.unwrap_or(0);
                Ok((ready, available))
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {
                // Deployment not found yet, return 0
                Ok((0, 0))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Handle running phase - monitor health and handle updates.
    async fn handle_running(
        &self,
        cluster: &XervCluster,
        _api: &Api<XervCluster>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        tracing::debug!(name = %name, "Cluster is running, checking for drift");

        // TODO: Check for spec changes that require updates
        // TODO: Check pod health
        // TODO: Update metrics

        // For now, just requeue for periodic check
        Ok(ReconcileAction::requeue_long())
    }

    /// Handle updating phase - rolling update in progress.
    async fn handle_updating(
        &self,
        cluster: &XervCluster,
        _api: &Api<XervCluster>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        tracing::info!(name = %name, "Cluster is updating");

        // TODO: Monitor rolling update progress
        Ok(ReconcileAction::requeue_short())
    }

    /// Handle degraded phase - some pods unhealthy.
    async fn handle_degraded(
        &self,
        cluster: &XervCluster,
        _api: &Api<XervCluster>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        tracing::warn!(name = %name, "Cluster is degraded, attempting recovery");

        // TODO: Attempt to recover unhealthy pods
        Ok(ReconcileAction::requeue_short())
    }

    /// Handle failed phase - cluster has failed.
    async fn handle_failed(
        &self,
        cluster: &XervCluster,
        _api: &Api<XervCluster>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        tracing::error!(name = %name, "Cluster has failed");

        // TODO: Implement recovery strategy
        Ok(ReconcileAction::requeue_medium())
    }

    /// Handle terminating phase - cleanup resources.
    async fn handle_terminating(
        &self,
        cluster: &XervCluster,
        _api: &Api<XervCluster>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        tracing::info!(name = %name, "Cluster is terminating, cleaning up");

        // TODO: Clean up resources
        // Finalizers will handle actual deletion
        Ok(ReconcileAction::Done)
    }

    /// Validate the cluster spec.
    fn validate_spec(&self, spec: &crate::crd::XervClusterSpec) -> OperatorResult<()> {
        // Validate replicas
        if spec.replicas < 1 {
            return Err(OperatorError::ValidationError(
                "Replicas must be at least 1".into(),
            ));
        }

        // For Raft, replicas should be odd
        if spec.dispatch.backend == "raft" && spec.replicas > 1 && spec.replicas % 2 == 0 {
            tracing::warn!(
                replicas = spec.replicas,
                "Raft clusters should have an odd number of replicas for quorum"
            );
        }

        // Validate backend
        match spec.dispatch.backend.as_str() {
            "raft" | "redis" | "nats" | "memory" => {}
            other => {
                return Err(OperatorError::ValidationError(format!(
                    "Unknown dispatch backend: {}",
                    other
                )));
            }
        }

        // Validate Redis config if backend is redis
        if spec.dispatch.backend == "redis" && spec.dispatch.redis.is_none() {
            return Err(OperatorError::ValidationError(
                "Redis backend requires redis configuration".into(),
            ));
        }

        // Validate NATS config if backend is nats
        if spec.dispatch.backend == "nats" && spec.dispatch.nats.is_none() {
            return Err(OperatorError::ValidationError(
                "NATS backend requires nats configuration".into(),
            ));
        }

        Ok(())
    }

    /// Update the cluster status.
    async fn update_status(
        &self,
        api: &Api<XervCluster>,
        name: &str,
        status: XervClusterStatus,
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
    _cluster: Arc<XervCluster>,
    error: &OperatorError,
    _ctx: Arc<ControllerContext>,
) -> kube::runtime::controller::Action {
    tracing::error!(error = %error, "Reconciliation error");
    // Requeue after error with backoff
    kube::runtime::controller::Action::requeue(std::time::Duration::from_secs(30))
}
