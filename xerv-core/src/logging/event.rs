//! Log event types for trace execution logging.
//!
//! Provides structured log events with correlation IDs (trace_id, node_id)
//! for debugging and observability of flow executions.

use crate::types::{NodeId, TraceId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// Log severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum LogLevel {
    /// Fine-grained debugging information.
    Trace,
    /// Debugging information.
    Debug,
    /// Informational messages.
    #[default]
    Info,
    /// Warning messages.
    Warn,
    /// Error messages.
    Error,
}

impl LogLevel {
    /// Parse a log level from a string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "trace" => Some(Self::Trace),
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" | "warning" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    /// Get the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for LogLevel {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or("invalid log level")
    }
}

/// Category of log event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogCategory {
    /// Trace lifecycle events (start, complete, fail).
    Trace,
    /// Node execution events (start, complete, error).
    Node,
    /// Trigger events (fire, pause, resume).
    Trigger,
    /// Pipeline events (deploy, start, stop).
    Pipeline,
    /// Schema/data validation events.
    Schema,
    /// System/internal events.
    System,
    /// User-defined custom events.
    Custom,
}

impl LogCategory {
    /// Get the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Node => "node",
            Self::Trigger => "trigger",
            Self::Pipeline => "pipeline",
            Self::Schema => "schema",
            Self::System => "system",
            Self::Custom => "custom",
        }
    }
}

impl fmt::Display for LogCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A structured log event with correlation IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    /// Unique event ID.
    pub id: u64,
    /// Timestamp in nanoseconds since UNIX epoch.
    pub timestamp_ns: u64,
    /// Log severity level.
    pub level: LogLevel,
    /// Event category.
    pub category: LogCategory,
    /// Associated trace ID (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<TraceId>,
    /// Associated node ID (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    /// Associated pipeline ID (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline_id: Option<String>,
    /// Human-readable message.
    pub message: String,
    /// Structured fields for additional context.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub fields: HashMap<String, serde_json::Value>,
}

impl LogEvent {
    /// Create a new log event with the current timestamp.
    pub fn new(level: LogLevel, category: LogCategory, message: impl Into<String>) -> Self {
        Self {
            id: 0, // Will be assigned by collector
            timestamp_ns: current_timestamp_ns(),
            level,
            category,
            trace_id: None,
            node_id: None,
            pipeline_id: None,
            message: message.into(),
            fields: HashMap::new(),
        }
    }

    /// Create a trace-level log event.
    pub fn trace(category: LogCategory, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Trace, category, message)
    }

    /// Create a debug-level log event.
    pub fn debug(category: LogCategory, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Debug, category, message)
    }

    /// Create an info-level log event.
    pub fn info(category: LogCategory, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Info, category, message)
    }

    /// Create a warn-level log event.
    pub fn warn(category: LogCategory, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Warn, category, message)
    }

    /// Create an error-level log event.
    pub fn error(category: LogCategory, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Error, category, message)
    }

    /// Set the trace ID.
    pub fn with_trace_id(mut self, trace_id: TraceId) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Set the node ID.
    pub fn with_node_id(mut self, node_id: NodeId) -> Self {
        self.node_id = Some(node_id);
        self
    }

    /// Set the pipeline ID.
    pub fn with_pipeline_id(mut self, pipeline_id: impl Into<String>) -> Self {
        self.pipeline_id = Some(pipeline_id.into());
        self
    }

    /// Add a string field.
    pub fn with_field(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields
            .insert(key.into(), serde_json::Value::String(value.into()));
        self
    }

    /// Add a numeric field.
    pub fn with_field_i64(mut self, key: impl Into<String>, value: i64) -> Self {
        self.fields
            .insert(key.into(), serde_json::Value::Number(value.into()));
        self
    }

    /// Add a boolean field.
    pub fn with_field_bool(mut self, key: impl Into<String>, value: bool) -> Self {
        self.fields
            .insert(key.into(), serde_json::Value::Bool(value));
        self
    }

    /// Add a JSON value field.
    pub fn with_field_json(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.fields.insert(key.into(), value);
        self
    }

    /// Get the timestamp as a DateTime string (ISO 8601).
    pub fn timestamp_iso(&self) -> String {
        let secs = self.timestamp_ns / 1_000_000_000;
        let nanos = (self.timestamp_ns % 1_000_000_000) as u32;

        if let Some(datetime) = chrono::DateTime::from_timestamp(secs as i64, nanos) {
            datetime.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
        } else {
            format!("{}ns", self.timestamp_ns)
        }
    }

    /// Format as a single log line.
    pub fn format_line(&self) -> String {
        let mut parts = vec![
            self.timestamp_iso(),
            format!("[{}]", self.level.as_str().to_uppercase()),
            format!("[{}]", self.category.as_str()),
        ];

        if let Some(ref trace_id) = self.trace_id {
            parts.push(format!("trace={}", trace_id));
        }

        if let Some(node_id) = self.node_id {
            parts.push(format!("node={}", node_id.as_u32()));
        }

        if let Some(ref pipeline_id) = self.pipeline_id {
            parts.push(format!("pipeline={}", pipeline_id));
        }

        parts.push(self.message.clone());

        if !self.fields.is_empty() {
            let fields_str: Vec<String> = self
                .fields
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            parts.push(format!("{{{}}}", fields_str.join(", ")));
        }

        parts.join(" ")
    }
}

