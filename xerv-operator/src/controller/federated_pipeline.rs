//! FederatedPipeline controller.
//!
//! Reconciles FederatedPipeline resources to deploy pipelines across federated clusters.

use super::{ControllerContext, ReconcileAction};
use crate::crd::{
    ClusterPipelineStatus, FederatedPipeline, FederatedPipelineCondition, FederatedPipelineStatus,
    RolloutStrategy, SyncStatus, XervFederation,
};
use crate::error::{OperatorError, OperatorResult};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use kube::api::{Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use std::sync::Arc;

/// Controller for FederatedPipeline resources.
#[derive(Clone)]
pub struct FederatedPipelineController {
    ctx: Arc<ControllerContext>,
}

impl FederatedPipelineController {
    /// Create a new federated pipeline controller.
    pub fn new(ctx: Arc<ControllerContext>) -> Self {
        Self { ctx }
    }

    /// Reconcile a FederatedPipeline resource.
    ///
    /// This is the main reconciliation loop that:
    /// 1. Resolves the target federation
    /// 2. Determines target clusters based on placement rules
    /// 3. Deploys/updates the pipeline on each target cluster
    /// 4. Tracks sync status across clusters
    pub async fn reconcile(
        &self,
        pipeline: Arc<FederatedPipeline>,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        let namespace = pipeline.namespace().ok_or_else(|| {
            OperatorError::InvalidConfig("FederatedPipeline must be namespaced".into())
        })?;

        tracing::info!(
            name = %name,
            namespace = %namespace,
            federation = %pipeline.spec.federation,
            "Reconciling FederatedPipeline"
        );

        // Get APIs
        let pipelines: Api<FederatedPipeline> =
            Api::namespaced(self.ctx.client.clone(), &namespace);
        let federations: Api<XervFederation> = Api::namespaced(self.ctx.client.clone(), &namespace);

        // Get the target federation
        let federation = match federations.get(&pipeline.spec.federation).await {
            Ok(f) => f,
            Err(kube::Error::Api(err)) if err.code == 404 => {
                tracing::error!(
                    name = %name,
                    federation = %pipeline.spec.federation,
                    "Federation not found"
                );
                self.update_status(
                    &pipelines,
                    &name,
                    FederatedPipelineStatus {
                        sync_status: SyncStatus::Failed,
                        message: Some(format!(
                            "Federation '{}' not found",
                            pipeline.spec.federation
                        )),
                        last_updated: Some(chrono::Utc::now().to_rfc3339()),
                        ..Default::default()
                    },
                )
                .await?;
                return Ok(ReconcileAction::requeue_medium());
            }
            Err(e) => return Err(e.into()),
        };

        // Check current sync status
        let current_status = pipeline
            .status
            .as_ref()
            .map(|s| &s.sync_status)
            .unwrap_or(&SyncStatus::Pending);

        let action = match current_status {
            SyncStatus::Pending => {
                self.handle_pending(&pipeline, &pipelines, &federation)
                    .await?
            }
            SyncStatus::Syncing => {
                self.handle_syncing(&pipeline, &pipelines, &federation)
                    .await?
            }
            SyncStatus::Synced => {
                self.handle_synced(&pipeline, &pipelines, &federation)
                    .await?
            }
            SyncStatus::OutOfSync => {
                self.handle_out_of_sync(&pipeline, &pipelines, &federation)
                    .await?
            }
            SyncStatus::Failed => {
                self.handle_failed(&pipeline, &pipelines, &federation)
                    .await?
            }
        };

        Ok(action)
    }

    /// Handle pending status - validate and start sync.
    async fn handle_pending(
        &self,
        pipeline: &FederatedPipeline,
        api: &Api<FederatedPipeline>,
        federation: &XervFederation,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::info!(name = %name, "Pipeline is pending, validating and starting sync");

        // Validate the spec
        self.validate_spec(&pipeline.spec, federation)?;

        // Determine target clusters
        let target_clusters = self.resolve_target_clusters(pipeline, federation);

        if target_clusters.is_empty() {
            tracing::warn!(name = %name, "No target clusters match placement rules");
            self.update_status(
                api,
                &name,
                FederatedPipelineStatus {
                    sync_status: SyncStatus::Failed,
                    message: Some("No target clusters match placement rules".into()),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    ..Default::default()
                },
            )
            .await?;
            return Ok(ReconcileAction::requeue_medium());
        }

        // Update status to syncing
        self.update_status(
            api,
            &name,
            FederatedPipelineStatus {
                sync_status: SyncStatus::Syncing,
                target_clusters: target_clusters.len() as i32,
                message: Some(format!(
                    "Starting sync to {} clusters",
                    target_clusters.len()
                )),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                observed_generation: pipeline.metadata.generation.unwrap_or(1),
                ..Default::default()
            },
        )
        .await?;

        Ok(ReconcileAction::requeue_short())
    }

    /// Handle syncing status - deploy to clusters.
    async fn handle_syncing(
        &self,
        pipeline: &FederatedPipeline,
        api: &Api<FederatedPipeline>,
        federation: &XervFederation,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::info!(name = %name, "Syncing pipeline to clusters");

        // Determine target clusters
        let target_clusters = self.resolve_target_clusters(pipeline, federation);

        // Deploy to each cluster based on rollout strategy
        let cluster_statuses = match pipeline.spec.rollout.strategy {
            RolloutStrategy::AllAtOnce => {
                self.deploy_all_at_once(pipeline, &target_clusters, federation)
                    .await
            }
            RolloutStrategy::Rolling => {
                self.deploy_rolling(pipeline, &target_clusters, federation)
                    .await
            }
            RolloutStrategy::Canary => {
                self.deploy_canary(pipeline, &target_clusters, federation)
                    .await
            }
        };

        // Check if all clusters are synced
        let deployed_count = cluster_statuses.iter().filter(|s| s.deployed).count() as i32;
        let synced_count = cluster_statuses.iter().filter(|s| s.synced).count() as i32;
        let total_count = cluster_statuses.len() as i32;

        let (status, message) = if synced_count == total_count {
            (
                SyncStatus::Synced,
                format!("Pipeline synced to all {} clusters", total_count),
            )
        } else if deployed_count > 0 {
            (
                SyncStatus::Syncing,
                format!(
                    "Syncing: {}/{} clusters deployed, {}/{} synced",
                    deployed_count, total_count, synced_count, total_count
                ),
            )
        } else {
            (
                SyncStatus::Failed,
                "Failed to deploy to any cluster".to_string(),
            )
        };

        // Update status
        self.update_status(
            api,
            &name,
            FederatedPipelineStatus {
                sync_status: status.clone(),
                deployed_clusters: deployed_count,
                target_clusters: total_count,
                cluster_statuses,
                message: Some(message),
                last_sync_time: if status == SyncStatus::Synced {
                    Some(chrono::Utc::now().to_rfc3339())
                } else {
                    pipeline
                        .status
                        .as_ref()
                        .and_then(|s| s.last_sync_time.clone())
                },
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                observed_generation: pipeline.metadata.generation.unwrap_or(1),
                conditions: vec![FederatedPipelineCondition {
                    condition_type: "Synced".to_string(),
                    status: if status == SyncStatus::Synced {
                        "True"
                    } else {
                        "False"
                    }
                    .to_string(),
                    last_transition_time: Some(chrono::Utc::now().to_rfc3339()),
                    reason: Some(format!("{:?}", status)),
                    message: None,
                }],
            },
        )
        .await?;

        if status == SyncStatus::Synced {
            Ok(ReconcileAction::requeue_long())
        } else if status == SyncStatus::Failed {
            Ok(ReconcileAction::requeue_medium())
        } else {
            Ok(ReconcileAction::requeue_short())
        }
    }

    /// Handle synced status - monitor for changes.
    async fn handle_synced(
        &self,
        pipeline: &FederatedPipeline,
        api: &Api<FederatedPipeline>,
        federation: &XervFederation,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();

        // Check if spec generation changed (indicates update)
        let current_gen = pipeline.metadata.generation.unwrap_or(1);
        let observed_gen = pipeline
            .status
            .as_ref()
            .map(|s| s.observed_generation)
            .unwrap_or(0);

        if current_gen != observed_gen {
            tracing::info!(
                name = %name,
                current_gen = current_gen,
                observed_gen = observed_gen,
                "Pipeline spec changed, re-syncing"
            );
            // Spec changed, need to re-sync
            self.update_status(
                api,
                &name,
                FederatedPipelineStatus {
                    sync_status: SyncStatus::OutOfSync,
                    message: Some("Pipeline spec changed, re-syncing".into()),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    ..pipeline.status.clone().unwrap_or_default()
                },
            )
            .await?;
            return Ok(ReconcileAction::requeue_short());
        }

        // Periodic check - verify clusters are still in sync
        tracing::debug!(name = %name, "Pipeline is synced, performing periodic check");

        // Verify pipeline state on each cluster
        let target_clusters = self.resolve_target_clusters(pipeline, federation);
        let cluster_statuses = self
            .verify_pipeline_on_clusters(pipeline, &target_clusters, federation)
            .await;

        // Check if any clusters are out of sync
        let out_of_sync_count = cluster_statuses
            .iter()
            .filter(|s| !s.synced || !s.deployed)
            .count();

        let out_of_sync_clusters: Vec<String> = cluster_statuses
            .iter()
            .filter(|s| !s.synced || !s.deployed)
            .map(|s| s.cluster.clone())
            .collect();

        if out_of_sync_count > 0 {
            tracing::warn!(
                pipeline = %name,
                out_of_sync_count = out_of_sync_count,
                clusters = ?out_of_sync_clusters,
                "Some clusters are out of sync"
            );

            // Update status to out of sync
            self.update_status(
                api,
                &name,
                FederatedPipelineStatus {
                    sync_status: SyncStatus::OutOfSync,
                    cluster_statuses,
                    message: Some(format!(
                        "{}/{} clusters out of sync",
                        out_of_sync_count,
                        target_clusters.len()
                    )),
                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    ..pipeline.status.clone().unwrap_or_default()
                },
            )
            .await?;

            return Ok(ReconcileAction::requeue_short());
        }

        // All clusters are synced - update status with fresh cluster info
        let synced_count = cluster_statuses.len() as i32;
        self.update_status(
            api,
            &name,
            FederatedPipelineStatus {
                sync_status: SyncStatus::Synced,
                deployed_clusters: synced_count,
                target_clusters: synced_count,
                cluster_statuses,
                message: Some(format!("All {} clusters in sync", synced_count)),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..pipeline.status.clone().unwrap_or_default()
            },
        )
        .await?;

        // Requeue based on sync interval
        let interval = federation.spec.sync.interval_seconds as u64;
        Ok(ReconcileAction::Requeue(std::time::Duration::from_secs(
            interval,
        )))
    }

    /// Handle out of sync status - re-sync to clusters.
    async fn handle_out_of_sync(
        &self,
        pipeline: &FederatedPipeline,
        api: &Api<FederatedPipeline>,
        federation: &XervFederation,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::info!(name = %name, "Pipeline is out of sync, re-syncing");

        // Update status to syncing
        self.update_status(
            api,
            &name,
            FederatedPipelineStatus {
                sync_status: SyncStatus::Syncing,
                message: Some("Re-syncing pipeline to clusters".into()),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..pipeline.status.clone().unwrap_or_default()
            },
        )
        .await?;

        // Delegate to syncing handler
        self.handle_syncing(pipeline, api, federation).await
    }

    /// Handle failed status - attempt recovery.
    async fn handle_failed(
        &self,
        pipeline: &FederatedPipeline,
        api: &Api<FederatedPipeline>,
        federation: &XervFederation,
    ) -> OperatorResult<ReconcileAction> {
        let name = pipeline.name_any();
        tracing::info!(name = %name, "Pipeline sync failed, attempting recovery");

        // Update status to syncing to retry
        self.update_status(
            api,
            &name,
            FederatedPipelineStatus {
                sync_status: SyncStatus::Syncing,
                message: Some("Retrying sync after failure".into()),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..pipeline.status.clone().unwrap_or_default()
            },
        )
        .await?;

        // Delegate to syncing handler
        self.handle_syncing(pipeline, api, federation).await
    }

    /// Validate the pipeline spec against the federation.
    fn validate_spec(
        &self,
        spec: &crate::crd::FederatedPipelineSpec,
        federation: &XervFederation,
    ) -> OperatorResult<()> {
        // Get cluster names from federation
        let cluster_names: std::collections::HashSet<_> =
            federation.spec.clusters.iter().map(|c| &c.name).collect();

        // If not deploying to all, validate specified clusters exist
        if !spec.placement.all {
            for cluster in &spec.placement.clusters {
                if !cluster_names.contains(cluster) {
                    return Err(OperatorError::InvalidConfig(format!(
                        "Placement cluster '{}' not found in federation",
                        cluster
                    )));
                }
            }
        }

        // Validate overrides reference valid clusters
        for override_config in &spec.overrides {
            if !cluster_names.contains(&override_config.cluster) {
                return Err(OperatorError::InvalidConfig(format!(
                    "Override cluster '{}' not found in federation",
                    override_config.cluster
                )));
            }
        }

        Ok(())
    }

    /// Resolve target clusters based on placement configuration.
    fn resolve_target_clusters(
        &self,
        pipeline: &FederatedPipeline,
        federation: &XervFederation,
    ) -> Vec<String> {
        let placement = &pipeline.spec.placement;

        if placement.all {
            // Deploy to all clusters (except those disabled by overrides)
            federation
                .spec
                .clusters
                .iter()
                .filter(|c| c.enabled)
                .filter(|c| {
                    // Check if this cluster is disabled by override
                    !pipeline
                        .spec
                        .overrides
                        .iter()
                        .any(|o| o.cluster == c.name && o.disabled)
                })
                .map(|c| c.name.clone())
                .collect()
        } else if !placement.clusters.is_empty() {
            // Deploy to specified clusters
            placement
                .clusters
                .iter()
                .filter(|name| {
                    // Verify cluster exists and is enabled
                    federation
                        .spec
                        .clusters
                        .iter()
                        .any(|c| &c.name == *name && c.enabled)
                })
                .filter(|name| {
                    // Check if disabled by override
                    !pipeline
                        .spec
                        .overrides
                        .iter()
                        .any(|o| &o.cluster == *name && o.disabled)
                })
                .cloned()
                .collect()
        } else if !placement.cluster_selector.is_empty() {
            // Deploy to clusters matching labels
            federation
                .spec
                .clusters
                .iter()
                .filter(|c| c.enabled)
                .filter(|c| {
                    // Check if all selector labels match
                    placement
                        .cluster_selector
                        .iter()
                        .all(|(k, v)| c.labels.get(k) == Some(v))
                })
                .filter(|c| {
                    // Check if disabled by override
                    !pipeline
                        .spec
                        .overrides
                        .iter()
                        .any(|o| o.cluster == c.name && o.disabled)
                })
                .map(|c| c.name.clone())
                .collect()
        } else {
            // No placement rules, deploy to all
            federation
                .spec
                .clusters
                .iter()
                .filter(|c| c.enabled)
                .map(|c| c.name.clone())
                .collect()
        }
    }

    /// Deploy to all clusters simultaneously.
    async fn deploy_all_at_once(
        &self,
        pipeline: &FederatedPipeline,
        target_clusters: &[String],
        federation: &XervFederation,
    ) -> Vec<ClusterPipelineStatus> {
        let mut statuses = Vec::with_capacity(target_clusters.len());

        for cluster_name in target_clusters {
            let status = self
                .deploy_to_cluster(pipeline, cluster_name, federation)
                .await;
            statuses.push(status);
        }

        statuses
    }

    /// Deploy to clusters one at a time.
    async fn deploy_rolling(
        &self,
        pipeline: &FederatedPipeline,
        target_clusters: &[String],
        federation: &XervFederation,
    ) -> Vec<ClusterPipelineStatus> {
        let mut statuses = Vec::with_capacity(target_clusters.len());
        let max_surge = pipeline.spec.rollout.max_surge.max(1) as usize;
        let pause = std::time::Duration::from_secs(pipeline.spec.rollout.pause_seconds as u64);

        for chunk in target_clusters.chunks(max_surge) {
            for cluster_name in chunk {
                let status = self
                    .deploy_to_cluster(pipeline, cluster_name, federation)
                    .await;
                statuses.push(status);
            }

            // Pause between batches (except for last batch)
            if chunk.len() == max_surge && statuses.len() < target_clusters.len() {
                tokio::time::sleep(pause).await;
            }
        }

        statuses
    }

    /// Deploy canary style (one cluster first, then rest).
    async fn deploy_canary(
        &self,
        pipeline: &FederatedPipeline,
        target_clusters: &[String],
        federation: &XervFederation,
    ) -> Vec<ClusterPipelineStatus> {
        let mut statuses = Vec::with_capacity(target_clusters.len());

        if let Some(first) = target_clusters.first() {
            // Deploy to first cluster (canary)
            let status = self.deploy_to_cluster(pipeline, first, federation).await;
            let canary_success = status.synced;
            statuses.push(status);

            // If canary succeeded, deploy to rest
            if canary_success {
                let pause =
                    std::time::Duration::from_secs(pipeline.spec.rollout.pause_seconds as u64);
                tokio::time::sleep(pause).await;

                for cluster_name in target_clusters.iter().skip(1) {
                    let status = self
                        .deploy_to_cluster(pipeline, cluster_name, federation)
                        .await;
                    statuses.push(status);
                }
            }
        }

        statuses
    }

    /// Verify pipeline state on multiple clusters.
    async fn verify_pipeline_on_clusters(
        &self,
        pipeline: &FederatedPipeline,
        target_clusters: &[String],
        federation: &XervFederation,
    ) -> Vec<ClusterPipelineStatus> {
        let mut statuses = Vec::with_capacity(target_clusters.len());

        for cluster_name in target_clusters {
            let status = self
                .verify_pipeline_on_cluster(pipeline, cluster_name, federation)
                .await;
            statuses.push(status);
        }

        statuses
    }

    /// Verify pipeline state on a single cluster via HTTP GET.
    async fn verify_pipeline_on_cluster(
        &self,
        pipeline: &FederatedPipeline,
        cluster_name: &str,
        federation: &XervFederation,
    ) -> ClusterPipelineStatus {
        let name = pipeline.name_any();
        tracing::debug!(
            pipeline = %name,
            cluster = %cluster_name,
            "Verifying pipeline state on cluster"
        );

        // Get cluster endpoint from federation
        let endpoint = match self.get_cluster_endpoint(cluster_name, federation) {
            Some(ep) => ep,
            None => {
                // Cluster not found in federation
                return ClusterPipelineStatus {
                    cluster: cluster_name.to_string(),
                    deployed: false,
                    synced: false,
                    generation: 0,
                    status: "NotFound".to_string(),
                    active_traces: 0,
                    last_deployed: None,
                    error: Some(format!(
                        "Cluster '{}' not found in federation '{}'",
                        cluster_name,
                        federation.name_any()
                    )),
                };
            }
        };

        // Query pipeline status via HTTP GET
        match self.http_get_pipeline_status(&endpoint, &name).await {
            Ok(status) => status,
            Err(e) => {
                tracing::warn!(
                    pipeline = %name,
                    cluster = %cluster_name,
                    error = %e,
                    "Failed to verify pipeline on cluster"
                );
                ClusterPipelineStatus {
                    cluster: cluster_name.to_string(),
                    deployed: false,
                    synced: false,
                    generation: 0,
                    status: "Unknown".to_string(),
                    active_traces: 0,
                    last_deployed: None,
                    error: Some(e.to_string()),
                }
            }
        }
    }

    /// Get pipeline status from cluster via HTTP GET.
    async fn http_get_pipeline_status(
        &self,
        endpoint: &str,
        pipeline_name: &str,
    ) -> Result<ClusterPipelineStatus, OperatorError> {
        let url = format!("{}/api/v1/pipelines/{}/status", endpoint, pipeline_name);

        // Parse URL
        let uri: http::Uri = url.parse().map_err(|e| {
            OperatorError::HealthCheck(format!("Invalid status URL '{}': {}", url, e))
        })?;

        let host = uri
            .host()
            .ok_or_else(|| OperatorError::HealthCheck(format!("Missing host in URL: {}", url)))?;

        let scheme = uri.scheme_str().unwrap_or("http");
        let port = uri
            .port_u16()
            .unwrap_or(if scheme == "https" { 443 } else { 80 });
        let addr = format!("{}:{}", host, port);
        let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");

        // Build GET request
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .header("Host", host)
            .header("User-Agent", "xerv-operator/0.1")
            .header("Accept", "application/json")
            .body(http_body_util::Empty::<Bytes>::new())
            .map_err(|e| OperatorError::HealthCheck(format!("Failed to build request: {}", e)))?;

        // Connect
        let stream = tokio::net::TcpStream::connect(&addr).await.map_err(|e| {
            OperatorError::HealthCheck(format!("Failed to connect to {}: {}", addr, e))
        })?;

        let io = hyper_util::rt::TokioIo::new(stream);

        let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
            .map_err(|e| OperatorError::HealthCheck(format!("HTTP handshake failed: {}", e)))?;

        tokio::spawn(async move {
            if let Err(e) = conn.await {
                tracing::debug!(error = %e, "Status check connection error (may be normal)");
            }
        });

        let response = sender
            .send_request(req)
            .await
            .map_err(|e| OperatorError::HealthCheck(format!("Status request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(OperatorError::HealthCheck(format!(
                "Pipeline status endpoint returned {}",
                response.status()
            )));
        }

        // Parse response
        let body = response
            .into_body()
            .collect()
            .await
            .map_err(|e| OperatorError::HealthCheck(format!("Failed to read body: {}", e)))?
            .to_bytes();

        #[derive(serde::Deserialize, Default)]
        struct StatusResponse {
            #[serde(default)]
            cluster: String,
            #[serde(default)]
            deployed: bool,
            #[serde(default)]
            synced: bool,
            #[serde(default)]
            generation: i64,
            #[serde(default)]
            status: String,
            #[serde(default)]
            active_traces: i32,
            last_deployed: Option<String>,
            error: Option<String>,
        }

        let resp: StatusResponse = serde_json::from_slice(&body).unwrap_or_default();

        Ok(ClusterPipelineStatus {
            cluster: resp.cluster,
            deployed: resp.deployed,
            synced: resp.synced,
            generation: resp.generation,
            status: resp.status,
            active_traces: resp.active_traces,
            last_deployed: resp.last_deployed,
            error: resp.error,
        })
    }

    /// Deploy pipeline to a single cluster via HTTP API.
    ///
    /// Makes an HTTP POST request to the cluster's pipeline deploy endpoint.
    /// The cluster endpoint is retrieved from the federation spec.
    async fn deploy_to_cluster(
        &self,
        pipeline: &FederatedPipeline,
        cluster_name: &str,
        federation: &XervFederation,
    ) -> ClusterPipelineStatus {
        let name = pipeline.name_any();
        tracing::info!(
            pipeline = %name,
            cluster = %cluster_name,
            "Deploying pipeline to cluster"
        );

        // Get cluster endpoint from federation spec
        let endpoint = match self.get_cluster_endpoint(cluster_name, federation) {
            Some(ep) => ep,
            None => {
                return ClusterPipelineStatus {
                    cluster: cluster_name.to_string(),
                    deployed: false,
                    synced: false,
                    generation: 0,
                    status: "Failed".to_string(),
                    active_traces: 0,
                    last_deployed: None,
                    error: Some(format!(
                        "Cluster '{}' not found in federation '{}'",
                        cluster_name,
                        federation.name_any()
                    )),
                };
            }
        };

        // Serialize pipeline spec
        let payload = match serde_json::to_vec(&pipeline.spec.pipeline) {
            Ok(p) => p,
            Err(e) => {
                return ClusterPipelineStatus {
                    cluster: cluster_name.to_string(),
                    deployed: false,
                    synced: false,
                    generation: 0,
                    status: "Failed".to_string(),
                    active_traces: 0,
                    last_deployed: None,
                    error: Some(format!("Failed to serialize pipeline: {}", e)),
                };
            }
        };

        // Deploy via HTTP POST
        match self.http_deploy(&endpoint, &name, payload).await {
            Ok(()) => ClusterPipelineStatus {
                cluster: cluster_name.to_string(),
                deployed: true,
                synced: true,
                generation: pipeline.metadata.generation.unwrap_or(1),
                status: "Running".to_string(),
                active_traces: 0,
                last_deployed: Some(chrono::Utc::now().to_rfc3339()),
                error: None,
            },
            Err(e) => {
                tracing::warn!(
                    pipeline = %name,
                    cluster = %cluster_name,
                    error = %e,
                    "Failed to deploy pipeline to cluster"
                );
                ClusterPipelineStatus {
                    cluster: cluster_name.to_string(),
                    deployed: false,
                    synced: false,
                    generation: 0,
                    status: "Failed".to_string(),
                    active_traces: 0,
                    last_deployed: None,
                    error: Some(e.to_string()),
                }
            }
        }
    }

    /// Get cluster endpoint from the federation spec.
    ///
    /// Looks up the cluster by name in the federation's cluster list and returns
    /// the endpoint URL if the cluster is found and enabled.
    fn get_cluster_endpoint(
        &self,
        cluster_name: &str,
        federation: &XervFederation,
    ) -> Option<String> {
        // Find the cluster in the federation's cluster list
        let cluster = federation
            .spec
            .clusters
            .iter()
            .find(|c| c.name == cluster_name)?;

        // Check if the cluster is enabled
        if !cluster.enabled {
            tracing::debug!(
                cluster = %cluster_name,
                federation = %federation.name_any(),
                "Cluster is disabled in federation"
            );
            return None;
        }

        tracing::debug!(
            cluster = %cluster_name,
            endpoint = %cluster.endpoint,
            federation = %federation.name_any(),
            "Found cluster endpoint in federation"
        );

        Some(cluster.endpoint.clone())
    }

    /// Deploy pipeline via HTTP POST to cluster's pipeline API.
    async fn http_deploy(
        &self,
        endpoint: &str,
        pipeline_name: &str,
        payload: Vec<u8>,
    ) -> Result<(), OperatorError> {
        let url = format!("{}/api/v1/pipelines/{}", endpoint, pipeline_name);

        // Parse URL
        let uri: http::Uri = url.parse().map_err(|e| {
            OperatorError::DeploymentError(format!("Invalid deploy URL '{}': {}", url, e))
        })?;

        let host = uri.host().ok_or_else(|| {
            OperatorError::DeploymentError(format!("Missing host in URL: {}", url))
        })?;

        let scheme = uri.scheme_str().unwrap_or("http");
        let port = uri
            .port_u16()
            .unwrap_or(if scheme == "https" { 443 } else { 80 });
        let addr = format!("{}:{}", host, port);

        let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");

        // Build request
        let req = Request::builder()
            .method("POST")
            .uri(path)
            .header("Host", host)
            .header("User-Agent", "xerv-operator/0.1")
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(Full::new(Bytes::from(payload)))
            .map_err(|e| {
                OperatorError::DeploymentError(format!("Failed to build request: {}", e))
            })?;

        // Connect
        let stream = tokio::net::TcpStream::connect(&addr).await.map_err(|e| {
            OperatorError::DeploymentError(format!("Failed to connect to {}: {}", addr, e))
        })?;

        let io = hyper_util::rt::TokioIo::new(stream);

        // HTTP handshake
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
            .map_err(|e| OperatorError::DeploymentError(format!("HTTP handshake failed: {}", e)))?;

        // Spawn connection
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                tracing::debug!(error = %e, "Deploy connection error (may be normal)");
            }
        });

        // Send request
        let response = sender
            .send_request(req)
            .await
            .map_err(|e| OperatorError::DeploymentError(format!("Deploy request failed: {}", e)))?;

        // Check response
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            let body = response
                .into_body()
                .collect()
                .await
                .map(|b| b.to_bytes())
                .ok()
                .and_then(|b| String::from_utf8(b.to_vec()).ok())
                .unwrap_or_default();

            Err(OperatorError::DeploymentError(format!(
                "Deploy returned status {}: {}",
                status,
                body.chars().take(200).collect::<String>()
            )))
        }
    }

    /// Update the pipeline status.
    async fn update_status(
        &self,
        api: &Api<FederatedPipeline>,
        name: &str,
        status: FederatedPipelineStatus,
    ) -> OperatorResult<()> {
        let patch = serde_json::json!({
            "status": status
        });

        api.patch_status(
            name,
            &PatchParams::apply("xerv-operator"),
            &Patch::Merge(&patch),
        )
        .await?;

        Ok(())
    }
}

