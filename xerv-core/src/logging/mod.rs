//! Structured logging for trace execution.
//!
//! This module provides a comprehensive logging system for XERV with:
//!
//! - **Correlation IDs**: Every log event can be associated with trace_id, node_id, and pipeline_id
//! - **Structured Events**: Events contain typed fields for filtering and aggregation
//! - **Buffered Collection**: Thread-safe ring buffer for in-memory log storage
//! - **Flexible Filtering**: Query logs by level, category, trace, node, pipeline, time range, and message content
//! - **Real-time Subscribers**: Register callbacks for immediate event notifications
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐     ┌──────────────┐     ┌───────────────┐
//! │ LogEvent    │────>│ LogCollector │────>│ Subscribers   │
//! │ (with IDs)  │     │ (buffer)     │     │ (callbacks)   │
//! └─────────────┘     └──────────────┘     └───────────────┘
//!                            │
//!                            v
//!                     ┌──────────────┐
//!                     │ LogFilter    │
//!                     │ (query)      │
//!                     └──────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use xerv_core::logging::{LogEvent, LogCategory, LogLevel, BufferedCollector, LogContext};
//! use std::sync::Arc;
//!
//! // Create a collector
//! let collector = Arc::new(BufferedCollector::with_default_capacity());
//!
//! // Create a context for a specific trace
//! let ctx = LogContext::new(collector.clone())
//!     .with_trace_id(trace_id)
//!     .with_pipeline_id("order_pipeline");
//!
//! // Log events with automatic correlation
//! ctx.info(LogCategory::Trace, "Trace started");
//!
//! // Create a node-specific context
//! let node_ctx = ctx.for_node(NodeId::new(1));
//! node_ctx.debug(LogCategory::Node, "Processing input");
//!
//! // Query logs
//! let errors = collector.by_level(LogLevel::Error);
//! let trace_logs = collector.by_trace(trace_id);
//! ```

mod collector;
mod event;
mod filter;

pub use collector::{
    BufferedCollector, DEFAULT_BUFFER_CAPACITY, LogCollector, LogContext, MultiCollector,
    NullCollector, SubscriberId,
};
pub use event::{LogCategory, LogEvent, LogEventBuilder, LogLevel};
pub use filter::{LogFilter, LogFilterBuilder};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{NodeId, TraceId};
    use std::sync::Arc;

    #[test]
    fn integration_test_logging_workflow() {
        // Create a collector
        let collector = Arc::new(BufferedCollector::with_default_capacity());

        // Simulate a trace execution
        let trace_id = TraceId::new();
        let pipeline_id = "order_processing";

        // Create trace context
        let ctx = LogContext::new(collector.clone())
            .with_trace_id(trace_id)
            .with_pipeline_id(pipeline_id);

        // Log trace start
        ctx.info(LogCategory::Trace, "Trace started");

        // Simulate node execution
        for node_id in [1, 2, 3] {
            let node_ctx = ctx.for_node(NodeId::new(node_id));
            node_ctx.debug(LogCategory::Node, format!("Node {} started", node_id));
            node_ctx.info(LogCategory::Node, format!("Node {} completed", node_id));
        }

        // Log trace completion
        ctx.info(LogCategory::Trace, "Trace completed");

        // Verify collection
        assert_eq!(collector.len(), 8); // 1 start + 3*2 nodes + 1 complete

        // Query by trace
        let trace_logs = collector.by_trace(trace_id);
        assert_eq!(trace_logs.len(), 8);

        // Query by level
        let debug_logs = collector.query(&LogFilter::new().level(LogLevel::Debug));
        assert_eq!(debug_logs.len(), 3); // 3 node starts

        // Query by category
        let node_logs = collector.query(&LogFilter::new().category(LogCategory::Node));
        assert_eq!(node_logs.len(), 6); // 3 starts + 3 completes

        // Query with multiple filters
        let filter = LogFilter::new()
            .trace_id(trace_id)
            .min_level(LogLevel::Info)
            .category(LogCategory::Trace);
        let filtered = collector.query(&filter);
        assert_eq!(filtered.len(), 2); // start + complete
    }

    #[test]
    fn integration_test_subscriber() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let collector = BufferedCollector::with_default_capacity();
        let error_count = Arc::new(AtomicUsize::new(0));

        // Subscribe to errors
        let count = Arc::clone(&error_count);
        collector.subscribe(Arc::new(move |event| {
            if event.level >= LogLevel::Error {
                count.fetch_add(1, Ordering::SeqCst);
            }
        }));

        // Log some events
        collector.collect(LogEvent::info(LogCategory::System, "Info message"));
        collector.collect(LogEvent::warn(LogCategory::System, "Warning message"));
        collector.collect(LogEvent::error(LogCategory::System, "Error message"));
        collector.collect(LogEvent::error(LogCategory::Node, "Another error"));

        // Verify subscriber was called
        assert_eq!(error_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn integration_test_event_formatting() {
        let trace_id = TraceId::new();
        let event = LogEvent::warn(LogCategory::Node, "Node timeout")
            .with_trace_id(trace_id)
            .with_node_id(NodeId::new(42))
            .with_pipeline_id("my_pipeline")
            .with_field("timeout_ms", "5000")
            .with_field_i64("retry_count", 3);

        let line = event.format_line();

        // Verify all parts are present
        assert!(line.contains("[WARN]"));
        assert!(line.contains("[node]"));
        assert!(line.contains(&format!("trace={}", trace_id)));
        assert!(line.contains("node=42"));
        assert!(line.contains("pipeline=my_pipeline"));
        assert!(line.contains("Node timeout"));
        assert!(line.contains("timeout_ms"));
        assert!(line.contains("retry_count"));
    }

    #[test]
    fn integration_test_filter_serialization() {
        let trace_id = TraceId::new();
        let filter = LogFilter::new()
            .min_level(LogLevel::Warn)
            .trace_id(trace_id)
            .category(LogCategory::Node)
            .limit(100);

        // Serialize to JSON
        let json = serde_json::to_string(&filter).unwrap();

        // Deserialize back
        let parsed: LogFilter = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.min_level, Some(LogLevel::Warn));
        assert_eq!(parsed.trace_id, Some(trace_id));
        assert_eq!(parsed.categories, vec![LogCategory::Node]);
        assert_eq!(parsed.limit, Some(100));
    }
}
