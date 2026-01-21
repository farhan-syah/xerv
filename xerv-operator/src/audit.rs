//! Audit logging for XERV operator security events.
//!
//! Provides structured audit logs for security-sensitive operations:
//! - Secret access (reads, resolves)
//! - Credential injection
//! - Secret distribution to remote clusters
//! - mTLS certificate usage
//! - RBAC violations

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use tracing::{info, warn};

/// Audit event types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    /// Secret was accessed from Kubernetes API.
    SecretAccess,
    /// Credentials were resolved from a secret.
    CredentialResolve,
    /// Credentials were injected into an HTTP request.
    CredentialInjection,
    /// Secret was distributed to a remote cluster.
    SecretDistribution,
    /// mTLS certificate was loaded and used.
    MtlsUsage,
    /// Secret rotation was detected.
    SecretRotation,
    /// Pipeline was re-synced due to secret rotation.
    PipelineResync,
    /// Access to a secret was denied by webhook.
    SecretAccessDenied,
    /// Unauthorized secret access attempt.
    UnauthorizedAccess,
}

/// Audit event severity levels.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditSeverity {
    /// Informational event (routine operation).
    Info,
    /// Warning (unusual but not necessarily problematic).
    Warning,
    /// Security violation or potential breach.
    Critical,
}

/// Audit event metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Timestamp in RFC3339 format.
    pub timestamp: String,
    /// Event type.
    pub event_type: AuditEventType,
    /// Severity level.
    pub severity: AuditSeverity,
    /// User or service account that performed the action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Source IP address if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ip: Option<IpAddr>,
    /// Resource being accessed (secret name, pipeline name, etc.).
    pub resource: String,
    /// Resource namespace.
    pub namespace: String,
    /// Target of the operation (cluster name for distribution, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Outcome of the operation (success, denied, error).
    pub outcome: String,
    /// Additional context or error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Structured metadata for the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl AuditEvent {
    /// Create a new audit event.
    pub fn new(
        event_type: AuditEventType,
        severity: AuditSeverity,
        resource: String,
        namespace: String,
        outcome: String,
    ) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            event_type,
            severity,
            subject: None,
            source_ip: None,
            resource,
            namespace,
            target: None,
            outcome,
            message: None,
            metadata: None,
        }
    }

    /// Set the subject (user/service account).
    pub fn with_subject(mut self, subject: String) -> Self {
        self.subject = Some(subject);
        self
    }

    /// Set the source IP address.
    pub fn with_source_ip(mut self, ip: IpAddr) -> Self {
        self.source_ip = Some(ip);
        self
    }

    /// Set the target (cluster, endpoint, etc.).
    pub fn with_target(mut self, target: String) -> Self {
        self.target = Some(target);
        self
    }

    /// Set the message.
    pub fn with_message(mut self, message: String) -> Self {
        self.message = Some(message);
        self
    }

    /// Set structured metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Log the audit event.
    ///
    /// Logs as JSON for machine parsing and exports to audit backends.
    pub fn log(&self) {
        let json = match serde_json::to_string(self) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize audit event");
                return;
            }
        };

        match self.severity {
            AuditSeverity::Info => {
                info!(
                    target: "audit",
                    event_type = ?self.event_type,
                    resource = %self.resource,
                    outcome = %self.outcome,
                    "{}",
                    json
                );
            }
            AuditSeverity::Warning => {
                warn!(
                    target: "audit",
                    event_type = ?self.event_type,
                    resource = %self.resource,
                    outcome = %self.outcome,
                    "{}",
                    json
                );
            }
            AuditSeverity::Critical => {
                tracing::error!(
                    target: "audit",
                    event_type = ?self.event_type,
                    resource = %self.resource,
                    outcome = %self.outcome,
                    "{}",
                    json
                );
            }
        }
    }
}

/// Audit logger for XERV operator.
pub struct AuditLogger {
    /// Service account name of the operator.
    service_account: String,
}

impl AuditLogger {
    /// Create a new audit logger.
    pub fn new(service_account: String) -> Self {
        Self { service_account }
    }

    /// Log secret access.
    pub fn log_secret_access(
        &self,
        secret_name: &str,
        namespace: &str,
        outcome: &str,
        message: Option<String>,
    ) {
        AuditEvent::new(
            AuditEventType::SecretAccess,
            AuditSeverity::Info,
            secret_name.to_string(),
            namespace.to_string(),
            outcome.to_string(),
        )
        .with_subject(self.service_account.clone())
        .with_message(message.unwrap_or_else(|| "Secret accessed".to_string()))
        .log();
    }

    /// Log credential resolution.
    pub fn log_credential_resolve(
        &self,
        secret_name: &str,
        namespace: &str,
        credential_type: &str,
        outcome: &str,
    ) {
        AuditEvent::new(
            AuditEventType::CredentialResolve,
            AuditSeverity::Info,
            secret_name.to_string(),
            namespace.to_string(),
            outcome.to_string(),
        )
        .with_subject(self.service_account.clone())
        .with_metadata(serde_json::json!({
            "credential_type": credential_type,
        }))
        .with_message(format!("Resolved {} credentials", credential_type))
        .log();
    }

    /// Log credential injection into HTTP request.
    pub fn log_credential_injection(
        &self,
        target_cluster: &str,
        credential_type: &str,
        outcome: &str,
    ) {
        AuditEvent::new(
            AuditEventType::CredentialInjection,
            AuditSeverity::Info,
            "credentials".to_string(),
            "xerv-system".to_string(),
            outcome.to_string(),
        )
        .with_subject(self.service_account.clone())
        .with_target(target_cluster.to_string())
        .with_metadata(serde_json::json!({
            "credential_type": credential_type,
        }))
        .with_message(format!(
            "Injected {} credentials for cluster {}",
            credential_type, target_cluster
        ))
        .log();
    }

