//! Flow runtime settings from YAML.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use ts_rs::TS;

/// Runtime settings for a flow.
///
/// These settings control execution behavior like concurrency limits,
/// timeouts, and circuit breaker configuration.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export, export_to = "../web/src/bindings/")]
pub struct FlowSettings {
    /// Maximum concurrent trace executions.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_executions: u32,

    /// Execution timeout in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub execution_timeout_ms: u64,

    /// Error rate threshold for circuit breaker (0.0 to 1.0).
    #[serde(default = "default_circuit_breaker_threshold")]
    pub circuit_breaker_threshold: f64,

    /// Window for measuring error rate in milliseconds.
    #[serde(default = "default_circuit_breaker_window_ms")]
    pub circuit_breaker_window_ms: u64,

    /// Maximum concurrent versions during deployment.
    #[serde(default = "default_max_concurrent_versions")]
    pub max_concurrent_versions: u32,

    /// Drain timeout in milliseconds.
    #[serde(default = "default_drain_timeout_ms")]
    pub drain_timeout_ms: u64,

    /// Grace period before hard drain in milliseconds.
    #[serde(default = "default_drain_grace_period_ms")]
    pub drain_grace_period_ms: u64,

    /// Enable debug tracing for this flow.
    #[serde(default)]
    pub debug: bool,

    /// Custom environment variables for this flow.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

fn default_max_concurrent() -> u32 {
    100
}
fn default_timeout_ms() -> u64 {
    60_000
}
fn default_circuit_breaker_threshold() -> f64 {
    0.05
}
fn default_circuit_breaker_window_ms() -> u64 {
    60_000
}
fn default_max_concurrent_versions() -> u32 {
    5
}
fn default_drain_timeout_ms() -> u64 {
    30 * 60 * 1000
}
fn default_drain_grace_period_ms() -> u64 {
    5 * 60 * 1000
}

impl Default for FlowSettings {
    fn default() -> Self {
        Self {
            max_concurrent_executions: default_max_concurrent(),
            execution_timeout_ms: default_timeout_ms(),
            circuit_breaker_threshold: default_circuit_breaker_threshold(),
            circuit_breaker_window_ms: default_circuit_breaker_window_ms(),
            max_concurrent_versions: default_max_concurrent_versions(),
            drain_timeout_ms: default_drain_timeout_ms(),
            drain_grace_period_ms: default_drain_grace_period_ms(),
            debug: false,
            env: std::collections::HashMap::new(),
        }
    }
}

impl FlowSettings {
    /// Get execution timeout as Duration.
    pub fn execution_timeout(&self) -> Duration {
        Duration::from_millis(self.execution_timeout_ms)
    }

    /// Get circuit breaker window as Duration.
    pub fn circuit_breaker_window(&self) -> Duration {
        Duration::from_millis(self.circuit_breaker_window_ms)
    }

    /// Get drain timeout as Duration.
    pub fn drain_timeout(&self) -> Duration {
        Duration::from_millis(self.drain_timeout_ms)
    }

    /// Get drain grace period as Duration.
    pub fn drain_grace_period(&self) -> Duration {
        Duration::from_millis(self.drain_grace_period_ms)
    }

    /// Convert to PipelineSettings.
    pub fn to_pipeline_settings(&self) -> crate::traits::PipelineSettings {
        crate::traits::PipelineSettings {
            max_concurrent_executions: self.max_concurrent_executions,
            execution_timeout: self.execution_timeout(),
            circuit_breaker_threshold: self.circuit_breaker_threshold,
            circuit_breaker_window: self.circuit_breaker_window(),
            max_concurrent_versions: self.max_concurrent_versions,
            drain_timeout: self.drain_timeout(),
            drain_grace_period: self.drain_grace_period(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings() {
        let settings = FlowSettings::default();
        assert_eq!(settings.max_concurrent_executions, 100);
        assert_eq!(settings.execution_timeout_ms, 60_000);
        assert_eq!(settings.circuit_breaker_threshold, 0.05);
    }

    #[test]
    fn deserialize_settings() {
        let yaml = r#"
max_concurrent_executions: 50
execution_timeout_ms: 30000
debug: true
env:
  API_KEY: test123
"#;
        let settings: FlowSettings = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(settings.max_concurrent_executions, 50);
        assert_eq!(settings.execution_timeout_ms, 30_000);
        assert!(settings.debug);
        assert_eq!(settings.env.get("API_KEY"), Some(&"test123".to_string()));
    }

    #[test]
    fn to_duration() {
        let settings = FlowSettings {
            execution_timeout_ms: 5000,
            ..Default::default()
        };
        assert_eq!(settings.execution_timeout(), Duration::from_millis(5000));
    }
}
