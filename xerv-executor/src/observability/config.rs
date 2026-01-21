//! Configuration types for observability.

use std::env;
use std::str::FromStr;

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// JSON format for structured logging (ELK, Loki).
    Json,
    /// Human-readable pretty format with colors.
    Pretty,
    /// Compact single-line format.
    #[default]
    Compact,
}

impl FromStr for LogFormat {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "json" => Self::Json,
            "pretty" => Self::Pretty,
            "compact" => Self::Compact,
            _ => Self::default(),
        })
    }
}

/// Configuration for tracing and observability.
#[derive(Debug, Clone)]
pub struct TracingConfig {
    /// Service name for traces and logs.
    service_name: String,
    /// Log output format.
    log_format: LogFormat,
    /// Log level filter (e.g., "info", "debug,xerv=trace").
    log_filter: String,
    /// Whether OpenTelemetry is enabled.
    otel_enabled: bool,
    /// OTLP endpoint for traces.
    otel_endpoint: Option<String>,
    /// Whether to include source location in logs.
    include_location: bool,
    /// Whether to include target in logs.
    include_target: bool,
    /// Whether to include thread names in logs.
    include_thread_names: bool,
    /// Whether to include thread IDs in logs.
    include_thread_ids: bool,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            service_name: "xerv".to_string(),
            log_format: LogFormat::default(),
            log_filter: "info".to_string(),
            otel_enabled: false,
            otel_endpoint: None,
            include_location: false,
            include_target: true,
            include_thread_names: false,
            include_thread_ids: false,
        }
    }
}

impl TracingConfig {
    /// Create a new builder.
    pub fn builder() -> TracingConfigBuilder {
        TracingConfigBuilder::default()
    }

    /// Create configuration from environment variables.
    ///
    /// Environment variables:
    /// - `XERV_LOG_FORMAT`: "json", "pretty", or "compact"
    /// - `XERV_LOG_LEVEL` or `RUST_LOG`: Log filter string
    /// - `OTEL_ENABLED`: "true" to enable OpenTelemetry
    /// - `OTEL_EXPORTER_OTLP_ENDPOINT`: OTLP endpoint URL
    /// - `OTEL_SERVICE_NAME`: Service name (defaults to "xerv")
    pub fn from_env() -> Self {
        let log_format = env::var("XERV_LOG_FORMAT")
            .ok()
            .and_then(|s| s.parse::<LogFormat>().ok())
            .unwrap_or_else(|| {
                // Auto-detect: JSON for non-TTY, pretty for TTY
                if atty_check() {
                    LogFormat::Pretty
                } else {
                    LogFormat::Json
                }
            });

        let log_filter = env::var("XERV_LOG_LEVEL")
            .or_else(|_| env::var("RUST_LOG"))
            .unwrap_or_else(|_| "info".to_string());

        let otel_enabled = env::var("OTEL_ENABLED")
            .map(|s| s.to_lowercase() == "true" || s == "1")
            .unwrap_or(false);

        let otel_endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();

        let service_name = env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "xerv".to_string());

        Self {
            service_name,
            log_format,
            log_filter,
            otel_enabled,
            otel_endpoint,
            include_location: env::var("XERV_LOG_LOCATION")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false),
            include_target: true,
            include_thread_names: env::var("XERV_LOG_THREAD_NAMES")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false),
            include_thread_ids: env::var("XERV_LOG_THREAD_IDS")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false),
        }
    }

    /// Get the service name.
    pub fn service_name(&self) -> &str {
        &self.service_name
    }

    /// Get the log format.
    pub fn log_format(&self) -> LogFormat {
        self.log_format
    }

    /// Get the log filter.
    pub fn log_filter(&self) -> &str {
        &self.log_filter
    }

    /// Check if OpenTelemetry is enabled.
    pub fn otel_enabled(&self) -> bool {
        self.otel_enabled
    }

    /// Get the OTLP endpoint.
    pub fn otel_endpoint(&self) -> Option<String> {
        self.otel_endpoint.clone()
    }

    /// Check if source location should be included.
    pub fn include_location(&self) -> bool {
        self.include_location
    }

    /// Check if target should be included.
    pub fn include_target(&self) -> bool {
        self.include_target
    }

    /// Check if thread names should be included.
    pub fn include_thread_names(&self) -> bool {
        self.include_thread_names
    }

    /// Check if thread IDs should be included.
    pub fn include_thread_ids(&self) -> bool {
        self.include_thread_ids
    }
}

/// Builder for TracingConfig.
#[derive(Debug, Clone, Default)]
pub struct TracingConfigBuilder {
    service_name: Option<String>,
    log_format: Option<LogFormat>,
    log_filter: Option<String>,
    otel_enabled: Option<bool>,
    otel_endpoint: Option<String>,
    include_location: Option<bool>,
    include_target: Option<bool>,
    include_thread_names: Option<bool>,
    include_thread_ids: Option<bool>,
}

impl TracingConfigBuilder {
    /// Set the service name.
    pub fn service_name(mut self, name: impl Into<String>) -> Self {
        self.service_name = Some(name.into());
        self
    }

    /// Set the log format.
    pub fn log_format(mut self, format: LogFormat) -> Self {
        self.log_format = Some(format);
        self
    }

    /// Set JSON format (shorthand for `log_format(LogFormat::Json)`).
    pub fn json_format(self, enable: bool) -> Self {
        if enable {
            self.log_format(LogFormat::Json)
        } else {
            self
        }
    }

    /// Set the log filter.
    pub fn log_filter(mut self, filter: impl Into<String>) -> Self {
        self.log_filter = Some(filter.into());
        self
    }

    /// Enable or disable OpenTelemetry.
    pub fn otel_enabled(mut self, enabled: bool) -> Self {
        self.otel_enabled = Some(enabled);
        self
    }

    /// Set the OTLP endpoint.
    pub fn otel_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        let endpoint = endpoint.into();
        self.otel_endpoint = Some(endpoint);
        self.otel_enabled = Some(true);
        self
    }

    /// Include source location in logs.
    pub fn include_location(mut self, include: bool) -> Self {
        self.include_location = Some(include);
        self
    }

    /// Include target in logs.
    pub fn include_target(mut self, include: bool) -> Self {
        self.include_target = Some(include);
        self
    }

    /// Include thread names in logs.
    pub fn include_thread_names(mut self, include: bool) -> Self {
        self.include_thread_names = Some(include);
        self
    }

    /// Include thread IDs in logs.
    pub fn include_thread_ids(mut self, include: bool) -> Self {
        self.include_thread_ids = Some(include);
        self
    }

    /// Build the configuration.
    pub fn build(self) -> TracingConfig {
        let defaults = TracingConfig::default();
        TracingConfig {
            service_name: self.service_name.unwrap_or(defaults.service_name),
            log_format: self.log_format.unwrap_or(defaults.log_format),
            log_filter: self.log_filter.unwrap_or(defaults.log_filter),
            otel_enabled: self.otel_enabled.unwrap_or(defaults.otel_enabled),
            otel_endpoint: self.otel_endpoint.or(defaults.otel_endpoint),
            include_location: self.include_location.unwrap_or(defaults.include_location),
            include_target: self.include_target.unwrap_or(defaults.include_target),
            include_thread_names: self
                .include_thread_names
                .unwrap_or(defaults.include_thread_names),
            include_thread_ids: self
                .include_thread_ids
                .unwrap_or(defaults.include_thread_ids),
        }
    }
}

/// Check if stdout is a TTY.
fn atty_check() -> bool {
    // Simple heuristic: check if we're in a terminal
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}
