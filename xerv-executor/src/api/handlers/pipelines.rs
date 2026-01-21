//! Pipeline management handlers.

use crate::api::error::ApiError;
use crate::api::request;
use crate::api::response;
use crate::api::state::AppState;
use crate::loader::FlowLoader;
use crate::pipeline::PipelineBuilder;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::{Request, Response};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use xerv_core::error::XervError;
use xerv_core::traits::{PipelineConfig, PipelineSettings};
use xerv_core::types::PipelineId;

/// Parse a version string into a numeric version.
///
/// Supports various formats:
/// - "1" -> 1
/// - "1.0" -> 1
/// - "1.2.3" -> 1 (major version only)
/// - "v1" -> 1
/// - "v1.0" -> 1
/// - "1.0.0-beta" -> 1
///
/// Returns 1 if parsing fails.
fn parse_version_number(version: &str) -> u32 {
    let version = version.trim();

    // Strip leading 'v' or 'V'
    let version = version
        .strip_prefix('v')
        .or_else(|| version.strip_prefix('V'))
        .unwrap_or(version);

    // Try to parse as simple integer first
    if let Ok(v) = version.parse::<u32>() {
        return v;
    }

    // Handle semver-like versions (take major version)
    if let Some(major) = version.split('.').next() {
        // Handle pre-release suffixes like "1-beta"
        let major = major.split('-').next().unwrap_or(major);
        if let Ok(v) = major.parse::<u32>() {
            return v;
        }
    }

    // Default to version 1 if parsing fails
    tracing::warn!(version = %version, "Failed to parse version, defaulting to 1");
    1
}

/// GET /api/v1/pipelines
///
/// List all deployed pipelines.
pub async fn list(state: Arc<AppState>) -> Response<Full<Bytes>> {
    let pipeline_ids = state.controller.list();

    let mut pipelines = Vec::with_capacity(pipeline_ids.len());
    for id in &pipeline_ids {
        if let Some(pipeline) = state.controller.get(id) {
            let metrics = pipeline.metrics();
            pipelines.push(serde_json::json!({
                "id": id,
                "state": format!("{:?}", pipeline.state().await).to_lowercase(),
                "metrics": {
                    "traces_started": metrics.traces_started.load(Ordering::Relaxed),
                    "traces_completed": metrics.traces_completed.load(Ordering::Relaxed),
                    "traces_failed": metrics.traces_failed.load(Ordering::Relaxed),
                    "traces_active": metrics.traces_active.load(Ordering::Relaxed)
                }
            }));
        }
    }

    let body = serde_json::json!({
        "pipelines": pipelines,
        "count": pipelines.len()
    });

    response::ok(&body)
}

/// POST /api/v1/pipelines
///
/// Deploy a new pipeline from YAML.
pub async fn create(req: Request<Incoming>, state: Arc<AppState>) -> Response<Full<Bytes>> {
    // Read YAML body
    let yaml = match request::read_body_string(req).await {
        Ok(y) => y,
        Err(e) => return e.into_response(),
    };

    // Load and validate the flow
    let loaded = match FlowLoader::from_yaml(&yaml) {
        Ok(f) => f,
        Err(e) => {
            return ApiError::bad_request("E801", e.to_string()).into_response();
        }
    };

    // Create pipeline ID with properly parsed version
    let name = loaded.name().to_string();
    let version_str = loaded.version().to_string();

    // Parse version: supports "1", "1.0", "v1", "v1.0", etc.
    let version_num = parse_version_number(&version_str);
    let pipeline_id = PipelineId::new(&name, version_num);

    // Check if already exists
    let key = pipeline_id.to_string();
    if state.controller.get(&key).is_some() {
        return ApiError::from(XervError::PipelineExists { pipeline_id: key }).into_response();
    }

    // Build pipeline config
    let config = PipelineConfig::new(&key);

    // Convert FlowSettings to PipelineSettings
    let flow_settings = &loaded.settings;
    let settings = PipelineSettings {
        max_concurrent_executions: flow_settings.max_concurrent_executions,
        execution_timeout: Duration::from_millis(flow_settings.execution_timeout_ms),
        circuit_breaker_threshold: flow_settings.circuit_breaker_threshold,
        circuit_breaker_window: Duration::from_millis(flow_settings.circuit_breaker_window_ms),
        max_concurrent_versions: flow_settings.max_concurrent_versions,
        drain_timeout: Duration::from_millis(flow_settings.drain_timeout_ms),
        drain_grace_period: Duration::from_millis(flow_settings.drain_grace_period_ms),
    };

    // Build the pipeline
    let pipeline = match PipelineBuilder::new(pipeline_id.clone(), config)
        .with_settings(settings)
        .with_graph(loaded.graph)
        .build()
    {
        Ok(p) => p,
        Err(e) => {
            return ApiError::from(e).into_response();
        }
    };

    // Deploy it
    if let Err(e) = state.controller.deploy(pipeline) {
        return ApiError::from(e).into_response();
    }

    tracing::info!(
        pipeline_id = %pipeline_id,
        "Pipeline deployed"
    );

    let body = serde_json::json!({
        "pipeline_id": pipeline_id.to_string(),
        "name": name,
        "version": version_str,
        "status": "deployed"
    });

    response::created(&body)
}

