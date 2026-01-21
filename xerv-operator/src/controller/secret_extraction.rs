//! Secret reference extraction from pipeline definitions.
//!
//! This module provides utilities to scan pipeline YAML definitions and configuration
//! for secret references that need to be distributed to federated clusters.
//!
//! # Supported Secret Reference Patterns
//!
//! - Direct field references: `secretName: my-secret`, `credentialsSecret: creds`
//! - Template syntax: `${secret.name}` or `${secrets/name}`
//! - Kubernetes secret references: `valueFrom.secretKeyRef.name: secret-name`
//! - Prefix notation: `secret:name` or `secret/name`
//!
//! # Example
//!
//! ```ignore
//! use xerv_operator::controller::secret_extraction::SecretExtractor;
//!
//! let extractor = SecretExtractor::new();
//! let secrets = extractor.extract_from_yaml(pipeline_yaml);
//! ```

use crate::crd::{ClusterOverride, PipelineDefinition};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;

/// Macro for creating static regex patterns with consistent error messages.
///
/// All regex patterns in this module are statically compiled and guaranteed to be valid.
/// If any pattern fails to compile, it indicates a bug in the operator code.
macro_rules! static_regex {
    ($pattern:expr, $name:expr) => {
        Regex::new($pattern).unwrap_or_else(|_| {
            panic!(
                "Static regex '{}' failed to compile - this is a bug in the operator",
                $name
            )
        })
    };
}

/// Matches secret field declarations in YAML: "secretName: my-secret" or "credentialsSecret: creds"
///
/// Pattern: `^\s*\w*[Ss]ecret\w*:\s*([a-zA-Z0-9\-_.]+)`
///
/// Captures the secret name from lines like:
/// - `secretName: my-secret`
/// - `credentialsSecret: github-token`
/// - `authSecret: api-key`
static SECRET_KEY_PATTERN: Lazy<Regex> = Lazy::new(|| {
    static_regex!(
        r"(?m)^\s*\w*[Ss]ecret\w*:\s*([a-zA-Z0-9\-_.]+)",
        "SECRET_KEY_PATTERN"
    )
});

/// Matches template syntax: `${secret.name}` or `${secrets/name}`
///
/// Pattern: `\$\{secrets?[/.]([a-zA-Z0-9\-_.]+)\}`
///
/// Captures the secret name from:
/// - `${secret.my-secret}`
/// - `${secrets/my-secret}`
/// - `${secret.github-token}`
static TEMPLATE_PATTERN: Lazy<Regex> =
    Lazy::new(|| static_regex!(r"\$\{secrets?[/.]([a-zA-Z0-9\-_.]+)\}", "TEMPLATE_PATTERN"));

/// Matches name fields in Kubernetes secret references (used with context checking)
///
/// Pattern: `^\s*name:\s*([a-zA-Z0-9\-_.]+)`
///
/// Used in conjunction with `secretKeyRef` context to extract:
/// ```yaml
/// valueFrom:
///   secretKeyRef:
///     name: my-secret  # <-- This line
/// ```
static K8S_SECRET_NAME_PATTERN: Lazy<Regex> = Lazy::new(|| {
    static_regex!(
        r"(?m)^\s*name:\s*([a-zA-Z0-9\-_.]+)",
        "K8S_SECRET_NAME_PATTERN"
    )
});

/// Matches secret prefix in values: "secret:name" or "secret/name"
///
/// Pattern: `^secrets?[:/]([a-zA-Z0-9\-_.]+)$`
///
/// Captures the secret name from:
/// - `secret:my-secret`
/// - `secrets/my-secret`
static SECRET_PREFIX_PATTERN: Lazy<Regex> =
    Lazy::new(|| static_regex!(r"^secrets?[:/]([a-zA-Z0-9\-_.]+)$", "SECRET_PREFIX_PATTERN"));

/// Secret reference extractor for pipeline definitions.
///
/// Scans pipeline YAML, trigger configurations, and override settings for
/// references to Kubernetes secrets that need to be distributed to federated clusters.
///
/// # Design Philosophy
///
/// The extractor uses multiple heuristics to catch secret references:
/// 1. Field name heuristics (keys containing "secret")
/// 2. Value pattern matching (template syntax, prefix notation)
/// 3. Kubernetes-specific patterns (secretKeyRef references)
///
/// This multi-pronged approach minimizes false negatives while accepting
/// some false positives (which are harmless - they just trigger unnecessary secret lookups).
pub struct SecretExtractor;

impl SecretExtractor {
    /// Create a new secret extractor.
    pub fn new() -> Self {
        Self
    }

