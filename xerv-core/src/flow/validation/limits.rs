//! Validation limits for DoS protection.

use super::error::{ValidationError, ValidationErrorKind};

/// Limits for flow validation to prevent DoS attacks.
#[derive(Debug, Clone)]
pub struct ValidationLimits {
    /// Maximum YAML file size in bytes (default: 10MB).
    pub max_file_size: usize,
    /// Maximum nesting depth in YAML structure (default: 100).
    pub max_nesting_depth: usize,
    /// Maximum number of nodes in a flow (default: 1000).
    pub max_node_count: usize,
    /// Maximum number of edges in a flow (default: 5000).
    pub max_edge_count: usize,
    /// Maximum number of triggers in a flow (default: 100).
    pub max_trigger_count: usize,
}

impl Default for ValidationLimits {
    fn default() -> Self {
        Self {
            max_file_size: 10 * 1024 * 1024, // 10MB
            max_nesting_depth: 100,
            max_node_count: 1000,
            max_edge_count: 5000,
            max_trigger_count: 100,
        }
    }
}

impl ValidationLimits {
    /// Create new validation limits with custom values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum file size.
    pub fn with_max_file_size(mut self, size: usize) -> Self {
        self.max_file_size = size;
        self
    }

    /// Set maximum nesting depth.
    pub fn with_max_nesting_depth(mut self, depth: usize) -> Self {
        self.max_nesting_depth = depth;
        self
    }

    /// Set maximum node count.
    pub fn with_max_node_count(mut self, count: usize) -> Self {
        self.max_node_count = count;
        self
    }

    /// Validate raw YAML content size before parsing.
    pub fn validate_content_size(&self, content: &str) -> Result<(), ValidationError> {
        if content.len() > self.max_file_size {
            return Err(ValidationError::new(
                ValidationErrorKind::LimitExceeded,
                "flow",
                format!(
                    "YAML content size ({} bytes) exceeds maximum allowed ({} bytes)",
                    content.len(),
                    self.max_file_size
                ),
            ));
        }
        Ok(())
    }

    /// Validate YAML nesting depth using serde_yaml::Value.
    ///
    /// This uses a defensive recursion limit to prevent stack overflow
    /// on maliciously crafted deeply nested YAML.
    pub fn validate_nesting_depth(&self, value: &serde_yaml::Value) -> Result<(), ValidationError> {
        // Use a hard recursion limit slightly above max_nesting_depth
        // to catch malicious payloads before they cause stack overflow
        let hard_limit = self.max_nesting_depth.saturating_add(10);

        match Self::measure_depth_limited(value, hard_limit) {
            Ok(depth) if depth > self.max_nesting_depth => Err(ValidationError::new(
                ValidationErrorKind::LimitExceeded,
                "flow",
                format!(
                    "YAML nesting depth ({}) exceeds maximum allowed ({})",
                    depth, self.max_nesting_depth
                ),
            )),
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Measure the maximum nesting depth with a hard recursion limit.
    ///
    /// Returns `Err` if the recursion limit is exceeded (potential attack).
    /// Returns `Ok(depth)` with the actual depth otherwise.
    fn measure_depth_limited(
        value: &serde_yaml::Value,
        remaining: usize,
    ) -> Result<usize, ValidationError> {
        if remaining == 0 {
            return Err(ValidationError::new(
                ValidationErrorKind::LimitExceeded,
                "flow",
                "YAML recursion depth exceeded hard limit (possible attack)",
            ));
        }

        match value {
            serde_yaml::Value::Mapping(map) => {
                let mut max_child = 0;
                for child in map.values() {
                    let child_depth = Self::measure_depth_limited(child, remaining - 1)?;
                    max_child = max_child.max(child_depth);
                }
                Ok(1 + max_child)
            }
            serde_yaml::Value::Sequence(seq) => {
                let mut max_child = 0;
                for child in seq {
                    let child_depth = Self::measure_depth_limited(child, remaining - 1)?;
                    max_child = max_child.max(child_depth);
                }
                Ok(1 + max_child)
            }
            _ => Ok(1),
        }
    }

    /// Measure the maximum nesting depth of a YAML value.
    ///
    /// This is the simple version without recursion limit, used only in tests.
    #[cfg(test)]
    pub(crate) fn measure_depth(value: &serde_yaml::Value) -> usize {
        match value {
            serde_yaml::Value::Mapping(map) => {
                1 + map.values().map(Self::measure_depth).max().unwrap_or(0)
            }
            serde_yaml::Value::Sequence(seq) => {
                1 + seq.iter().map(Self::measure_depth).max().unwrap_or(0)
            }
            _ => 1,
        }
    }
}
