//! In-memory dispatch configuration.

use serde::{Deserialize, Serialize};

/// Configuration for in-memory dispatch.
///
/// This is the simplest dispatch backend, storing traces in memory.
/// Best for development, testing, and single-node deployments.
///
/// # Example
///
/// ```
/// use xerv_core::dispatch::MemoryConfig;
///
/// let config = MemoryConfig::default();
/// assert_eq!(config.max_queue_size, 10_000);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Maximum queue size before backpressure is applied.
    #[serde(default = "default_max_queue_size")]
    pub max_queue_size: usize,

    /// Enable priority queue (higher priority traces processed first).
    #[serde(default)]
    pub priority_queue: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            max_queue_size: default_max_queue_size(),
            priority_queue: false,
        }
    }
}

impl MemoryConfig {
    /// Create a new memory config with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum queue size.
    pub fn max_queue_size(mut self, size: usize) -> Self {
        self.max_queue_size = size;
        self
    }

    /// Enable priority queue.
    pub fn with_priority_queue(mut self) -> Self {
        self.priority_queue = true;
        self
    }
}

fn default_max_queue_size() -> usize {
    10_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let config = MemoryConfig::default();
        assert_eq!(config.max_queue_size, 10_000);
        assert!(!config.priority_queue);
    }

    #[test]
    fn builder_pattern() {
        let config = MemoryConfig::new()
            .max_queue_size(5_000)
            .with_priority_queue();

        assert_eq!(config.max_queue_size, 5_000);
        assert!(config.priority_queue);
    }
}
