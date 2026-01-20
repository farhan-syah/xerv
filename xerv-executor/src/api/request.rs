//! Request body parsing utilities.

use crate::api::error::ApiError;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::Request;
use hyper::body::Incoming;

/// Maximum request body size (1MB).
const MAX_BODY_SIZE: usize = 1024 * 1024;

/// Read the request body as bytes.
pub async fn read_body(req: Request<Incoming>) -> Result<Bytes, ApiError> {
    let collected = req.into_body().collect().await.map_err(|e| {
        ApiError::bad_request("E000", format!("Failed to read request body: {}", e))
    })?;

    let body = collected.to_bytes();

    if body.len() > MAX_BODY_SIZE {
        return Err(ApiError::bad_request(
            "E000",
            format!(
                "Request body too large: {} bytes (max {})",
                body.len(),
                MAX_BODY_SIZE
            ),
        ));
    }

    Ok(body)
}

/// Read the request body as a string.
pub async fn read_body_string(req: Request<Incoming>) -> Result<String, ApiError> {
    let body = read_body(req).await?;

    String::from_utf8(body.to_vec())
        .map_err(|e| ApiError::bad_request("E000", format!("Invalid UTF-8 in request body: {}", e)))
}

/// Read the request body as JSON and deserialize it.
pub async fn read_body_json<T: serde::de::DeserializeOwned>(
    req: Request<Incoming>,
) -> Result<T, ApiError> {
    let body = read_body_string(req).await?;

    serde_json::from_str(&body)
        .map_err(|e| ApiError::bad_request("E000", format!("Invalid JSON: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_body_size_defined() {
        // 1MB
        assert_eq!(MAX_BODY_SIZE, 1024 * 1024);
    }
}