    /// Log secret distribution to remote cluster.
    pub fn log_secret_distribution(
        &self,
        secret_name: &str,
        namespace: &str,
        target_cluster: &str,
        outcome: &str,
        message: Option<String>,
    ) {
        let severity = if outcome == "success" {
            AuditSeverity::Info
        } else {
            AuditSeverity::Warning
        };

        AuditEvent::new(
            AuditEventType::SecretDistribution,
            severity,
            secret_name.to_string(),
            namespace.to_string(),
            outcome.to_string(),
        )
        .with_subject(self.service_account.clone())
        .with_target(target_cluster.to_string())
        .with_message(
            message.unwrap_or_else(|| format!("Distributed secret to {}", target_cluster)),
        )
        .log();
    }

    /// Log mTLS certificate usage.
    pub fn log_mtls_usage(
        &self,
        cert_secret: &str,
        namespace: &str,
        target_cluster: &str,
        outcome: &str,
    ) {
        AuditEvent::new(
            AuditEventType::MtlsUsage,
            AuditSeverity::Info,
            cert_secret.to_string(),
            namespace.to_string(),
            outcome.to_string(),
        )
        .with_subject(self.service_account.clone())
        .with_target(target_cluster.to_string())
        .with_message(format!(
            "Used mTLS certificate for cluster {}",
            target_cluster
        ))
        .log();
    }

    /// Log secret rotation detection.
    pub fn log_secret_rotation(
        &self,
        secret_name: &str,
        namespace: &str,
        affected_pipelines: &[String],
    ) {
        AuditEvent::new(
            AuditEventType::SecretRotation,
            AuditSeverity::Warning,
            secret_name.to_string(),
            namespace.to_string(),
            "detected".to_string(),
        )
        .with_subject(self.service_account.clone())
        .with_metadata(serde_json::json!({
            "affected_pipelines": affected_pipelines,
            "count": affected_pipelines.len(),
        }))
        .with_message(format!(
            "Secret rotation affects {} pipelines",
            affected_pipelines.len()
        ))
        .log();
    }

    /// Log unauthorized access attempt.
    pub fn log_unauthorized_access(
        &self,
        secret_name: &str,
        namespace: &str,
        reason: &str,
        source_ip: Option<IpAddr>,
    ) {
        let mut event = AuditEvent::new(
            AuditEventType::UnauthorizedAccess,
            AuditSeverity::Critical,
            secret_name.to_string(),
            namespace.to_string(),
            "denied".to_string(),
        )
        .with_subject(self.service_account.clone())
        .with_message(format!("Unauthorized access attempt: {}", reason));

        if let Some(ip) = source_ip {
            event = event.with_source_ip(ip);
        }

        event.log();
    }
}

/// Global audit logger instance.
///
/// Shared across the entire operator for security event auditing.
/// Uses `once_cell::sync::Lazy` for thread-safe, lazy initialization
/// on first access.
///
/// The logger is initialized with the XERV operator's service account
/// identity for proper attribution of audit events.
static AUDIT_LOGGER: once_cell::sync::Lazy<AuditLogger> = once_cell::sync::Lazy::new(|| {
    AuditLogger::new("system:serviceaccount:xerv-system:xerv-operator".to_string())
});

/// Get the global audit logger.
///
/// Returns a reference to the process-wide audit logger instance.
/// The logger is initialized on first access with the XERV operator's
/// service account identity.
///
/// # Thread Safety
///
/// This function is thread-safe. Multiple threads can call it concurrently
/// without synchronization overhead after the first initialization.
///
/// # Examples
///
/// ```ignore
/// use xerv_operator::audit::audit_logger;
///
/// // Log a secret access event
/// audit_logger().log_secret_access(
///     "github-credentials",
///     "default",
///     "success",
///     None
/// );
///
/// // Log a cluster operation
/// audit_logger().log_cluster_operation(
///     "prod-cluster",
///     "deploy",
///     "success",
///     Some("Deployed pipeline v1.2.3")
/// );
///
/// // Log an authentication failure
/// audit_logger().log_auth_failure(
///     "user@example.com",
///     "Invalid credentials",
///     Some("192.168.1.100")
/// );
/// ```
///
/// # Usage Patterns
///
/// The audit logger is typically used in two scenarios:
///
/// 1. **Security-sensitive operations**: Secret access, authentication,
///    authorization decisions
/// 2. **Cluster operations**: Pipeline deployments, federation changes,
///    configuration updates
///
/// All audit events are logged with structured metadata for compliance
/// and forensic analysis.
pub fn audit_logger() -> &'static AuditLogger {
    &AUDIT_LOGGER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_creation() {
        let event = AuditEvent::new(
            AuditEventType::SecretAccess,
            AuditSeverity::Info,
            "test-secret".to_string(),
            "xerv-system".to_string(),
            "success".to_string(),
        )
        .with_subject("test-user".to_string())
        .with_message("Test access".to_string());

        assert_eq!(event.resource, "test-secret");
        assert_eq!(event.namespace, "xerv-system");
        assert_eq!(event.outcome, "success");
        assert_eq!(event.subject, Some("test-user".to_string()));
    }

    #[test]
    fn test_audit_event_serialization() {
        let event = AuditEvent::new(
            AuditEventType::SecretDistribution,
            AuditSeverity::Info,
            "app-secret".to_string(),
            "default".to_string(),
            "success".to_string(),
        )
        .with_target("us-east-cluster".to_string());

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("secret_distribution"));
        assert!(json.contains("app-secret"));
        assert!(json.contains("us-east-cluster"));
    }
}
