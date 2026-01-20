//! Trace inspection endpoint handlers.

use crate::api::error::ApiError;
use crate::api::response;
use crate::api::state::AppState;
use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;
use std::sync::Arc;
use xerv_core::types::TraceId;

/// GET /api/v1/traces
///
/// List recent traces.
pub async fn list(state: Arc<AppState>) -> Response<Full<Bytes>> {
    let history = state.trace_history.read();

    let traces: Vec<_> = history
        .all()
        .take(100) // Limit to 100 most recent
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
        let response = list(state).await;

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
