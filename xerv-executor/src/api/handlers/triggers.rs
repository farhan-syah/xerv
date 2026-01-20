//! Trigger control endpoint handlers.

use crate::api::error::ApiError;
use crate::api::response;
use crate::api::state::{AppState, TraceRecord};
use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;
use std::sync::Arc;
use xerv_core::error::XervError;
use xerv_core::traits::TriggerEvent;
use xerv_core::types::{RelPtr, TraceId};

/// GET /api/v1/pipelines/{id}/triggers
///
/// List all triggers for a pipeline.
pub async fn list(state: Arc<AppState>, pipeline_id: &str) -> Response<Full<Bytes>> {
    // Verify pipeline exists
    if state.controller.get(pipeline_id).is_none() {
        return ApiError::from(XervError::PipelineNotFound {
            pipeline_id: pipeline_id.to_string(),
        })
        .into_response();
    }

    // Get triggers from the listener pool
    let trigger_ids = state.listener_pool.trigger_ids();

    let mut triggers = Vec::new();
    for trigger_id in &trigger_ids {
        if let Some(trigger) = state.listener_pool.get_trigger(trigger_id) {
            triggers.push(serde_json::json!({
                "id": trigger_id,
                "type": format!("{:?}", trigger.trigger_type()).to_lowercase(),
                "running": trigger.is_running(),
                "listeners": state.listener_pool.listener_count(trigger_id)
            }));
        }
    }

    let body = serde_json::json!({
        "pipeline_id": pipeline_id,
        "triggers": triggers,
        "count": triggers.len()
    });

    response::ok(&body)
}

/// POST /api/v1/pipelines/{id}/triggers/{tid}/fire
///
/// Manually fire a trigger to create a new trace.
pub async fn fire(
    state: Arc<AppState>,
    pipeline_id: &str,
    trigger_id: &str,
) -> Response<Full<Bytes>> {
    // Verify pipeline exists
    let pipeline = match state.controller.get(pipeline_id) {
        Some(p) => p,
        None => {
            return ApiError::from(XervError::PipelineNotFound {
                pipeline_id: pipeline_id.to_string(),
            })
            .into_response();
        }
    };

    // Verify trigger exists
    if state.listener_pool.get_trigger(trigger_id).is_none() {
        return ApiError::not_found("E305", format!("Trigger '{}' not found", trigger_id))
            .into_response();
    }

    // Create a trigger event
    let trace_id = TraceId::new();
    let event = TriggerEvent {
        trigger_id: trigger_id.to_string(),
        trace_id,
        data: RelPtr::null(), // No initial data for manual trigger
        schema_hash: 0,
        timestamp_ns: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64,
        metadata: Default::default(),
    };

    // Record in trace history
    {
        let mut history = state.trace_history.write();
        history.push(TraceRecord::new(trace_id, pipeline_id.to_string()));
    }

    // Fire the event through the pipeline
    match pipeline.handle_event(event).await {
        Ok(returned_trace_id) => {
            tracing::info!(
                pipeline_id = %pipeline_id,
                trigger_id = %trigger_id,
                trace_id = %returned_trace_id,
                "Trigger fired"
            );

            let body = serde_json::json!({
                "trace_id": returned_trace_id.as_uuid().to_string(),
                "status": "triggered"
            });

            response::ok(&body)
        }
        Err(e) => {
            // Update trace as failed
            {
                let mut history = state.trace_history.write();
                if let Some(record) = history.get_mut(trace_id) {
                    record.fail(e.to_string());
                }
            }

            ApiError::from(e).into_response()
        }
    }
}

/// POST /api/v1/pipelines/{id}/triggers/{tid}/pause
///
/// Pause a trigger.
pub async fn pause(
    state: Arc<AppState>,
    pipeline_id: &str,
    trigger_id: &str,
) -> Response<Full<Bytes>> {
    // Verify pipeline exists
    if state.controller.get(pipeline_id).is_none() {
        return ApiError::from(XervError::PipelineNotFound {
            pipeline_id: pipeline_id.to_string(),
        })
        .into_response();
    }

    // Get and pause the trigger
    let trigger = match state.listener_pool.get_trigger(trigger_id) {
        Some(t) => t,
        None => {
            return ApiError::not_found("E305", format!("Trigger '{}' not found", trigger_id))
                .into_response();
        }
    };

    if let Err(e) = trigger.pause().await {
        return ApiError::from(e).into_response();
    }

    tracing::info!(
        pipeline_id = %pipeline_id,
        trigger_id = %trigger_id,
        "Trigger paused"
    );

    let body = serde_json::json!({
        "trigger_id": trigger_id,
        "status": "paused"
    });

    response::ok(&body)
}

/// POST /api/v1/pipelines/{id}/triggers/{tid}/resume
///
/// Resume a paused trigger.
pub async fn resume(
    state: Arc<AppState>,
    pipeline_id: &str,
    trigger_id: &str,
) -> Response<Full<Bytes>> {
    // Verify pipeline exists
    if state.controller.get(pipeline_id).is_none() {
        return ApiError::from(XervError::PipelineNotFound {
            pipeline_id: pipeline_id.to_string(),
        })
        .into_response();
    }

    // Get and resume the trigger
    let trigger = match state.listener_pool.get_trigger(trigger_id) {
        Some(t) => t,
        None => {
            return ApiError::not_found("E305", format!("Trigger '{}' not found", trigger_id))
                .into_response();
        }
    };

    if let Err(e) = trigger.resume().await {
        return ApiError::from(e).into_response();
    }

    tracing::info!(
        pipeline_id = %pipeline_id,
        trigger_id = %trigger_id,
        "Trigger resumed"
    );

    let body = serde_json::json!({
        "trigger_id": trigger_id,
        "status": "running"
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
    async fn list_triggers_pipeline_not_found() {
        let state = test_state();
        let response = list(state, "nonexistent@v1").await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
