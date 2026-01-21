//! Trace inspection endpoint handlers.

use crate::api::error::ApiError;
use crate::api::handlers::logs;
use crate::api::request;
use crate::api::response;
use crate::api::state::AppState;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use serde::Deserialize;
use std::sync::Arc;
use xerv_core::types::TraceId;

#[derive(Debug, Deserialize)]
struct TraceCreateRequest {
    pipeline_id: String,
    #[serde(default)]
    trigger_data: Option<serde_json::Value>,
}

/// GET /api/v1/traces
///
/// List recent traces.
pub async fn list(query: Option<&str>, state: Arc<AppState>) -> Response<Full<Bytes>> {
    let history = state.trace_history.read();
    let query = query.unwrap_or("");
    let mut pipeline_filter: Option<String> = None;
    let mut status_filter: Option<String> = None;
    let mut limit = 100usize;

    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");
        match key {
            "pipeline_id" => pipeline_filter = Some(value.to_string()),
            "status" => status_filter = Some(value.to_string()),
            "limit" => {
                if let Ok(parsed) = value.parse::<usize>() {
                    limit = parsed;
                }
            }
            _ => {}
        }
    }

    let traces: Vec<_> = history
        .all()
        .filter(|record| {
            pipeline_filter
                .as_deref()
                .is_none_or(|p| record.pipeline_id == p)
        })
        .filter(|record| {
            status_filter
                .as_deref()
                .is_none_or(|s| record.status.as_str() == s)
        })
        .take(limit)
        .map(|record| {
            serde_json::json!({
                "trace_id": record.trace_id.as_uuid().to_string(),
                "pipeline_id": record.pipeline_id,
                "status": record.status.as_str(),
                "elapsed_ms": record.elapsed_ms(),
                "error": record.error
            })
        })
        .collect();

    let body = serde_json::json!({
        "traces": traces,
        "count": traces.len()
    });

    response::ok(&body)
}

/// POST /api/v1/traces
///
/// Start a new trace execution.
pub async fn create(
    req: hyper::Request<hyper::body::Incoming>,
    state: Arc<AppState>,
) -> Response<Full<Bytes>> {
    let payload: TraceCreateRequest = match request::read_body_json(req).await {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    let trace_id = TraceId::new();
    let mut history = state.trace_history.write();
    history.push(crate::api::state::TraceRecord::new(
        trace_id,
        payload.pipeline_id.clone(),
    ));

    let body = serde_json::json!({
        "trace_id": trace_id.as_uuid().to_string(),
        "status": "running",
        "started_at": chrono::Utc::now().to_rfc3339(),
        "trigger_data": payload.trigger_data
    });

    response::created(&body)
}

/// GET /api/v1/traces/{trace_id}
///
/// Get details for a specific trace.
pub async fn get(state: Arc<AppState>, trace_id_str: &str) -> Response<Full<Bytes>> {
    // Parse the trace ID
    let trace_id = match uuid::Uuid::parse_str(trace_id_str) {
        Ok(uuid) => TraceId::from_uuid(uuid),
        Err(_) => {
            return ApiError::bad_request("E000", format!("Invalid trace ID: {}", trace_id_str))
                .into_response();
        }
    };

    let history = state.trace_history.read();

    match history.get(trace_id) {
        Some(record) => {
            let body = serde_json::json!({
                "trace_id": record.trace_id.as_uuid().to_string(),
                "pipeline_id": record.pipeline_id,
                "status": record.status.as_str(),
                "elapsed_ms": record.elapsed_ms(),
                "error": record.error
            });

            response::ok(&body)
        }
        None => ApiError::not_found("E000", format!("Trace '{}' not found", trace_id_str))
            .into_response(),
    }
}

/// GET /api/v1/traces/{trace_id}/stream
///
/// Stream trace events (SSE).
pub async fn stream(state: Arc<AppState>, trace_id_str: &str) -> Response<Full<Bytes>> {
    let trace_id = match uuid::Uuid::parse_str(trace_id_str) {
        Ok(uuid) => TraceId::from_uuid(uuid),
        Err(_) => {
            return ApiError::bad_request("E000", format!("Invalid trace ID: {}", trace_id_str))
                .into_response();
        }
    };

    let history = state.trace_history.read();
    if history.get(trace_id).is_none() {
        return ApiError::not_found("E000", format!("Trace '{}' not found", trace_id_str))
            .into_response();
    }

    let now = chrono::Utc::now().to_rfc3339();
    let event = serde_json::json!({
        "trace_id": trace_id.as_uuid().to_string(),
        "timestamp": now
    });
    let body = format!("event: trace_completed\\ndata: {}\\n\\n", event);

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Full::new(Bytes::from(body)))
        .expect("response builder should not fail")
}

