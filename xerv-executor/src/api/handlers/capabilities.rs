//! Capabilities endpoint handler.

use crate::api::response;
use crate::api::state::AppState;
use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;
use std::sync::Arc;

/// GET /api/v1/capabilities
pub async fn get(state: Arc<AppState>) -> Response<Full<Bytes>> {
    let body = serde_json::json!({
        "can_debug": true,
        "retention_days": 30,
        "sso_enabled": false,
        "max_pipelines": 1000,
        "max_nodes_per_pipeline": 100,
        "advanced_nodes": ["std::python", "std::ai", "std::sql"],
        "active_pipelines": state.pipeline_count()
    });

    response::ok(&body)
}
