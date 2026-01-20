//! Log streaming and query endpoint handlers.
//!
//! Provides endpoints for querying and streaming log events from trace executions.

use crate::api::error::ApiError;
use crate::api::response;
use crate::api::state::AppState;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::{Request, Response};
use std::sync::Arc;
use xerv_core::logging::{LogCategory, LogCollector, LogFilter, LogLevel};
use xerv_core::types::{NodeId, TraceId};

/// GET /api/v1/logs
///
/// Query logs with optional filters via query parameters:
/// - `level`: Minimum log level (trace, debug, info, warn, error)
/// - `category`: Filter by category (trace, node, trigger, pipeline, schema, system, custom)
/// - `trace_id`: Filter by trace ID
/// - `node_id`: Filter by node ID
/// - `pipeline_id`: Filter by pipeline ID
/// - `contains`: Filter by message content (case-insensitive)
/// - `limit`: Maximum number of events to return (default: 100)
/// - `offset`: Pagination offset
pub async fn query(req: Request<Incoming>, state: Arc<AppState>) -> Response<Full<Bytes>> {
    // Parse query parameters
    let query_string = req.uri().query().unwrap_or("");
    let filter = parse_log_filter(query_string);

    // Apply a reasonable default limit if none specified
    let limit = filter.limit.unwrap_or(100);
    let effective_filter = if filter.limit.is_none() {
        LogFilter {
            limit: Some(limit),
            ..filter
        }
    } else {
        filter
    };

    // Query the collector
    let events = state.log_collector.query(&effective_filter);

    // Format events for JSON response
    let log_entries: Vec<serde_json::Value> = events
        .iter()
        .map(|event| {
            let mut entry = serde_json::json!({
                "id": event.id,
                "timestamp": event.timestamp_iso(),
                "level": event.level.as_str(),
                "category": event.category.as_str(),
                "message": event.message
            });

            // Add optional correlation IDs
            if let Some(trace_id) = event.trace_id {
                entry["trace_id"] = serde_json::json!(trace_id.as_uuid().to_string());
            }
            if let Some(node_id) = event.node_id {
                entry["node_id"] = serde_json::json!(node_id.as_u32());
            }
            if let Some(ref pipeline_id) = event.pipeline_id {
                entry["pipeline_id"] = serde_json::json!(pipeline_id);
            }

            // Add structured fields if present
            if !event.fields.is_empty() {
                entry["fields"] = serde_json::json!(event.fields);
            }

            entry
        })
        .collect();

    let body = serde_json::json!({
        "logs": log_entries,
        "count": log_entries.len(),
        "filter": effective_filter.describe()
    });

    response::ok(&body)
}

/// GET /api/v1/logs/recent
///
/// Get the most recent log events.
/// Query parameters:
/// - `limit`: Number of events to return (default: 50, max: 500)
pub async fn recent(req: Request<Incoming>, state: Arc<AppState>) -> Response<Full<Bytes>> {
    let query_string = req.uri().query().unwrap_or("");
    let limit = parse_limit(query_string).unwrap_or(50).min(500);

    let events = state.log_collector.recent(limit);

    let log_entries: Vec<serde_json::Value> = events.iter().map(format_log_entry).collect();

    let body = serde_json::json!({
        "logs": log_entries,
        "count": log_entries.len()
    });

    response::ok(&body)
}

/// GET /api/v1/logs/by-trace/{trace_id}
///
/// Get all logs for a specific trace.
pub async fn by_trace(state: Arc<AppState>, trace_id_str: &str) -> Response<Full<Bytes>> {
    // Parse the trace ID
    let trace_id = match uuid::Uuid::parse_str(trace_id_str) {
        Ok(uuid) => TraceId::from_uuid(uuid),
        Err(_) => {
            return ApiError::bad_request("E100", format!("Invalid trace ID: {}", trace_id_str))
                .into_response();
        }
    };

    let events = state.log_collector.by_trace(trace_id);

    let log_entries: Vec<serde_json::Value> = events.iter().map(format_log_entry).collect();

    let body = serde_json::json!({
        "trace_id": trace_id_str,
        "logs": log_entries,
        "count": log_entries.len()
    });

    response::ok(&body)
}

