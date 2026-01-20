//! Request routing for the API.
//!
//! Routes requests to appropriate handlers based on method and path.

use super::handlers;
use super::response;
use super::state::AppState;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::{Method, Request, Response};
use std::convert::Infallible;
use std::sync::Arc;
use xerv_core::auth::{AuthContext, AuthMiddleware, HeaderAccess};

/// Route prefix for all API endpoints.
const API_PREFIX: &str = "/api/v1";

/// Hyper header adapter for auth middleware.
struct HyperHeaders<'a>(&'a hyper::HeaderMap);

impl<'a> HeaderAccess for HyperHeaders<'a> {
    fn get_header(&self, name: &str) -> Option<String> {
        self.0
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }
}

/// Route an incoming request to the appropriate handler.
pub async fn route(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    tracing::debug!(method = %method, path = %path, "Routing request");

    // Authenticate the request
    let headers = HyperHeaders(req.headers());
    let auth_ctx = match AuthMiddleware::authenticate(&state.auth_config, &path, &headers) {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!(path = %path, error = %e, "Authentication failed");
            return Ok(response::unauthorized(&e.to_string()));
        }
    };

    // Strip API prefix
    let path = path.strip_prefix(API_PREFIX).unwrap_or(&path);

    let response = match (method, path) {
        // Health endpoints (no auth required - exempt paths)
        (Method::GET, "/health") => handlers::health::get_health(state).await,
        (Method::GET, "/status") => handlers::health::get_status(state).await,

        // Pipeline endpoints
        (Method::GET, "/pipelines") => {
            if let Err(r) = require_scope(&auth_ctx, xerv_core::auth::AuthScope::PipelineRead) {
                return Ok(r);
            }
            handlers::pipelines::list(state).await
        }
        (Method::POST, "/pipelines") => {
            if let Err(r) = require_scope(&auth_ctx, xerv_core::auth::AuthScope::PipelineWrite) {
                return Ok(r);
            }
            handlers::pipelines::create(req, state).await
        }

        // Pipeline subpaths
        (_, p) if p.starts_with("/pipelines/") => {
            route_pipeline_subpath(req, state, p, &auth_ctx).await
        }

        // Trace endpoints
        (Method::GET, "/traces") => {
            if let Err(r) = require_scope(&auth_ctx, xerv_core::auth::AuthScope::TraceRead) {
                return Ok(r);
            }
            handlers::traces::list(state).await
        }
        (_, p) if p.starts_with("/traces/") => route_trace_subpath(req, state, p, &auth_ctx).await,

        // Log endpoints
        (Method::GET, "/logs") => {
            if let Err(r) = require_scope(&auth_ctx, xerv_core::auth::AuthScope::TraceRead) {
                return Ok(r);
            }
            handlers::logs::query(req, state).await
        }
        (Method::GET, "/logs/recent") => {
            if let Err(r) = require_scope(&auth_ctx, xerv_core::auth::AuthScope::TraceRead) {
                return Ok(r);
            }
            handlers::logs::recent(req, state).await
        }
        (Method::GET, "/logs/stats") => {
            if let Err(r) = require_scope(&auth_ctx, xerv_core::auth::AuthScope::TraceRead) {
                return Ok(r);
            }
            handlers::logs::stats(state).await
        }
        (Method::DELETE, "/logs") => {
            if let Err(r) = require_scope(&auth_ctx, xerv_core::auth::AuthScope::Admin) {
                return Ok(r);
            }
            handlers::logs::clear(state).await
        }
        (_, p) if p.starts_with("/logs/") => route_log_subpath(req, state, p, &auth_ctx).await,

        // Suspension endpoints
        (Method::GET, "/suspensions") => {
            if let Err(r) = require_scope(&auth_ctx, xerv_core::auth::AuthScope::TraceRead) {
                return Ok(r);
            }
            handlers::resume::list(state).await
        }
        (_, p) if p.starts_with("/resume/") => route_resume_subpath(req, state, p, &auth_ctx).await,

        // Not found
        _ => response::not_found(),
    };

    Ok(response)
}

