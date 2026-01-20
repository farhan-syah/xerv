//! Log collector for capturing and storing log events.
//!
//! Provides a thread-safe collector that accumulates log events
//! with automatic ID assignment and optional filtering.

use super::event::{LogCategory, LogEvent, LogLevel};
use super::filter::LogFilter;
use crate::types::{NodeId, TraceId};
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Maximum number of events to keep in the default buffer.
pub const DEFAULT_BUFFER_CAPACITY: usize = 10_000;

/// Trait for log event collectors.
pub trait LogCollector: Send + Sync {
    /// Collect a log event.
    fn collect(&self, event: LogEvent);

    /// Get the number of collected events.
    fn len(&self) -> usize;

    /// Check if the collector is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Type alias for log event subscriber callbacks.
type LogSubscribers = RwLock<Vec<Arc<dyn Fn(&LogEvent) + Send + Sync>>>;

/// Thread-safe log collector with a bounded ring buffer.
pub struct BufferedCollector {
    /// Ring buffer of events.
    buffer: RwLock<VecDeque<LogEvent>>,
    /// Maximum buffer capacity.
    capacity: usize,
    /// Next event ID counter.
    next_id: AtomicU64,
    /// Optional filter for incoming events.
    filter: Option<LogFilter>,
    /// Subscribers for real-time event notifications.
    subscribers: LogSubscribers,
}

impl BufferedCollector {
    /// Create a new collector with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
            next_id: AtomicU64::new(1),
            filter: None,
            subscribers: RwLock::new(Vec::new()),
        }
    }

    /// Create a collector with default capacity.
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_BUFFER_CAPACITY)
    }

    /// Set a filter for incoming events.
    pub fn with_filter(mut self, filter: LogFilter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Add a subscriber for real-time event notifications.
    pub fn subscribe(&self, callback: Arc<dyn Fn(&LogEvent) + Send + Sync>) {
        let mut subscribers = self.subscribers.write();
        subscribers.push(callback);
    }

    /// Get events matching a filter.
    pub fn query(&self, filter: &LogFilter) -> Vec<LogEvent> {
        let buffer = self.buffer.read();
        buffer
            .iter()
            .filter(|e| filter.matches(e))
            .cloned()
            .collect()
    }

    /// Get the most recent N events.
    pub fn recent(&self, limit: usize) -> Vec<LogEvent> {
        let buffer = self.buffer.read();
        buffer.iter().rev().take(limit).cloned().collect()
    }

    /// Get events for a specific trace.
    pub fn by_trace(&self, trace_id: TraceId) -> Vec<LogEvent> {
        let buffer = self.buffer.read();
        buffer
            .iter()
            .filter(|e| e.trace_id == Some(trace_id))
            .cloned()
            .collect()
    }

    /// Get events for a specific node within a trace.
    pub fn by_trace_node(&self, trace_id: TraceId, node_id: NodeId) -> Vec<LogEvent> {
        let buffer = self.buffer.read();
        buffer
            .iter()
            .filter(|e| e.trace_id == Some(trace_id) && e.node_id == Some(node_id))
            .cloned()
            .collect()
    }

    /// Get events for a specific pipeline.
    pub fn by_pipeline(&self, pipeline_id: &str) -> Vec<LogEvent> {
        let buffer = self.buffer.read();
        buffer
            .iter()
            .filter(|e| e.pipeline_id.as_deref() == Some(pipeline_id))
            .cloned()
            .collect()
    }

    /// Get events at or above a certain level.
    pub fn by_level(&self, min_level: LogLevel) -> Vec<LogEvent> {
        let buffer = self.buffer.read();
        buffer
            .iter()
            .filter(|e| e.level >= min_level)
            .cloned()
            .collect()
    }

    /// Get all events (up to capacity).
    pub fn all(&self) -> Vec<LogEvent> {
        let buffer = self.buffer.read();
        buffer.iter().cloned().collect()
    }

    /// Clear all events.
    pub fn clear(&self) {
        let mut buffer = self.buffer.write();
        buffer.clear();
    }

    /// Get buffer capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get events since a given event ID.
    pub fn since(&self, after_id: u64) -> Vec<LogEvent> {
        let buffer = self.buffer.read();
        buffer.iter().filter(|e| e.id > after_id).cloned().collect()
    }

    /// Get events within a time range (nanoseconds since epoch).
    pub fn time_range(&self, start_ns: u64, end_ns: u64) -> Vec<LogEvent> {
        let buffer = self.buffer.read();
        buffer
            .iter()
            .filter(|e| e.timestamp_ns >= start_ns && e.timestamp_ns <= end_ns)
            .cloned()
            .collect()
    }
}