/// GET /api/v1/logs/by-level/{level}
///
/// Get logs at or above a specific level.
pub async fn by_level(
    req: Request<Incoming>,
    state: Arc<AppState>,
    level_str: &str,
) -> Response<Full<Bytes>> {
    // Parse the level
    let level = match LogLevel::parse(level_str) {
        Some(l) => l,
        None => {
            return ApiError::bad_request(
                "E101",
                format!(
                    "Invalid log level: '{}'. Valid levels: trace, debug, info, warn, error",
                    level_str
                ),
            )
            .into_response();
        }
    };

    let query_string = req.uri().query().unwrap_or("");
    let limit = parse_limit(query_string).unwrap_or(100);

    let filter = LogFilter::new().min_level(level).limit(limit);
    let events = state.log_collector.query(&filter);

    let log_entries: Vec<serde_json::Value> = events.iter().map(format_log_entry).collect();

    let body = serde_json::json!({
        "level": level_str,
        "logs": log_entries,
        "count": log_entries.len()
    });

    response::ok(&body)
}

/// GET /api/v1/logs/stats
///
/// Get log statistics.
pub async fn stats(state: Arc<AppState>) -> Response<Full<Bytes>> {
    let collector = &state.log_collector;

    // Count by level
    let trace_count = collector
        .query(&LogFilter::new().level(LogLevel::Trace))
        .len();
    let debug_count = collector
        .query(&LogFilter::new().level(LogLevel::Debug))
        .len();
    let info_count = collector
        .query(&LogFilter::new().level(LogLevel::Info))
        .len();
    let warn_count = collector
        .query(&LogFilter::new().level(LogLevel::Warn))
        .len();
    let error_count = collector
        .query(&LogFilter::new().level(LogLevel::Error))
        .len();

    let body = serde_json::json!({
        "total": collector.len(),
        "capacity": collector.capacity(),
        "by_level": {
            "trace": trace_count,
            "debug": debug_count,
            "info": info_count,
            "warn": warn_count,
            "error": error_count
        }
    });

    response::ok(&body)
}

/// DELETE /api/v1/logs
///
/// Clear all logs.
pub async fn clear(state: Arc<AppState>) -> Response<Full<Bytes>> {
    state.log_collector.clear();

    let body = serde_json::json!({
        "status": "cleared",
        "message": "All logs have been cleared"
    });

    response::ok(&body)
}

// Helper functions

/// Parse a LogFilter from query parameters.
fn parse_log_filter(query_string: &str) -> LogFilter {
    let mut filter = LogFilter::new();

    for pair in query_string.split('&') {
        if pair.is_empty() {
            continue;
        }

        let (key, value) = match pair.split_once('=') {
            Some((k, v)) => (k, urlencoding::decode(v).unwrap_or_default()),
            None => continue,
        };

        match key {
            "level" => {
                if let Some(level) = LogLevel::parse(&value) {
                    filter.min_level = Some(level);
                }
            }
            "category" => {
                if let Some(category) = parse_category(&value) {
                    filter.categories.push(category);
                }
            }
            "trace_id" => {
                if let Ok(uuid) = uuid::Uuid::parse_str(&value) {
                    filter.trace_id = Some(TraceId::from_uuid(uuid));
                }
            }
            "node_id" => {
                if let Ok(id) = value.parse::<u32>() {
                    filter.node_id = Some(NodeId::new(id));
                }
            }
            "pipeline_id" => {
                filter.pipeline_id = Some(value.into_owned());
            }
            "contains" => {
                filter.message_contains = Some(value.into_owned());
            }
            "limit" => {
                if let Ok(limit) = value.parse::<usize>() {
                    filter.limit = Some(limit.min(1000)); // Cap at 1000
                }
            }
            "offset" => {
                if let Ok(offset) = value.parse::<usize>() {
                    filter.offset = Some(offset);
                }
            }
            _ => {}
        }
    }

    filter
}

/// Parse a LogCategory from a string.
fn parse_category(s: &str) -> Option<LogCategory> {
    match s.to_lowercase().as_str() {
        "trace" => Some(LogCategory::Trace),
        "node" => Some(LogCategory::Node),
        "trigger" => Some(LogCategory::Trigger),
        "pipeline" => Some(LogCategory::Pipeline),
        "schema" => Some(LogCategory::Schema),
        "system" => Some(LogCategory::System),
        "custom" => Some(LogCategory::Custom),
        _ => None,
    }
}