/// GET /api/v1/pipelines/{id}
///
/// Get pipeline details.
pub async fn get(state: Arc<AppState>, pipeline_id: &str) -> Response<Full<Bytes>> {
    let pipeline = match state.controller.get(pipeline_id) {
        Some(p) => p,
        None => {
            return ApiError::from(XervError::PipelineNotFound {
                pipeline_id: pipeline_id.to_string(),
            })
            .into_response();
        }
    };

    let metrics = pipeline.metrics();
    let pipeline_state = pipeline.state().await;

    let body = serde_json::json!({
        "pipeline_id": pipeline_id,
        "state": format!("{:?}", pipeline_state).to_lowercase(),
        "metrics": {
            "traces_started": metrics.traces_started.load(Ordering::Relaxed),
            "traces_completed": metrics.traces_completed.load(Ordering::Relaxed),
            "traces_failed": metrics.traces_failed.load(Ordering::Relaxed),
            "traces_active": metrics.traces_active.load(Ordering::Relaxed),
            "error_rate": metrics.error_rate()
        }
    });

    response::ok(&body)
}

/// DELETE /api/v1/pipelines/{id}
///
/// Remove a pipeline.
pub async fn delete(state: Arc<AppState>, pipeline_id: &str) -> Response<Full<Bytes>> {
    if state.controller.get(pipeline_id).is_none() {
        return ApiError::from(XervError::PipelineNotFound {
            pipeline_id: pipeline_id.to_string(),
        })
        .into_response();
    }

    if let Err(e) = state.controller.remove(pipeline_id).await {
        return ApiError::from(e).into_response();
    }

    tracing::info!(
        pipeline_id = %pipeline_id,
        "Pipeline removed"
    );

    response::no_content()
}

/// POST /api/v1/pipelines/{id}/start
///
/// Start a pipeline.
pub async fn start(state: Arc<AppState>, pipeline_id: &str) -> Response<Full<Bytes>> {
    let pipeline = match state.controller.get(pipeline_id) {
        Some(p) => p,
        None => {
            return ApiError::from(XervError::PipelineNotFound {
                pipeline_id: pipeline_id.to_string(),
            })
            .into_response();
        }
    };

    if let Err(e) = pipeline.start().await {
        return ApiError::from(e).into_response();
    }

    tracing::info!(
        pipeline_id = %pipeline_id,
        "Pipeline started"
    );

    let body = serde_json::json!({
        "pipeline_id": pipeline_id,
        "status": "started"
    });

    response::ok(&body)
}

