//! JSON response builders for the API.

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use serde::Serialize;

/// Build a JSON response with status code.
pub fn json_response<T: Serialize>(status: StatusCode, body: &T) -> Response<Full<Bytes>> {
    let json = serde_json::to_string(body).unwrap_or_else(|e| {
        serde_json::json!({
            "error": {
                "code": "E804",
                "message": format!("Serialization error: {}", e),
                "status": 500
            }
        })
        .to_string()
    });

    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(json)))
        .expect("response builder should not fail")
}

/// Build a 200 OK JSON response.
pub fn ok<T: Serialize>(body: &T) -> Response<Full<Bytes>> {
    json_response(StatusCode::OK, body)
}

/// Build a 201 Created JSON response.
pub fn created<T: Serialize>(body: &T) -> Response<Full<Bytes>> {
    json_response(StatusCode::CREATED, body)
}

/// Build a 204 No Content response.
pub fn no_content() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Full::new(Bytes::new()))
        .expect("response builder should not fail")
}

/// Build a 404 Not Found response.
pub fn not_found() -> Response<Full<Bytes>> {
    let body = serde_json::json!({
        "error": {
            "code": "E000",
            "message": "Not found",
            "status": 404
        }
    });
    json_response(StatusCode::NOT_FOUND, &body)
}

/// Build a 405 Method Not Allowed response.
pub fn method_not_allowed(allowed: &[&str]) -> Response<Full<Bytes>> {
    let body = serde_json::json!({
        "error": {
            "code": "E000",
            "message": format!("Method not allowed. Allowed: {}", allowed.join(", ")),
            "status": 405
        }
    });

    Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .header("Content-Type", "application/json")
        .header("Allow", allowed.join(", "))
        .body(Full::new(Bytes::from(body.to_string())))
        .expect("response builder should not fail")
}

/// Build a 401 Unauthorized response.
pub fn unauthorized(message: &str) -> Response<Full<Bytes>> {
    let body = serde_json::json!({
        "error": {
            "code": "E1001",
            "message": message,
            "status": 401
        }
    });

    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("Content-Type", "application/json")
        .header("WWW-Authenticate", "ApiKey")
        .body(Full::new(Bytes::from(body.to_string())))
        .expect("response builder should not fail")
}

/// Build a 403 Forbidden response.
pub fn forbidden(message: &str) -> Response<Full<Bytes>> {
    let body = serde_json::json!({
        "error": {
            "code": "E1002",
            "message": message,
            "status": 403
        }
    });
    json_response(StatusCode::FORBIDDEN, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_response() {
        let body = serde_json::json!({"status": "healthy"});
        let response = ok(&body);

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("Content-Type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn created_response() {
        let body = serde_json::json!({"id": "test"});
        let response = created(&body);

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[test]
    fn no_content_response() {
        let response = no_content();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[test]
    fn not_found_response() {
        let response = not_found();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn method_not_allowed_response() {
        let response = method_not_allowed(&["GET", "POST"]);
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers().get("Allow").unwrap(), "GET, POST");
    }
}
