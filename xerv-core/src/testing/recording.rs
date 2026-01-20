//! Event recording for test replay and debugging.
//!
//! Records events during test execution for later analysis, replay, or debugging.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// An event recorded during test execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecordedEvent {
    /// Clock time was read.
    ClockNow {
        /// Time in nanoseconds.
        nanos: u64,
    },

    /// System time was read.
    SystemTime {
        /// Time in milliseconds since UNIX epoch.
        millis: u64,
    },

    /// Sleep was requested.
    Sleep {
        /// Duration in milliseconds.
        duration_ms: u64,
    },

    /// Time was advanced (mock clock only).
    TimeAdvanced {
        /// Duration in milliseconds.
        duration_ms: u64,
    },

    /// UUID was generated.
    UuidGenerated {
        /// The generated UUID.
        uuid: String,
    },

    /// Random u64 was generated.
    RandomU64 {
        /// The generated value.
        value: u64,
    },

    /// Random f64 was generated.
    RandomF64 {
        /// The generated value.
        value: f64,
    },

    /// Random bytes were generated.
    RandomBytes {
        /// Number of bytes generated.
        count: usize,
    },

    /// HTTP request was made.
    HttpRequest {
        /// HTTP method.
        method: String,
        /// Request URL.
        url: String,
        /// Request headers.
        headers: HashMap<String, String>,
        /// Request body size.
        body_size: usize,
    },

    /// HTTP response was received.
    HttpResponse {
        /// HTTP status code.
        status: u16,
        /// Response body size.
        body_size: usize,
    },

    /// File was read.
    FileRead {
        /// File path.
        path: String,
        /// Whether the read succeeded.
        success: bool,
        /// Bytes read (if successful).
        bytes_read: Option<usize>,
    },

    /// File was written.
    FileWrite {
        /// File path.
        path: String,
        /// Whether the write succeeded.
        success: bool,
        /// Bytes written.
        bytes_written: usize,
    },

    /// Environment variable was read.
    EnvRead {
        /// Variable name.
        key: String,
        /// Value found (if any).
        value: Option<String>,
    },

    /// Environment variable was set.
    EnvSet {
        /// Variable name.
        key: String,
        /// New value.
        value: String,
    },

    /// Secret was read.
    SecretRead {
        /// Secret key.
        key: String,
        /// Whether the secret was found.
        found: bool,
    },

    /// Node execution started.
    NodeExecutionStart {
        /// Node ID.
        node_id: u32,
        /// Node type.
        node_type: String,
    },

    /// Node execution completed.
    NodeExecutionComplete {
        /// Node ID.
        node_id: u32,
        /// Output port.
        output_port: String,
        /// Whether execution succeeded.
        success: bool,
    },

    /// Custom event for application-specific recording.
    Custom {
        /// Event name.
        name: String,
        /// Event data as JSON.
        data: serde_json::Value,
    },
}

impl RecordedEvent {
    /// Create a custom event.
    pub fn custom(name: impl Into<String>, data: serde_json::Value) -> Self {
        Self::Custom {
            name: name.into(),
            data,
        }
    }

    /// Get the event type as a string.
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::ClockNow { .. } => "clock_now",
            Self::SystemTime { .. } => "system_time",
            Self::Sleep { .. } => "sleep",
            Self::TimeAdvanced { .. } => "time_advanced",
            Self::UuidGenerated { .. } => "uuid_generated",
            Self::RandomU64 { .. } => "random_u64",
            Self::RandomF64 { .. } => "random_f64",
            Self::RandomBytes { .. } => "random_bytes",
            Self::HttpRequest { .. } => "http_request",
            Self::HttpResponse { .. } => "http_response",
            Self::FileRead { .. } => "file_read",
            Self::FileWrite { .. } => "file_write",
            Self::EnvRead { .. } => "env_read",
            Self::EnvSet { .. } => "env_set",
            Self::SecretRead { .. } => "secret_read",
            Self::NodeExecutionStart { .. } => "node_execution_start",
            Self::NodeExecutionComplete { .. } => "node_execution_complete",
            Self::Custom { .. } => "custom",
        }
    }
}

/// Recorder for capturing events during test execution.
///
/// Thread-safe and can be shared across async tasks.
///
/// # Example
///
/// ```
/// use xerv_core::testing::{EventRecorder, RecordedEvent};
///
/// let recorder = EventRecorder::new();
///
/// recorder.record(RecordedEvent::ClockNow { nanos: 1000 });
/// recorder.record(RecordedEvent::UuidGenerated { uuid: "test-uuid".to_string() });
///
/// let events = recorder.events();
/// assert_eq!(events.len(), 2);
///
/// let json = recorder.to_json();
/// assert!(json.contains("clock_now"));
/// ```
pub struct EventRecorder {
    events: RwLock<Vec<RecordedEvent>>,
    enabled: std::sync::atomic::AtomicBool,
}

impl EventRecorder {
    /// Create a new event recorder.
    pub fn new() -> Self {
        Self {
            events: RwLock::new(Vec::new()),
            enabled: std::sync::atomic::AtomicBool::new(true),
        }
    }

    /// Record an event.
    pub fn record(&self, event: RecordedEvent) {
        if self.enabled.load(std::sync::atomic::Ordering::SeqCst) {
            self.events.write().push(event);
        }
    }

    /// Get all recorded events.
    pub fn events(&self) -> Vec<RecordedEvent> {
        self.events.read().clone()
    }

    /// Get the number of recorded events.
    pub fn len(&self) -> usize {
        self.events.read().len()
    }