/// Helper to check scope and return forbidden response if not authorized.
#[allow(clippy::result_large_err)]
fn require_scope(
    ctx: &AuthContext,
    scope: xerv_core::auth::AuthScope,
) -> Result<(), Response<Full<Bytes>>> {
    if ctx.has_scope(scope) {
        Ok(())
    } else {
        Err(response::forbidden(&format!(
            "Missing required scope: {}",
            scope.as_str()
        )))
    }
}

/// Route requests under /pipelines/{id}/...
async fn route_pipeline_subpath(
    req: Request<Incoming>,
    state: Arc<AppState>,
    path: &str,
    auth_ctx: &AuthContext,
) -> Response<Full<Bytes>> {
    let method = req.method();

    // Parse /pipelines/{id} or /pipelines/{id}/...
    let path = path.strip_prefix("/pipelines/").unwrap_or("");
    let (pipeline_id, subpath) = match path.split_once('/') {
        Some((id, rest)) => (id, Some(rest)),
        None => (path, None),
    };

    if pipeline_id.is_empty() {
        return response::not_found();
    }

    match (method, subpath) {
        // GET /pipelines/{id}
        (&Method::GET, None) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::PipelineRead) {
                return r;
            }
            handlers::pipelines::get(state, pipeline_id).await
        }

        // DELETE /pipelines/{id}
        (&Method::DELETE, None) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::PipelineWrite) {
                return r;
            }
            handlers::pipelines::delete(state, pipeline_id).await
        }

        // POST /pipelines/{id}/start
        (&Method::POST, Some("start")) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::PipelineWrite) {
                return r;
            }
            handlers::pipelines::start(state, pipeline_id).await
        }

        // POST /pipelines/{id}/pause
        (&Method::POST, Some("pause")) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::PipelineWrite) {
                return r;
            }
            handlers::pipelines::pause(state, pipeline_id).await
        }

        // POST /pipelines/{id}/resume
        (&Method::POST, Some("resume")) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::PipelineWrite) {
                return r;
            }
            handlers::pipelines::resume(state, pipeline_id).await
        }

        // POST /pipelines/{id}/drain
        (&Method::POST, Some("drain")) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::PipelineWrite) {
                return r;
            }
            handlers::pipelines::drain(state, pipeline_id).await
        }

        // POST /pipelines/{id}/stop
        (&Method::POST, Some("stop")) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::PipelineWrite) {
                return r;
            }
            handlers::pipelines::stop(state, pipeline_id).await
        }

        // GET /pipelines/{id}/metrics
        (&Method::GET, Some("metrics")) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::PipelineRead) {
                return r;
            }
            handlers::pipelines::metrics(state, pipeline_id).await
        }

        // GET /pipelines/{id}/traces
        (&Method::GET, Some("traces")) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::TraceRead) {
                return r;
            }
            handlers::traces::list_by_pipeline(state, pipeline_id).await
        }

        // GET /pipelines/{id}/suspensions
        (&Method::GET, Some("suspensions")) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::TraceRead) {
                return r;
            }
            handlers::resume::list_by_pipeline(state, pipeline_id).await
        }

        // Trigger subpaths
        _ if subpath.map(|s| s.starts_with("triggers")).unwrap_or(false) => {
            route_trigger_subpath(req, state, pipeline_id, subpath.unwrap(), auth_ctx).await
        }

        _ => response::not_found(),
    }
}

