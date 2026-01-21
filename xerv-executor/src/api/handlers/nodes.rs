//! Node metadata endpoint handler.

use crate::api::response;
use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;

/// GET /api/v1/nodes/metadata
pub async fn metadata() -> Response<Full<Bytes>> {
    let body = serde_json::json!({
        "nodes": [
            {
                "type": "std::log",
                "category": "logging",
                "display_name": "Log Message",
                "description": "Write structured log entry",
                "icon": "📝",
                "inputs": [
                    { "name": "input", "type": "Any", "required": true }
                ],
                "outputs": [
                    { "name": "output", "type": "Any" }
                ],
                "config_schema": {
                    "type": "object",
                    "properties": {
                        "level": {
                            "type": "string",
                            "enum": ["debug", "info", "warn", "error"],
                            "default": "info"
                        },
                        "message": {
                            "type": "string",
                            "format": "selector"
                        }
                    },
                    "required": ["message"]
                }
            },
            {
                "type": "std::switch",
                "category": "flow_control",
                "display_name": "Conditional Branch",
                "description": "Route data based on condition",
                "icon": "🔀",
                "inputs": [
                    { "name": "input", "type": "Any", "required": true }
                ],
                "outputs": [
                    { "name": "true", "type": "Any" },
                    { "name": "false", "type": "Any" }
                ],
                "config_schema": {
                    "type": "object",
                    "properties": {
                        "condition": {
                            "type": "string",
                            "description": "Selector expression to evaluate (e.g., ${input.value} > 100)"
                        }
                    },
                    "required": ["condition"]
                }
            },
            {
                "type": "std::http",
                "category": "network",
                "display_name": "HTTP Request",
                "description": "Perform an HTTP request",
                "icon": "🌐",
                "inputs": [],
                "outputs": [
                    { "name": "response", "type": "Any" },
                    { "name": "status_code", "type": "Number" }
                ],
                "config_schema": {
                    "type": "object",
                    "properties": {
                        "method": {
                            "type": "string",
                            "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"],
                            "default": "GET"
                        },
                        "url": {
                            "type": "string"
                        },
                        "headers": {
                            "type": "string"
                        },
                        "body": {
                            "type": "string",
                            "format": "selector"
                        }
                    },
                    "required": ["url"]
                }
            }
        ]
    });

    response::ok(&body)
}
