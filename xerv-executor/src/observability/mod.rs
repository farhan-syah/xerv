//! Observability infrastructure for XERV.
//!
//! This module provides comprehensive observability features:
//! - Structured logging with JSON format support
//! - OpenTelemetry distributed tracing (optional, requires `otel` feature)
//! - Trace context propagation
//!
//! # Configuration
//!
//! Logging format is controlled via `XERV_LOG_FORMAT` env var:
//! - `json` - Structured JSON output (for ELK/Loki)
//! - `pretty` - Human-readable colored output (default for TTY)
//! - `compact` - Compact single-line format (default for non-TTY)
//!
//! OpenTelemetry (requires `otel` feature) is controlled via:
//! - `OTEL_EXPORTER_OTLP_ENDPOINT` - OTLP endpoint (e.g., http://localhost:4317)
//! - `OTEL_SERVICE_NAME` - Service name (defaults to "xerv")
//! - `OTEL_ENABLED` - Set to "true" to enable (disabled by default)
//!
//! # Example
//!
//! ```ignore
//! use xerv_executor::observability::{TracingConfig, init_tracing};
//!
//! // Initialize with default config (no OpenTelemetry)
//! let _guard = init_tracing(TracingConfig::default())?;
//!
//! // Or with explicit settings (otel feature required for tracing export)
//! let config = TracingConfig::builder()
//!     .json_format(true)
//!     .service_name("my-xerv-instance")
//!     .build();
//! let _guard = init_tracing(config)?;
//! ```
//!
//! # Feature Flags
//!
//! - `otel` - Enables OpenTelemetry distributed tracing with OTLP export.
//!   Without this feature, `otel_enabled` config is ignored.

mod config;
#[cfg(feature = "otel")]
mod otel;
mod tracing_setup;

pub use config::{LogFormat, TracingConfig, TracingConfigBuilder};
pub use tracing_setup::{TracingGuard, init_tracing};

#[cfg(feature = "otel")]
pub use otel::{init_otel_tracer, shutdown_otel};

/// Macro for instrumenting node execution with proper span attributes.
///
/// This creates a span with all relevant context for tracing node execution.
#[macro_export]
macro_rules! instrument_node {
    ($trace_id:expr, $node_id:expr, $node_type:expr) => {
        tracing::info_span!(
            "node_execution",
            trace_id = %$trace_id,
            node_id = $node_id,
            node_type = $node_type,
            otel.kind = "internal"
        )
    };
}

/// Macro for instrumenting pipeline operations.
#[macro_export]
macro_rules! instrument_pipeline {
    ($pipeline_id:expr, $operation:expr) => {
        tracing::info_span!(
            "pipeline_operation",
            pipeline_id = %$pipeline_id,
            operation = $operation,
            otel.kind = "internal"
        )
    };
}

/// Macro for instrumenting trace execution.
#[macro_export]
macro_rules! instrument_trace {
    ($trace_id:expr, $pipeline_id:expr) => {
        tracing::info_span!(
            "trace_execution",
            trace_id = %$trace_id,
            pipeline_id = %$pipeline_id,
            otel.kind = "server"
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TracingConfig::default();
        assert_eq!(config.service_name(), "xerv");
        assert!(!config.otel_enabled());
    }

    #[test]
    fn test_config_builder() {
        let config = TracingConfig::builder()
            .service_name("test-service")
            .log_format(LogFormat::Json)
            .otel_enabled(true)
            .otel_endpoint("http://localhost:4317")
            .build();

        assert_eq!(config.service_name(), "test-service");
        assert_eq!(config.log_format(), LogFormat::Json);
        assert!(config.otel_enabled());
        assert_eq!(
            config.otel_endpoint(),
            Some("http://localhost:4317".to_string())
        );
    }

    #[test]
    fn test_config_from_env() {
        // Test that from_env doesn't panic with missing env vars
        let config = TracingConfig::from_env();
        // Default values should be used
        assert_eq!(config.service_name(), "xerv");
    }
}
