//! Resume handlers for suspended traces.

use crate::api::error::ApiError;
use crate::api::request;
use crate::api::response;
use crate::api::state::AppState;
use crate::suspension::ResumeDecision;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::{Request, Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Request body for resuming a suspended trace.
#[derive(Debug, Deserialize)]
struct ResumeRequest {
    /// The decision to make: "approve", "reject", or "escalate".
    decision: String,
    /// Optional response data (for approve).
    response_data: Option<serde_json::Value>,
    /// Optional reason (for reject).
    reason: Option<String>,
    /// Optional details (for escalate).
    details: Option<serde_json::Value>,
}

/// Response body for a successful resume.
#[derive(Debug, Serialize)]
struct ResumeResponse {
    /// The hook ID that was resumed.
    hook_id: String,
    /// The trace ID that was resumed.
    trace_id: String,
    /// The decision that was applied.
    decision: String,
    /// The output port that will be triggered.
    output_port: String,
}

/// POST /api/v1/resume/{hook_id}
///
/// Resume a suspended trace with an approval decision.
pub async fn resume(
    req: Request<Incoming>,
    state: Arc<AppState>,
    hook_id: &str,
) -> Response<Full<Bytes>> {
    // Parse request body
    let body: ResumeRequest = match request::read_body_json(req).await {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };

    // Convert to ResumeDecision
    let decision = match body.decision.to_lowercase().as_str() {
        "approve" | "approved" => ResumeDecision::Approve {
            response_data: body.response_data,
        },
        "reject" | "rejected" => ResumeDecision::Reject {
            reason: body.reason,
        },
        "escalate" | "escalated" => ResumeDecision::Escalate {
            details: body.details,
        },
        other => {
            return ApiError::bad_request(
                "E000",
                format!(
                    "Invalid decision '{}'. Must be 'approve', 'reject', or 'escalate'",
                    other
                ),
            )
            .into_response();
        }
    };

    let output_port = decision.output_port().to_string();
    let decision_str = body.decision.to_lowercase();

    // First, get the suspended state to find the pipeline ID
    let suspended_state = match state.suspension_store.get(hook_id) {
        Ok(s) => s,
        Err(e) => return ApiError::from(e).into_response(),
    };

    let pipeline_id = suspended_state.pipeline_id.clone();
    let trace_id = suspended_state.trace_id;

    // Get the pipeline that owns this trace
    let pipeline = match state.controller.get(&pipeline_id) {
        Some(p) => p,
        None => {
            return ApiError::not_found("E501", format!("Pipeline '{}' not found", pipeline_id))
                .into_response();
        }
    };

    // Resume the trace through the pipeline (which calls executor.resume_suspended_trace)
    match pipeline.resume_suspended_trace(hook_id, decision).await {
        Ok(resumed_trace_id) => {
            tracing::info!(
                hook_id = %hook_id,
                trace_id = %resumed_trace_id,
                pipeline_id = %pipeline_id,
                decision = %decision_str,
                "Trace resumed and execution continued"
            );

            let resp = ResumeResponse {
                hook_id: hook_id.to_string(),
                trace_id: resumed_trace_id.to_string(),
                decision: decision_str,
                output_port,
            };

            response::ok(&resp)
        }
        Err(e) => {
            tracing::error!(
                hook_id = %hook_id,
                trace_id = %trace_id,
                error = %e,
                "Failed to resume trace"
            );
            ApiError::from(e).into_response()
        }
    }
}

/// GET /api/v1/resume/{hook_id}
///
/// Get information about a suspended trace.
pub async fn get(state: Arc<AppState>, hook_id: &str) -> Response<Full<Bytes>> {
    match state.suspension_store.get(hook_id) {
        Ok(state) => {
            let body = serde_json::json!({
                "hook_id": state.hook_id,
                "trace_id": state.trace_id.to_string(),
                "pipeline_id": state.pipeline_id,
                "suspended_at": state.suspended_at.as_u32(),
                "created_at": state.created_at.to_rfc3339(),
                "expires_at": state.expires_at.map(|t: chrono::DateTime<chrono::Utc>| t.to_rfc3339()),
                "is_expired": state.is_expired(),
                "metadata": state.metadata,
                "resume_url": format!("/api/v1/resume/{}", hook_id)
            });
            response::ok(&body)
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// GET /api/v1/suspensions
///
/// List all suspended traces.
pub async fn list(state: Arc<AppState>) -> Response<Full<Bytes>> {
    let suspensions = state.suspension_store.list_all();

    let items: Vec<serde_json::Value> = suspensions
        .iter()
        .map(|s| {
            serde_json::json!({
                "hook_id": s.hook_id,
                "trace_id": s.trace_id.to_string(),
                "pipeline_id": s.pipeline_id,
                "created_at": s.created_at.to_rfc3339(),
                "expires_at": s.expires_at.map(|t: chrono::DateTime<chrono::Utc>| t.to_rfc3339()),
                "is_expired": s.is_expired(),
            })
        })
        .collect();

    let body = serde_json::json!({
        "suspensions": items,
        "count": items.len()
    });

    response::ok(&body)
}

/// GET /api/v1/pipelines/{id}/suspensions
///
/// List suspended traces for a specific pipeline.
pub async fn list_by_pipeline(state: Arc<AppState>, pipeline_id: &str) -> Response<Full<Bytes>> {
    let suspensions = state.suspension_store.list_by_pipeline(pipeline_id);

    let items: Vec<serde_json::Value> = suspensions
        .iter()
        .map(|s| {
            serde_json::json!({
                "hook_id": s.hook_id,
                "trace_id": s.trace_id.to_string(),
                "created_at": s.created_at.to_rfc3339(),
                "expires_at": s.expires_at.map(|t: chrono::DateTime<chrono::Utc>| t.to_rfc3339()),
                "is_expired": s.is_expired(),
            })
        })
        .collect();

    let body = serde_json::json!({
        "pipeline_id": pipeline_id,
        "suspensions": items,
        "count": items.len()
    });

    response::ok(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_response_serializes() {
        let resp = ResumeResponse {
            hook_id: "hook-1".to_string(),
            trace_id: "abc-123".to_string(),
            decision: "approve".to_string(),
            output_port: "out".to_string(),
        };

        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("hook-1"));
        assert!(json.contains("approve"));
    }
}
