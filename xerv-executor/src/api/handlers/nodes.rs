//! Node metadata endpoint handler.

use crate::api::response;
use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;
use xerv_nodes::registry;

/// GET /api/v1/nodes/metadata
///
/// Returns metadata for all standard library nodes.
///
/// Metadata is sourced directly from node implementations via the NodeMetadata trait,
/// ensuring it stays in sync with actual node behavior.
pub async fn metadata() -> Response<Full<Bytes>> {
    // Create the standard node registry
    let registry = registry::create_standard_registry();

    // Get all node metadata as JSON-serializable vector
    let nodes = registry.all_json();

    let body = serde_json::json!({
        "nodes": nodes
    });

    response::ok(&body)
}
