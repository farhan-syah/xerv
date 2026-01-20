//! Log filtering for querying and streaming events.
//!
//! Provides composable filters for log events based on level, category,
//! trace ID, node ID, pipeline ID, time range, and message content.

use super::event::{LogCategory, LogEvent, LogLevel};
use crate::types::{NodeId, TraceId};
use serde::{Deserialize, Serialize};

/// A filter for log events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogFilter {
    /// Minimum log level (inclusive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_level: Option<LogLevel>,
    /// Maximum log level (inclusive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_level: Option<LogLevel>,
    /// Allowed categories (empty = all).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<LogCategory>,
    /// Filter by trace ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<TraceId>,
    /// Filter by node ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    /// Filter by pipeline ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline_id: Option<String>,
    /// Filter by message content (case-insensitive contains).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_contains: Option<String>,
    /// Start timestamp (nanoseconds since epoch, inclusive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time_ns: Option<u64>,
    /// End timestamp (nanoseconds since epoch, inclusive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time_ns: Option<u64>,
    /// Maximum number of events to return.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Offset for pagination.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

impl LogFilter {
    /// Create a new empty filter (matches all events).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set minimum log level.
    pub fn min_level(mut self, level: LogLevel) -> Self {
        self.min_level = Some(level);
        self
    }

    /// Set maximum log level.
    pub fn max_level(mut self, level: LogLevel) -> Self {
        self.max_level = Some(level);
        self
    }

    /// Set exact log level.
    pub fn level(mut self, level: LogLevel) -> Self {
        self.min_level = Some(level);
        self.max_level = Some(level);
        self
    }

    /// Set allowed categories.
    pub fn categories(mut self, categories: Vec<LogCategory>) -> Self {
        self.categories = categories;
        self
    }

    /// Add an allowed category.
    pub fn category(mut self, category: LogCategory) -> Self {
        self.categories.push(category);
        self
    }

    /// Set trace ID filter.
    pub fn trace_id(mut self, trace_id: TraceId) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Set node ID filter.
    pub fn node_id(mut self, node_id: NodeId) -> Self {
        self.node_id = Some(node_id);
        self
    }

    /// Set pipeline ID filter.
    pub fn pipeline_id(mut self, pipeline_id: impl Into<String>) -> Self {
        self.pipeline_id = Some(pipeline_id.into());
        self
    }

    /// Set message content filter.
    pub fn message_contains(mut self, pattern: impl Into<String>) -> Self {
        self.message_contains = Some(pattern.into());
        self
    }

    /// Set start time filter.
    pub fn start_time(mut self, timestamp_ns: u64) -> Self {
        self.start_time_ns = Some(timestamp_ns);
        self
    }

    /// Set end time filter.
    pub fn end_time(mut self, timestamp_ns: u64) -> Self {
        self.end_time_ns = Some(timestamp_ns);
        self
    }

    /// Set time range filter.
    pub fn time_range(mut self, start_ns: u64, end_ns: u64) -> Self {
        self.start_time_ns = Some(start_ns);
        self.end_time_ns = Some(end_ns);
        self
    }

    /// Set result limit.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set pagination offset.
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Check if an event matches this filter.
    pub fn matches(&self, event: &LogEvent) -> bool {
        // Check level range
        if let Some(min) = self.min_level {
            if event.level < min {
                return false;
            }
        }
        if let Some(max) = self.max_level {
            if event.level > max {
                return false;
            }
        }

        // Check categories
        if !self.categories.is_empty() && !self.categories.contains(&event.category) {
            return false;
        }

        // Check trace ID
        if let Some(trace_id) = self.trace_id {
            if event.trace_id != Some(trace_id) {
                return false;
            }
        }

        // Check node ID
        if let Some(node_id) = self.node_id {
            if event.node_id != Some(node_id) {
                return false;
            }
        }

        // Check pipeline ID
        if let Some(ref pipeline_id) = self.pipeline_id {
            if event.pipeline_id.as_ref() != Some(pipeline_id) {
                return false;
            }
        }

        // Check message content
        if let Some(ref pattern) = self.message_contains {
            if !event
                .message
                .to_lowercase()
                .contains(&pattern.to_lowercase())
            {
                return false;
            }
        }

        // Check time range
        if let Some(start) = self.start_time_ns {
            if event.timestamp_ns < start {
                return false;
            }
        }
        if let Some(end) = self.end_time_ns {
            if event.timestamp_ns > end {
                return false;
            }
        }

        true
    }

