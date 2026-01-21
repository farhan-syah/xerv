//! HTTP client with authentication credential injection.
//!
//! This module provides utilities for injecting authentication credentials
//! (bearer tokens, basic auth, mTLS) into HTTP requests.

use super::credentials::Credentials;

/// HTTP client builder with credential injection.
pub struct AuthenticatedHttpClient {
    credentials: Option<Credentials>,
}

impl AuthenticatedHttpClient {
    /// Create a new client without credentials.
    pub fn new() -> Self {
        Self { credentials: None }
    }

    /// Create a client with credentials.
    pub fn with_credentials(credentials: Credentials) -> Self {
        Self {
            credentials: Some(credentials),
        }
    }

    /// Build an HTTP request with injected authentication.
    ///
    /// This method adds appropriate headers based on the credential type:
    /// - Bearer token: `Authorization: Bearer <token>`
    /// - Basic auth: `Authorization: Basic <base64(username:password)>`
    /// - mTLS: Returns credentials for TLS handshake (handled by connector)
    pub fn inject_auth<B>(&self, mut request: http::Request<B>) -> http::Request<B> {
        if let Some(ref creds) = self.credentials {
            match creds {
                Credentials::BearerToken(token) => {
                    request.headers_mut().insert(
                        http::header::AUTHORIZATION,
                        http::HeaderValue::from_str(&format!("Bearer {}", token))
                            .expect("Invalid bearer token - this should never happen"),
                    );
                    tracing::debug!("Injected bearer token authentication");

                    // Note: Audit logging should be done by caller with cluster context
                }
                Credentials::BasicAuth { username, password } => {
                    use base64::Engine;
                    let auth_value = base64::engine::general_purpose::STANDARD
                        .encode(format!("{}:{}", username, password));
                    request.headers_mut().insert(
                        http::header::AUTHORIZATION,
                        http::HeaderValue::from_str(&format!("Basic {}", auth_value))
                            .expect("Invalid basic auth - this should never happen"),
                    );
                    tracing::debug!("Injected basic authentication");
                }
                Credentials::MutualTls { .. } => {
                    // mTLS is handled at the connection level, not via headers
                    tracing::debug!("mTLS credentials present (will be used at connection level)");
                }
            }
        }

        request
    }

    /// Get mTLS credentials if available.
    ///
    /// Returns `None` if credentials are not mTLS type.
    pub fn get_mtls_credentials(&self) -> Option<&Credentials> {
        match &self.credentials {
            Some(creds @ Credentials::MutualTls { .. }) => Some(creds),
            _ => None,
        }
    }
}

impl Default for AuthenticatedHttpClient {
    fn default() -> Self {
        Self::new()
    }
}
