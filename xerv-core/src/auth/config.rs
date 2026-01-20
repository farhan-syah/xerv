//! Authentication configuration types.

use super::AuthScope;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Configuration for API authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Whether authentication is enabled.
    pub enabled: bool,
    /// API key configuration.
    pub api_key: Option<ApiKeyConfig>,
    /// Paths exempt from authentication (e.g., ["/health"]).
    pub exempt_paths: Vec<String>,
}

impl AuthConfig {
    /// Create a new authentication configuration with defaults.
    pub fn new() -> Self {
        Self {
            enabled: false,
            api_key: None,
            exempt_paths: vec!["/health".to_string()],
        }
    }

    /// Enable authentication.
    pub fn enabled(mut self) -> Self {
        self.enabled = true;
        self
    }

    /// Set API key configuration.
    pub fn with_api_key(mut self, config: ApiKeyConfig) -> Self {
        self.api_key = Some(config);
        self
    }

    /// Add an exempt path.
    pub fn with_exempt_path(mut self, path: impl Into<String>) -> Self {
        self.exempt_paths.push(path.into());
        self
    }

    /// Check if a path is exempt from authentication.
    pub fn is_path_exempt(&self, path: &str) -> bool {
        // Strip API prefix for matching
        let path = path
            .strip_prefix("/api/v1")
            .unwrap_or(path)
            .trim_end_matches('/');

        self.exempt_paths.iter().any(|exempt| {
            let exempt = exempt.trim_end_matches('/');
            path == exempt || path.starts_with(&format!("{}/", exempt))
        })
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration for API key authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyConfig {
    /// Header name for the API key.
    pub header_name: String,
    /// Hashed API keys mapped to their scopes.
    pub keys: Vec<ApiKeyEntry>,
}

impl ApiKeyConfig {
    /// Create a new API key configuration with default header.
    pub fn new() -> Self {
        Self {
            header_name: "X-API-Key".to_string(),
            keys: Vec::new(),
        }
    }

    /// Set the header name.
    pub fn with_header(mut self, name: impl Into<String>) -> Self {
        self.header_name = name.into();
        self
    }

    /// Add an API key entry.
    pub fn with_key(mut self, entry: ApiKeyEntry) -> Self {
        self.keys.push(entry);
        self
    }
}

impl Default for ApiKeyConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// An API key entry with associated scopes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    /// Identity associated with this key (for audit logs).
    pub identity: String,
    /// SHA-256 hash of the API key (hex encoded).
    pub key_hash: String,
    /// Scopes granted to this key.
    pub scopes: HashSet<AuthScope>,
    /// Optional expiration time (Unix timestamp in seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

impl ApiKeyEntry {
    /// Create a new API key entry.
    pub fn new(identity: impl Into<String>, key_hash: impl Into<String>) -> Self {
        Self {
            identity: identity.into(),
            key_hash: key_hash.into(),
            scopes: HashSet::new(),
            expires_at: None,
        }
    }

    /// Add a scope to this entry.
    pub fn with_scope(mut self, scope: AuthScope) -> Self {
        self.scopes.insert(scope);
        self
    }

    /// Add multiple scopes.
    pub fn with_scopes(mut self, scopes: impl IntoIterator<Item = AuthScope>) -> Self {
        self.scopes.extend(scopes);
        self
    }

    /// Grant all scopes (admin).
    pub fn with_all_scopes(mut self) -> Self {
        self.scopes = AuthScope::all();
        self
    }

    /// Set expiration time as Unix timestamp (seconds since epoch).
    pub fn with_expires_at(mut self, timestamp: u64) -> Self {
        self.expires_at = Some(timestamp);
        self
    }

    /// Set expiration relative to now (in seconds).
    pub fn expires_in(mut self, seconds: u64) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.expires_at = Some(now + seconds);
        self
    }

    /// Check if this key has expired.
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            now >= expires_at
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_config_defaults() {
        let config = AuthConfig::default();
        assert!(!config.enabled);
        assert!(config.api_key.is_none());
        assert!(config.exempt_paths.contains(&"/health".to_string()));
    }

    #[test]
    fn path_exemption() {
        let config = AuthConfig::new()
            .with_exempt_path("/health")
            .with_exempt_path("/status");

        assert!(config.is_path_exempt("/health"));
        assert!(config.is_path_exempt("/api/v1/health"));
        assert!(config.is_path_exempt("/status"));
        assert!(!config.is_path_exempt("/pipelines"));
    }

    #[test]
    fn api_key_entry_scopes() {
        let entry = ApiKeyEntry::new("test-client", "abc123")
            .with_scope(AuthScope::PipelineRead)
            .with_scope(AuthScope::TraceRead);

        assert!(entry.scopes.contains(&AuthScope::PipelineRead));
        assert!(entry.scopes.contains(&AuthScope::TraceRead));
        assert!(!entry.scopes.contains(&AuthScope::Admin));
    }

    #[test]
    fn api_key_entry_all_scopes() {
        let entry = ApiKeyEntry::new("admin", "xyz789").with_all_scopes();
        assert!(entry.scopes.contains(&AuthScope::Admin));
        assert_eq!(entry.scopes.len(), AuthScope::all().len());
    }
}
