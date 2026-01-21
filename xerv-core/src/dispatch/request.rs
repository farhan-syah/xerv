//! Trace request types for dispatch.

use crate::types::{PipelineId, TraceId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// A request to execute a trace.
///
/// This is the unit of work that gets queued and dispatched to workers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRequest {
    /// Unique identifier for this trace.
    pub trace_id: TraceId,

    /// Pipeline this trace belongs to.
    pub pipeline_id: PipelineId,

    /// ID of the trigger that initiated this trace.
    pub trigger_id: String,

    /// Serialized trigger payload (JSON or rkyv bytes).
    pub payload: Vec<u8>,

    /// Content type of the payload.
    pub payload_type: PayloadType,

    /// When the request was created (Unix timestamp in milliseconds).
    pub created_at_ms: u64,

    /// Priority (higher = more urgent). Default is 0.
    pub priority: i32,

    /// Number of times this request has been attempted.
    pub attempt: u32,

    /// Maximum number of attempts before giving up.
    pub max_attempts: u32,

    /// Optional metadata for routing, filtering, etc.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Content type of the request payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PayloadType {
    /// JSON-encoded payload.
    #[default]
    Json,
    /// Rkyv-serialized payload (zero-copy).
    Rkyv,
    /// Raw bytes (opaque to dispatch layer).
    Raw,
}

impl TraceRequest {
    /// Create a new trace request.
    pub fn new(
        trace_id: TraceId,
        pipeline_id: PipelineId,
        trigger_id: impl Into<String>,
        payload: Vec<u8>,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            trace_id,
            pipeline_id,
            trigger_id: trigger_id.into(),
            payload,
            payload_type: PayloadType::Json,
            created_at_ms: now,
            priority: 0,
            attempt: 0,
            max_attempts: 3,
            metadata: HashMap::new(),
        }
    }

    /// Create a builder for more complex requests.
    pub fn builder(pipeline_id: PipelineId, trigger_id: impl Into<String>) -> TraceRequestBuilder {
        TraceRequestBuilder::new(pipeline_id, trigger_id)
    }

    /// Check if this request has exceeded max attempts.
    pub fn is_exhausted(&self) -> bool {
        self.attempt >= self.max_attempts
    }

    /// Increment the attempt counter.
    pub fn increment_attempt(&mut self) {
        self.attempt += 1;
    }

    /// Get the age of this request in milliseconds.
    pub fn age_ms(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        now.saturating_sub(self.created_at_ms)
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Get a metadata value.
    pub fn get_metadata(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).map(|s| s.as_str())
    }
}

/// Builder for creating trace requests.
pub struct TraceRequestBuilder {
    pipeline_id: PipelineId,
    trigger_id: String,
    trace_id: Option<TraceId>,
    payload: Vec<u8>,
    payload_type: PayloadType,
    priority: i32,
    max_attempts: u32,
    metadata: HashMap<String, String>,
}

impl TraceRequestBuilder {
    /// Create a new builder.
    pub fn new(pipeline_id: PipelineId, trigger_id: impl Into<String>) -> Self {
        Self {
            pipeline_id,
            trigger_id: trigger_id.into(),
            trace_id: None,
            payload: Vec::new(),
            payload_type: PayloadType::Json,
            priority: 0,
            max_attempts: 3,
            metadata: HashMap::new(),
        }
    }

    /// Set a specific trace ID (otherwise one is generated).
    pub fn trace_id(mut self, id: TraceId) -> Self {
        self.trace_id = Some(id);
        self
    }

    /// Set the payload as JSON.
    pub fn json_payload(mut self, payload: impl Serialize) -> Self {
        self.payload = serde_json::to_vec(&payload).unwrap_or_default();
        self.payload_type = PayloadType::Json;
        self
    }

    /// Set the payload as raw bytes.
    pub fn raw_payload(mut self, payload: Vec<u8>) -> Self {
        self.payload = payload;
        self.payload_type = PayloadType::Raw;
        self
    }

    /// Set the payload as rkyv-serialized bytes.
    pub fn rkyv_payload(mut self, payload: Vec<u8>) -> Self {
        self.payload = payload;
        self.payload_type = PayloadType::Rkyv;
        self
    }

    /// Set the priority (higher = more urgent).
    pub fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Set the maximum number of attempts.
    pub fn max_attempts(mut self, max: u32) -> Self {
        self.max_attempts = max;
        self
    }

    /// Add metadata.
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Build the trace request.
    pub fn build(self) -> TraceRequest {
        let trace_id = self.trace_id.unwrap_or_default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        TraceRequest {
            trace_id,
            pipeline_id: self.pipeline_id,
            trigger_id: self.trigger_id,
            payload: self.payload,
            payload_type: self.payload_type,
            created_at_ms: now,
            priority: self.priority,
            attempt: 0,
            max_attempts: self.max_attempts,
            metadata: self.metadata,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_creation() {
        let pipeline_id = PipelineId::new("test", 1);
        let request = TraceRequest::new(
            TraceId::new(),
            pipeline_id.clone(),
            "webhook",
            b"{}".to_vec(),
        );

        assert_eq!(request.pipeline_id.name(), "test");
        assert_eq!(request.trigger_id, "webhook");
        assert_eq!(request.attempt, 0);
        assert!(!request.is_exhausted());
    }

    #[test]
    fn builder_usage() {
        let request = TraceRequest::builder(PipelineId::new("orders", 2), "cron")
            .priority(10)
            .max_attempts(5)
            .metadata("region", "us-east")
            .json_payload(serde_json::json!({"key": "value"}))
            .build();

        assert_eq!(request.priority, 10);
        assert_eq!(request.max_attempts, 5);
        assert_eq!(request.get_metadata("region"), Some("us-east"));
        assert_eq!(request.payload_type, PayloadType::Json);
    }

    #[test]
    fn attempt_tracking() {
        let mut request = TraceRequest::new(
            TraceId::new(),
            PipelineId::new("test", 1),
            "trigger",
            vec![],
        );
        request.max_attempts = 3;

        assert!(!request.is_exhausted());

        request.increment_attempt();
        assert!(!request.is_exhausted());

        request.increment_attempt();
        assert!(!request.is_exhausted());

        request.increment_attempt();
        assert!(request.is_exhausted());
    }

    #[test]
    fn age_calculation() {
        let request = TraceRequest::new(
            TraceId::new(),
            PipelineId::new("test", 1),
            "trigger",
            vec![],
        );

        // Request was just created, age should be very small
        assert!(request.age_ms() < 1000);
    }
}
