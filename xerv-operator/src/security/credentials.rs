//! Credential resolution from Kubernetes secrets.
//!
//! This module provides types and utilities for resolving various authentication
//! credentials (bearer tokens, basic auth, mTLS) from Kubernetes secrets.

use crate::audit::audit_logger;
use crate::error::{OperatorError, OperatorResult};
use k8s_openapi::api::core::v1::Secret;
use kube::Api;
use std::collections::BTreeMap;

/// Represents credentials retrieved from a Kubernetes secret.
#[derive(Debug, Clone)]
pub enum Credentials {
    /// Bearer token authentication.
    BearerToken(String),
    /// Basic authentication (username:password).
    BasicAuth {
        /// Username for basic authentication.
        username: String,
        /// Password for basic authentication.
        password: String,
    },
    /// mTLS certificates (cert PEM, key PEM, CA PEM).
    MutualTls {
        /// Client certificate in PEM format.
        cert: Vec<u8>,
        /// Client private key in PEM format.
        key: Vec<u8>,
        /// Optional CA certificate in PEM format for server verification.
        ca: Option<Vec<u8>>,
    },
}

/// Credentials resolver for fetching secrets from Kubernetes.
pub struct CredentialsResolver {
    client: kube::Client,
}

impl CredentialsResolver {
    /// Create a new credentials resolver.
    pub fn new(client: kube::Client) -> Self {
        Self { client }
    }

    /// Resolve credentials from a secret reference.
    ///
    /// # Arguments
    ///
    /// * `secret_name` - Name of the Kubernetes secret
    /// * `namespace` - Namespace containing the secret
    ///
    /// # Returns
    ///
    /// Parsed credentials or an error if the secret doesn't exist or is malformed.
    pub async fn resolve(&self, secret_name: &str, namespace: &str) -> OperatorResult<Credentials> {
        tracing::debug!(
            secret = %secret_name,
            namespace = %namespace,
            "Resolving credentials from secret"
        );

        // Audit log: Secret access attempt
        audit_logger().log_secret_access(
            secret_name,
            namespace,
            "attempt",
            Some(format!("Resolving credentials from secret {}", secret_name)),
        );

        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);

        // Fetch the secret
        let secret = secrets.get(secret_name).await.map_err(|e| match &e {
            kube::Error::Api(api_err) if api_err.code == 404 => OperatorError::NotFound {
                kind: "Secret".to_string(),
                name: secret_name.to_string(),
                namespace: namespace.to_string(),
            },
            _ => OperatorError::KubeError(e),
        })?;

        // Parse the secret data
        let data = secret.data.ok_or_else(|| {
            OperatorError::InvalidConfig(format!("Secret '{}' has no data field", secret_name))
        })?;

        // Determine credential type based on keys present
        if let Some(token_bytes) = data.get("token") {
            // Bearer token auth
            let token = String::from_utf8(token_bytes.0.clone()).map_err(|_| {
                OperatorError::InvalidConfig(format!(
                    "Secret '{}' contains invalid UTF-8 in 'token' field",
                    secret_name
                ))
            })?;

            tracing::debug!(
                secret = %secret_name,
                "Resolved bearer token credentials"
            );

            // Audit log: Credential resolution success
            audit_logger().log_credential_resolve(
                secret_name,
                namespace,
                "bearer_token",
                "success",
            );

            audit_logger().log_secret_access(
                secret_name,
                namespace,
                "success",
                Some("Successfully resolved bearer token".to_string()),
            );

            Ok(Credentials::BearerToken(token))
        } else if data.contains_key("username") && data.contains_key("password") {
            // Basic auth
            let username = parse_secret_string(&data, "username", secret_name)?;
            let password = parse_secret_string(&data, "password", secret_name)?;

            tracing::debug!(
                secret = %secret_name,
                "Resolved basic auth credentials"
            );

            // Audit log: Credential resolution success
            audit_logger().log_credential_resolve(secret_name, namespace, "basic_auth", "success");

            audit_logger().log_secret_access(
                secret_name,
                namespace,
                "success",
                Some("Successfully resolved basic auth".to_string()),
            );

            Ok(Credentials::BasicAuth { username, password })
        } else if data.contains_key("tls.crt") && data.contains_key("tls.key") {
            // mTLS certificates
            let cert = data
                .get("tls.crt")
                .ok_or_else(|| {
                    OperatorError::InvalidConfig(format!(
                        "Secret '{}' missing 'tls.crt' field",
                        secret_name
                    ))
                })?
                .0
                .clone();

            let key = data
                .get("tls.key")
                .ok_or_else(|| {
                    OperatorError::InvalidConfig(format!(
                        "Secret '{}' missing 'tls.key' field",
                        secret_name
                    ))
                })?
                .0
                .clone();

            let ca = data.get("ca.crt").map(|b| b.0.clone());

            tracing::debug!(
                secret = %secret_name,
                has_ca = ca.is_some(),
                "Resolved mTLS credentials"
            );

            // Audit log: Credential resolution success
            audit_logger().log_credential_resolve(secret_name, namespace, "mtls", "success");

            audit_logger().log_secret_access(
                secret_name,
                namespace,
                "success",
                Some("Successfully resolved mTLS credentials".to_string()),
            );

            Ok(Credentials::MutualTls { cert, key, ca })
        } else {
            Err(OperatorError::InvalidConfig(format!(
                "Secret '{}' does not contain recognized credential format. \
                 Expected: 'token' (bearer), 'username'+'password' (basic), \
                 or 'tls.crt'+'tls.key' (mTLS)",
                secret_name
            )))
        }
    }

    /// Resolve multiple secrets and merge them.
    ///
    /// Useful when federation security requires both cluster credentials
    /// and CA certificates from separate secrets.
    pub async fn resolve_multiple(
        &self,
        secret_names: &[String],
        namespace: &str,
    ) -> OperatorResult<Vec<Credentials>> {
        let mut credentials = Vec::with_capacity(secret_names.len());

        for secret_name in secret_names {
            let creds = self.resolve(secret_name, namespace).await?;
            credentials.push(creds);
        }

        Ok(credentials)
    }
}

/// Parse a string field from secret data.
fn parse_secret_string(
    data: &BTreeMap<String, k8s_openapi::ByteString>,
    key: &str,
    secret_name: &str,
) -> OperatorResult<String> {
    let bytes = data
        .get(key)
        .ok_or_else(|| {
            OperatorError::InvalidConfig(format!(
                "Secret '{}' missing '{}' field",
                secret_name, key
            ))
        })?
        .0
        .clone();

    String::from_utf8(bytes).map_err(|_| {
        OperatorError::InvalidConfig(format!(
            "Secret '{}' contains invalid UTF-8 in '{}' field",
            secret_name, key
        ))
    })
}