/// Error policy for the federated pipeline controller.
pub fn error_policy(
    _pipeline: Arc<FederatedPipeline>,
    error: &OperatorError,
    _ctx: Arc<ControllerContext>,
) -> Action {
    tracing::error!(error = %error, "FederatedPipeline reconciliation error");
    Action::requeue(std::time::Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{
        FederatedPipelineSource, FederatedPipelineSpec, FederationMember, HealthCheckConfig,
        PipelineDefinition, PlacementConfig, RolloutConfig, RoutingConfig, SecurityConfig,
        SyncConfig, XervFederationSpec,
    };

    fn test_federation() -> XervFederation {
        XervFederation::new(
            "test-federation",
            XervFederationSpec {
                clusters: vec![
                    FederationMember {
                        name: "us-east".to_string(),
                        endpoint: "https://us-east.example.com".to_string(),
                        region: "us-east-1".to_string(),
                        zone: None,
                        credentials_secret: None,
                        weight: 100,
                        enabled: true,
                        labels: [("tier".to_string(), "primary".to_string())]
                            .into_iter()
                            .collect(),
                    },
                    FederationMember {
                        name: "eu-west".to_string(),
                        endpoint: "https://eu-west.example.com".to_string(),
                        region: "eu-west-1".to_string(),
                        zone: None,
                        credentials_secret: None,
                        weight: 100,
                        enabled: true,
                        labels: [("tier".to_string(), "secondary".to_string())]
                            .into_iter()
                            .collect(),
                    },
                    FederationMember {
                        name: "disabled".to_string(),
                        endpoint: "https://disabled.example.com".to_string(),
                        region: "ap-south-1".to_string(),
                        zone: None,
                        credentials_secret: None,
                        weight: 100,
                        enabled: false,
                        labels: Default::default(),
                    },
                ],
                routing: RoutingConfig::default(),
                health_check: HealthCheckConfig::default(),
                security: SecurityConfig::default(),
                sync: SyncConfig::default(),
            },
        )
    }

    fn test_pipeline_spec() -> FederatedPipelineSpec {
        FederatedPipelineSpec {
            federation: "test-federation".to_string(),
            placement: PlacementConfig::default(),
            pipeline: PipelineDefinition {
                source: FederatedPipelineSource {
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
        }
    }

    #[test]
    fn resolve_targets_all_clusters() {
        let federation = test_federation();
        let spec = FederatedPipelineSpec {
            placement: PlacementConfig {
                all: true,
                ..Default::default()
            },
            ..test_pipeline_spec()
        };
        let _pipeline = FederatedPipeline::new("test", spec);

        // Manually resolve targets
        let targets: Vec<_> = federation
            .spec
            .clusters
            .iter()
            .filter(|c| c.enabled)
            .map(|c| c.name.clone())
            .collect();

        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&"us-east".to_string()));
        assert!(targets.contains(&"eu-west".to_string()));
        assert!(!targets.contains(&"disabled".to_string()));
    }

    #[test]
    fn resolve_targets_specific_clusters() {
        let _federation = test_federation();
        let spec = FederatedPipelineSpec {
            placement: PlacementConfig {
                all: false,
                clusters: vec!["us-east".to_string()],
                ..Default::default()
            },
            ..test_pipeline_spec()
        };
        let pipeline = FederatedPipeline::new("test", spec);

        // Verify placement specifies only us-east
        assert_eq!(pipeline.spec.placement.clusters, vec!["us-east"]);
    }

    #[test]
    fn resolve_targets_by_label() {
        let federation = test_federation();
        let spec = FederatedPipelineSpec {
            placement: PlacementConfig {
                all: false,
                clusters: vec![],
                cluster_selector: [("tier".to_string(), "primary".to_string())]
                    .into_iter()
                    .collect(),
                ..Default::default()
            },
            ..test_pipeline_spec()
        };
        let _pipeline = FederatedPipeline::new("test", spec);

        // Find clusters matching label
        let targets: Vec<_> = federation
            .spec
            .clusters
            .iter()
            .filter(|c| c.enabled)
            .filter(|c| c.labels.get("tier") == Some(&"primary".to_string()))
            .map(|c| c.name.clone())
            .collect();

        assert_eq!(targets, vec!["us-east"]);
    }
}