/// POST /api/v1/traces/{trace_id}/suspend
pub async fn suspend(state: Arc<AppState>, trace_id_str: &str) -> Response<Full<Bytes>> {
    match update_trace_status(
        state,
        trace_id_str,
        crate::api::state::TraceStatus::Suspended,
    ) {
        Ok(body) => response::ok(&body),
        Err(resp) => *resp,
    }
}

/// POST /api/v1/traces/{trace_id}/resume
pub async fn resume(state: Arc<AppState>, trace_id_str: &str) -> Response<Full<Bytes>> {
    match update_trace_status(state, trace_id_str, crate::api::state::TraceStatus::Running) {
        Ok(body) => response::ok(&body),
        Err(resp) => *resp,
    }
}

/// POST /api/v1/traces/{trace_id}/cancel
pub async fn cancel(state: Arc<AppState>, trace_id_str: &str) -> Response<Full<Bytes>> {
    match update_trace_status(
        state,
        trace_id_str,
        crate::api::state::TraceStatus::Canceled,
    ) {
        Ok(body) => response::ok(&body),
        Err(resp) => *resp,
    }
}

/// GET /api/v1/traces/{trace_id}/logs
pub async fn logs_by_trace(state: Arc<AppState>, trace_id_str: &str) -> Response<Full<Bytes>> {
    logs::by_trace(state, trace_id_str).await
}

/// GET /api/v1/pipelines/{id}/traces
///
/// List traces for a specific pipeline.
pub async fn list_by_pipeline(state: Arc<AppState>, pipeline_id: &str) -> Response<Full<Bytes>> {
    let history = state.trace_history.read();

    let traces: Vec<_> = history
        .by_pipeline(pipeline_id)
        .iter()
        .take(100)
        .map(|record| {
            serde_json::json!({
                "trace_id": record.trace_id.as_uuid().to_string(),
                "status": record.status.as_str(),
                "elapsed_ms": record.elapsed_ms(),
                "error": record.error
            })
        })
        .collect();

    let body = serde_json::json!({
        "pipeline_id": pipeline_id,
        "traces": traces,
        "count": traces.len()
    });

    response::ok(&body)
}

fn update_trace_status(
    state: Arc<AppState>,
    trace_id_str: &str,
    status: crate::api::state::TraceStatus,
) -> Result<serde_json::Value, Box<Response<Full<Bytes>>>> {
    let trace_id = match uuid::Uuid::parse_str(trace_id_str) {
        Ok(uuid) => TraceId::from_uuid(uuid),
        Err(_) => {
            return Err(Box::new(
                ApiError::bad_request("E000", format!("Invalid trace ID: {}", trace_id_str))
                    .into_response(),
            ));
        }
    };

    let mut history = state.trace_history.write();
    let record = match history.get_mut(trace_id) {
        Some(record) => record,
        None => {
            return Err(Box::new(
                ApiError::not_found("E000", format!("Trace '{}' not found", trace_id_str))
                    .into_response(),
            ));
        }
    };

    record.status = status;

    Ok(serde_json::json!({
        "trace_id": record.trace_id.as_uuid().to_string(),
        "status": record.status.as_str()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listener::ListenerPool;
    use crate::pipeline::PipelineController;
    use hyper::StatusCode;

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState::new(
            Arc::new(PipelineController::new()),
            Arc::new(ListenerPool::new()),
        ))
    }

    #[tokio::test]
    async fn list_traces_empty() {
        let state = test_state();
        let response = list(None, state).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_nonexistent_trace() {
        let state = test_state();
        let response = get(state, "550e8400-e29b-41d4-a716-446655440000").await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_invalid_trace_id() {
        let state = test_state();
        let response = get(state, "not-a-uuid").await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_by_pipeline_empty() {
        let state = test_state();
        let response = list_by_pipeline(state, "test@v1").await;

        assert_eq!(response.status(), StatusCode::OK);
    }
}
