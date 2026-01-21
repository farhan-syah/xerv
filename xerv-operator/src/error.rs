//! Error types for the XERV Kubernetes operator.

use thiserror::Error;

/// Errors that can occur during operator operations.
#[derive(Debug, Error)]
pub enum OperatorError {
    /// Kubernetes API error.
    #[error("Kubernetes API error: {0}")]
    KubeError(#[from] kube::Error),

    /// Resource not found.
    #[error("Resource not found: {kind}/{name} in namespace {namespace}")]
    NotFound {
        /// Resource kind.
        kind: String,
        /// Resource name.
        name: String,
        /// Resource namespace.
        namespace: String,
    },

    /// Invalid configuration.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Pipeline validation error.
    #[error("Pipeline validation failed: {0}")]
    ValidationError(String),

    /// Reconciliation error.
    #[error("Reconciliation failed for {kind}/{name}: {cause}")]
    ReconcileError {
        /// Resource kind.
        kind: String,
        /// Resource name.
        name: String,
        /// Error cause.
        cause: String,
    },

    /// Cluster not ready.
    #[error("Cluster {name} is not ready: {reason}")]
    ClusterNotReady {
        /// Cluster name.
        name: String,
        /// Reason.
        reason: String,
    },

    /// Serialization error.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Health check error.
    #[error("Health check failed: {0}")]
    HealthCheck(String),

    /// Deployment error.
    #[error("Deployment failed: {0}")]
    DeploymentError(String),

    /// IO error.
    #[error("IO error: {0}")]
    IoError(String),

    /// Git error.
    #[error("Git error: {0}")]
    GitError(String),

    /// HTTP error.
    #[error("HTTP error: {0}")]
    HttpError(String),

    /// ConfigMap error.
    #[error("ConfigMap error: {0}")]
    ConfigMapError(String),

    /// API error.
    #[error("API error: {0}")]
    ApiError(String),
}

/// Result type for operator operations.
pub type OperatorResult<T> = Result<T, OperatorError>;

impl From<serde_json::Error> for OperatorError {
    fn from(err: serde_json::Error) -> Self {
        OperatorError::SerializationError(err.to_string())
    }
}

impl From<serde_yaml::Error> for OperatorError {
    fn from(err: serde_yaml::Error) -> Self {
        OperatorError::SerializationError(err.to_string())
    }
}
