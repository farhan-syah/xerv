//! Prometheus metrics endpoint handler.

use crate::api::state::AppState;
use crate::metrics::try_global_metrics;
use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;
use std::sync::Arc;

/// Content type for Prometheus metrics.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// GET /metrics
///
/// Returns Prometheus-formatted metrics for monitoring and auto-scaling.
/// This endpoint is typically exempt from authentication for Prometheus scraping.
pub async fn get_metrics(state: Arc<AppState>) -> Response<Full<Bytes>> {
    // Get or create metrics instance
    let metrics = match try_global_metrics() {
        Some(m) => m,
        None => {
            // If metrics aren't initialized, return empty metrics
            return Response::builder()
                .status(200)
                .header("Content-Type", PROMETHEUS_CONTENT_TYPE)
                .body(Full::new(Bytes::from_static(b"# No metrics available\n")))
                .expect("response build should not fail");
        }
    };

    // Update gauges from current state
    update_metrics_from_state(&metrics, &state);

    // Encode and return
    let output = metrics.encode();

    Response::builder()
        .status(200)
        .header("Content-Type", PROMETHEUS_CONTENT_TYPE)
        .body(Full::new(Bytes::from(output)))
        .expect("response build should not fail")
}

/// Update metrics from current application state.
fn update_metrics_from_state(metrics: &crate::metrics::Metrics, state: &AppState) {
    // Update uptime
    metrics.set_uptime(state.uptime_secs() as i64);

    // Update pipeline count
    let pipeline_ids = state.controller.list();
    metrics.set_active_pipelines(pipeline_ids.len() as i64);

    // Update trigger count
    metrics.set_registered_triggers(state.listener_pool.trigger_ids().len() as i64);

    // Count traces from history
    let trace_history = state.trace_history.read();
    let mut running = 0i64;
    let mut _completed = 0i64;
    let mut _failed = 0i64;

    for trace in trace_history.all() {
        match trace.status {
            crate::api::state::TraceStatus::Running => running += 1,
            crate::api::state::TraceStatus::Completed => _completed += 1,
            crate::api::state::TraceStatus::Failed => _failed += 1,
        }
    }

    // Set active traces (running) - use "global" as pipeline for aggregate
    // TODO: In a full implementation, we'd track per-pipeline metrics
    metrics.set_active_traces("_global", "default", running);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listener::ListenerPool;
    use crate::metrics::init_global_metrics;
    use crate::pipeline::PipelineController;
    use hyper::StatusCode;

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState::new(
            Arc::new(PipelineController::new()),
            Arc::new(ListenerPool::new()),
        ))
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_prometheus_format() {
        // Initialize global metrics for test
        let _ = init_global_metrics();

        let state = test_state();
        let response = get_metrics(state).await;

        assert_eq!(response.status(), StatusCode::OK);

        let content_type = response
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok());
        assert!(
            content_type
                .unwrap_or("")
                .starts_with("text/plain; version=0.0.4")
        );
    }

    #[tokio::test]
    async fn metrics_contains_expected_metrics() {
        let _ = init_global_metrics();

        let state = test_state();
        let response = get_metrics(state).await;

        let body = response.into_body();
        let bytes = http_body_util::BodyExt::collect(body)
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();

        // Check for expected metric names
        assert!(text.contains("xerv_uptime_seconds"));
        assert!(text.contains("xerv_active_pipelines"));
        assert!(text.contains("xerv_registered_triggers"));
    }
}
