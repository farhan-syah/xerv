//! Security utilities for federation and cross-cluster communication.
//!
//! This module provides:
//! - Secret resolution from Kubernetes
//! - Credential injection into HTTP requests
//! - mTLS certificate management
//! - Secure secret distribution to remote clusters

mod auth_client;
mod credentials;
mod distributor;
mod tls;

// Re-export public types
pub use auth_client::AuthenticatedHttpClient;
pub use credentials::{Credentials, CredentialsResolver};
pub use distributor::SecretDistributor;
pub use tls::{FederationSecurityManager, TlsConfigBuilder};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_bearer_token() {
        let creds = Credentials::BearerToken("test-token".to_string());
        assert!(matches!(creds, Credentials::BearerToken(_)));
    }

    #[test]
    fn authenticated_client_builder() {
        let client = AuthenticatedHttpClient::new();
        assert!(client.get_mtls_credentials().is_none());

        let client = AuthenticatedHttpClient::with_credentials(Credentials::BearerToken(
            "token".to_string(),
        ));
        assert!(client.get_mtls_credentials().is_none());
    }

    #[test]
    fn inject_bearer_token() {
        let client = AuthenticatedHttpClient::with_credentials(Credentials::BearerToken(
            "test-token".to_string(),
        ));

        let req = http::Request::builder()
            .method("GET")
            .uri("/test")
            .body(())
            .unwrap();

        let req = client.inject_auth(req);

        assert_eq!(
            req.headers().get(http::header::AUTHORIZATION).unwrap(),
            "Bearer test-token"
        );
    }

    #[test]
    fn inject_basic_auth() {
        let client = AuthenticatedHttpClient::with_credentials(Credentials::BasicAuth {
            username: "user".to_string(),
            password: "pass".to_string(),
        });

        let req = http::Request::builder()
            .method("GET")
            .uri("/test")
            .body(())
            .unwrap();

        let req = client.inject_auth(req);

        let auth_header = req.headers().get(http::header::AUTHORIZATION).unwrap();
        assert!(auth_header.to_str().unwrap().starts_with("Basic "));
    }

    #[test]
    fn tls_config_builder() {
        let builder = TlsConfigBuilder::new();
        // TlsConfigBuilder fields are private, but we can test the builder pattern
        let builder = builder
            .with_ca_cert(vec![1, 2, 3])
            .with_verify_server(false);

        // Builder pattern works correctly
        let _ = builder.with_client_cert(vec![4, 5, 6], vec![7, 8, 9]);
    }

    #[test]
    fn tls_config_from_credentials() {
        let mtls_creds = Credentials::MutualTls {
            cert: vec![1, 2, 3],
            key: vec![4, 5, 6],
            ca: Some(vec![7, 8, 9]),
        };

        let _builder = TlsConfigBuilder::from_credentials(&mtls_creds);

        let bearer_creds = Credentials::BearerToken("token".to_string());
        let _builder = TlsConfigBuilder::from_credentials(&bearer_creds);
    }
}
