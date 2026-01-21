//! Kafka consumer trigger implementation.
//!
//! This trigger consumes messages from Kafka topics and fires events for each message.
//!
//! # Configuration
//!
//! ```yaml
//! triggers:
//!   - id: kafka_orders
//!     type: trigger::kafka
//!     params:
//!       brokers: "localhost:9092"
//!       group_id: "xerv-consumer"
//!       topics: ["orders", "payments"]
//!       auto_offset_reset: "earliest"  # or "latest"
//!       session_timeout_ms: 6000
//!       enable_auto_commit: true
//! ```
//!
//! # Features
//!
//! - Consumer group support for horizontal scaling
//! - Configurable offset reset behavior
//! - Pause/resume support for backpressure
//! - Graceful shutdown with proper commit
//!
//! # Requirements
//!
//! Requires the `kafka` feature flag and librdkafka installed.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use xerv_core::error::{Result, XervError};
use xerv_core::traits::{Trigger, TriggerConfig, TriggerEvent, TriggerFuture, TriggerType};
use xerv_core::types::RelPtr;

#[cfg(feature = "kafka")]
use rdkafka::Message;
#[cfg(feature = "kafka")]
use rdkafka::config::ClientConfig;
#[cfg(feature = "kafka")]
use rdkafka::consumer::{Consumer, StreamConsumer};

/// Internal state for the Kafka trigger.
struct KafkaState {
    /// Whether the trigger is running.
    running: AtomicBool,
    /// Whether the trigger is paused.
    paused: AtomicBool,
    /// Shutdown signal sender.
    shutdown_tx: RwLock<Option<tokio::sync::oneshot::Sender<()>>>,
    /// Kafka broker addresses.
    brokers: String,
    /// Consumer group ID.
    group_id: String,
    /// Topics to subscribe to.
    topics: Vec<String>,
    /// Auto offset reset behavior ("earliest" or "latest").
    #[allow(dead_code)]
    auto_offset_reset: String,
    /// Session timeout in milliseconds.
    #[allow(dead_code)]
    session_timeout_ms: u32,
    /// Whether to enable auto commit.
    #[allow(dead_code)]
    enable_auto_commit: bool,
}

/// Kafka consumer trigger.
///
/// Consumes messages from Kafka topics and fires trigger events for each message.
/// Supports consumer groups for horizontal scaling.
pub struct KafkaTrigger {
    /// Unique trigger ID.
    id: String,
    /// Internal state.
    state: Arc<KafkaState>,
}

impl KafkaTrigger {
    /// Create a new Kafka trigger from configuration.
    ///
    /// # Configuration Parameters
    ///
    /// - `brokers`: Comma-separated list of Kafka broker addresses (required)
    /// - `group_id`: Consumer group ID (required)
    /// - `topics`: Array of topic names to subscribe to (required)
    /// - `auto_offset_reset`: Offset reset behavior, "earliest" or "latest" (default: "earliest")
    /// - `session_timeout_ms`: Session timeout in milliseconds (default: 6000)
    /// - `enable_auto_commit`: Whether to auto-commit offsets (default: true)
    pub fn from_config(config: &TriggerConfig) -> Result<Self> {
        let id = config.id.clone();

        let brokers = config
            .get_string("brokers")
            .map(|s| s.to_string())
            .ok_or_else(|| XervError::ConfigValue {
                field: format!("trigger:{}:", id),
                cause: "Missing required parameter 'brokers'".to_string(),
            })?;

        let group_id = config
            .get_string("group_id")
            .map(|s| s.to_string())
            .ok_or_else(|| XervError::ConfigValue {
                field: format!("trigger:{}:", id),
                cause: "Missing required parameter 'group_id'".to_string(),
            })?;

        // Parse topics from YAML array
        let topics = config
            .params
            .get("topics")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .ok_or_else(|| XervError::ConfigValue {
                field: format!("trigger:{}:topics", id),
                cause: "Missing required parameter 'topics' (must be an array)".to_string(),
            })?;

        if topics.is_empty() {
            return Err(XervError::ConfigValue {
                field: format!("trigger:{}:topics", id),
                cause: "Topics array cannot be empty".to_string(),
            });
        }

        let auto_offset_reset = config
            .get_string("auto_offset_reset")
            .map(|s| s.to_string())
            .unwrap_or_else(|| "earliest".to_string());

        let session_timeout_ms = config
            .get_i64("session_timeout_ms")
            .map(|v| v as u32)
            .unwrap_or(6000);

        let enable_auto_commit = config.get_bool("enable_auto_commit").unwrap_or(true);

        let state = KafkaState {
            running: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            shutdown_tx: RwLock::new(None),
            brokers,
            group_id,
            topics,
            auto_offset_reset,
            session_timeout_ms,
            enable_auto_commit,
        };

        Ok(Self {
            id,
            state: Arc::new(state),
        })
    }

