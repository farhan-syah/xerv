//! Core XERV client implementation.

use crate::error::{ClientError, Result};
use reqwest::{Client as HttpClient, RequestBuilder, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

/// A client for interacting with the XERV API.
///
/// # Example
///
/// ```no_run
/// use xerv_client::Client;
/// use std::time::Duration;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let client = Client::new("http://localhost:8080")?
///     .with_api_key("my-secret-key")
///     .with_timeout(Duration::from_secs(30))?;
///
/// let pipelines = client.list_pipelines().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Client {
    /// Base URL for the XERV server.
    base_url: String,
    /// HTTP client.
    http: HttpClient,
    /// Optional API key for authentication.
    api_key: Option<String>,
}

impl Client {
    /// Create a new XERV client.
    ///
    /// # Arguments
    ///
    /// * `base_url` - Base URL of the XERV server (e.g., "http://localhost:8080")
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is invalid or the HTTP client cannot be created.
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let base_url = base_url.into();

        // Validate URL format
        if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
            return Err(ClientError::InvalidUrl(format!(
                "URL must start with http:// or https://, got: {}",
                base_url
            )));
        }

        let http = HttpClient::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self {
            base_url,
            http,
            api_key: None,
        })
    }

    /// Set an API key for authentication.
    ///
    /// The API key will be sent in the `Authorization` header as `Bearer <key>`.
    #[must_use]
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Set a custom timeout for all requests.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be rebuilt.
    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self> {
        self.http = HttpClient::builder().timeout(timeout).build()?;
        Ok(self)
    }

    /// Build a full URL from a path.
    fn url(&self, path: &str) -> String {
        let path = path.strip_prefix('/').unwrap_or(path);
        format!("{}/api/v1/{}", self.base_url.trim_end_matches('/'), path)
    }

    /// Add authentication headers to a request.
    fn with_auth(&self, builder: RequestBuilder) -> RequestBuilder {
        if let Some(ref key) = self.api_key {
            builder.header("Authorization", format!("Bearer {}", key))
        } else {
            builder
        }
    }

    /// Execute a GET request.
    pub(crate) async fn get(&self, path: &str) -> Result<Response> {
        let url = self.url(path);
        let request = self.with_auth(self.http.get(&url));

        request.send().await.map_err(ClientError::Http)
    }

    /// Execute a POST request with a JSON body.
    pub(crate) async fn post<T: Serialize>(&self, path: &str, body: &T) -> Result<Response> {
        let url = self.url(path);
        let request = self.with_auth(self.http.post(&url)).json(body);

        request.send().await.map_err(ClientError::Http)
    }

    /// Execute a POST request with a raw body and content type.
    pub(crate) async fn post_raw(
        &self,
        path: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<Response> {
        let url = self.url(path);
        let request = self
            .with_auth(self.http.post(&url))
            .header("Content-Type", content_type)
            .body(body);

        request.send().await.map_err(ClientError::Http)
    }

    /// Execute a DELETE request.
    pub(crate) async fn delete(&self, path: &str) -> Result<Response> {
        let url = self.url(path);
        let request = self.with_auth(self.http.delete(&url));

        request.send().await.map_err(ClientError::Http)
    }

    /// Handle a response and deserialize JSON.
    pub(crate) async fn handle_response<T: DeserializeOwned>(
        &self,
        response: Response,
    ) -> Result<T> {
        let status = response.status();

        if status.is_success() {
            response.json::<T>().await.map_err(ClientError::Http)
        } else {
            // Try to extract error message from JSON response
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            // Try to parse as JSON error
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                let message = json["error"]
                    .as_str()
                    .or_else(|| json["message"].as_str())
                    .unwrap_or(&body)
                    .to_string();

                Err(ClientError::Api {
                    status: status.as_u16(),
                    message,
                })
            } else {
                Err(ClientError::Api {
                    status: status.as_u16(),
                    message: body,
                })
            }
        }
    }

    /// Handle a response that returns no body (204 No Content).
    pub(crate) async fn handle_empty_response(&self, response: Response) -> Result<()> {
        let status = response.status();

        if status.is_success() || status == StatusCode::NO_CONTENT {
            Ok(())
        } else {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                let message = json["error"]
                    .as_str()
                    .or_else(|| json["message"].as_str())
                    .unwrap_or(&body)
                    .to_string();

                Err(ClientError::Api {
                    status: status.as_u16(),
                    message,
                })
            } else {
                Err(ClientError::Api {
                    status: status.as_u16(),
                    message: body,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_new() {
        let client = Client::new("http://localhost:8080").unwrap();
        assert_eq!(client.base_url, "http://localhost:8080");
        assert!(client.api_key.is_none());
    }

    #[test]
    fn test_client_with_api_key() {
        let client = Client::new("http://localhost:8080")
            .unwrap()
            .with_api_key("test-key");
        assert_eq!(client.api_key, Some("test-key".to_string()));
    }

    #[test]
    fn test_client_invalid_url() {
        let result = Client::new("not-a-url");
        assert!(result.is_err());
    }

    #[test]
    fn test_url_building() {
        let client = Client::new("http://localhost:8080").unwrap();
        assert_eq!(
            client.url("pipelines"),
            "http://localhost:8080/api/v1/pipelines"
        );
        assert_eq!(
            client.url("/pipelines"),
            "http://localhost:8080/api/v1/pipelines"
        );
    }

    #[test]
    fn test_url_building_with_trailing_slash() {
        let client = Client::new("http://localhost:8080/").unwrap();
        assert_eq!(
            client.url("pipelines"),
            "http://localhost:8080/api/v1/pipelines"
        );
    }
}
