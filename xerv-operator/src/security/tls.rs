//! TLS configuration and management for secure federation communication.
//!
//! This module provides utilities for building rustls configurations with support
//! for mTLS (mutual TLS) authentication.

use super::credentials::{Credentials, CredentialsResolver};
use crate::error::{OperatorError, OperatorResult};
use kube::ResourceExt;
use std::sync::Arc;

/// TLS configuration builder for creating rustls ClientConfig.
pub struct TlsConfigBuilder {
    ca_cert: Option<Vec<u8>>,
    client_cert: Option<Vec<u8>>,
    client_key: Option<Vec<u8>>,
    verify_server: bool,
}

impl TlsConfigBuilder {
    /// Create a new TLS config builder.
    pub fn new() -> Self {
        Self {
            ca_cert: None,
            client_cert: None,
            client_key: None,
            verify_server: true,
        }
    }

    /// Set the CA certificate for server verification.
    pub fn with_ca_cert(mut self, ca: Vec<u8>) -> Self {
        self.ca_cert = Some(ca);
        self
    }

    /// Set client certificate and key for mTLS.
    pub fn with_client_cert(mut self, cert: Vec<u8>, key: Vec<u8>) -> Self {
        self.client_cert = Some(cert);
        self.client_key = Some(key);
        self
    }

    /// Set whether to verify the server certificate.
    pub fn with_verify_server(mut self, verify: bool) -> Self {
        self.verify_server = verify;
        self
    }

    /// Build from credentials.
    pub fn from_credentials(creds: &Credentials) -> Self {
        match creds {
            Credentials::MutualTls { cert, key, ca } => {
                let mut builder = Self::new().with_client_cert(cert.clone(), key.clone());

                if let Some(ca_cert) = ca {
                    builder = builder.with_ca_cert(ca_cert.clone());
                }

                builder
            }
            _ => Self::new(),
        }
    }

    /// Build a rustls ClientConfig for use with hyper.
    ///
    /// This creates a TLS configuration that can be used with hyper's
    /// HTTPS connector for mTLS communication with remote clusters.
    pub fn build_rustls_config(self) -> OperatorResult<Arc<rustls::ClientConfig>> {
        use rustls::RootCertStore;
        use rustls::pki_types::CertificateDer;

        let config = rustls::ClientConfig::builder().with_root_certificates({
            let mut root_store = RootCertStore::empty();

            if let Some(ca_cert) = self.ca_cert {
                // Parse CA certificate
                let ca_certs = rustls_pemfile::certs(&mut ca_cert.as_slice())
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| {
                        OperatorError::InvalidConfig(format!("Failed to parse CA cert: {}", e))
                    })?;

                for cert in ca_certs {
                    root_store.add(cert).map_err(|e| {
                        OperatorError::InvalidConfig(format!("Failed to add CA cert: {}", e))
                    })?;
                }
            } else {
                // Use system root certificates
                root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            }

            root_store
        });

        // Add client certificate and key if available (for mTLS)
        let config = if let (Some(cert_pem), Some(key_pem)) = (self.client_cert, self.client_key) {
            // Parse client certificate
            let certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut cert_pem.as_slice())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    OperatorError::InvalidConfig(format!("Failed to parse client cert: {}", e))
                })?;

            // Parse private key
            let key = rustls_pemfile::private_key(&mut key_pem.as_slice())
                .map_err(|e| {
                    OperatorError::InvalidConfig(format!("Failed to parse private key: {}", e))
                })?
                .ok_or_else(|| {
                    OperatorError::InvalidConfig("No private key found in PEM".to_string())
                })?;

            config.with_client_auth_cert(certs, key).map_err(|e| {
                OperatorError::InvalidConfig(format!("Failed to set client auth: {}", e))
            })?
        } else {
            config.with_no_client_auth()
        };

        Ok(Arc::new(config))
    }
}

impl Default for TlsConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Federation security manager.
///
/// Handles loading security configuration from the XervFederation CRD
/// and building TLS configs for secure communication.
pub struct FederationSecurityManager {
    client: kube::Client,
}

impl FederationSecurityManager {
    /// Create a new federation security manager.
    pub fn new(client: kube::Client) -> Self {
        Self { client }
    }

    /// Load security configuration from federation spec.
    ///
    /// Resolves CA and client certificates from Kubernetes secrets
    /// and builds a TLS config for secure federation communication.
    pub async fn load_security_config(
        &self,
        federation: &crate::crd::XervFederation,
        namespace: &str,
    ) -> OperatorResult<Option<Arc<rustls::ClientConfig>>> {
        let security = &federation.spec.security;

        if !security.mtls_enabled {
            tracing::debug!(
                federation = %federation.name_any(),
                "mTLS is disabled for federation"
            );
            return Ok(None);
        }

        tracing::info!(
            federation = %federation.name_any(),
            "Loading mTLS security configuration"
        );

        let resolver = CredentialsResolver::new(self.client.clone());
        let mut builder = TlsConfigBuilder::new().with_verify_server(security.verify_server);

        // Load CA certificate if specified
        if let Some(ref ca_secret_name) = security.ca_secret {
            tracing::debug!(
                secret = %ca_secret_name,
                "Loading CA certificate"
            );

            let ca_creds = resolver.resolve(ca_secret_name, namespace).await?;
            if let Credentials::MutualTls { ca: Some(ca), .. } = ca_creds {
                builder = builder.with_ca_cert(ca);
            }
        }

        // Load client certificate and key if specified
        if let Some(ref cert_secret_name) = security.cert_secret {
            tracing::debug!(
                secret = %cert_secret_name,
                "Loading client certificate for mTLS"
            );

            let cert_creds = resolver.resolve(cert_secret_name, namespace).await?;
            if let Credentials::MutualTls { cert, key, .. } = cert_creds {
                builder = builder.with_client_cert(cert, key);
            }
        }

        let tls_config = builder.build_rustls_config()?;

        tracing::info!(
            federation = %federation.name_any(),
            "Successfully loaded mTLS configuration"
        );

        Ok(Some(tls_config))
    }
}
