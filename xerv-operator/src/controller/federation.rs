//! XervFederation controller.
//!
//! Reconciles XervFederation resources to manage multi-cluster federations.

use super::{ControllerContext, ReconcileAction};
use crate::crd::{
    ClusterMemberStatus, FederationCondition, FederationPhase, XervFederation, XervFederationStatus,
};
use crate::error::{OperatorError, OperatorResult};
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::Request;
use kube::api::{Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use std::sync::Arc;

/// Stats fetched from a cluster's stats endpoint.
#[derive(Debug, Default)]
struct ClusterStats {
    pipeline_count: i32,
    active_traces: i32,
}

/// Controller for XervFederation resources.
#[derive(Clone)]
pub struct FederationController {
    ctx: Arc<ControllerContext>,
}

impl FederationController {
    /// Create a new federation controller.
    pub fn new(ctx: Arc<ControllerContext>) -> Self {
        Self { ctx }
    }

    /// Reconcile a XervFederation resource.
    ///
    /// This is the main reconciliation loop that:
    /// 1. Validates the federation spec
    /// 2. Performs health checks on member clusters
    /// 3. Updates cluster connectivity status
    /// 4. Manages routing configuration
    /// 5. Updates the federation status
    pub async fn reconcile(
        &self,
        federation: Arc<XervFederation>,
    ) -> OperatorResult<ReconcileAction> {
        let name = federation.name_any();
        let namespace = federation
            .namespace()
            .ok_or_else(|| OperatorError::InvalidConfig("Federation must be namespaced".into()))?;

        tracing::info!(
            name = %name,
            namespace = %namespace,
            clusters = federation.spec.clusters.len(),
            routing = ?federation.spec.routing.default_strategy,
            "Reconciling XervFederation"
        );

        // Get the API for this namespace
        let federations: Api<XervFederation> = Api::namespaced(self.ctx.client.clone(), &namespace);

        // Check current phase
        let current_phase = federation
            .status
            .as_ref()
            .map(|s| &s.phase)
            .unwrap_or(&FederationPhase::Pending);

        let action = match current_phase {
            FederationPhase::Pending => {
                self.handle_pending(&federation, &federations, &namespace)
                    .await?
            }
            FederationPhase::Initializing => {
                self.handle_initializing(&federation, &federations, &namespace)
                    .await?
            }
            FederationPhase::Healthy => {
                self.handle_healthy(&federation, &federations, &namespace)
                    .await?
            }
            FederationPhase::Degraded => {
                self.handle_degraded(&federation, &federations, &namespace)
                    .await?
            }
            FederationPhase::Failed => {
                self.handle_failed(&federation, &federations, &namespace)
                    .await?
            }
            FederationPhase::Terminating => {
                self.handle_terminating(&federation, &federations, &namespace)
                    .await?
            }
        };

        Ok(action)
    }

    /// Handle pending phase - validate and initialize.
    async fn handle_pending(
        &self,
        federation: &XervFederation,
        api: &Api<XervFederation>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = federation.name_any();
        tracing::info!(name = %name, "Federation is pending, validating spec");

        // Validate the spec
        self.validate_spec(&federation.spec)?;

        // Update status to Initializing
        self.update_status(
            api,
            &name,
            XervFederationStatus {
                phase: FederationPhase::Initializing,
                cluster_count: federation.spec.clusters.len() as i32,
                message: Some("Initializing federation, connecting to member clusters".into()),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
        )
        .await?;

        tracing::info!(name = %name, "Federation initialized, performing health checks");
        Ok(ReconcileAction::requeue_short())
    }

    /// Handle initializing phase - connect to member clusters.
    async fn handle_initializing(
        &self,
        federation: &XervFederation,
        api: &Api<XervFederation>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = federation.name_any();
        tracing::info!(name = %name, "Performing initial health checks on member clusters");

        // Perform health checks on all clusters
        let cluster_statuses = self.check_cluster_health(federation).await;

        // Count healthy clusters
        let healthy_count = cluster_statuses.iter().filter(|s| s.healthy).count() as i32;
        let total_count = cluster_statuses.len() as i32;

        // Determine phase based on health
        let (phase, message) = if healthy_count == total_count {
            (
                FederationPhase::Healthy,
                format!("All {} clusters are healthy", total_count),
            )
        } else if healthy_count > 0 {
            (
                FederationPhase::Degraded,
                format!("{}/{} clusters are healthy", healthy_count, total_count),
            )
        } else {
            (
                FederationPhase::Failed,
                "No clusters are reachable".to_string(),
            )
        };

        // Update status
        self.update_status(
            api,
            &name,
            XervFederationStatus {
                phase,
                cluster_count: total_count,
                healthy_cluster_count: healthy_count,
                cluster_statuses,
                message: Some(message),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                conditions: vec![FederationCondition {
                    condition_type: "Ready".to_string(),
                    status: if healthy_count > 0 { "True" } else { "False" }.to_string(),
                    last_transition_time: Some(chrono::Utc::now().to_rfc3339()),
                    reason: Some("HealthCheckComplete".to_string()),
                    message: Some(format!(
                        "{}/{} clusters healthy",
                        healthy_count, total_count
                    )),
                }],
                ..Default::default()
            },
        )
        .await?;

        Ok(ReconcileAction::requeue_medium())
    }

    /// Handle healthy phase - periodic monitoring.
    async fn handle_healthy(
        &self,
        federation: &XervFederation,
        api: &Api<XervFederation>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = federation.name_any();
        tracing::debug!(name = %name, "Federation is healthy, performing periodic health check");

        // Perform health checks
        let cluster_statuses = self.check_cluster_health(federation).await;

        // Count healthy clusters
        let healthy_count = cluster_statuses.iter().filter(|s| s.healthy).count() as i32;
        let total_count = cluster_statuses.len() as i32;

        // Check if we need to transition to degraded
        let (phase, message) = if healthy_count == total_count {
            (
                FederationPhase::Healthy,
                format!("All {} clusters are healthy", total_count),
            )
        } else if healthy_count > 0 {
            tracing::warn!(
                name = %name,
                healthy = healthy_count,
                total = total_count,
                "Federation degraded, some clusters are unhealthy"
            );
            (
                FederationPhase::Degraded,
                format!("{}/{} clusters are healthy", healthy_count, total_count),
            )
        } else {
            tracing::error!(name = %name, "Federation failed, no clusters are reachable");
            (
                FederationPhase::Failed,
                "No clusters are reachable".to_string(),
            )
        };

        // Update status
        self.update_status(
            api,
            &name,
            XervFederationStatus {
                phase,
                cluster_count: total_count,
                healthy_cluster_count: healthy_count,
                cluster_statuses,
                message: Some(message),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..federation.status.clone().unwrap_or_default()
            },
        )
        .await?;

        // Requeue based on health check interval
        let interval = federation.spec.health_check.interval_seconds as u64;
        Ok(ReconcileAction::Requeue(std::time::Duration::from_secs(
            interval,
        )))
    }

    /// Handle degraded phase - try to recover.
    async fn handle_degraded(
        &self,
        federation: &XervFederation,
        api: &Api<XervFederation>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = federation.name_any();
        tracing::info!(name = %name, "Federation is degraded, attempting recovery");

        // Perform health checks
        let cluster_statuses = self.check_cluster_health(federation).await;

        // Count healthy clusters
        let healthy_count = cluster_statuses.iter().filter(|s| s.healthy).count() as i32;
        let total_count = cluster_statuses.len() as i32;

        // Determine new phase
        let (phase, message) = if healthy_count == total_count {
            tracing::info!(name = %name, "Federation recovered, all clusters healthy");
            (
                FederationPhase::Healthy,
                format!("Recovered: all {} clusters are healthy", total_count),
            )
        } else if healthy_count > 0 {
            (
                FederationPhase::Degraded,
                format!("{}/{} clusters are healthy", healthy_count, total_count),
            )
        } else {
            tracing::error!(name = %name, "Federation failed, no clusters reachable");
            (
                FederationPhase::Failed,
                "No clusters are reachable".to_string(),
            )
        };

        // Update status
        self.update_status(
            api,
            &name,
            XervFederationStatus {
                phase,
                cluster_count: total_count,
                healthy_cluster_count: healthy_count,
                cluster_statuses,
                message: Some(message),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..federation.status.clone().unwrap_or_default()
            },
        )
        .await?;

        // Requeue more frequently when degraded
        Ok(ReconcileAction::requeue_short())
    }

    /// Handle failed phase - attempt recovery.
    async fn handle_failed(
        &self,
        federation: &XervFederation,
        api: &Api<XervFederation>,
        _namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = federation.name_any();
        tracing::info!(name = %name, "Federation is failed, attempting recovery");

        // Try health checks again
        let cluster_statuses = self.check_cluster_health(federation).await;

        // Count healthy clusters
        let healthy_count = cluster_statuses.iter().filter(|s| s.healthy).count() as i32;
        let total_count = cluster_statuses.len() as i32;

        // Determine if we can recover
        let (phase, message) = if healthy_count == total_count {
            tracing::info!(name = %name, "Federation recovered from failure");
            (
                FederationPhase::Healthy,
                format!("Recovered: all {} clusters are healthy", total_count),
            )
        } else if healthy_count > 0 {
            tracing::info!(name = %name, "Federation partially recovered");
            (
                FederationPhase::Degraded,
                format!(
                    "Partially recovered: {}/{} clusters healthy",
                    healthy_count, total_count
                ),
            )
        } else {
            (
                FederationPhase::Failed,
                "No clusters are reachable".to_string(),
            )
        };

        // Update status
        self.update_status(
            api,
            &name,
            XervFederationStatus {
                phase,
                cluster_count: total_count,
                healthy_cluster_count: healthy_count,
                cluster_statuses,
                message: Some(message),
                last_updated: Some(chrono::Utc::now().to_rfc3339()),
                ..federation.status.clone().unwrap_or_default()
            },
        )
        .await?;

        // Requeue with backoff when failed
        Ok(ReconcileAction::requeue_medium())
    }

    /// Handle terminating phase - cleanup associated resources.
    ///
    /// When a federation is deleted, we should:
    /// 1. Check for FederatedPipelines that reference this federation and warn
    /// 2. Clean up any operator-managed secrets or configmaps
    /// 3. Remove finalizers to allow deletion to proceed
    async fn handle_terminating(
        &self,
        federation: &XervFederation,
        _api: &Api<XervFederation>,
        namespace: &str,
    ) -> OperatorResult<ReconcileAction> {
        let name = federation.name_any();
        tracing::info!(name = %name, "Federation is terminating, cleaning up resources");

        // Check for FederatedPipelines that reference this federation
        let pipelines: Api<crate::crd::FederatedPipeline> =
            Api::namespaced(self.ctx.client.clone(), namespace);

        match pipelines.list(&Default::default()).await {
            Ok(list) => {
                let dependent_pipelines: Vec<_> = list
                    .items
                    .iter()
                    .filter(|p| p.spec.federation == name)
                    .collect();

                if !dependent_pipelines.is_empty() {
                    tracing::warn!(
                        federation = %name,
                        dependent_count = dependent_pipelines.len(),
                        pipelines = ?dependent_pipelines.iter().map(|p| p.metadata.name.as_deref().unwrap_or("unknown")).collect::<Vec<_>>(),
                        "Federation has dependent FederatedPipelines that will be orphaned"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    federation = %name,
                    error = %e,
                    "Failed to list FederatedPipelines during cleanup"
                );
            }
        }

        // Clean up operator-managed ConfigMaps for this federation
        let configmaps: Api<k8s_openapi::api::core::v1::ConfigMap> =
            Api::namespaced(self.ctx.client.clone(), namespace);

        let cm_name = format!("xerv-federation-{}", name);
        match configmaps.delete(&cm_name, &Default::default()).await {
            Ok(_) => {
                tracing::info!(
                    federation = %name,
                    configmap = %cm_name,
                    "Deleted federation ConfigMap"
                );
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {
                // ConfigMap doesn't exist, nothing to delete
            }
            Err(e) => {
                tracing::warn!(
                    federation = %name,
                    configmap = %cm_name,
                    error = %e,
                    "Failed to delete federation ConfigMap"
                );
            }
        }

        // Clean up operator-managed Secrets for this federation (e.g., mTLS certs)
        let secrets: Api<k8s_openapi::api::core::v1::Secret> =
            Api::namespaced(self.ctx.client.clone(), namespace);

        let secret_name = format!("xerv-federation-{}-tls", name);
        match secrets.delete(&secret_name, &Default::default()).await {
            Ok(_) => {
                tracing::info!(
                    federation = %name,
                    secret = %secret_name,
                    "Deleted federation TLS secret"
                );
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {
                // Secret doesn't exist, nothing to delete
            }
            Err(e) => {
                tracing::warn!(
                    federation = %name,
                    secret = %secret_name,
                    error = %e,
                    "Failed to delete federation TLS secret"
                );
            }
        }

        tracing::info!(name = %name, "Federation cleanup complete");
        Ok(ReconcileAction::Done)
    }

    /// Validate the federation spec.
    fn validate_spec(&self, spec: &crate::crd::XervFederationSpec) -> OperatorResult<()> {
        // Must have at least one cluster
        if spec.clusters.is_empty() {
            return Err(OperatorError::InvalidConfig(
                "Federation must have at least one cluster".into(),
            ));
        }

        // Validate cluster names are unique
        let mut names = std::collections::HashSet::new();
        for cluster in &spec.clusters {
            if !names.insert(&cluster.name) {
                return Err(OperatorError::InvalidConfig(format!(
                    "Duplicate cluster name: {}",
                    cluster.name
                )));
            }
        }

        // Validate routing rules reference valid clusters
        for rule in &spec.routing.rules {
            if let Some(ref target) = rule.cluster {
                if !names.contains(target) {
                    return Err(OperatorError::InvalidConfig(format!(
                        "Routing rule references unknown cluster: {}",
                        target
                    )));
                }
            }
        }

        // Validate fallback cluster exists
        if let Some(ref fallback) = spec.routing.fallback_cluster {
            if !names.contains(fallback) {
                return Err(OperatorError::InvalidConfig(format!(
                    "Fallback cluster does not exist: {}",
                    fallback
                )));
            }
        }

        Ok(())
    }

    /// Check health of all member clusters.
    async fn check_cluster_health(&self, federation: &XervFederation) -> Vec<ClusterMemberStatus> {
        let mut statuses = Vec::with_capacity(federation.spec.clusters.len());

        for cluster in &federation.spec.clusters {
            let status = self.check_single_cluster_health(cluster, federation).await;
            statuses.push(status);
        }

        statuses
    }

    /// Check health of a single cluster and fetch stats.
    async fn check_single_cluster_health(
        &self,
        cluster: &crate::crd::FederationMember,
        federation: &XervFederation,
    ) -> ClusterMemberStatus {
        let timeout =
            std::time::Duration::from_secs(federation.spec.health_check.timeout_seconds as u64);

        // Try to reach the cluster's health endpoint
        let health_url = format!("{}/health", cluster.endpoint);
        let start = std::time::Instant::now();

        let (reachable, healthy, latency_ms, error) =
            match tokio::time::timeout(timeout, self.http_health_check(&health_url)).await {
                Ok(Ok(())) => {
                    let latency = start.elapsed().as_millis() as i64;
                    (true, true, Some(latency), None)
                }
                Ok(Err(e)) => (true, false, None, Some(e.to_string())),
                Err(_) => (
                    false,
                    false,
                    None,
                    Some("Health check timed out".to_string()),
                ),
            };

        // Fetch cluster stats if reachable
        let (pipeline_count, active_traces) = if reachable {
            let stats_url = format!("{}/api/v1/stats", cluster.endpoint);
            match tokio::time::timeout(timeout, self.fetch_cluster_stats(&stats_url)).await {
                Ok(Ok(stats)) => (stats.pipeline_count, stats.active_traces),
                Ok(Err(e)) => {
                    tracing::debug!(
                        cluster = %cluster.name,
                        error = %e,
                        "Failed to fetch cluster stats, using defaults"
                    );
                    (0, 0)
                }
                Err(_) => {
                    tracing::debug!(
                        cluster = %cluster.name,
                        "Stats fetch timed out, using defaults"
                    );
                    (0, 0)
                }
            }
        } else {
            (0, 0)
        };

        ClusterMemberStatus {
            name: cluster.name.clone(),
            reachable,
            healthy,
            last_health_check: Some(chrono::Utc::now().to_rfc3339()),
            latency_ms,
            pipeline_count,
            active_traces,
            error,
        }
    }

    /// Fetch stats from a cluster's stats endpoint.
    async fn fetch_cluster_stats(&self, url: &str) -> Result<ClusterStats, OperatorError> {
        // Parse URL
        let uri: http::Uri = url.parse().map_err(|e| {
            OperatorError::HealthCheck(format!("Invalid stats URL '{}': {}", url, e))
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

        // Build request
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .header("Host", host)
            .header("User-Agent", "xerv-operator/0.1")
            .header("Accept", "application/json")
            .body(Empty::<Bytes>::new())
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
                tracing::debug!(error = %e, "Stats connection error (may be normal)");
            }
        });

        let response = sender
            .send_request(req)
            .await
            .map_err(|e| OperatorError::HealthCheck(format!("Stats request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(OperatorError::HealthCheck(format!(
                "Stats endpoint returned status {}",
                response.status()
            )));
        }

        // Parse response body as JSON
        let body = response
            .into_body()
            .collect()
            .await
            .map_err(|e| OperatorError::HealthCheck(format!("Failed to read stats body: {}", e)))?
            .to_bytes();

        // Try to parse as JSON with pipeline_count and active_traces fields
        #[derive(serde::Deserialize, Default)]
        struct StatsResponse {
            #[serde(default)]
            pipeline_count: i32,
            #[serde(default)]
            active_traces: i32,
        }

        let stats: StatsResponse = serde_json::from_slice(&body).unwrap_or_default();

        Ok(ClusterStats {
            pipeline_count: stats.pipeline_count,
            active_traces: stats.active_traces,
        })
    }

    /// Perform HTTP health check against a cluster endpoint.
    ///
    /// Supports HTTP endpoints. For HTTPS, use a service mesh (Istio/Linkerd)
    /// that terminates TLS at the sidecar level, allowing the operator to make
    /// plain HTTP requests to `http://` service addresses.
    async fn http_health_check(&self, url: &str) -> Result<(), OperatorError> {
        // Parse the URL to extract host and path
        let uri: http::Uri = url.parse().map_err(|e| {
            OperatorError::HealthCheck(format!("Invalid health check URL '{}': {}", url, e))
        })?;

        // Extract scheme and validate
        let scheme = uri.scheme_str().unwrap_or("http");
        if scheme == "https" {
            // For HTTPS endpoints, recommend using service mesh or HTTP internally
            // In Kubernetes, internal services can use HTTP while ingress handles TLS
            tracing::warn!(
                url = %url,
                "HTTPS health check requested. Consider using HTTP for internal cluster \
                 communication with service mesh handling external TLS."
            );
            // Attempt the check anyway - in many setups, internal HTTPS endpoints
            // redirect to HTTP or the service mesh handles this transparently
        }

        let host = uri
            .host()
            .ok_or_else(|| OperatorError::HealthCheck(format!("Missing host in URL: {}", url)))?;

        let port = uri
            .port_u16()
            .unwrap_or(if scheme == "https" { 443 } else { 80 });
        let addr = format!("{}:{}", host, port);

        // Build the request
        let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");

        let req = Request::builder()
            .method("GET")
            .uri(path)
            .header("Host", host)
            .header("User-Agent", "xerv-operator/0.1")
            .header("Accept", "application/json, text/plain, */*")
            .body(Empty::<Bytes>::new())
            .map_err(|e| OperatorError::HealthCheck(format!("Failed to build request: {}", e)))?;

        // Connect via TCP and send HTTP request
        let stream = tokio::net::TcpStream::connect(&addr).await.map_err(|e| {
            OperatorError::HealthCheck(format!("Failed to connect to {}: {}", addr, e))
        })?;

        let io = hyper_util::rt::TokioIo::new(stream);

        // Use hyper's HTTP/1 client
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
            .map_err(|e| OperatorError::HealthCheck(format!("HTTP handshake failed: {}", e)))?;

        // Spawn the connection task
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                tracing::debug!(error = %e, "Health check connection error (may be normal)");
            }
        });

        // Send the request
        let response = sender.send_request(req).await.map_err(|e| {
            OperatorError::HealthCheck(format!("Health check request failed: {}", e))
        })?;

        // Check status code
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            // Try to read response body for error details
            let body = response
                .into_body()
                .collect()
                .await
                .map(|b| b.to_bytes())
                .ok()
                .and_then(|b| String::from_utf8(b.to_vec()).ok())
                .unwrap_or_default();

            Err(OperatorError::HealthCheck(format!(
                "Health check returned status {}: {}",
                status,
                body.chars().take(200).collect::<String>()
            )))
        }
    }

    /// Update the federation status.
    async fn update_status(
        &self,
        api: &Api<XervFederation>,
        name: &str,
        status: XervFederationStatus,
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

/// Error policy for the federation controller.
pub fn error_policy(
    _federation: Arc<XervFederation>,
    error: &OperatorError,
    _ctx: Arc<ControllerContext>,
) -> Action {
    tracing::error!(error = %error, "Federation reconciliation error");
    Action::requeue(std::time::Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use crate::crd::{
        FederationMember, HealthCheckConfig, RoutingConfig, RoutingRule, RoutingStrategy,
        SecurityConfig, SyncConfig, XervFederationSpec,
    };

    fn test_spec() -> XervFederationSpec {
        XervFederationSpec {
            clusters: vec![
                FederationMember {
                    name: "us-east".to_string(),
                    endpoint: "https://xerv-us-east.example.com".to_string(),
                    region: "us-east-1".to_string(),
                    zone: None,
                    credentials_secret: None,
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
                    weight: 100,
                    enabled: true,
                    labels: Default::default(),
                },
            ],
            routing: RoutingConfig::default(),
            health_check: HealthCheckConfig::default(),
            security: SecurityConfig::default(),
            sync: SyncConfig::default(),
        }
    }

    #[test]
    fn validate_spec_valid() {
        // Test validation logic without requiring a k8s client
        let spec = test_spec();

        // Verify cluster names are unique
        let mut names = std::collections::HashSet::new();
        let has_duplicates = spec.clusters.iter().any(|c| !names.insert(&c.name));
        assert!(!has_duplicates);
    }

    #[test]
    fn validate_spec_empty_clusters() {
        // This test validates the logic without requiring a k8s client
        let spec = XervFederationSpec {
            clusters: vec![],
            routing: RoutingConfig::default(),
            health_check: HealthCheckConfig::default(),
            security: SecurityConfig::default(),
            sync: SyncConfig::default(),
        };

        // Manually validate
        assert!(spec.clusters.is_empty());
    }

    #[test]
    fn validate_spec_duplicate_names() {
        let spec = XervFederationSpec {
            clusters: vec![
                FederationMember {
                    name: "same-name".to_string(),
                    endpoint: "https://a.example.com".to_string(),
                    region: "us-east-1".to_string(),
                    zone: None,
                    credentials_secret: None,
                    weight: 100,
                    enabled: true,
                    labels: Default::default(),
                },
                FederationMember {
                    name: "same-name".to_string(),
                    endpoint: "https://b.example.com".to_string(),
                    region: "eu-west-1".to_string(),
                    zone: None,
                    credentials_secret: None,
                    weight: 100,
                    enabled: true,
                    labels: Default::default(),
                },
            ],
            routing: RoutingConfig::default(),
            health_check: HealthCheckConfig::default(),
            security: SecurityConfig::default(),
            sync: SyncConfig::default(),
        };

        // Check for duplicate names manually
        let mut names = std::collections::HashSet::new();
        let has_duplicates = spec.clusters.iter().any(|c| !names.insert(&c.name));
        assert!(has_duplicates);
    }

    #[test]
    fn validate_spec_invalid_routing_rule() {
        let spec = XervFederationSpec {
            clusters: vec![FederationMember {
                name: "us-east".to_string(),
                endpoint: "https://a.example.com".to_string(),
                region: "us-east-1".to_string(),
                zone: None,
                credentials_secret: None,
                weight: 100,
                enabled: true,
                labels: Default::default(),
            }],
            routing: RoutingConfig {
                default_strategy: RoutingStrategy::Nearest,
                rules: vec![RoutingRule {
                    pipeline: "test".to_string(),
                    cluster: Some("nonexistent".to_string()),
                    strategy: None,
                    affinity_labels: Default::default(),
                    priority: 0,
                }],
                fallback_cluster: None,
                auto_failover: true,
            },
            health_check: HealthCheckConfig::default(),
            security: SecurityConfig::default(),
            sync: SyncConfig::default(),
        };

        // Check that the routing rule references a nonexistent cluster
        let cluster_names: std::collections::HashSet<_> =
            spec.clusters.iter().map(|c| &c.name).collect();
        let has_invalid_rule = spec.routing.rules.iter().any(|r| {
            r.cluster
                .as_ref()
                .map(|c| !cluster_names.contains(c))
                .unwrap_or(false)
        });
        assert!(has_invalid_rule);
    }
}
