//! API key validation utilities.

use super::config::ApiKeyConfig;
use super::context::{AuthContext, AuthScope};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// A hashed API key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyHash(String);

impl ApiKeyHash {
    /// Create a new hash from raw bytes (hex encoded).
    pub fn new(hash: impl Into<String>) -> Self {
        Self(hash.into().to_lowercase())
    }

    /// Hash a plaintext API key.
    pub fn from_plaintext(key: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let result = hasher.finalize();
        Self(hex::encode(result))
    }

    /// Get the hex-encoded hash.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ApiKeyHash {
    fn from(s: &str) -> Self {
        Self::from_plaintext(s)
    }
}

/// Validates API keys against configured entries.
pub struct ApiKeyValidator<'a> {
    config: &'a ApiKeyConfig,
}

impl<'a> ApiKeyValidator<'a> {
    /// Create a new validator with the given configuration.
    pub fn new(config: &'a ApiKeyConfig) -> Self {
        Self { config }
    }

    /// Get the expected header name for API keys.
    pub fn header_name(&self) -> &str {
        &self.config.header_name
    }

    /// Validate an API key and return the authentication context.
    ///
    /// Returns `None` if the key is invalid or not found.
    pub fn validate(&self, key: &str) -> Option<AuthContext> {
        let key_hash = ApiKeyHash::from_plaintext(key);

        for entry in &self.config.keys {
            if entry.key_hash.to_lowercase() == key_hash.as_str() {
                return Some(AuthContext::new(
                    entry.identity.clone(),
                    entry.scopes.clone(),
                ));
            }
        }

        None
    }

    /// Validate a pre-hashed API key.
    pub fn validate_hash(&self, hash: &ApiKeyHash) -> Option<AuthContext> {
        for entry in &self.config.keys {
            if entry.key_hash.to_lowercase() == hash.as_str() {
                return Some(AuthContext::new(
                    entry.identity.clone(),
                    entry.scopes.clone(),
                ));
            }
        }

        None
    }
}

/// Builder for creating API key entries for testing or configuration.
pub struct ApiKeyBuilder {
    identity: String,
    scopes: HashSet<AuthScope>,
}

impl ApiKeyBuilder {
    /// Create a new builder with the given identity.
    pub fn new(identity: impl Into<String>) -> Self {
        Self {
            identity: identity.into(),
            scopes: HashSet::new(),
        }
    }

    /// Add a scope.
    pub fn with_scope(mut self, scope: AuthScope) -> Self {
        self.scopes.insert(scope);
        self
    }

    /// Add read-only scopes.
    pub fn read_only(mut self) -> Self {
        self.scopes.extend(AuthScope::read_only());
        self
    }

    /// Add operator scopes.
    pub fn operator(mut self) -> Self {
        self.scopes.extend(AuthScope::operator());
        self
    }

    /// Add all scopes (admin).
    pub fn admin(mut self) -> Self {
        self.scopes.extend(AuthScope::all());
        self
    }

    /// Build the API key entry with a plaintext key.
    pub fn build_with_key(self, plaintext_key: &str) -> super::config::ApiKeyEntry {
        let hash = ApiKeyHash::from_plaintext(plaintext_key);
        super::config::ApiKeyEntry {
            identity: self.identity,
            key_hash: hash.0,
            scopes: self.scopes,
        }
    }

    /// Build the API key entry with a pre-computed hash.
    pub fn build_with_hash(self, hash: impl Into<String>) -> super::config::ApiKeyEntry {
        super::config::ApiKeyEntry {
            identity: self.identity,
            key_hash: hash.into().to_lowercase(),
            scopes: self.scopes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::config::ApiKeyConfig;

    fn test_config() -> ApiKeyConfig {
        let entry = ApiKeyBuilder::new("test-client")
            .with_scope(AuthScope::PipelineRead)
            .with_scope(AuthScope::TraceRead)
            .build_with_key("test-api-key-12345");

        ApiKeyConfig::new().with_key(entry)
    }

    #[test]
    fn hash_consistency() {
        let hash1 = ApiKeyHash::from_plaintext("test-key");
        let hash2 = ApiKeyHash::from_plaintext("test-key");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn hash_different_keys() {
        let hash1 = ApiKeyHash::from_plaintext("key1");
        let hash2 = ApiKeyHash::from_plaintext("key2");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn validate_valid_key() {
        let config = test_config();
        let validator = ApiKeyValidator::new(&config);

        let ctx = validator.validate("test-api-key-12345");
        assert!(ctx.is_some());

        let ctx = ctx.unwrap();
        assert_eq!(ctx.identity, "test-client");
        assert!(ctx.has_scope(AuthScope::PipelineRead));
        assert!(ctx.has_scope(AuthScope::TraceRead));
        assert!(!ctx.has_scope(AuthScope::PipelineWrite));
    }

    #[test]
    fn validate_invalid_key() {
        let config = test_config();
        let validator = ApiKeyValidator::new(&config);

        let ctx = validator.validate("invalid-key");
        assert!(ctx.is_none());
    }

    #[test]
    fn builder_admin() {
        let entry = ApiKeyBuilder::new("admin-user")
            .admin()
            .build_with_key("admin-key");

        assert!(entry.scopes.contains(&AuthScope::Admin));
        assert!(entry.scopes.contains(&AuthScope::PipelineRead));
        assert!(entry.scopes.contains(&AuthScope::PipelineWrite));
    }

    #[test]
    fn builder_operator() {
        let entry = ApiKeyBuilder::new("operator")
            .operator()
            .build_with_key("op-key");

        assert!(entry.scopes.contains(&AuthScope::PipelineRead));
        assert!(entry.scopes.contains(&AuthScope::TraceRead));
        assert!(entry.scopes.contains(&AuthScope::TraceResume));
        assert!(!entry.scopes.contains(&AuthScope::PipelineWrite));
    }
}
