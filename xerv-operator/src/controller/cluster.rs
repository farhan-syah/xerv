//! XervCluster controller.
//!
//! Reconciles XervCluster resources to create and manage XERV cluster deployments.

use super::{ControllerContext, ReconcileAction};
use crate::crd::{ClusterPhase, XervCluster, XervClusterStatus};
use crate::error::{OperatorError, OperatorResult};
use crate::resources;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{ConfigMap, Pod, Service};
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
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
                observed_generation: cluster.metadata.generation,
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
                    observed_generation: cluster.metadata.generation,
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
        api: &Api<XervCluster>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        tracing::debug!(name = %name, "Cluster is running, checking for drift");

        // Check for spec changes that require updates (generation changed)
        let observed_gen = cluster.status.as_ref().and_then(|s| s.observed_generation);
        let current_gen = cluster.metadata.generation;

        if observed_gen != current_gen {
            tracing::info!(
                name = %name,
                observed = ?observed_gen,
                current = ?current_gen,
                "Cluster spec changed, transitioning to Updating phase"
            );

            self.update_status(
                api,
                &name,
                XervClusterStatus {
                    phase: ClusterPhase::Updating,
                    message: Some("Applying configuration changes".into()),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    ..cluster.status.clone().unwrap_or_default()
                },
            )
            .await?;

            // Reapply resources
            self.ensure_configmap(cluster, namespace).await?;
            if cluster.spec.dispatch.backend == "raft" {
                self.ensure_statefulset(cluster, namespace).await?;
            } else {
                self.ensure_deployment(cluster, namespace).await?;
            }

            return Ok(ReconcileAction::requeue_short());
        }

        // Check pod health
        let (healthy_pods, unhealthy_pods) = self.check_pod_health(&name, namespace).await?;
        let total_pods = healthy_pods + unhealthy_pods;

        if unhealthy_pods > 0 {
            tracing::warn!(
                name = %name,
                healthy = healthy_pods,
                unhealthy = unhealthy_pods,
                "Cluster has unhealthy pods"
            );

            // Transition to Degraded if more than 50% unhealthy
            if unhealthy_pods > healthy_pods {
                self.update_status(
                    api,
                    &name,
                    XervClusterStatus {
                        phase: ClusterPhase::Degraded,
                        ready_replicas: healthy_pods,
                        available_replicas: healthy_pods,
                        message: Some(format!("{}/{} pods unhealthy", unhealthy_pods, total_pods)),
                        last_updated: Some(chrono::Utc::now().to_rfc3339()),
                        ..cluster.status.clone().unwrap_or_default()
                    },
                )
                .await?;
                return Ok(ReconcileAction::requeue_short());
            }
        }

        // Update metrics
        let (ready_replicas, available_replicas) = if cluster.spec.dispatch.backend == "raft" {
            self.get_statefulset_status(&name, namespace).await?
        } else {
            self.get_deployment_status(&name, namespace).await?
        };

        self.update_status(
            api,
            &name,
            XervClusterStatus {
                ready_replicas,
                available_replicas,
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                observed_generation: current_gen,
                ..cluster.status.clone().unwrap_or_default()
            },
        )
        .await?;

        Ok(ReconcileAction::requeue_long())
    }

    /// Check pod health for the cluster.
    async fn check_pod_health(&self, name: &str, namespace: &str) -> OperatorResult<(i32, i32)> {
        let pods: Api<Pod> = Api::namespaced(self.ctx.client.clone(), namespace);
        let label_selector = format!("app.kubernetes.io/instance={}", name);

        let pod_list = pods
            .list(&ListParams::default().labels(&label_selector))
            .await?;

        let mut healthy = 0;
        let mut unhealthy = 0;

        for pod in pod_list.items {
            let is_healthy = pod
                .status
                .as_ref()
                .and_then(|s| s.conditions.as_ref())
                .map(|conditions| {
                    conditions
                        .iter()
                        .any(|c| c.type_ == "Ready" && c.status == "True")
                })
                .unwrap_or(false);

            if is_healthy {
                healthy += 1;
            } else {
                unhealthy += 1;
            }
        }

        Ok((healthy, unhealthy))
    }

    /// Handle updating phase - rolling update in progress.
    async fn handle_updating(
        &self,
        cluster: &XervCluster,
        api: &Api<XervCluster>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        tracing::info!(name = %name, "Cluster is updating, monitoring progress");

        // Check update progress
        let (ready_replicas, _available_replicas) = if cluster.spec.dispatch.backend == "raft" {
            self.get_statefulset_status(&name, namespace).await?
        } else {
            self.get_deployment_status(&name, namespace).await?
        };

        // Check if update is complete
        if ready_replicas >= cluster.spec.replicas {
            tracing::info!(name = %name, "Rolling update complete");
            self.update_status(
                api,
                &name,
                XervClusterStatus {
                    phase: ClusterPhase::Running,
                    ready_replicas,
                    available_replicas: ready_replicas,
                    message: Some("Update complete".into()),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    observed_generation: cluster.metadata.generation,
                    ..cluster.status.clone().unwrap_or_default()
                },
            )
            .await?;
            return Ok(ReconcileAction::requeue_long());
        }

        // Still updating, log progress
        tracing::info!(
            name = %name,
            ready = ready_replicas,
            desired = cluster.spec.replicas,
            "Rolling update in progress"
        );

        // Update status with progress
        self.update_status(
            api,
            &name,
            XervClusterStatus {
                ready_replicas,
                message: Some(format!(
                    "Updating: {}/{} pods ready",
                    ready_replicas, cluster.spec.replicas
                )),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..cluster.status.clone().unwrap_or_default()
            },
        )
        .await?;

        Ok(ReconcileAction::requeue_short())
    }

    /// Handle degraded phase - some pods unhealthy.
    async fn handle_degraded(
        &self,
        cluster: &XervCluster,
        api: &Api<XervCluster>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        tracing::warn!(name = %name, "Cluster is degraded, attempting recovery");

        // Check pod health again
        let (healthy_pods, unhealthy_pods) = self.check_pod_health(&name, namespace).await?;

        if unhealthy_pods == 0 {
            // All pods recovered
            tracing::info!(name = %name, "All pods recovered, transitioning to Running");
            self.update_status(
                api,
                &name,
                XervClusterStatus {
                    phase: ClusterPhase::Running,
                    ready_replicas: healthy_pods,
                    available_replicas: healthy_pods,
                    message: Some("Recovered from degraded state".into()),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    ..cluster.status.clone().unwrap_or_default()
                },
            )
            .await?;
            return Ok(ReconcileAction::requeue_long());
        }

        // Attempt pod recovery by deleting unhealthy pods
        // Kubernetes will recreate them via the StatefulSet/Deployment
        let pods: Api<Pod> = Api::namespaced(self.ctx.client.clone(), namespace);
        let label_selector = format!("app.kubernetes.io/instance={}", name);

        let pod_list = pods
            .list(&ListParams::default().labels(&label_selector))
            .await?;

        let mut deleted_count = 0;
        for pod in pod_list.items {
            let pod_name = pod.metadata.name.as_deref().unwrap_or("");
            let is_healthy = pod
                .status
                .as_ref()
                .and_then(|s| s.conditions.as_ref())
                .map(|conditions| {
                    conditions
                        .iter()
                        .any(|c| c.type_ == "Ready" && c.status == "True")
                })
                .unwrap_or(false);

            if !is_healthy && deleted_count < 1 {
                // Delete one unhealthy pod at a time to avoid disruption
                tracing::info!(pod = %pod_name, "Deleting unhealthy pod for recovery");
                if let Err(e) = pods.delete(pod_name, &DeleteParams::default()).await {
                    tracing::warn!(pod = %pod_name, error = %e, "Failed to delete unhealthy pod");
                } else {
                    deleted_count += 1;
                }
            }
        }

        // If no healthy pods remain, transition to Failed
        if healthy_pods == 0 && unhealthy_pods > 0 {
            tracing::error!(name = %name, "No healthy pods, transitioning to Failed");
            self.update_status(
                api,
                &name,
                XervClusterStatus {
                    phase: ClusterPhase::Failed,
                    message: Some("All pods are unhealthy".into()),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    ..cluster.status.clone().unwrap_or_default()
                },
            )
            .await?;
            return Ok(ReconcileAction::requeue_medium());
        }

        // Update status
        self.update_status(
            api,
            &name,
            XervClusterStatus {
                ready_replicas: healthy_pods,
                available_replicas: healthy_pods,
                message: Some(format!(
                    "Degraded: {}/{} pods healthy, attempting recovery",
                    healthy_pods,
                    healthy_pods + unhealthy_pods
                )),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..cluster.status.clone().unwrap_or_default()
            },
        )
        .await?;

        Ok(ReconcileAction::requeue_short())
    }

    /// Handle failed phase - cluster has failed.
    async fn handle_failed(
        &self,
        cluster: &XervCluster,
        api: &Api<XervCluster>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        let retry_count = cluster
            .status
            .as_ref()
            .and_then(|s| s.retry_count)
            .unwrap_or(0);

        tracing::error!(
            name = %name,
            retry_count = retry_count,
            "Cluster has failed, implementing recovery strategy"
        );

        // Limit retries to prevent infinite loops
        if retry_count >= 5 {
            tracing::error!(
                name = %name,
                "Max retries reached, manual intervention required"
            );
            self.update_status(
                api,
                &name,
                XervClusterStatus {
                    message: Some("Max retries reached, manual intervention required".into()),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    ..cluster.status.clone().unwrap_or_default()
                },
            )
            .await?;
            return Ok(ReconcileAction::requeue_long());
        }

        // Attempt recovery by recreating resources
        tracing::info!(
            name = %name,
            attempt = retry_count + 1,
            "Attempting cluster recovery"
        );

        // Delete and recreate workload
        if cluster.spec.dispatch.backend == "raft" {
            let statefulsets: Api<StatefulSet> =
                Api::namespaced(self.ctx.client.clone(), namespace);
            let _ = statefulsets.delete(&name, &DeleteParams::default()).await;
            // Wait a bit before recreating
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            self.ensure_statefulset(cluster, namespace).await?;
        } else {
            let deployments: Api<Deployment> = Api::namespaced(self.ctx.client.clone(), namespace);
            let _ = deployments.delete(&name, &DeleteParams::default()).await;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            self.ensure_deployment(cluster, namespace).await?;
        }

        // Update status
        self.update_status(
            api,
            &name,
            XervClusterStatus {
                phase: ClusterPhase::Initializing,
                retry_count: Some(retry_count + 1),
                message: Some(format!("Recovery attempt {}", retry_count + 1)),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
        )
        .await?;

        Ok(ReconcileAction::requeue_short())
    }

    /// Handle terminating phase - cleanup resources.
    async fn handle_terminating(
        &self,
        cluster: &XervCluster,
        _api: &Api<XervCluster>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = cluster.name_any();
        tracing::info!(name = %name, "Cluster is terminating, cleaning up resources");

        // Delete workload
        if cluster.spec.dispatch.backend == "raft" {
            let statefulsets: Api<StatefulSet> =
                Api::namespaced(self.ctx.client.clone(), namespace);
            if let Err(e) = statefulsets.delete(&name, &DeleteParams::default()).await {
                if !matches!(&e, kube::Error::Api(err) if err.code == 404) {
                    tracing::warn!(error = %e, "Failed to delete StatefulSet");
                }
            }
        } else {
            let deployments: Api<Deployment> = Api::namespaced(self.ctx.client.clone(), namespace);
            if let Err(e) = deployments.delete(&name, &DeleteParams::default()).await {
                if !matches!(&e, kube::Error::Api(err) if err.code == 404) {
                    tracing::warn!(error = %e, "Failed to delete Deployment");
                }
            }
        }

        // Delete services
        let services: Api<Service> = Api::namespaced(self.ctx.client.clone(), namespace);
        for suffix in ["", "-headless"] {
            let svc_name = format!("{}{}", name, suffix);
            if let Err(e) = services.delete(&svc_name, &DeleteParams::default()).await {
                if !matches!(&e, kube::Error::Api(err) if err.code == 404) {
                    tracing::warn!(name = %svc_name, error = %e, "Failed to delete Service");
                }
            }
        }

        // Delete ConfigMap
        let configmaps: Api<ConfigMap> = Api::namespaced(self.ctx.client.clone(), namespace);
        if let Err(e) = configmaps.delete(&name, &DeleteParams::default()).await {
            if !matches!(&e, kube::Error::Api(err) if err.code == 404) {
                tracing::warn!(error = %e, "Failed to delete ConfigMap");
            }
        }

        tracing::info!(name = %name, "Cluster resource cleanup complete");
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
