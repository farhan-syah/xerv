//! Secrets provider for abstracting secret management.
//!
//! Allows tests to use mock secrets while production code uses a real secret manager.

use parking_lot::RwLock;
use std::collections::HashMap;

/// Provider trait for secret management.
///
/// Secrets are sensitive values that should not be logged or exposed.
/// Unlike environment variables, secrets are typically stored in a
/// secure vault or secret manager.
pub trait SecretsProvider: Send + Sync {
    /// Get a secret by key.
    fn get(&self, key: &str) -> Option<String>;

    /// Check if a secret exists.
    fn exists(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// List all secret keys (but not values).
    fn keys(&self) -> Vec<String>;

    /// Check if this is a mock provider.
    fn is_mock(&self) -> bool;
}

/// Real secrets provider that reads from environment variables.
///
/// This is a simple implementation that reads secrets from environment
/// variables. In production, you might want to integrate with a proper
/// secret manager like HashiCorp Vault, AWS Secrets Manager, etc.
pub struct RealSecrets {
    prefix: String,
}

impl RealSecrets {
    /// Create a new real secrets provider.
    ///
    /// Secrets are read from environment variables with the given prefix.
    /// For example, with prefix "APP_SECRET_", the key "API_KEY" would
    /// be read from "APP_SECRET_API_KEY".
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }

    /// Create a real secrets provider with no prefix.
    pub fn no_prefix() -> Self {
        Self {
            prefix: String::new(),
        }
    }
}

impl Default for RealSecrets {
    fn default() -> Self {
        Self::no_prefix()
    }
}

impl SecretsProvider for RealSecrets {
    fn get(&self, key: &str) -> Option<String> {
        let env_key = format!("{}{}", self.prefix, key);
        std::env::var(&env_key).ok()
    }

    fn keys(&self) -> Vec<String> {
        std::env::vars()
            .filter_map(|(k, _)| {
                if k.starts_with(&self.prefix) {
                    Some(k[self.prefix.len()..].to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    fn is_mock(&self) -> bool {
        false
    }
}

/// Mock secrets provider for testing.
///
/// # Example
///
/// ```
/// use xerv_core::testing::{MockSecrets, SecretsProvider};
///
/// let secrets = MockSecrets::new()
///     .with_secret("API_KEY", "sk-test-12345")
///     .with_secret("DB_PASSWORD", "super-secret");
///
/// assert_eq!(secrets.get("API_KEY"), Some("sk-test-12345".to_string()));
/// assert!(secrets.exists("DB_PASSWORD"));
/// assert!(!secrets.exists("MISSING"));
/// ```
pub struct MockSecrets {
    secrets: RwLock<HashMap<String, String>>,
}

impl MockSecrets {
    /// Create a new empty mock secrets provider.
    pub fn new() -> Self {
        Self {
            secrets: RwLock::new(HashMap::new()),
        }
    }

    /// Add a secret.
    pub fn with_secret(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.secrets.write().insert(key.into(), value.into());
        self
    }

    /// Create from key-value pairs.
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        let secrets: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Self {
            secrets: RwLock::new(secrets),
        }
    }

    /// Set a secret (for dynamic updates during tests).
    pub fn set(&self, key: impl Into<String>, value: impl Into<String>) {
        self.secrets.write().insert(key.into(), value.into());
    }

    /// Remove a secret.
    pub fn remove(&self, key: &str) {
        self.secrets.write().remove(key);
    }

    /// Clear all secrets.
    pub fn clear(&self) {
        self.secrets.write().clear();
    }
}

impl Default for MockSecrets {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretsProvider for MockSecrets {
    fn get(&self, key: &str) -> Option<String> {
        self.secrets.read().get(key).cloned()
    }

    fn keys(&self) -> Vec<String> {
        self.secrets.read().keys().cloned().collect()
    }

    fn is_mock(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_secrets_basic() {
        let secrets = MockSecrets::new()
            .with_secret("KEY1", "value1")
            .with_secret("KEY2", "value2");

        assert_eq!(secrets.get("KEY1"), Some("value1".to_string()));
        assert_eq!(secrets.get("KEY2"), Some("value2".to_string()));
        assert_eq!(secrets.get("KEY3"), None);
    }

    #[test]
    fn mock_secrets_exists() {
        let secrets = MockSecrets::new().with_secret("EXISTS", "value");

        assert!(secrets.exists("EXISTS"));
        assert!(!secrets.exists("MISSING"));
    }

    #[test]
    fn mock_secrets_keys() {
        let secrets = MockSecrets::new()
            .with_secret("A", "1")
            .with_secret("B", "2")
            .with_secret("C", "3");

        let mut keys = secrets.keys();
        keys.sort();
        assert_eq!(keys, vec!["A", "B", "C"]);
    }

    #[test]
    fn mock_secrets_dynamic_update() {
        let secrets = MockSecrets::new();

        secrets.set("DYNAMIC", "initial");
        assert_eq!(secrets.get("DYNAMIC"), Some("initial".to_string()));

        secrets.set("DYNAMIC", "updated");
        assert_eq!(secrets.get("DYNAMIC"), Some("updated".to_string()));

        secrets.remove("DYNAMIC");
        assert_eq!(secrets.get("DYNAMIC"), None);
    }

    #[test]
    fn mock_secrets_from_pairs() {
        let secrets = MockSecrets::from_pairs(&[("X", "1"), ("Y", "2")]);

        assert_eq!(secrets.get("X"), Some("1".to_string()));
        assert_eq!(secrets.get("Y"), Some("2".to_string()));
    }
}
