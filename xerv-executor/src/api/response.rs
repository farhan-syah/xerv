//! JSON response builders for the API.

use bytes::Bytes;
use http_body_util::Full;
use hyper::http::header::{self, HeaderValue};
use hyper::{Response, StatusCode};
use serde::Serialize;

const CSP_HEADER: &str = "default-src 'self'; \
 script-src 'self' 'wasm-unsafe-eval'; \
 style-src 'self' 'unsafe-inline'; \
 img-src 'self' data: https:; \
 connect-src 'self'; \
 font-src 'self'; \
 object-src 'none'; \
 base-uri 'self'; \
 form-action 'self'; \
 frame-ancestors 'none'; \
 upgrade-insecure-requests;";

fn apply_security_headers(mut response: Response<Full<Bytes>>) -> Response<Full<Bytes>> {
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP_HEADER),
    );
    response
}

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

    let response = Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(json)))
        .expect("response builder should not fail");
    apply_security_headers(response)
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
    let response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Full::new(Bytes::new()))
        .expect("response builder should not fail");
    apply_security_headers(response)
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

    let response = Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .header("Content-Type", "application/json")
        .header("Allow", allowed.join(", "))
        .body(Full::new(Bytes::from(body.to_string())))
        .expect("response builder should not fail");
    apply_security_headers(response)
}

/// Build a YAML response with status code.
pub fn yaml_response(status: StatusCode, yaml: impl Into<String>) -> Response<Full<Bytes>> {
    let response = Response::builder()
        .status(status)
        .header("Content-Type", "application/x-yaml")
        .body(Full::new(Bytes::from(yaml.into())))
        .expect("response builder should not fail");
    apply_security_headers(response)
}

/// Build a 200 OK YAML response.
pub fn ok_yaml(yaml: impl Into<String>) -> Response<Full<Bytes>> {
    yaml_response(StatusCode::OK, yaml)
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

    let response = Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("Content-Type", "application/json")
        .header("WWW-Authenticate", "ApiKey")
        .body(Full::new(Bytes::from(body.to_string())))
        .expect("response builder should not fail");
    apply_security_headers(response)
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

/// Build a raw bytes response (used for static assets).
pub fn bytes_response(
    status: StatusCode,
    content_type: &str,
    body: Bytes,
) -> Response<Full<Bytes>> {
    let response = Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .body(Full::new(body))
        .expect("response builder should not fail");
    apply_security_headers(response)
}

/// Build a 429 Too Many Requests response.
pub fn too_many_requests(message: &str) -> Response<Full<Bytes>> {
    let body = serde_json::json!({
        "error": {
            "code": "E1003",
            "message": message,
            "status": 429
        }
    });
    json_response(StatusCode::TOO_MANY_REQUESTS, &body)
}

/// Build a CORS preflight response.
pub fn preflight() -> Response<Full<Bytes>> {
    let response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Full::new(Bytes::new()))
        .expect("response builder should not fail");
    apply_security_headers(response)
}

/// Apply CORS headers for a request.
pub fn apply_cors_headers(
    mut response: Response<Full<Bytes>>,
    origin: Option<&str>,
) -> Response<Full<Bytes>> {
    let allow_origin = origin.unwrap_or("*");
    if let Ok(value) = HeaderValue::from_str(allow_origin) {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
    }
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, X-API-Key, X-CSRF-Token"),
    );
    if origin.is_some() {
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
    }
    response
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
