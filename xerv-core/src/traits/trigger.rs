//! Trigger trait and related types.

use crate::error::Result;
use crate::types::{RelPtr, TraceId};
use std::future::Future;
use std::pin::Pin;

/// Type of trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriggerType {
    /// HTTP webhook trigger.
    Webhook,
    /// Kafka message trigger.
    Kafka,
    /// Cron schedule trigger.
    Cron,
    /// Message queue trigger.
    Queue,
    /// Filesystem event trigger.
    Filesystem,
    /// Manual trigger (for testing).
    Manual,
    /// Memory trigger (for benchmarking).
    Memory,
}

impl TriggerType {
    /// Get the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Webhook => "trigger::webhook",
            Self::Kafka => "trigger::kafka",
            Self::Cron => "trigger::cron",
            Self::Queue => "trigger::queue",
            Self::Filesystem => "trigger::filesystem",
            Self::Manual => "trigger::manual",
            Self::Memory => "trigger::memory",
        }
    }

    /// Parse from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "trigger::webhook" | "webhook" => Some(Self::Webhook),
            "trigger::kafka" | "kafka" => Some(Self::Kafka),
            "trigger::cron" | "cron" => Some(Self::Cron),
            "trigger::queue" | "queue" => Some(Self::Queue),
            "trigger::filesystem" | "filesystem" => Some(Self::Filesystem),
            "trigger::manual" | "manual" => Some(Self::Manual),
            "trigger::memory" | "memory" => Some(Self::Memory),
            _ => None,
        }
    }
}

/// Configuration for a trigger.
#[derive(Debug, Clone)]
pub struct TriggerConfig {
    /// Unique ID for this trigger instance.
    pub id: String,
    /// Type of trigger.
    pub trigger_type: TriggerType,
    /// Type-specific parameters (from YAML).
    pub params: serde_yaml::Value,
}

impl TriggerConfig {
    /// Create a new trigger config.
    pub fn new(id: impl Into<String>, trigger_type: TriggerType) -> Self {
        Self {
            id: id.into(),
            trigger_type,
            params: serde_yaml::Value::Null,
        }
    }

    /// Set parameters.
    pub fn with_params(mut self, params: serde_yaml::Value) -> Self {
        self.params = params;
        self
    }

    /// Get a string parameter.
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(|v| v.as_str())
    }

    /// Get an integer parameter.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.params.get(key).and_then(|v| v.as_i64())
    }

    /// Get a boolean parameter.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.params.get(key).and_then(|v| v.as_bool())
    }
}

/// An event from a trigger.
#[derive(Debug, Clone)]
pub struct TriggerEvent {
    /// The trigger ID that generated this event.
    pub trigger_id: String,
    /// Trace ID assigned to this event.
    pub trace_id: TraceId,
    /// Pointer to the event data in the arena.
    pub data: RelPtr<()>,
    /// Schema hash of the event data.
    pub schema_hash: u64,
    /// Timestamp when the event was received.
    pub timestamp_ns: u64,
    /// Optional metadata.
    pub metadata: Option<String>,
}

impl TriggerEvent {
    /// Create a new trigger event.
    pub fn new(trigger_id: impl Into<String>, data: RelPtr<()>) -> Self {
        Self {
            trigger_id: trigger_id.into(),
            trace_id: TraceId::new(),
            data,
            schema_hash: 0,
            timestamp_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
            metadata: None,
        }
    }

    /// Set the schema hash.
    pub fn with_schema_hash(mut self, hash: u64) -> Self {
        self.schema_hash = hash;
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: impl Into<String>) -> Self {
        self.metadata = Some(metadata.into());
        self
    }
}

/// A boxed future for async trigger operations.
pub type TriggerFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The trait for XERV triggers.
///
/// Triggers are entry points that inject events into a flow.
/// Each trigger type has its own configuration and event format.
pub trait Trigger: Send + Sync {
    /// Get the trigger type.
    fn trigger_type(&self) -> TriggerType;

    /// Get the trigger ID.
    fn id(&self) -> &str;

    /// Start the trigger.
    ///
    /// The trigger should begin listening for events and call the
    /// provided callback when events arrive.
    fn start<'a>(
        &'a self,
        callback: Box<dyn Fn(TriggerEvent) + Send + Sync + 'static>,
    ) -> TriggerFuture<'a, ()>;

    /// Stop the trigger.
    fn stop<'a>(&'a self) -> TriggerFuture<'a, ()>;

    /// Pause the trigger (stop accepting new events).
    fn pause<'a>(&'a self) -> TriggerFuture<'a, ()>;

    /// Resume the trigger.
    fn resume<'a>(&'a self) -> TriggerFuture<'a, ()>;

    /// Check if the trigger is running.
    fn is_running(&self) -> bool;
}

/// A trigger factory that creates trigger instances from configuration.
pub trait TriggerFactory: Send + Sync {
    /// Get the trigger type this factory creates.
    fn trigger_type(&self) -> TriggerType;

    /// Create a new trigger instance from configuration.
    fn create(&self, config: &TriggerConfig) -> Result<Box<dyn Trigger>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_type_parsing() {
        assert_eq!(
            TriggerType::parse("trigger::webhook"),
            Some(TriggerType::Webhook)
        );
        assert_eq!(TriggerType::parse("kafka"), Some(TriggerType::Kafka));
        assert_eq!(TriggerType::parse("unknown"), None);
    }

    #[test]
    fn trigger_config_params() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("method".to_string()),
            serde_yaml::Value::String("POST".to_string()),
        );
        params.insert(
            serde_yaml::Value::String("port".to_string()),
            serde_yaml::Value::Number(8080.into()),
        );

        let config = TriggerConfig::new("my_webhook", TriggerType::Webhook)
            .with_params(serde_yaml::Value::Mapping(params));

        assert_eq!(config.get_string("method"), Some("POST"));
        assert_eq!(config.get_i64("port"), Some(8080));
    }
}