impl LogCollector for BufferedCollector {
    fn collect(&self, mut event: LogEvent) {
        // Apply filter if configured
        if let Some(ref filter) = self.filter {
            if !filter.matches(&event) {
                return;
            }
        }

        // Assign event ID
        event.id = self.next_id.fetch_add(1, Ordering::SeqCst);

        // Notify subscribers
        {
            let subscribers = self.subscribers.read();
            for subscriber in subscribers.iter() {
                subscriber(&event);
            }
        }

        // Add to buffer
        let mut buffer = self.buffer.write();
        if buffer.len() >= self.capacity {
            buffer.pop_front();
        }
        buffer.push_back(event);
    }

    fn len(&self) -> usize {
        self.buffer.read().len()
    }
}

impl Default for BufferedCollector {
    fn default() -> Self {
        Self::with_default_capacity()
    }
}

/// A no-op collector that discards all events.
pub struct NullCollector;

impl LogCollector for NullCollector {
    fn collect(&self, _event: LogEvent) {
        // Discard
    }

    fn len(&self) -> usize {
        0
    }
}

/// A collector that writes events to multiple collectors.
pub struct MultiCollector {
    collectors: Vec<Arc<dyn LogCollector>>,
}

impl MultiCollector {
    /// Create a new multi-collector.
    pub fn new(collectors: Vec<Arc<dyn LogCollector>>) -> Self {
        Self { collectors }
    }
}

impl LogCollector for MultiCollector {
    fn collect(&self, event: LogEvent) {
        for collector in &self.collectors {
            collector.collect(event.clone());
        }
    }

    fn len(&self) -> usize {
        self.collectors.first().map(|c| c.len()).unwrap_or(0)
    }
}

/// Context for logging within a specific trace/node.
pub struct LogContext {
    collector: Arc<dyn LogCollector>,
    trace_id: Option<TraceId>,
    node_id: Option<NodeId>,
    pipeline_id: Option<String>,
}

impl LogContext {
    /// Create a new log context.
    pub fn new(collector: Arc<dyn LogCollector>) -> Self {
        Self {
            collector,
            trace_id: None,
            node_id: None,
            pipeline_id: None,
        }
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

    /// Create a child context for a specific node.
    pub fn for_node(&self, node_id: NodeId) -> Self {
        Self {
            collector: Arc::clone(&self.collector),
            trace_id: self.trace_id,
            node_id: Some(node_id),
            pipeline_id: self.pipeline_id.clone(),
        }
    }

    /// Log an event with context fields automatically applied.
    pub fn log(&self, mut event: LogEvent) {
        if event.trace_id.is_none() {
            event.trace_id = self.trace_id;
        }
        if event.node_id.is_none() {
            event.node_id = self.node_id;
        }
        if event.pipeline_id.is_none() {
            event.pipeline_id = self.pipeline_id.clone();
        }
        self.collector.collect(event);
    }

    /// Log a trace-level message.
    pub fn trace(&self, category: LogCategory, message: impl Into<String>) {
        self.log(LogEvent::trace(category, message));
    }

    /// Log a debug-level message.
    pub fn debug(&self, category: LogCategory, message: impl Into<String>) {
        self.log(LogEvent::debug(category, message));
    }

    /// Log an info-level message.
    pub fn info(&self, category: LogCategory, message: impl Into<String>) {
        self.log(LogEvent::info(category, message));
    }

    /// Log a warn-level message.
    pub fn warn(&self, category: LogCategory, message: impl Into<String>) {
        self.log(LogEvent::warn(category, message));
    }

    /// Log an error-level message.
    pub fn error(&self, category: LogCategory, message: impl Into<String>) {
        self.log(LogEvent::error(category, message));
    }
}

impl Clone for LogContext {
    fn clone(&self) -> Self {
        Self {
            collector: Arc::clone(&self.collector),
            trace_id: self.trace_id,
            node_id: self.node_id,
            pipeline_id: self.pipeline_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffered_collector_basic() {
        let collector = BufferedCollector::new(100);

        collector.collect(LogEvent::info(LogCategory::System, "Test message"));
        collector.collect(LogEvent::warn(LogCategory::Node, "Warning"));

        assert_eq!(collector.len(), 2);
    }

    #[test]
    fn buffered_collector_capacity() {
        let collector = BufferedCollector::new(3);

        collector.collect(LogEvent::info(LogCategory::System, "Event 1"));
        collector.collect(LogEvent::info(LogCategory::System, "Event 2"));
        collector.collect(LogEvent::info(LogCategory::System, "Event 3"));
        collector.collect(LogEvent::info(LogCategory::System, "Event 4"));

        assert_eq!(collector.len(), 3);

        let events = collector.all();
        assert_eq!(events[0].message, "Event 2");
        assert_eq!(events[2].message, "Event 4");
    }

    #[test]
    fn buffered_collector_event_ids() {
        let collector = BufferedCollector::new(100);

        collector.collect(LogEvent::info(LogCategory::System, "Event 1"));
        collector.collect(LogEvent::info(LogCategory::System, "Event 2"));

        let events = collector.all();
        assert_eq!(events[0].id, 1);
        assert_eq!(events[1].id, 2);
    }

    #[test]
    fn buffered_collector_by_trace() {
        let collector = BufferedCollector::new(100);
        let trace_id = TraceId::new();

        collector.collect(LogEvent::info(LogCategory::System, "Unrelated"));
        collector
            .collect(LogEvent::info(LogCategory::Trace, "Trace event").with_trace_id(trace_id));
        collector.collect(
            LogEvent::info(LogCategory::Node, "Node event")
                .with_trace_id(trace_id)
                .with_node_id(NodeId::new(1)),
        );

        let trace_events = collector.by_trace(trace_id);
        assert_eq!(trace_events.len(), 2);
    }

    #[test]
    fn buffered_collector_by_level() {
        let collector = BufferedCollector::new(100);

        collector.collect(LogEvent::debug(LogCategory::System, "Debug"));
        collector.collect(LogEvent::info(LogCategory::System, "Info"));
        collector.collect(LogEvent::warn(LogCategory::System, "Warn"));
        collector.collect(LogEvent::error(LogCategory::System, "Error"));

        let warnings = collector.by_level(LogLevel::Warn);
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().all(|e| e.level >= LogLevel::Warn));
    }

    #[test]
    fn buffered_collector_recent() {
        let collector = BufferedCollector::new(100);

        for i in 1..=10 {
            collector.collect(LogEvent::info(LogCategory::System, format!("Event {}", i)));
        }

        let recent = collector.recent(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].message, "Event 10");
        assert_eq!(recent[2].message, "Event 8");
    }

