//! Secret rotation detection and pipeline re-sync automation.
//!
//! Watches for changes to secrets referenced by federated pipelines and
//! automatically triggers re-deployment when secrets are rotated.

use crate::crd::FederatedPipeline;
use crate::error::OperatorResult;
use futures::StreamExt;
use k8s_openapi::api::core::v1::Secret;
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::{self, Config, watcher};
use kube::{Api, Client, ResourceExt};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Tracks which pipelines reference which secrets.
#[derive(Debug, Clone, Default)]
pub struct SecretPipelineIndex {
    /// Map of secret name -> set of pipeline names that reference it
    secret_to_pipelines: HashMap<String, HashSet<String>>,
    /// Map of pipeline name -> set of secrets it references
    pipeline_to_secrets: HashMap<String, HashSet<String>>,
}

impl SecretPipelineIndex {
    /// Add a pipeline-secret relationship.
    pub fn add_reference(&mut self, pipeline: String, secret: String) {
        self.secret_to_pipelines
            .entry(secret.clone())
            .or_default()
            .insert(pipeline.clone());

        self.pipeline_to_secrets
            .entry(pipeline)
            .or_default()
            .insert(secret);
    }

    /// Remove all references for a pipeline.
    pub fn remove_pipeline(&mut self, pipeline: &str) {
        if let Some(secrets) = self.pipeline_to_secrets.remove(pipeline) {
            for secret in secrets {
                if let Some(pipelines) = self.secret_to_pipelines.get_mut(&secret) {
                    pipelines.remove(pipeline);
                    if pipelines.is_empty() {
                        self.secret_to_pipelines.remove(&secret);
                    }
                }
            }
        }
    }

    /// Get pipelines that reference a secret.
    pub fn get_pipelines_for_secret(&self, secret: &str) -> Option<&HashSet<String>> {
        self.secret_to_pipelines.get(secret)
    }

    /// Get secrets referenced by a pipeline.
    pub fn get_secrets_for_pipeline(&self, pipeline: &str) -> Option<&HashSet<String>> {
        self.pipeline_to_secrets.get(pipeline)
    }
}

/// Secret rotation detector.
///
/// Watches for secret changes and triggers pipeline re-sync.
pub struct SecretRotationDetector {
    client: Client,
    namespace: String,
    index: Arc<RwLock<SecretPipelineIndex>>,
}

impl SecretRotationDetector {
    /// Create a new secret rotation detector.
    pub fn new(client: Client, namespace: String) -> Self {
        Self {
            client,
            namespace,
            index: Arc::new(RwLock::new(SecretPipelineIndex::default())),
        }
    }

    /// Get a clone of the index for external use.
    pub fn get_index(&self) -> Arc<RwLock<SecretPipelineIndex>> {
        Arc::clone(&self.index)
    }