/// Parse limit parameter from query string.
fn parse_limit(query_string: &str) -> Option<usize> {
    for pair in query_string.split('&') {
        if let Some(("limit", value)) = pair.split_once('=') {
            return value.parse().ok();
        }
    }
    None
}

/// Format a log event for JSON output.
fn format_log_entry(event: &xerv_core::logging::LogEvent) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "id": event.id,
        "timestamp": event.timestamp_iso(),
        "level": event.level.as_str(),
        "category": event.category.as_str(),
        "message": event.message
    });

    if let Some(trace_id) = event.trace_id {
        entry["trace_id"] = serde_json::json!(trace_id.as_uuid().to_string());
    }
    if let Some(node_id) = event.node_id {
        entry["node_id"] = serde_json::json!(node_id.as_u32());
    }
    if let Some(ref pipeline_id) = event.pipeline_id {
        entry["pipeline_id"] = serde_json::json!(pipeline_id);
    }
    if !event.fields.is_empty() {
        entry["fields"] = serde_json::json!(event.fields);
    }

    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listener::ListenerPool;
    use crate::pipeline::PipelineController;
    use hyper::StatusCode;
    use xerv_core::logging::{LogCollector, LogEvent};

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState::new(
            Arc::new(PipelineController::new()),
            Arc::new(ListenerPool::new()),
        ))
    }

    #[tokio::test]
    async fn query_logs_filter_parsing() {
        // Test the filter parsing functionality
        let filter = parse_log_filter("");
        assert!(filter.min_level.is_none());
        assert!(filter.trace_id.is_none());
    }

    #[tokio::test]
    async fn stats_empty() {
        let state = test_state();
        let response = stats(state).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn clear_logs() {
        let state = test_state();

        // Add some logs
        state
            .log_collector
            .collect(LogEvent::info(LogCategory::System, "Test log"));
        assert_eq!(state.log_collector.len(), 1);

        // Clear
        let response = clear(state.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.log_collector.len(), 0);
    }

    #[test]
    fn parse_log_filter_basic() {
        let filter = parse_log_filter("level=warn&limit=50");
        assert_eq!(filter.min_level, Some(LogLevel::Warn));
        assert_eq!(filter.limit, Some(50));
    }

    #[test]
    fn parse_log_filter_with_trace_id() {
        let filter = parse_log_filter("trace_id=550e8400-e29b-41d4-a716-446655440000");
        assert!(filter.trace_id.is_some());
    }

    #[test]
    fn parse_log_filter_with_category() {
        let filter = parse_log_filter("category=node");
        assert_eq!(filter.categories, vec![LogCategory::Node]);
    }

    #[test]
    fn parse_log_filter_with_message_contains() {
        let filter = parse_log_filter("contains=error");
        assert_eq!(filter.message_contains, Some("error".to_string()));
    }

    #[test]
    fn parse_log_filter_limit_capped() {
        let filter = parse_log_filter("limit=10000");
        assert_eq!(filter.limit, Some(1000)); // Capped at 1000
    }

    #[test]
    fn parse_category_valid() {
        assert_eq!(parse_category("trace"), Some(LogCategory::Trace));
        assert_eq!(parse_category("NODE"), Some(LogCategory::Node));
        assert_eq!(parse_category("Pipeline"), Some(LogCategory::Pipeline));
        assert_eq!(parse_category("invalid"), None);
    }

    #[test]
    fn format_log_entry_basic() {
        let event = LogEvent::info(LogCategory::System, "Test message");
        let entry = format_log_entry(&event);

        assert_eq!(entry["level"], "info");
        assert_eq!(entry["category"], "system");
        assert_eq!(entry["message"], "Test message");
    }

    #[test]
    fn format_log_entry_with_ids() {
        let trace_id = TraceId::new();
        let event = LogEvent::warn(LogCategory::Node, "Node warning")
            .with_trace_id(trace_id)
            .with_node_id(NodeId::new(42))
            .with_pipeline_id("test_pipeline");

        let entry = format_log_entry(&event);

        assert_eq!(entry["level"], "warn");
        assert!(entry.get("trace_id").is_some());
        assert_eq!(entry["node_id"], 42);
        assert_eq!(entry["pipeline_id"], "test_pipeline");
    }
}