    #[test]
    fn buffered_collector_since() {
        let collector = BufferedCollector::new(100);

        collector.collect(LogEvent::info(LogCategory::System, "Event 1"));
        collector.collect(LogEvent::info(LogCategory::System, "Event 2"));
        collector.collect(LogEvent::info(LogCategory::System, "Event 3"));

        let since = collector.since(1);
        assert_eq!(since.len(), 2);
        assert_eq!(since[0].id, 2);
    }

    #[test]
    fn log_context_auto_fields() {
        let collector = Arc::new(BufferedCollector::new(100));
        let trace_id = TraceId::new();

        let ctx = LogContext::new(collector.clone())
            .with_trace_id(trace_id)
            .with_pipeline_id("test_pipeline");

        ctx.info(LogCategory::Trace, "Trace started");

        let events = collector.all();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].trace_id, Some(trace_id));
        assert_eq!(events[0].pipeline_id, Some("test_pipeline".to_string()));
    }

    #[test]
    fn log_context_for_node() {
        let collector = Arc::new(BufferedCollector::new(100));
        let trace_id = TraceId::new();

        let ctx = LogContext::new(collector.clone()).with_trace_id(trace_id);
        let node_ctx = ctx.for_node(NodeId::new(42));

        node_ctx.info(LogCategory::Node, "Node processing");

        let events = collector.all();
        assert_eq!(events[0].trace_id, Some(trace_id));
        assert_eq!(events[0].node_id, Some(NodeId::new(42)));
    }

    #[test]
    fn subscriber_notification() {
        use std::sync::atomic::AtomicUsize;

        let collector = BufferedCollector::new(100);
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&count);

        collector.subscribe(Arc::new(move |_event| {
            count_clone.fetch_add(1, Ordering::SeqCst);
        }));

        collector.collect(LogEvent::info(LogCategory::System, "Event 1"));
        collector.collect(LogEvent::info(LogCategory::System, "Event 2"));

        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn null_collector() {
        let collector = NullCollector;

        collector.collect(LogEvent::info(LogCategory::System, "Discarded"));

        assert_eq!(collector.len(), 0);
    }
}