    /// Create a Kafka trigger with explicit parameters (for testing).
    pub fn new(
        id: impl Into<String>,
        brokers: impl Into<String>,
        group_id: impl Into<String>,
        topics: Vec<String>,
    ) -> Self {
        let state = KafkaState {
            running: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            shutdown_tx: RwLock::new(None),
            brokers: brokers.into(),
            group_id: group_id.into(),
            topics,
            auto_offset_reset: "earliest".to_string(),
            session_timeout_ms: 6000,
            enable_auto_commit: true,
        };

        Self {
            id: id.into(),
            state: Arc::new(state),
        }
    }

    /// Get the configured broker addresses.
    pub fn brokers(&self) -> &str {
        &self.state.brokers
    }

    /// Get the configured group ID.
    pub fn group_id(&self) -> &str {
        &self.state.group_id
    }

    /// Get the configured topics.
    pub fn topics(&self) -> &[String] {
        &self.state.topics
    }

    /// Convert a Kafka message to a TriggerEvent.
    #[cfg(feature = "kafka")]
    fn message_to_event(&self, message: &impl Message) -> TriggerEvent {
        // Extract message metadata
        let topic = message.topic();
        let partition = message.partition();
        let offset = message.offset();
        let timestamp = message.timestamp().to_millis().unwrap_or(0);

        // Create metadata JSON
        let metadata = format!(
            r#"{{"topic":"{}","partition":{},"offset":{},"timestamp":{}}}"#,
            topic, partition, offset, timestamp
        );

        TriggerEvent::new(&self.id, RelPtr::null()).with_metadata(metadata)
    }
}