    /// Start watching for secret changes.
    ///
    /// This spawns a background task that watches secrets and triggers
    /// pipeline re-sync when secrets are modified.
    pub async fn start_watching(self: Arc<Self>) -> OperatorResult<()> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &self.namespace);

        let watcher_config =
            Config::default().labels("app.kubernetes.io/managed-by in (xerv,xerv-operator)");

        let mut watch_stream = watcher(secrets, watcher_config).default_backoff().boxed();

        info!(
            namespace = %self.namespace,
            "Secret rotation detector started"
        );

        while let Some(event) = watch_stream.next().await {
            match event {
                Ok(watcher::Event::Apply(secret)) => {
                    self.handle_secret_change(&secret).await;
                }
                Ok(watcher::Event::Delete(secret)) => {
                    self.handle_secret_deletion(&secret).await;
                }
                Ok(watcher::Event::Init) | Ok(watcher::Event::InitApply(_)) => {
                    info!("Secret watcher initialized");
                }
                Ok(watcher::Event::InitDone) => {
                    info!("Secret watcher initialization complete");
                }
                Err(e) => {
                    error!(error = %e, "Secret watch error");
                }
            }
        }

        warn!("Secret watcher stream ended");
        Ok(())
    }

    /// Handle secret change (creation or update).
    async fn handle_secret_change(&self, secret: &Secret) {
        let secret_name = secret.name_any();
        let resource_version = secret
            .metadata
            .resource_version
            .as_deref()
            .unwrap_or("unknown");

        info!(
            secret = %secret_name,
            resource_version = %resource_version,
            "Secret changed, checking for affected pipelines"
        );

        // Check if any pipelines reference this secret
        let index = self.index.read().await;
        let affected_pipelines = index.get_pipelines_for_secret(&secret_name);

        if let Some(pipelines) = affected_pipelines {
            if !pipelines.is_empty() {
                info!(
                    secret = %secret_name,
                    pipelines = ?pipelines,
                    count = pipelines.len(),
                    "Secret rotation detected, triggering pipeline re-sync"
                );

                // Trigger re-sync for each affected pipeline
                for pipeline_name in pipelines {
                    if let Err(e) = self.trigger_pipeline_resync(pipeline_name).await {
                        error!(
                            pipeline = %pipeline_name,
                            secret = %secret_name,
                            error = %e,
                            "Failed to trigger pipeline re-sync"
                        );
                    }
                }
            }
        }
    }

    /// Handle secret deletion.
    async fn handle_secret_deletion(&self, secret: &Secret) {
        let secret_name = secret.name_any();

        warn!(
            secret = %secret_name,
            "Secret deleted, checking for affected pipelines"
        );

        let index = self.index.read().await;
        let affected_pipelines = index.get_pipelines_for_secret(&secret_name);

        if let Some(pipelines) = affected_pipelines {
            if !pipelines.is_empty() {
                warn!(
                    secret = %secret_name,
                    pipelines = ?pipelines,
                    "Secret deletion affects {} pipelines - they may fail",
                    pipelines.len()
                );
            }
        }
    }

    /// Trigger pipeline re-sync by annotating the resource.
    ///
    /// This forces the controller to reconcile the pipeline, which will
    /// re-distribute the updated secret to remote clusters.
    async fn trigger_pipeline_resync(&self, pipeline_name: &str) -> OperatorResult<()> {
        use kube::api::{Patch, PatchParams};

        let pipelines: Api<FederatedPipeline> =
            Api::namespaced(self.client.clone(), &self.namespace);

        // Add an annotation to trigger reconciliation
        let patch = serde_json::json!({
            "metadata": {
                "annotations": {
                    "xerv.io/secret-rotation-detected": chrono::Utc::now().to_rfc3339(),
                }
            }
        });

        pipelines
            .patch(
                pipeline_name,
                &PatchParams::default(),
                &Patch::Merge(&patch),
            )
            .await?;

        info!(
            pipeline = %pipeline_name,
            "Triggered pipeline re-sync due to secret rotation"
        );

        Ok(())
    }

    /// Update the index with pipeline secret references.
    ///
    /// Should be called whenever a pipeline is created, updated, or deleted.
    pub async fn update_pipeline_index(&self, pipeline_name: String, secret_refs: Vec<String>) {
        let mut index = self.index.write().await;

        // Remove old references
        index.remove_pipeline(&pipeline_name);

        // Add new references
        for secret in secret_refs {
            index.add_reference(pipeline_name.clone(), secret);
        }

        info!(
            pipeline = %pipeline_name,
            "Updated secret index for pipeline"
        );
    }

    /// Remove a pipeline from the index.
    pub async fn remove_pipeline_from_index(&self, pipeline_name: &str) {
        let mut index = self.index.write().await;
        index.remove_pipeline(pipeline_name);

        info!(
            pipeline = %pipeline_name,
            "Removed pipeline from secret index"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_pipeline_index() {
        let mut index = SecretPipelineIndex::default();

        // Add references
        index.add_reference("pipeline1".to_string(), "secret1".to_string());
        index.add_reference("pipeline1".to_string(), "secret2".to_string());
        index.add_reference("pipeline2".to_string(), "secret1".to_string());

        // Check secret -> pipelines mapping
        let pipelines = index.get_pipelines_for_secret("secret1").unwrap();
        assert_eq!(pipelines.len(), 2);
        assert!(pipelines.contains("pipeline1"));
        assert!(pipelines.contains("pipeline2"));

        // Check pipeline -> secrets mapping
        let secrets = index.get_secrets_for_pipeline("pipeline1").unwrap();
        assert_eq!(secrets.len(), 2);
        assert!(secrets.contains("secret1"));
        assert!(secrets.contains("secret2"));

        // Remove pipeline
        index.remove_pipeline("pipeline1");

        let pipelines = index.get_pipelines_for_secret("secret1").unwrap();
        assert_eq!(pipelines.len(), 1);
        assert!(pipelines.contains("pipeline2"));

        assert!(index.get_secrets_for_pipeline("pipeline1").is_none());
    }
}