    /// Check if no events have been recorded.
    pub fn is_empty(&self) -> bool {
        self.events.read().is_empty()
    }

    /// Clear all recorded events.
    pub fn clear(&self) {
        self.events.write().clear();
    }

    /// Enable or disable recording.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    /// Check if recording is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Convert recorded events to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&*self.events.read()).expect("Failed to serialize events")
    }

    /// Convert recorded events to compact JSON.
    pub fn to_json_compact(&self) -> String {
        serde_json::to_string(&*self.events.read()).expect("Failed to serialize events")
    }

    /// Load events from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let events: Vec<RecordedEvent> = serde_json::from_str(json)?;
        Ok(Self {
            events: RwLock::new(events),
            enabled: std::sync::atomic::AtomicBool::new(true),
        })
    }

    /// Filter events by type.
    pub fn events_of_type(&self, event_type: &str) -> Vec<RecordedEvent> {
        self.events
            .read()
            .iter()
            .filter(|e| e.event_type() == event_type)
            .cloned()
            .collect()
    }

    /// Find events matching a predicate.
    pub fn find<F>(&self, predicate: F) -> Vec<RecordedEvent>
    where
        F: Fn(&RecordedEvent) -> bool,
    {
        self.events
            .read()
            .iter()
            .filter(|e| predicate(e))
            .cloned()
            .collect()
    }

    /// Assert that an event of the given type was recorded.
    pub fn assert_recorded(&self, event_type: &str) -> bool {
        self.events
            .read()
            .iter()
            .any(|e| e.event_type() == event_type)
    }

    /// Assert that a specific HTTP request was made.
    pub fn assert_http_request(&self, method: &str, url_pattern: &str) -> bool {
        let re = regex::Regex::new(url_pattern).expect("Invalid URL pattern");
        self.events.read().iter().any(|e| {
            if let RecordedEvent::HttpRequest { method: m, url, .. } = e {
                m.eq_ignore_ascii_case(method) && re.is_match(url)
            } else {
                false
            }
        })
    }

    /// Get all HTTP requests.
    pub fn http_requests(&self) -> Vec<(String, String)> {
        self.events
            .read()
            .iter()
            .filter_map(|e| {
                if let RecordedEvent::HttpRequest { method, url, .. } = e {
                    Some((method.clone(), url.clone()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all generated UUIDs.
    pub fn generated_uuids(&self) -> Vec<Uuid> {
        self.events
            .read()
            .iter()
            .filter_map(|e| {
                if let RecordedEvent::UuidGenerated { uuid } = e {
                    Uuid::parse_str(uuid).ok()
                } else {
                    None
                }
            })
            .collect()
    }
}

impl Default for EventRecorder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_retrieve() {
        let recorder = EventRecorder::new();

        recorder.record(RecordedEvent::ClockNow { nanos: 1000 });
        recorder.record(RecordedEvent::UuidGenerated {
            uuid: "test-uuid".to_string(),
        });

        let events = recorder.events();
        assert_eq!(events.len(), 2);

        assert!(matches!(events[0], RecordedEvent::ClockNow { nanos: 1000 }));
    }

    #[test]
    fn json_serialization() {
        let recorder = EventRecorder::new();
        recorder.record(RecordedEvent::RandomU64 { value: 42 });

        let json = recorder.to_json();
        assert!(json.contains("random_u64"));
        assert!(json.contains("42"));

        let loaded = EventRecorder::from_json(&json).unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn filter_by_type() {
        let recorder = EventRecorder::new();
        recorder.record(RecordedEvent::ClockNow { nanos: 100 });
        recorder.record(RecordedEvent::UuidGenerated {
            uuid: "a".to_string(),
        });
        recorder.record(RecordedEvent::ClockNow { nanos: 200 });

        let clock_events = recorder.events_of_type("clock_now");
        assert_eq!(clock_events.len(), 2);
    }

    #[test]
    fn disable_recording() {
        let recorder = EventRecorder::new();

        recorder.record(RecordedEvent::ClockNow { nanos: 100 });
        assert_eq!(recorder.len(), 1);

        recorder.set_enabled(false);
        recorder.record(RecordedEvent::ClockNow { nanos: 200 });
        assert_eq!(recorder.len(), 1); // Still 1, recording was disabled

        recorder.set_enabled(true);
        recorder.record(RecordedEvent::ClockNow { nanos: 300 });
        assert_eq!(recorder.len(), 2);
    }

    #[test]
    fn assert_http_request() {
        let recorder = EventRecorder::new();
        recorder.record(RecordedEvent::HttpRequest {
            method: "POST".to_string(),
            url: "https://api.example.com/users".to_string(),
            headers: HashMap::new(),
            body_size: 0,
        });

        assert!(recorder.assert_http_request("POST", r"api\.example\.com/users"));
        assert!(!recorder.assert_http_request("GET", r"api\.example\.com/users"));
        assert!(!recorder.assert_http_request("POST", r"other\.com"));
    }

    #[test]
    fn custom_events() {
        let recorder = EventRecorder::new();
        recorder.record(RecordedEvent::custom(
            "my_event",
            serde_json::json!({"key": "value"}),
        ));

        let events = recorder.events_of_type("custom");
        assert_eq!(events.len(), 1);

        if let RecordedEvent::Custom { name, data } = &events[0] {
            assert_eq!(name, "my_event");
            assert_eq!(data["key"], "value");
        } else {
            panic!("Expected Custom event");
        }
    }
}