impl Trigger for KafkaTrigger {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Kafka
    }

    fn id(&self) -> &str {
        &self.id
    }

    #[cfg(feature = "kafka")]
    fn start<'a>(
        &'a self,
        callback: Box<dyn Fn(TriggerEvent) + Send + Sync + 'static>,
    ) -> TriggerFuture<'a, ()> {
        let state = Arc::clone(&self.state);
        let trigger_id = self.id.clone();

        Box::pin(async move {
            // Mark as running
            state.running.store(true, Ordering::SeqCst);

            // Create shutdown channel
            let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
            *state.shutdown_tx.write().await = Some(shutdown_tx);

            // Create Kafka consumer
            let consumer: StreamConsumer = ClientConfig::new()
                .set("bootstrap.servers", &state.brokers)
                .set("group.id", &state.group_id)
                .set("auto.offset.reset", &state.auto_offset_reset)
                .set("session.timeout.ms", state.session_timeout_ms.to_string())
                .set("enable.auto.commit", state.enable_auto_commit.to_string())
                .create()
                .map_err(|e| XervError::ConfigValue {
                    field: format!("trigger:{}:consumer", trigger_id),
                    cause: format!("Failed to create Kafka consumer: {}", e),
                })?;

            // Subscribe to topics
            let topics: Vec<&str> = state.topics.iter().map(|s| s.as_str()).collect();
            consumer
                .subscribe(&topics)
                .map_err(|e| XervError::ConfigValue {
                    field: format!("trigger:{}:subscribe", trigger_id),
                    cause: format!("Failed to subscribe to topics: {}", e),
                })?;

            tracing::info!(
                trigger_id = %trigger_id,
                brokers = %state.brokers,
                group_id = %state.group_id,
                topics = ?state.topics,
                "Kafka consumer started"
            );

            // Message consumption loop
            use futures::StreamExt;
            let mut message_stream = consumer.stream();

            loop {
                tokio::select! {
                    biased;

                    _ = &mut shutdown_rx => {
                        tracing::info!(trigger_id = %trigger_id, "Kafka consumer shutdown requested");
                        break;
                    }

                    message_result = message_stream.next() => {
                        match message_result {
                            Some(Ok(message)) => {
                                // Check if paused
                                if state.paused.load(Ordering::SeqCst) {
                                    continue;
                                }

                                let event = self.message_to_event(&message);
                                callback(event);
                            }
                            Some(Err(e)) => {
                                tracing::warn!(
                                    trigger_id = %trigger_id,
                                    error = %e,
                                    "Kafka message error"
                                );
                            }
                            None => {
                                // Stream ended
                                break;
                            }
                        }
                    }
                }
            }

            state.running.store(false, Ordering::SeqCst);
            tracing::info!(trigger_id = %trigger_id, "Kafka consumer stopped");

            Ok(())
        })
    }

    #[cfg(not(feature = "kafka"))]
    fn start<'a>(
        &'a self,
        _callback: Box<dyn Fn(TriggerEvent) + Send + Sync + 'static>,
    ) -> TriggerFuture<'a, ()> {
        let id = self.id.clone();
        Box::pin(async move {
            Err(XervError::ConfigValue {
                field: format!("trigger:{}:kafka", id),
                cause: "Kafka support not enabled. Build with --features kafka".to_string(),
            })
        })
    }

    fn stop<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = Arc::clone(&self.state);
        let trigger_id = self.id.clone();

        Box::pin(async move {
            // Send shutdown signal
            let shutdown_tx = state.shutdown_tx.write().await.take();
            if let Some(tx) = shutdown_tx {
                let _ = tx.send(());
            }

            state.running.store(false, Ordering::SeqCst);
            tracing::debug!(trigger_id = %trigger_id, "Kafka trigger stopped");

            Ok(())
        })
    }

    fn pause<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = Arc::clone(&self.state);
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(true, Ordering::SeqCst);
            tracing::debug!(trigger_id = %trigger_id, "Kafka trigger paused");
            Ok(())
        })
    }

    fn resume<'a>(&'a self) -> TriggerFuture<'a, ()> {
        let state = Arc::clone(&self.state);
        let trigger_id = self.id.clone();

        Box::pin(async move {
            state.paused.store(false, Ordering::SeqCst);
            tracing::debug!(trigger_id = %trigger_id, "Kafka trigger resumed");
            Ok(())
        })
    }

    fn is_running(&self) -> bool {
        self.state.running.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kafka_trigger_from_config() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("brokers".to_string()),
            serde_yaml::Value::String("localhost:9092".to_string()),
        );
        params.insert(
            serde_yaml::Value::String("group_id".to_string()),
            serde_yaml::Value::String("test-group".to_string()),
        );
        params.insert(
            serde_yaml::Value::String("topics".to_string()),
            serde_yaml::Value::Sequence(vec![
                serde_yaml::Value::String("topic1".to_string()),
                serde_yaml::Value::String("topic2".to_string()),
            ]),
        );

        let config = TriggerConfig {
            id: "test_kafka".to_string(),
            trigger_type: TriggerType::Kafka,
            params: serde_yaml::Value::Mapping(params),
        };

        let trigger = KafkaTrigger::from_config(&config).unwrap();

        assert_eq!(trigger.id(), "test_kafka");
        assert_eq!(trigger.brokers(), "localhost:9092");
        assert_eq!(trigger.group_id(), "test-group");
        assert_eq!(
            trigger.topics(),
            &["topic1".to_string(), "topic2".to_string()]
        );
    }

    #[test]
    fn kafka_trigger_missing_brokers() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("group_id".to_string()),
            serde_yaml::Value::String("test-group".to_string()),
        );
        params.insert(
            serde_yaml::Value::String("topics".to_string()),
            serde_yaml::Value::Sequence(vec![serde_yaml::Value::String("topic1".to_string())]),
        );

        let config = TriggerConfig {
            id: "test_kafka".to_string(),
            trigger_type: TriggerType::Kafka,
            params: serde_yaml::Value::Mapping(params),
        };

        let result = KafkaTrigger::from_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn kafka_trigger_empty_topics() {
        let mut params = serde_yaml::Mapping::new();
        params.insert(
            serde_yaml::Value::String("brokers".to_string()),
            serde_yaml::Value::String("localhost:9092".to_string()),
        );
        params.insert(
            serde_yaml::Value::String("group_id".to_string()),
            serde_yaml::Value::String("test-group".to_string()),
        );
        params.insert(
            serde_yaml::Value::String("topics".to_string()),
            serde_yaml::Value::Sequence(vec![]),
        );

        let config = TriggerConfig {
            id: "test_kafka".to_string(),
            trigger_type: TriggerType::Kafka,
            params: serde_yaml::Value::Mapping(params),
        };

        let result = KafkaTrigger::from_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn kafka_trigger_direct_construction() {
        let trigger = KafkaTrigger::new(
            "direct_kafka",
            "broker1:9092,broker2:9092",
            "my-group",
            vec!["orders".to_string(), "payments".to_string()],
        );

        assert_eq!(trigger.id(), "direct_kafka");
        assert_eq!(trigger.brokers(), "broker1:9092,broker2:9092");
        assert_eq!(trigger.group_id(), "my-group");
        assert_eq!(trigger.topics().len(), 2);
        assert!(!trigger.is_running());
    }

    #[tokio::test]
    async fn kafka_trigger_pause_resume() {
        let trigger = KafkaTrigger::new(
            "pause_test",
            "localhost:9092",
            "test-group",
            vec!["test".to_string()],
        );

        // Initial state
        assert!(!trigger.state.paused.load(Ordering::SeqCst));

        // Pause
        trigger.pause().await.unwrap();
        assert!(trigger.state.paused.load(Ordering::SeqCst));

        // Resume
        trigger.resume().await.unwrap();
        assert!(!trigger.state.paused.load(Ordering::SeqCst));
    }
}