/// Get current timestamp in nanoseconds since UNIX epoch.
fn current_timestamp_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Builder for creating log events with common context.
#[derive(Debug, Clone)]
pub struct LogEventBuilder {
    trace_id: Option<TraceId>,
    node_id: Option<NodeId>,
    pipeline_id: Option<String>,
}

impl LogEventBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            trace_id: None,
            node_id: None,
            pipeline_id: None,
        }
    }

    /// Set the trace ID for all events.
    pub fn with_trace_id(mut self, trace_id: TraceId) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Set the node ID for all events.
    pub fn with_node_id(mut self, node_id: NodeId) -> Self {
        self.node_id = Some(node_id);
        self
    }

    /// Set the pipeline ID for all events.
    pub fn with_pipeline_id(mut self, pipeline_id: impl Into<String>) -> Self {
        self.pipeline_id = Some(pipeline_id.into());
        self
    }

    /// Create a log event with the builder's context.
    pub fn event(
        &self,
        level: LogLevel,
        category: LogCategory,
        message: impl Into<String>,
    ) -> LogEvent {
        let mut event = LogEvent::new(level, category, message);
        if let Some(trace_id) = self.trace_id {
            event.trace_id = Some(trace_id);
        }
        if let Some(node_id) = self.node_id {
            event.node_id = Some(node_id);
        }
        if let Some(ref pipeline_id) = self.pipeline_id {
            event.pipeline_id = Some(pipeline_id.clone());
        }
        event
    }

    /// Create a trace-level event.
    pub fn trace(&self, category: LogCategory, message: impl Into<String>) -> LogEvent {
        self.event(LogLevel::Trace, category, message)
    }

    /// Create a debug-level event.
    pub fn debug(&self, category: LogCategory, message: impl Into<String>) -> LogEvent {
        self.event(LogLevel::Debug, category, message)
    }

    /// Create an info-level event.
    pub fn info(&self, category: LogCategory, message: impl Into<String>) -> LogEvent {
        self.event(LogLevel::Info, category, message)
    }

    /// Create a warn-level event.
    pub fn warn(&self, category: LogCategory, message: impl Into<String>) -> LogEvent {
        self.event(LogLevel::Warn, category, message)
    }

    /// Create an error-level event.
    pub fn error(&self, category: LogCategory, message: impl Into<String>) -> LogEvent {
        self.event(LogLevel::Error, category, message)
    }
}

impl Default for LogEventBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_parsing() {
        assert_eq!(LogLevel::parse("trace"), Some(LogLevel::Trace));
        assert_eq!(LogLevel::parse("DEBUG"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::parse("Info"), Some(LogLevel::Info));
        assert_eq!(LogLevel::parse("WARN"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::parse("warning"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::parse("error"), Some(LogLevel::Error));
        assert_eq!(LogLevel::parse("invalid"), None);
    }

    #[test]
    fn log_level_ordering() {
        assert!(LogLevel::Trace < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn log_event_creation() {
        let event = LogEvent::info(LogCategory::Node, "Node started")
            .with_trace_id(TraceId::new())
            .with_node_id(NodeId::new(42))
            .with_field("duration_ms", "123");

        assert_eq!(event.level, LogLevel::Info);
        assert_eq!(event.category, LogCategory::Node);
        assert_eq!(event.message, "Node started");
        assert!(event.trace_id.is_some());
        assert_eq!(event.node_id, Some(NodeId::new(42)));
        assert!(event.fields.contains_key("duration_ms"));
    }

    #[test]
    fn log_event_builder() {
        let trace_id = TraceId::new();
        let builder = LogEventBuilder::new()
            .with_trace_id(trace_id)
            .with_pipeline_id("test_pipeline");

        let event = builder.info(LogCategory::Trace, "Trace started");

        assert_eq!(event.trace_id, Some(trace_id));
        assert_eq!(event.pipeline_id, Some("test_pipeline".to_string()));
        assert_eq!(event.level, LogLevel::Info);
    }

    #[test]
    fn log_event_format_line() {
        let event = LogEvent::info(LogCategory::Node, "Processing order")
            .with_pipeline_id("order_pipeline")
            .with_field("order_id", "ORD-123");

        let line = event.format_line();
        assert!(line.contains("[INFO]"));
        assert!(line.contains("[node]"));
        assert!(line.contains("pipeline=order_pipeline"));
        assert!(line.contains("Processing order"));
        assert!(line.contains("order_id"));
    }

    #[test]
    fn log_event_serialization() {
        let event = LogEvent::error(LogCategory::System, "Connection failed")
            .with_field("host", "localhost")
            .with_field_i64("port", 8080);

        let json = serde_json::to_string(&event).unwrap();
        let parsed: LogEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.level, LogLevel::Error);
        assert_eq!(parsed.category, LogCategory::System);
        assert_eq!(parsed.message, "Connection failed");
        assert_eq!(parsed.fields.len(), 2);
    }
}
