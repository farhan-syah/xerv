//! Kubernetes controllers for XERV resources.
//!
//! This module contains the reconciliation logic for XERV custom resources:
//!
//! - [`ClusterController`]: Manages XervCluster resources
//! - [`PipelineController`]: Manages XervPipeline resources
//! - [`FederationController`]: Manages XervFederation resources
//! - [`FederatedPipelineController`]: Manages FederatedPipeline resources
//!
//! # Usage with kube-runtime
//!
//! The controller runtime requires both a reconcile function and an error policy:
//!
//! ```ignore
//! use xerv_operator::controller::{ClusterController, cluster_error_policy};
//!
//! Controller::new(clusters, watcher_config)
//!     .run(|cluster, ctx| async move {
//!         let controller = ClusterController::new(ctx.clone());
//!         controller.reconcile(cluster).await
//!     }, cluster_error_policy, context)
//!     .for_each(|_| futures::future::ready(()))
//!     .await;
//! ```

mod cluster;
mod federated_pipeline;
mod federation;
mod pipeline;

pub use cluster::{ClusterController, error_policy as cluster_error_policy};
pub use federated_pipeline::{
    FederatedPipelineController, error_policy as federated_pipeline_error_policy,
};
pub use federation::{FederationController, error_policy as federation_error_policy};
pub use pipeline::{PipelineController, error_policy as pipeline_error_policy};

/// Shared context for controllers.
pub struct ControllerContext {
    /// Kubernetes client.
    pub client: kube::Client,
}

impl ControllerContext {
    /// Create a new controller context.
    pub fn new(client: kube::Client) -> Self {
        Self { client }
    }
}

/// Result type for reconciliation actions.
#[derive(Debug)]
pub enum ReconcileAction {
    /// Requeue after the specified duration.
    Requeue(std::time::Duration),
    /// Don't requeue (reconciliation complete).
    Done,
}

impl ReconcileAction {
    /// Requeue after 5 seconds (default for transient errors).
    pub fn requeue_short() -> Self {
        Self::Requeue(std::time::Duration::from_secs(5))
    }

    /// Requeue after 30 seconds (default for waiting on external resources).
    pub fn requeue_medium() -> Self {
        Self::Requeue(std::time::Duration::from_secs(30))
    }

    /// Requeue after 5 minutes (default for periodic reconciliation).
    pub fn requeue_long() -> Self {
        Self::Requeue(std::time::Duration::from_secs(300))
    }
}
