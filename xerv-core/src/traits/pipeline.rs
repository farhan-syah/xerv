//! Pipeline configuration and lifecycle hooks.

use super::context::PipelineCtx;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// Pipeline state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineState {
    /// Pipeline is initializing.
    Initializing,
    /// Pipeline is running and accepting new traces.
    Running,
    /// Pipeline is paused (not accepting new traces, but processing existing ones).
    Paused,
    /// Pipeline is draining (not accepting new traces, waiting for existing to complete).
    Draining,
    /// Pipeline has stopped.
    Stopped,
    /// Pipeline has encountered an error.
    Error,
}

/// Pipeline runtime settings.
#[derive(Debug, Clone)]
pub struct PipelineSettings {
    /// Maximum concurrent trace executions.
    pub max_concurrent_executions: u32,
    /// Execution timeout.
    pub execution_timeout: Duration,
    /// Error rate threshold for circuit breaker (0.0 to 1.0).
    pub circuit_breaker_threshold: f64,
    /// Window for measuring error rate.
    pub circuit_breaker_window: Duration,
    /// Maximum concurrent versions during deployment.
    pub max_concurrent_versions: u32,
    /// Drain timeout for deployments.
    pub drain_timeout: Duration,
    /// Grace period before hard drain.
    pub drain_grace_period: Duration,
}

impl Default for PipelineSettings {
    fn default() -> Self {
        Self {
            max_concurrent_executions: 100,
            execution_timeout: Duration::from_secs(60),
            circuit_breaker_threshold: 0.05, // 5%
            circuit_breaker_window: Duration::from_secs(60),
            max_concurrent_versions: 5,
            drain_timeout: Duration::from_secs(30 * 60), // 30 minutes
            drain_grace_period: Duration::from_secs(5 * 60), // 5 minutes
        }
    }
}

impl PipelineSettings {
    /// Set max concurrent executions.
    pub fn with_concurrency(mut self, max: u32) -> Self {
        self.max_concurrent_executions = max;
        self
    }

    /// Set execution timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.execution_timeout = timeout;
        self
    }

    /// Set circuit breaker threshold.
    pub fn with_circuit_breaker(mut self, threshold: f64, window: Duration) -> Self {
        self.circuit_breaker_threshold = threshold;
        self.circuit_breaker_window = window;
        self
    }
}

/// Configuration for a pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Pipeline ID.
    pub id: String,
    /// Pipeline driver (Rust struct name).
    pub driver: String,
    /// Runtime settings.
    pub settings: PipelineSettings,
    /// Global configuration values (from YAML).
    pub config: serde_yaml::Value,
}

impl PipelineConfig {
    /// Create a new pipeline config.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            driver: String::new(),
            settings: PipelineSettings::default(),
            config: serde_yaml::Value::Null,
        }
    }

    /// Set the driver.
    pub fn with_driver(mut self, driver: impl Into<String>) -> Self {
        self.driver = driver.into();
        self
    }

    /// Set the settings.
    pub fn with_settings(mut self, settings: PipelineSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Set the config.
    pub fn with_config(mut self, config: serde_yaml::Value) -> Self {
        self.config = config;
        self
    }

    /// Get a config value as string.
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(|v| v.as_str())
    }

    /// Get a config value as i64.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.config.get(key).and_then(|v| v.as_i64())
    }

    /// Get a config value as f64.
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.config.get(key).and_then(|v| v.as_f64())
    }

    /// Get a config value as bool.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.config.get(key).and_then(|v| v.as_bool())
    }
}

/// Boxed async result for hooks.
pub type HookFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Lifecycle hooks for a pipeline.
///
/// These hooks allow pipelines to execute code at key lifecycle events.
pub trait PipelineHook: Send + Sync {
    /// Called when the pipeline starts.
    fn on_start<'a>(&'a self, ctx: PipelineCtx) -> HookFuture<'a> {
        let _ = ctx;
        Box::pin(async {})
    }

    /// Called when the pipeline begins draining (preparing for shutdown/upgrade).
    fn on_drain<'a>(&'a self, ctx: PipelineCtx) -> HookFuture<'a> {
        let _ = ctx;
        Box::pin(async {})
    }

    /// Called when the pipeline stops.
    fn on_stop<'a>(&'a self, ctx: PipelineCtx) -> HookFuture<'a> {
        let _ = ctx;
        Box::pin(async {})
    }

    /// Called when the pipeline encounters an error.
    fn on_error<'a>(&'a self, ctx: PipelineCtx, error: &'a str) -> HookFuture<'a> {
        let _ = (ctx, error);
        Box::pin(async {})
    }

    /// Called when a trace completes successfully.
    fn on_trace_complete<'a>(
        &'a self,
        ctx: PipelineCtx,
        trace_id: crate::types::TraceId,
    ) -> HookFuture<'a> {
        let _ = (ctx, trace_id);
        Box::pin(async {})
    }

    /// Called when a trace fails.
    fn on_trace_failed<'a>(
        &'a self,
        ctx: PipelineCtx,
        trace_id: crate::types::TraceId,
        error: &'a str,
    ) -> HookFuture<'a> {
        let _ = (ctx, trace_id, error);
        Box::pin(async {})
    }
}

/// A default pipeline hook implementation that does nothing.
///
/// This is provided as a convenience for users who need a no-op hook
/// implementation. It's useful when you need to satisfy a `PipelineHook`
/// bound but don't need any custom hook behavior.
///
/// # Example
///
/// ```ignore
/// use xerv_core::traits::pipeline::{DefaultPipelineHook, PipelineHook};
///
/// fn setup_pipeline(hook: Option<Box<dyn PipelineHook>>) {
///     let hook = hook.unwrap_or_else(|| Box::new(DefaultPipelineHook));
///     // ...
/// }
/// ```
#[allow(dead_code)]
pub struct DefaultPipelineHook;

impl PipelineHook for DefaultPipelineHook {}

#[cfg(test)]
mod pipeline_hook_tests {
    use super::*;

    #[tokio::test]
    async fn default_hook_does_nothing() {
        let hook = DefaultPipelineHook;
        let ctx = PipelineCtx::new("test_pipeline", 1);

        // All hooks should complete without error
        hook.on_start(ctx.clone()).await;
        hook.on_drain(ctx.clone()).await;
        hook.on_stop(ctx.clone()).await;
        hook.on_error(ctx.clone(), "test error").await;
        hook.on_trace_complete(ctx.clone(), crate::types::TraceId::new())
            .await;
        hook.on_trace_failed(ctx, crate::types::TraceId::new(), "test failure")
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_settings_default() {
        let settings = PipelineSettings::default();
        assert_eq!(settings.max_concurrent_executions, 100);
        assert_eq!(settings.execution_timeout, Duration::from_secs(60));
    }

    #[test]
    fn pipeline_config_creation() {
        let mut config_map = serde_yaml::Mapping::new();
        config_map.insert(
            serde_yaml::Value::String("api_key".to_string()),
            serde_yaml::Value::String("sk_test_123".to_string()),
        );
        config_map.insert(
            serde_yaml::Value::String("threshold".to_string()),
            serde_yaml::Value::Number(0.8.into()),
        );

        let config = PipelineConfig::new("order_processing")
            .with_driver("OrderPipeline")
            .with_config(serde_yaml::Value::Mapping(config_map));

        assert_eq!(config.id, "order_processing");
        assert_eq!(config.driver, "OrderPipeline");
        assert_eq!(config.get_string("api_key"), Some("sk_test_123"));
    }
}