/// POST /api/v1/pipelines/{id}/pause
///
/// Pause a pipeline.
pub async fn pause(state: Arc<AppState>, pipeline_id: &str) -> Response<Full<Bytes>> {
    let pipeline = match state.controller.get(pipeline_id) {
        Some(p) => p,
        None => {
            return ApiError::from(XervError::PipelineNotFound {
                pipeline_id: pipeline_id.to_string(),
            })
            .into_response();
        }
    };

    if let Err(e) = pipeline.pause().await {
        return ApiError::from(e).into_response();
    }

    tracing::info!(
        pipeline_id = %pipeline_id,
        "Pipeline paused"
    );

    let body = serde_json::json!({
        "pipeline_id": pipeline_id,
        "status": "paused"
    });

    response::ok(&body)
}

/// POST /api/v1/pipelines/{id}/resume
///
/// Resume a paused pipeline.
pub async fn resume(state: Arc<AppState>, pipeline_id: &str) -> Response<Full<Bytes>> {
    let pipeline = match state.controller.get(pipeline_id) {
        Some(p) => p,
        None => {
            return ApiError::from(XervError::PipelineNotFound {
                pipeline_id: pipeline_id.to_string(),
            })
            .into_response();
        }
    };

    if let Err(e) = pipeline.resume().await {
        return ApiError::from(e).into_response();
    }

    tracing::info!(
        pipeline_id = %pipeline_id,
        "Pipeline resumed"
    );

    let body = serde_json::json!({
        "pipeline_id": pipeline_id,
        "status": "running"
    });

    response::ok(&body)
}

/// POST /api/v1/pipelines/{id}/drain
///
/// Drain a pipeline (stop new events, wait for in-flight).
pub async fn drain(state: Arc<AppState>, pipeline_id: &str) -> Response<Full<Bytes>> {
    let pipeline = match state.controller.get(pipeline_id) {
        Some(p) => p,
        None => {
            return ApiError::from(XervError::PipelineNotFound {
                pipeline_id: pipeline_id.to_string(),
            })
            .into_response();
        }
    };

    if let Err(e) = pipeline.drain().await {
        return ApiError::from(e).into_response();
    }

    tracing::info!(
        pipeline_id = %pipeline_id,
        "Pipeline drained"
    );

    let body = serde_json::json!({
        "pipeline_id": pipeline_id,
        "status": "drained"
    });

    response::ok(&body)
}

/// POST /api/v1/pipelines/{id}/stop
///
/// Stop a pipeline immediately.
pub async fn stop(state: Arc<AppState>, pipeline_id: &str) -> Response<Full<Bytes>> {
    let pipeline = match state.controller.get(pipeline_id) {
        Some(p) => p,
        None => {
            return ApiError::from(XervError::PipelineNotFound {
                pipeline_id: pipeline_id.to_string(),
            })
            .into_response();
        }
    };

    if let Err(e) = pipeline.stop().await {
        return ApiError::from(e).into_response();
    }

    tracing::info!(
        pipeline_id = %pipeline_id,
        "Pipeline stopped"
    );

    let body = serde_json::json!({
        "pipeline_id": pipeline_id,
        "status": "stopped"
    });

    response::ok(&body)
}

/// GET /api/v1/pipelines/{id}/metrics
///
/// Get detailed metrics for a pipeline.
pub async fn metrics(state: Arc<AppState>, pipeline_id: &str) -> Response<Full<Bytes>> {
    let pipeline = match state.controller.get(pipeline_id) {
        Some(p) => p,
        None => {
            return ApiError::from(XervError::PipelineNotFound {
                pipeline_id: pipeline_id.to_string(),
            })
            .into_response();
        }
    };

    let m = pipeline.metrics();

    let body = serde_json::json!({
        "pipeline_id": pipeline_id,
        "traces_started": m.traces_started.load(Ordering::Relaxed),
        "traces_completed": m.traces_completed.load(Ordering::Relaxed),
        "traces_failed": m.traces_failed.load(Ordering::Relaxed),
        "traces_active": m.traces_active.load(Ordering::Relaxed),
        "node_executions": m.node_executions.load(Ordering::Relaxed),
        "total_execution_us": m.total_execution_us.load(Ordering::Relaxed),
        "error_rate": m.error_rate()
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
    async fn list_empty_pipelines() {
        let state = test_state();
        let response = list(state).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_nonexistent_pipeline() {
        let state = test_state();
        let response = get(state, "nonexistent@v1").await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
