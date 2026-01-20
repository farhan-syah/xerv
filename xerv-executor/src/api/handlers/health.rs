//! Health and status endpoint handlers.

use crate::api::response;
use crate::api::state::AppState;
use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;
use std::sync::Arc;

/// GET /api/v1/health
///
/// Simple health check that returns 200 OK if the server is running.
pub async fn get_health(_state: Arc<AppState>) -> Response<Full<Bytes>> {
    let body = serde_json::json!({
        "status": "healthy",
        "service": "xerv"
    });

    response::ok(&body)
}

/// GET /api/v1/status
///
/// Returns system status including uptime and pipeline counts.
pub async fn get_status(state: Arc<AppState>) -> Response<Full<Bytes>> {
    let pipeline_ids = state.controller.list();
    let trace_history = state.trace_history.read();

    // Count traces by status
    let mut running = 0u64;
    let mut completed = 0u64;
    let mut failed = 0u64;

    for trace in trace_history.all() {
        match trace.status {
            crate::api::state::TraceStatus::Running => running += 1,
            crate::api::state::TraceStatus::Completed => completed += 1,
            crate::api::state::TraceStatus::Failed => failed += 1,
        }
    }

    let body = serde_json::json!({
        "status": "running",
        "service": "xerv",
        "uptime_seconds": state.uptime_secs(),
        "pipelines": {
            "total": pipeline_ids.len(),
            "ids": pipeline_ids
        },
        "traces": {
            "history_size": trace_history.len(),
            "running": running,
            "completed": completed,
            "failed": failed
        },
        "triggers": {
            "registered": state.listener_pool.trigger_ids().len(),
            "pool_running": state.listener_pool.is_running()
        }
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
    async fn health_check_returns_ok() {
        let state = test_state();
        let response = get_health(state).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn status_returns_system_info() {
        let state = test_state();
        let response = get_status(state).await;

        assert_eq!(response.status(), StatusCode::OK);
    }
}