    /// Extract all secret references from a pipeline definition.
    ///
    /// This is the main entry point that scans:
    /// - Git credentials
    /// - Inline YAML pipeline definitions
    /// - Trigger configurations
    ///
    /// Returns a sorted, deduplicated list of secret names.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let extractor = SecretExtractor::new();
    /// let secrets = extractor.extract_from_pipeline(&pipeline_def);
    /// // secrets might be: ["github-token", "api-key", "webhook-secret"]
    /// ```
    pub fn extract_from_pipeline(&self, pipeline_def: &PipelineDefinition) -> Vec<String> {
        let mut secret_refs = HashSet::new();

        // 1. Extract Git credentials secret
        if let Some(ref git) = pipeline_def.source.git {
            if let Some(ref secret) = git.credentials_secret {
                secret_refs.insert(secret.clone());
            }
        }

        // 2. Parse inline YAML for secret references
        if let Some(ref inline_yaml) = pipeline_def.source.inline {
            secret_refs.extend(self.extract_from_yaml(inline_yaml));
        }

        // 3. Scan trigger configurations for secret references
        for trigger in &pipeline_def.triggers {
            for (key, value) in &trigger.config {
                // Look for secret references in trigger config
                // Common patterns: secretName, credentialsSecret, authSecret, tokenSecret
                if key.to_lowercase().contains("secret") {
                    secret_refs.insert(value.clone());
                }

                // Also check if the value itself references a secret
                // Format: secret:secret-name or ${secret.secret-name}
                secret_refs.extend(self.extract_from_value(value));
            }
        }

        // 4. Note: Override env vars are handled separately via extract_from_overrides()
        // They're in FederatedPipelineSpec.overrides, not PipelineDefinition

        let mut refs: Vec<String> = secret_refs.into_iter().collect();
        refs.sort();
        refs
    }

    /// Extract secret references from a YAML string.
    ///
    /// Scans for common secret reference patterns:
    /// - `secretName: some-secret`
    /// - `credentialsSecret: some-secret`
    /// - `secret: some-secret`
    /// - `${secret.some-secret}`
    /// - `valueFrom.secretKeyRef.name: some-secret`
    ///
    /// # Implementation Notes
    ///
    /// - Pattern 1: Uses SECRET_KEY_PATTERN to find YAML fields with "secret" in the name
    /// - Pattern 2: Uses TEMPLATE_PATTERN to find `${secret.name}` references
    /// - Pattern 3: Uses context-aware matching for `secretKeyRef.name` (Kubernetes style)
    ///
    /// Returns a sorted, deduplicated list of secret names.
    pub fn extract_from_yaml(&self, yaml: &str) -> Vec<String> {
        let mut secrets = Vec::new();

        // Pattern 1: YAML key-value pairs with "secret" in the key
        // Example: "secretName: my-secret" or "credentialsSecret: creds"
        for cap in SECRET_KEY_PATTERN.captures_iter(yaml) {
            if let Some(secret_name) = cap.get(1) {
                let name = secret_name.as_str().to_string();
                // Filter out boolean values and common non-secret words
                if name != "true" && name != "false" && name != "null" {
                    secrets.push(name);
                }
            }
        }

        // Pattern 2: Template syntax ${secret.name} or ${secrets/name}
        for cap in TEMPLATE_PATTERN.captures_iter(yaml) {
            if let Some(secret_name) = cap.get(1) {
                secrets.push(secret_name.as_str().to_string());
            }
        }

        // Pattern 3: valueFrom.secretKeyRef.name (Kubernetes secret reference)
        // Only capture if preceded by "secretKeyRef" within 5 lines
        let lines: Vec<&str> = yaml.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.contains("secretKeyRef") {
                // Check next few lines for "name:"
                for next_line in lines.iter().skip(i + 1).take(5) {
                    if let Some(cap) = K8S_SECRET_NAME_PATTERN.captures(next_line) {
                        if let Some(secret_name) = cap.get(1) {
                            secrets.push(secret_name.as_str().to_string());
                            break;
                        }
                    }
                }
            }
        }

