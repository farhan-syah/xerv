//! Authentication middleware for HTTP requests.

use super::api_key::ApiKeyValidator;
use super::config::AuthConfig;
use super::context::AuthContext;
use crate::error::{Result, XervError};

/// Authentication middleware that validates requests.
pub struct AuthMiddleware;

impl AuthMiddleware {
    /// Authenticate a request based on the provided configuration.
    ///
    /// Returns an `AuthContext` if authentication succeeds, or an error if it fails.
    pub fn authenticate(
        config: &AuthConfig,
        path: &str,
        headers: &impl HeaderAccess,
    ) -> Result<AuthContext> {
        // If auth is disabled, return anonymous context
        if !config.enabled {
            return Ok(AuthContext::new("anonymous", super::AuthScope::all()));
        }

        // Check if path is exempt
        if config.is_path_exempt(path) {
            return Ok(AuthContext::anonymous());
        }

        // Try API key authentication
        if let Some(api_key_config) = &config.api_key {
            let validator = ApiKeyValidator::new(api_key_config);
            let header_name = validator.header_name();

            if let Some(key) = headers.get_header(header_name) {
                if let Some(ctx) = validator.validate(&key) {
                    tracing::debug!(identity = %ctx.identity, "Authenticated via API key");
                    return Ok(ctx);
                } else {
                    tracing::warn!(path = %path, "Invalid API key provided");
                    return Err(XervError::AuthenticationFailed {
                        cause: "Invalid API key".to_string(),
                    });
                }
            }
        }

        // No valid authentication method found
        tracing::warn!(path = %path, "Missing authentication credentials");
        Err(XervError::AuthenticationFailed {
            cause: "Missing authentication credentials".to_string(),
        })
    }

    /// Check if a path requires authentication based on config.
    pub fn requires_auth(config: &AuthConfig, path: &str) -> bool {
        config.enabled && !config.is_path_exempt(path)
    }
}

/// Trait for accessing HTTP headers.
///
/// This allows the middleware to be used with different HTTP libraries.
pub trait HeaderAccess {
    /// Get the value of a header by name (case-insensitive).
    fn get_header(&self, name: &str) -> Option<String>;
}

/// Simple header map implementation for testing.
#[derive(Debug, Default)]
pub struct SimpleHeaders {
    headers: std::collections::HashMap<String, String>,
}

impl SimpleHeaders {
    /// Create a new empty header map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a header.
    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.headers
            .insert(name.into().to_lowercase(), value.into());
    }

    /// Create with a single header.
    pub fn with(name: impl Into<String>, value: impl Into<String>) -> Self {
        let mut headers = Self::new();
        headers.insert(name, value);
        headers
    }
}

impl HeaderAccess for SimpleHeaders {
    fn get_header(&self, name: &str) -> Option<String> {
        self.headers.get(&name.to_lowercase()).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthScope, api_key::ApiKeyBuilder, config::ApiKeyConfig};

    fn test_config() -> AuthConfig {
        let api_key = ApiKeyBuilder::new("test-client")
            .with_scope(AuthScope::PipelineRead)
            .build_with_key("valid-key");

        AuthConfig::new()
            .enabled()
            .with_api_key(ApiKeyConfig::new().with_key(api_key))
            .with_exempt_path("/health")
    }

    #[test]
    fn auth_disabled_allows_all() {
        let config = AuthConfig::new(); // disabled by default
        let headers = SimpleHeaders::new();

        let result = AuthMiddleware::authenticate(&config, "/pipelines", &headers);
        assert!(result.is_ok());
        let ctx = result.unwrap();
        assert_eq!(ctx.identity, "anonymous");
        assert!(ctx.has_scope(AuthScope::PipelineRead));
        assert!(ctx.has_scope(AuthScope::PipelineWrite));
    }

    #[test]
    fn exempt_path_allows_anonymous() {
        let config = test_config();
        let headers = SimpleHeaders::new();

        let result = AuthMiddleware::authenticate(&config, "/health", &headers);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().identity, "anonymous");
    }

    #[test]
    fn valid_api_key_authenticates() {
        let config = test_config();
        let headers = SimpleHeaders::with("X-API-Key", "valid-key");

        let result = AuthMiddleware::authenticate(&config, "/pipelines", &headers);
        assert!(result.is_ok());

        let ctx = result.unwrap();
        assert_eq!(ctx.identity, "test-client");
        assert!(ctx.has_scope(AuthScope::PipelineRead));
    }

    #[test]
    fn invalid_api_key_fails() {
        let config = test_config();
        let headers = SimpleHeaders::with("X-API-Key", "invalid-key");

        let result = AuthMiddleware::authenticate(&config, "/pipelines", &headers);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            XervError::AuthenticationFailed { .. }
        ));
    }

    #[test]
    fn missing_credentials_fails() {
        let config = test_config();
        let headers = SimpleHeaders::new();

        let result = AuthMiddleware::authenticate(&config, "/pipelines", &headers);
        assert!(result.is_err());
    }

    #[test]
    fn requires_auth_check() {
        let config = test_config();

        assert!(AuthMiddleware::requires_auth(&config, "/pipelines"));
        assert!(!AuthMiddleware::requires_auth(&config, "/health"));

        let disabled_config = AuthConfig::new();
        assert!(!AuthMiddleware::requires_auth(
            &disabled_config,
            "/pipelines"
        ));
    }
}
