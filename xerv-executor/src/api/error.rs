//! API error types and XervError → HTTP status mapping.

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use xerv_core::error::XervError;

/// API error with HTTP status code and error code.
#[derive(Debug)]
pub struct ApiError {
    /// Error code (e.g., "E501").
    pub code: &'static str,
    /// Human-readable error message.
    pub message: String,
    /// HTTP status code.
    pub status: StatusCode,
}

impl ApiError {
    /// Create a new API error.
    pub fn new(code: &'static str, message: impl Into<String>, status: StatusCode) -> Self {
        Self {
            code,
            message: message.into(),
            status,
        }
    }

    /// Create a 400 Bad Request error.
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(code, message, StatusCode::BAD_REQUEST)
    }

    /// Create a 404 Not Found error.
    pub fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(code, message, StatusCode::NOT_FOUND)
    }

    /// Create a 409 Conflict error.
    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(code, message, StatusCode::CONFLICT)
    }

    /// Create a 429 Too Many Requests error.
    pub fn too_many_requests(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(code, message, StatusCode::TOO_MANY_REQUESTS)
    }

    /// Create a 500 Internal Server Error.
    pub fn internal(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(code, message, StatusCode::INTERNAL_SERVER_ERROR)
    }

    /// Create a 503 Service Unavailable error.
    pub fn service_unavailable(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(code, message, StatusCode::SERVICE_UNAVAILABLE)
    }

    /// Convert to HTTP response.
    pub fn into_response(self) -> Response<Full<Bytes>> {
        let body = serde_json::json!({
            "error": {
                "code": self.code,
                "message": self.message,
                "status": self.status.as_u16()
            }
        });

        Response::builder()
            .status(self.status)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(body.to_string())))
            .expect("response builder should not fail")
    }
}

impl From<XervError> for ApiError {
    fn from(err: XervError) -> Self {
        let code = err.code();
        let message = err.to_string();

        let status = match &err {
            // 404 Not Found
            XervError::PipelineNotFound { .. } | XervError::NodeNotFound { .. } => {
                StatusCode::NOT_FOUND
            }

            // 409 Conflict
            XervError::PipelineExists { .. } => StatusCode::CONFLICT,

            // 503 Service Unavailable
            XervError::CircuitBreakerOpen { .. } => StatusCode::SERVICE_UNAVAILABLE,

            // 429 Too Many Requests
            XervError::ConcurrencyLimit { .. } => StatusCode::TOO_MANY_REQUESTS,

            // 400 Bad Request (configuration/validation errors)
            XervError::SelectorSyntax { .. }
            | XervError::YamlParse { .. }
            | XervError::ConfigValue { .. }
            | XervError::SchemaValidation { .. }
            | XervError::InvalidTopology { .. }
            | XervError::InvalidPort { .. }
            | XervError::InvalidEdge { .. }
            | XervError::MissingEdge { .. } => StatusCode::BAD_REQUEST,

            // 500 Internal Server Error (everything else)
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        Self {
            code,
            message,
            status,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xerv_error_mapping_not_found() {
        let err = XervError::PipelineNotFound {
            pipeline_id: "test@v1".to_string(),
        };
        let api_err: ApiError = err.into();

        assert_eq!(api_err.code, "E501");
        assert_eq!(api_err.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn xerv_error_mapping_conflict() {
        let err = XervError::PipelineExists {
            pipeline_id: "test@v1".to_string(),
        };
        let api_err: ApiError = err.into();

        assert_eq!(api_err.code, "E502");
        assert_eq!(api_err.status, StatusCode::CONFLICT);
    }

    #[test]
    fn xerv_error_mapping_circuit_breaker() {
        let err = XervError::CircuitBreakerOpen {
            pipeline_id: "test@v1".to_string(),
            error_rate: 0.5,
        };
        let api_err: ApiError = err.into();

        assert_eq!(api_err.code, "E504");
        assert_eq!(api_err.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn xerv_error_mapping_bad_request() {
        let err = XervError::SelectorSyntax {
            selector: "${invalid}".to_string(),
            cause: "missing field".to_string(),
        };
        let api_err: ApiError = err.into();

        assert_eq!(api_err.code, "E103");
        assert_eq!(api_err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn error_response_format() {
        let err = ApiError::not_found("E501", "Pipeline 'test' not found");
        let response = err.into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