    /// Apply pagination to a list of events.
    pub fn paginate<'a>(&self, events: impl Iterator<Item = &'a LogEvent>) -> Vec<&'a LogEvent> {
        let offset = self.offset.unwrap_or(0);
        let limit = self.limit.unwrap_or(usize::MAX);

        events.skip(offset).take(limit).collect()
    }

    /// Check if this filter has any constraints.
    pub fn is_empty(&self) -> bool {
        self.min_level.is_none()
            && self.max_level.is_none()
            && self.categories.is_empty()
            && self.trace_id.is_none()
            && self.node_id.is_none()
            && self.pipeline_id.is_none()
            && self.message_contains.is_none()
            && self.start_time_ns.is_none()
            && self.end_time_ns.is_none()
    }

    /// Create a human-readable description of the filter.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();

        if let Some(min) = self.min_level {
            if let Some(max) = self.max_level {
                if min == max {
                    parts.push(format!("level={}", min));
                } else {
                    parts.push(format!("level={}-{}", min, max));
                }
            } else {
                parts.push(format!("level>={}", min));
            }
        } else if let Some(max) = self.max_level {
            parts.push(format!("level<={}", max));
        }

        if !self.categories.is_empty() {
            let cats: Vec<&str> = self.categories.iter().map(|c| c.as_str()).collect();
            parts.push(format!("category={}", cats.join(",")));
        }

        if let Some(trace_id) = self.trace_id {
            parts.push(format!("trace={}", trace_id));
        }

        if let Some(node_id) = self.node_id {
            parts.push(format!("node={}", node_id.as_u32()));
        }

        if let Some(ref pipeline_id) = self.pipeline_id {
            parts.push(format!("pipeline={}", pipeline_id));
        }

        if let Some(ref pattern) = self.message_contains {
            parts.push(format!("contains=\"{}\"", pattern));
        }

        if parts.is_empty() {
            "all events".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// Builder for creating filters from CLI arguments or query parameters.
#[derive(Debug, Default)]
pub struct LogFilterBuilder {
    filter: LogFilter,
}

impl LogFilterBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse level from string.
    pub fn parse_level(mut self, level: &str) -> Self {
        if let Some(parsed) = LogLevel::parse(level) {
            self.filter.min_level = Some(parsed);
        }
        self
    }

    /// Parse trace ID from string.
    pub fn parse_trace_id(mut self, trace_id: &str) -> Self {
        if let Some(parsed) = TraceId::parse(trace_id) {
            self.filter.trace_id = Some(parsed);
        }
        self
    }

    /// Parse node ID from string.
    pub fn parse_node_id(mut self, node_id: &str) -> Self {
        if let Ok(id) = node_id.parse::<u32>() {
            self.filter.node_id = Some(NodeId::new(id));
        }
        self
    }

    /// Set pipeline ID.
    pub fn pipeline_id(mut self, pipeline_id: impl Into<String>) -> Self {
        self.filter.pipeline_id = Some(pipeline_id.into());
        self
    }

    /// Set message pattern.
    pub fn message_contains(mut self, pattern: impl Into<String>) -> Self {
        self.filter.message_contains = Some(pattern.into());
        self
    }

    /// Set limit.
    pub fn limit(mut self, limit: usize) -> Self {
        self.filter.limit = Some(limit);
        self
    }

    /// Build the filter.
    pub fn build(self) -> LogFilter {
        self.filter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_event() -> LogEvent {
        LogEvent::info(LogCategory::Node, "Test message")
            .with_trace_id(TraceId::new())
            .with_node_id(NodeId::new(42))
            .with_pipeline_id("test_pipeline")
    }

    #[test]
    fn filter_empty_matches_all() {
        let filter = LogFilter::new();
        let event = test_event();

        assert!(filter.matches(&event));
    }

    #[test]
    fn filter_by_level() {
        let event = LogEvent::warn(LogCategory::System, "Warning");

        assert!(LogFilter::new().min_level(LogLevel::Info).matches(&event));
        assert!(LogFilter::new().min_level(LogLevel::Warn).matches(&event));
        assert!(!LogFilter::new().min_level(LogLevel::Error).matches(&event));

        assert!(LogFilter::new().max_level(LogLevel::Warn).matches(&event));
        assert!(LogFilter::new().max_level(LogLevel::Error).matches(&event));
        assert!(!LogFilter::new().max_level(LogLevel::Info).matches(&event));

        assert!(LogFilter::new().level(LogLevel::Warn).matches(&event));
        assert!(!LogFilter::new().level(LogLevel::Info).matches(&event));
    }

    #[test]
    fn filter_by_category() {
        let event = LogEvent::info(LogCategory::Node, "Node event");

        assert!(LogFilter::new().category(LogCategory::Node).matches(&event));
        assert!(
            !LogFilter::new()
                .category(LogCategory::Trace)
                .matches(&event)
        );
        assert!(
            LogFilter::new()
                .categories(vec![LogCategory::Node, LogCategory::Trace])
                .matches(&event)
        );
    }

    #[test]
    fn filter_by_trace_id() {
        let trace_id = TraceId::new();
        let event = LogEvent::info(LogCategory::Trace, "Event").with_trace_id(trace_id);

        assert!(LogFilter::new().trace_id(trace_id).matches(&event));
        assert!(!LogFilter::new().trace_id(TraceId::new()).matches(&event));
    }

    #[test]
    fn filter_by_node_id() {
        let event = LogEvent::info(LogCategory::Node, "Event").with_node_id(NodeId::new(42));

        assert!(LogFilter::new().node_id(NodeId::new(42)).matches(&event));
        assert!(!LogFilter::new().node_id(NodeId::new(99)).matches(&event));
    }

    #[test]
    fn filter_by_pipeline_id() {
        let event = LogEvent::info(LogCategory::Pipeline, "Event").with_pipeline_id("my_pipeline");

        assert!(LogFilter::new().pipeline_id("my_pipeline").matches(&event));
        assert!(!LogFilter::new().pipeline_id("other").matches(&event));
    }

    #[test]
    fn filter_by_message() {
        let event = LogEvent::info(LogCategory::System, "Processing order ORD-123");

        assert!(LogFilter::new().message_contains("order").matches(&event));
        assert!(LogFilter::new().message_contains("ORDER").matches(&event)); // Case-insensitive
        assert!(LogFilter::new().message_contains("ORD-123").matches(&event));
        assert!(!LogFilter::new().message_contains("user").matches(&event));
    }

    #[test]
    fn filter_combined() {
        let trace_id = TraceId::new();
        let event = LogEvent::warn(LogCategory::Node, "Node timeout")
            .with_trace_id(trace_id)
            .with_node_id(NodeId::new(5))
            .with_pipeline_id("order_flow");

        let filter = LogFilter::new()
            .min_level(LogLevel::Warn)
            .trace_id(trace_id)
            .category(LogCategory::Node)
            .pipeline_id("order_flow");

        assert!(filter.matches(&event));

        // Change one condition to fail
        let filter_wrong_pipeline = LogFilter::new()
            .min_level(LogLevel::Warn)
            .trace_id(trace_id)
            .category(LogCategory::Node)
            .pipeline_id("wrong_flow");

        assert!(!filter_wrong_pipeline.matches(&event));
    }

    #[test]
    fn filter_pagination() {
        let events: Vec<LogEvent> = (0..10)
            .map(|i| LogEvent::info(LogCategory::System, format!("Event {}", i)))
            .collect();

        let filter = LogFilter::new().offset(3).limit(4);
        let paginated: Vec<&LogEvent> = filter.paginate(events.iter()).into_iter().collect();

        assert_eq!(paginated.len(), 4);
        assert_eq!(paginated[0].message, "Event 3");
        assert_eq!(paginated[3].message, "Event 6");
    }

    #[test]
    fn filter_describe() {
        let filter = LogFilter::new()
            .min_level(LogLevel::Warn)
            .category(LogCategory::Node)
            .pipeline_id("test");

        let desc = filter.describe();
        assert!(desc.contains("level>=warn"));
        assert!(desc.contains("category=node"));
        assert!(desc.contains("pipeline=test"));
    }

    #[test]
    fn filter_builder() {
        let filter = LogFilterBuilder::new()
            .parse_level("warn")
            .pipeline_id("my_pipeline")
            .limit(100)
            .build();

        assert_eq!(filter.min_level, Some(LogLevel::Warn));
        assert_eq!(filter.pipeline_id, Some("my_pipeline".to_string()));
        assert_eq!(filter.limit, Some(100));
    }
}