        secrets.sort();
        secrets.dedup();
        secrets
    }

    /// Extract secret references from a configuration value string.
    ///
    /// Looks for patterns like:
    /// - `secret:my-secret`
    /// - `secret/my-secret`
    /// - `${secret.my-secret}`
    ///
    /// This is used for scanning individual configuration values (e.g., trigger config values,
    /// environment variable values) for embedded secret references.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let extractor = SecretExtractor::new();
    ///
    /// // Prefix notation
    /// assert_eq!(extractor.extract_from_value("secret:github-token"), vec!["github-token"]);
    ///
    /// // Template syntax
    /// assert_eq!(
    ///     extractor.extract_from_value("Bearer ${secret.api-key}"),
    ///     vec!["api-key"]
    /// );
    /// ```
    pub fn extract_from_value(&self, value: &str) -> Vec<String> {
        let mut secrets = Vec::new();

        // Pattern: secret:name or secret/name
        if let Some(cap) = SECRET_PREFIX_PATTERN.captures(value) {
            if let Some(secret_name) = cap.get(1) {
                secrets.push(secret_name.as_str().to_string());
            }
        }

        // Pattern: ${secret.name} embedded in string
        // Note: Using TEMPLATE_PATTERN here (previously was SECRET_EMBEDDED_PATTERN,
        // which was a duplicate). The patterns are identical.
        for cap in TEMPLATE_PATTERN.captures_iter(value) {
            if let Some(secret_name) = cap.get(1) {
                secrets.push(secret_name.as_str().to_string());
            }
        }

        secrets
    }

    /// Extract secret references from override configurations.
    ///
    /// Scans cluster override env vars and trigger configs for secret references.
    /// This is more aggressive than the main extraction since environment variables
    /// often contain credentials.
    ///
    /// # Heuristics
    ///
    /// For environment variables, triggers extraction if:
    /// - Key contains: `SECRET`, `PASSWORD`, `TOKEN`, `API_KEY`
    /// - Value contains: `secret` or `SECRET`
    ///
    /// Returns a sorted, deduplicated list of secret names.
    pub fn extract_from_overrides(&self, overrides: &[ClusterOverride]) -> Vec<String> {
        let mut secret_refs = HashSet::new();

        for override_config in overrides {
            // Scan environment variables for secret references
            for (key, value) in &override_config.env {
                // Check if the key suggests it's a secret
                if key.to_uppercase().contains("SECRET")
                    || key.to_uppercase().contains("PASSWORD")
                    || key.to_uppercase().contains("TOKEN")
                    || key.to_uppercase().contains("API_KEY")
                {
                    // Value might be "secret:name" or "${secret.name}"
                    secret_refs.extend(self.extract_from_value(value));
                }

                // Also check the value itself for secret references
                if value.contains("secret") || value.contains("SECRET") {
                    secret_refs.extend(self.extract_from_value(value));
                }
            }

            // Scan trigger overrides
            for trigger_override in &override_config.triggers {
                for (key, value) in &trigger_override.config {
                    if key.to_lowercase().contains("secret") {
                        secret_refs.insert(value.clone());
                    }
                    secret_refs.extend(self.extract_from_value(value));
                }
            }
        }

        let mut refs: Vec<String> = secret_refs.into_iter().collect();
        refs.sort();
        refs
    }
}

impl Default for SecretExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_yaml_secret_key_pattern() {
        let extractor = SecretExtractor::new();
        let yaml = r#"
            secretName: github-creds
            credentialsSecret: api-token
            notASecret: some-value
        "#;

        let secrets = extractor.extract_from_yaml(yaml);
        assert!(secrets.contains(&"github-creds".to_string()));
        assert!(secrets.contains(&"api-token".to_string()));
    }

    #[test]
    fn extract_yaml_template_pattern() {
        let extractor = SecretExtractor::new();
        let yaml = "password: ${secret.db-password}\ntoken: ${secrets/api-key}";

        let secrets = extractor.extract_from_yaml(yaml);
        assert!(secrets.contains(&"db-password".to_string()));
        assert!(secrets.contains(&"api-key".to_string()));
    }

    #[test]
    fn extract_yaml_k8s_secret_ref() {
        let extractor = SecretExtractor::new();
        let yaml = r#"
            valueFrom:
              secretKeyRef:
                name: my-k8s-secret
                key: password
        "#;

        let secrets = extractor.extract_from_yaml(yaml);
        assert!(secrets.contains(&"my-k8s-secret".to_string()));
    }

    #[test]
    fn extract_value_prefix_pattern() {
        let extractor = SecretExtractor::new();

        assert_eq!(
            extractor.extract_from_value("secret:github-token"),
            vec!["github-token"]
        );
        assert_eq!(
            extractor.extract_from_value("secrets/api-key"),
            vec!["api-key"]
        );
    }

    #[test]
    fn extract_value_template_pattern() {
        let extractor = SecretExtractor::new();
        let value = "Bearer ${secret.api-token}";

        let secrets = extractor.extract_from_value(value);
        assert_eq!(secrets, vec!["api-token"]);
    }

    #[test]
    fn extract_value_multiple_references() {
        let extractor = SecretExtractor::new();
        let value = "user=${secret.username} pass=${secret.password}";

        let mut secrets = extractor.extract_from_value(value);
        secrets.sort();

        assert_eq!(secrets, vec!["password", "username"]);
    }

    #[test]
    fn extract_filters_boolean_values() {
        let extractor = SecretExtractor::new();
        let yaml = "secretEnabled: true\nsecretName: actual-secret";

        let secrets = extractor.extract_from_yaml(yaml);
        assert!(!secrets.contains(&"true".to_string()));
        assert!(secrets.contains(&"actual-secret".to_string()));
    }

    #[test]
    fn extract_deduplicates_results() {
        let extractor = SecretExtractor::new();
        let yaml = r#"
            secret1: duplicate-secret
            secret2: duplicate-secret
            secret3: ${secret.duplicate-secret}
        "#;

        let secrets = extractor.extract_from_yaml(yaml);
        assert_eq!(
            secrets.iter().filter(|s| *s == "duplicate-secret").count(),
            1
        );
    }
}
