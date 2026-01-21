//! Secret distribution to remote clusters.
//!
//! This module provides functionality for distributing Kubernetes secrets from
//! the local cluster to remote federated clusters.

use super::auth_client::AuthenticatedHttpClient;
use super::credentials::Credentials;
use crate::audit::audit_logger;
use crate::error::{OperatorError, OperatorResult};
use k8s_openapi::api::core::v1::Secret;
use kube::Api;

/// Secret distributor for sending secrets to remote clusters.
pub struct SecretDistributor {
    client: kube::Client,
}

impl SecretDistributor {
    /// Create a new secret distributor.
    pub fn new(client: kube::Client) -> Self {
        Self { client }
    }

    /// Distribute a secret to a remote cluster.
    ///
    /// This ensures that when a pipeline references a secret, that secret
    /// is available on the target cluster.
    ///
    /// # Arguments
    ///
    /// * `secret_name` - Name of the secret to distribute
    /// * `source_namespace` - Source namespace (local)
    /// * `target_endpoint` - Target cluster endpoint
    /// * `target_namespace` - Target namespace on remote cluster
    /// * `credentials` - Credentials for authenticating to target cluster
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No credentials are provided
    /// - The secret doesn't exist in the source cluster
    /// - Network communication fails
    /// - The remote cluster rejects the secret
    pub async fn distribute_secret(
        &self,
        secret_name: &str,
        source_namespace: &str,
        target_endpoint: &str,
        target_namespace: &str,
        credentials: Option<&Credentials>,
    ) -> OperatorResult<()> {
        tracing::info!(
            secret = %secret_name,
            source_namespace = %source_namespace,
            target_endpoint = %target_endpoint,
            target_namespace = %target_namespace,
            "Distributing secret to remote cluster"
        );

        // Fetch the secret from source
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), source_namespace);
        let secret = secrets.get(secret_name).await.map_err(|e| {
            OperatorError::HttpError(format!(
                "Failed to fetch secret '{}' from source namespace '{}': {}",
                secret_name, source_namespace, e
            ))
        })?;

        // Serialize secret for transmission
        let mut secret_copy = secret.clone();
        secret_copy.metadata.namespace = Some(target_namespace.to_string());
        secret_copy.metadata.resource_version = None;
        secret_copy.metadata.uid = None;

        let payload = serde_json::to_vec(&secret_copy)?;

        // Send to remote cluster
        let url = format!(
            "{}/api/v1/namespaces/{}/secrets",
            target_endpoint, target_namespace
        );

        // Ensure credentials are provided
        let creds = credentials.ok_or_else(|| {
            OperatorError::InvalidConfig(format!(
                "No credentials provided for secret distribution to cluster: {}",
                target_endpoint
            ))
        })?;

        let client = AuthenticatedHttpClient::with_credentials(creds.clone());

        match self.http_put_secret(&url, payload, &client).await {
            Ok(()) => {
                tracing::info!(
                    secret = %secret_name,
                    target = %target_endpoint,
                    "Successfully distributed secret"
                );

                // Audit log: Successful secret distribution
                audit_logger().log_secret_distribution(
                    secret_name,
                    source_namespace,
                    target_endpoint,
                    "success",
                    None,
                );

                Ok(())
            }
            Err(e) => {
                tracing::error!(
                    secret = %secret_name,
                    target = %target_endpoint,
                    error = %e,
                    "Failed to distribute secret"
                );

                // Audit log: Failed secret distribution
                audit_logger().log_secret_distribution(
                    secret_name,
                    source_namespace,
                    target_endpoint,
                    "failed",
                    Some(e.to_string()),
                );

                // Add context to error
                Err(OperatorError::HttpError(format!(
                    "Failed to distribute secret '{}' to {}: {}",
                    secret_name, target_endpoint, e
                )))
            }
        }
    }

    /// Send secret via HTTP PUT to remote cluster.
    async fn http_put_secret(
        &self,
        url: &str,
        payload: Vec<u8>,
        client: &AuthenticatedHttpClient,
    ) -> OperatorResult<()> {
        use bytes::Bytes;
        use http_body_util::Full;

        let uri: http::Uri = url.parse().map_err(|e| {
            OperatorError::HttpError(format!("Invalid secret distribution URL '{}': {}", url, e))
        })?;

        let host = uri
            .host()
            .ok_or_else(|| OperatorError::HttpError(format!("Missing host in URL: {}", url)))?;

        let scheme = uri.scheme_str().unwrap_or("http");
        let port = uri
            .port_u16()
            .unwrap_or(if scheme == "https" { 443 } else { 80 });
        let addr = format!("{}:{}", host, port);
        let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");

        // Build request
        let req = http::Request::builder()
            .method("PUT")
            .uri(path)
            .header("Host", host)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(payload)))
            .map_err(|e| OperatorError::HttpError(format!("Failed to build request: {}", e)))?;

        // Inject authentication
        let req = client.inject_auth(req);

        // Connect and send
        let stream = tokio::net::TcpStream::connect(&addr).await.map_err(|e| {
            OperatorError::HttpError(format!("Failed to connect to {}: {}", addr, e))
        })?;

        let io = hyper_util::rt::TokioIo::new(stream);

        let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
            .map_err(|e| OperatorError::HttpError(format!("HTTP handshake failed: {}", e)))?;

        tokio::spawn(async move {
            if let Err(e) = conn.await {
                tracing::debug!(error = %e, "Secret distribution connection error");
            }
        });

        let response = sender.send_request(req).await.map_err(|e| {
            OperatorError::HttpError(format!("Secret distribution request failed: {}", e))
        })?;

        if !response.status().is_success() {
            return Err(OperatorError::HttpError(format!(
                "Secret distribution returned status {} for URL {}",
                response.status(),
                url
            )));
        }

        Ok(())
    }
}