/// Route requests under /pipelines/{id}/triggers/...
async fn route_trigger_subpath(
    req: Request<Incoming>,
    state: Arc<AppState>,
    pipeline_id: &str,
    subpath: &str,
    auth_ctx: &AuthContext,
) -> Response<Full<Bytes>> {
    let method = req.method();

    // Parse triggers or triggers/{tid}/...
    let trigger_path = subpath.strip_prefix("triggers").unwrap_or("");

    if trigger_path.is_empty() || trigger_path == "/" {
        // GET /pipelines/{id}/triggers
        return match method {
            &Method::GET => {
                if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::PipelineRead) {
                    return r;
                }
                handlers::triggers::list(state, pipeline_id).await
            }
            _ => response::method_not_allowed(&["GET"]),
        };
    }

    // Parse /{tid} or /{tid}/action
    let trigger_path = trigger_path.strip_prefix('/').unwrap_or(trigger_path);
    let (trigger_id, action) = match trigger_path.split_once('/') {
        Some((id, rest)) => (id, Some(rest)),
        None => (trigger_path, None),
    };

    if trigger_id.is_empty() {
        return response::not_found();
    }

    match (method, action) {
        // POST /pipelines/{id}/triggers/{tid}/fire
        (&Method::POST, Some("fire")) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::TraceWrite) {
                return r;
            }
            handlers::triggers::fire(state, pipeline_id, trigger_id).await
        }

        // POST /pipelines/{id}/triggers/{tid}/pause
        (&Method::POST, Some("pause")) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::PipelineWrite) {
                return r;
            }
            handlers::triggers::pause(state, pipeline_id, trigger_id).await
        }

        // POST /pipelines/{id}/triggers/{tid}/resume
        (&Method::POST, Some("resume")) => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::PipelineWrite) {
                return r;
            }
            handlers::triggers::resume(state, pipeline_id, trigger_id).await
        }

        _ => response::not_found(),
    }
}

/// Route requests under /traces/{trace_id}
async fn route_trace_subpath(
    _req: Request<Incoming>,
    state: Arc<AppState>,
    path: &str,
    auth_ctx: &AuthContext,
) -> Response<Full<Bytes>> {
    // Parse /traces/{trace_id}
    let trace_id = path.strip_prefix("/traces/").unwrap_or("");

    if trace_id.is_empty() {
        return response::not_found();
    }

    if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::TraceRead) {
        return r;
    }

    handlers::traces::get(state, trace_id).await
}

/// Route requests under /logs/...
async fn route_log_subpath(
    req: Request<Incoming>,
    state: Arc<AppState>,
    path: &str,
    auth_ctx: &AuthContext,
) -> Response<Full<Bytes>> {
    let subpath = path.strip_prefix("/logs/").unwrap_or("");

    if subpath.is_empty() {
        return response::not_found();
    }

    // Check TraceRead scope for all log endpoints
    if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::TraceRead) {
        return r;
    }

    // Handle /logs/by-trace/{trace_id}
    if let Some(trace_id) = subpath.strip_prefix("by-trace/") {
        if !trace_id.is_empty() {
            return handlers::logs::by_trace(state, trace_id).await;
        }
    }

    // Handle /logs/by-level/{level}
    if let Some(level) = subpath.strip_prefix("by-level/") {
        if !level.is_empty() {
            return handlers::logs::by_level(req, state, level).await;
        }
    }

    response::not_found()
}

/// Route requests under /resume/{hook_id}
async fn route_resume_subpath(
    req: Request<Incoming>,
    state: Arc<AppState>,
    path: &str,
    auth_ctx: &AuthContext,
) -> Response<Full<Bytes>> {
    let method = req.method();

    // Parse /resume/{hook_id}
    let hook_id = path.strip_prefix("/resume/").unwrap_or("");

    if hook_id.is_empty() {
        return response::not_found();
    }

    match *method {
        // GET /resume/{hook_id} - get suspension info
        Method::GET => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::TraceRead) {
                return r;
            }
            handlers::resume::get(state, hook_id).await
        }

        // POST /resume/{hook_id} - resume the trace
        Method::POST => {
            if let Err(r) = require_scope(auth_ctx, xerv_core::auth::AuthScope::TraceResume) {
                return r;
            }
            handlers::resume::resume(req, state, hook_id).await
        }

        _ => response::method_not_allowed(&["GET", "POST"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_prefix_defined() {
        assert_eq!(API_PREFIX, "/api/v1");
    }
}
