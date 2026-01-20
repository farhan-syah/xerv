//! Environment variable provider for abstracting environment access.
//!
//! Allows tests to use mock environment variables while production code
//! uses the real environment.

use parking_lot::RwLock;
use std::collections::HashMap;

/// Provider trait for environment variable operations.
pub trait EnvProvider: Send + Sync {
    /// Get an environment variable.
    fn var(&self, key: &str) -> Option<String>;

    /// Set an environment variable.
    fn set_var(&self, key: &str, value: &str);

    /// Remove an environment variable.
    fn remove_var(&self, key: &str);

    /// Get all environment variables.
    fn vars(&self) -> HashMap<String, String>;

    /// Check if this is a mock provider.
    fn is_mock(&self) -> bool;
}

/// Real environment provider that uses actual environment variables.
pub struct RealEnv;

impl RealEnv {
    /// Create a new real environment provider.
    pub fn new() -> Self {
        Self
    }
}

impl Default for RealEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvProvider for RealEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn set_var(&self, key: &str, value: &str) {
        // SAFETY: This modifies global state. The caller must ensure no other threads
        // are concurrently reading or modifying the same environment variable.
        unsafe { std::env::set_var(key, value) };
    }

    fn remove_var(&self, key: &str) {
        // SAFETY: This modifies global state. The caller must ensure no other threads
        // are concurrently reading or modifying the same environment variable.
        unsafe { std::env::remove_var(key) };
    }

    fn vars(&self) -> HashMap<String, String> {
        std::env::vars().collect()
    }

    fn is_mock(&self) -> bool {
        false
    }
}

/// Mock environment provider for testing.
///
/// Provides an isolated environment that doesn't affect the real process environment.
///
/// # Example
///
/// ```
/// use xerv_core::testing::{MockEnv, EnvProvider};
///
/// let env = MockEnv::new()
///     .with_var("API_KEY", "test-key")
///     .with_var("DEBUG", "true");
///
/// assert_eq!(env.var("API_KEY"), Some("test-key".to_string()));
/// assert_eq!(env.var("DEBUG"), Some("true".to_string()));
/// assert_eq!(env.var("MISSING"), None);
/// ```
pub struct MockEnv {
    vars: RwLock<HashMap<String, String>>,
}

impl MockEnv {
    /// Create a new empty mock environment.
    pub fn new() -> Self {
        Self {
            vars: RwLock::new(HashMap::new()),
        }
    }

    /// Create a mock environment with the given variables.
    pub fn with_vars(vars: HashMap<String, String>) -> Self {
        Self {
            vars: RwLock::new(vars),
        }
    }

    /// Add a variable to the mock environment.
    pub fn with_var(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.write().insert(key.into(), value.into());
        self
    }

    /// Create a mock environment from key-value pairs.
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        let vars: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Self::with_vars(vars)
    }

    /// Clear all variables.
    pub fn clear(&self) {
        self.vars.write().clear();
    }

    /// Get the number of variables.
    pub fn len(&self) -> usize {
        self.vars.read().len()
    }

    /// Check if the environment is empty.
    pub fn is_empty(&self) -> bool {
        self.vars.read().is_empty()
    }
}

impl Default for MockEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvProvider for MockEnv {
    fn var(&self, key: &str) -> Option<String> {
        self.vars.read().get(key).cloned()
    }

    fn set_var(&self, key: &str, value: &str) {
        self.vars.write().insert(key.to_string(), value.to_string());
    }

    fn remove_var(&self, key: &str) {
        self.vars.write().remove(key);
    }

    fn vars(&self) -> HashMap<String, String> {
        self.vars.read().clone()
    }

    fn is_mock(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_env_basic() {
        let env = MockEnv::new().with_var("FOO", "bar").with_var("BAZ", "qux");

        assert_eq!(env.var("FOO"), Some("bar".to_string()));
        assert_eq!(env.var("BAZ"), Some("qux".to_string()));
        assert_eq!(env.var("MISSING"), None);
    }

    #[test]
    fn mock_env_set_and_remove() {
        let env = MockEnv::new();

        env.set_var("KEY", "value");
        assert_eq!(env.var("KEY"), Some("value".to_string()));

        env.remove_var("KEY");
        assert_eq!(env.var("KEY"), None);
    }

    #[test]
    fn mock_env_from_pairs() {
        let env = MockEnv::from_pairs(&[("A", "1"), ("B", "2"), ("C", "3")]);

        assert_eq!(env.var("A"), Some("1".to_string()));
        assert_eq!(env.var("B"), Some("2".to_string()));
        assert_eq!(env.var("C"), Some("3".to_string()));
        assert_eq!(env.len(), 3);
    }

    #[test]
    fn mock_env_vars() {
        let env = MockEnv::new().with_var("X", "1").with_var("Y", "2");

        let vars = env.vars();
        assert_eq!(vars.len(), 2);
        assert_eq!(vars.get("X"), Some(&"1".to_string()));
        assert_eq!(vars.get("Y"), Some(&"2".to_string()));
    }

    #[test]
    fn mock_env_clear() {
        let env = MockEnv::new().with_var("A", "1").with_var("B", "2");

        assert_eq!(env.len(), 2);
        env.clear();
        assert!(env.is_empty());
    }

    #[test]
    fn mock_env_isolation() {
        // Verify mock doesn't affect real environment
        let key = "XERV_TEST_ISOLATION_KEY";
        let env = MockEnv::new().with_var(key, "mock_value");

        assert_eq!(env.var(key), Some("mock_value".to_string()));
        assert!(std::env::var(key).is_err());
    }
}
