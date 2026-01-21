//! HTTP provider for network request abstraction.
//!
//! Allows tests to mock HTTP responses while production code makes real requests.

use parking_lot::RwLock;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// HTTP response from a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Response body.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Create a new HTTP response.
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: body.into(),
        }
    }

    /// Create a JSON response.
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        let body = serde_json::to_vec(value).expect("Failed to serialize JSON");
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        Self {
            status,
            headers,
            body,
        }
    }

    /// Get the body as a string.
    pub fn body_string(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// Get the body as JSON.
    pub fn body_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    /// Add a header.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
}

/// HTTP request for recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequest {
    /// HTTP method.
    pub method: String,
    /// Request URL.
    pub url: String,
    /// Request headers.
    pub headers: HashMap<String, String>,
    /// Request body.
    pub body: Vec<u8>,
}

/// Error type for HTTP operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum HttpError {
    /// Connection failed.
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    /// Request timed out.
    #[error("Request timed out")]
    Timeout,
    /// No mock rule matched.
    #[error("No mock rule matched for {method} {url}")]
    NoMockMatch { method: String, url: String },
    /// Other error.
    #[error("{0}")]
    Other(String),
}

/// Provider trait for HTTP operations.
pub trait HttpProvider: Send + Sync {
    /// Make an HTTP request.
    fn request(
        &self,
        method: &str,
        url: &str,
        headers: HashMap<String, String>,
        body: Option<Vec<u8>>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HttpResponse, HttpError>> + Send + '_>,
    >;

    /// Check if this is a mock provider.
    fn is_mock(&self) -> bool;
}

/// Real HTTP provider that makes actual network requests.
#[derive(Debug, Clone)]
pub struct RealHttp {
    /// Request timeout duration.
    timeout: Duration,
}

impl RealHttp {
    /// Create a new real HTTP provider with default 30 second timeout.
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_secs(30),
        }
    }

    /// Create with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Get the configured timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl Default for RealHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpProvider for RealHttp {
    fn request(
        &self,
        method: &str,
        url: &str,
        headers: HashMap<String, String>,
        body: Option<Vec<u8>>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HttpResponse, HttpError>> + Send + '_>,
    > {
        let timeout = self.timeout;
        let method = method.to_string();
        let url = url.to_string();

        Box::pin(async move {
            use bytes::Bytes;
            use http::header::HeaderName;
            use http_body_util::{BodyExt, Full};
            use hyper::{Method as HyperMethod, Request};
            use hyper_util::client::legacy::Client;
            use hyper_util::rt::TokioExecutor;
            use std::str::FromStr;

            // Parse the URL
            let uri = url
                .parse::<hyper::Uri>()
                .map_err(|e| HttpError::Other(format!("Invalid URL: {}", e)))?;

            // Parse the HTTP method
            let hyper_method = HyperMethod::from_str(&method)
                .map_err(|e| HttpError::Other(format!("Invalid HTTP method: {}", e)))?;

            // Build the request
            let mut req_builder = Request::builder().method(hyper_method).uri(uri);

            // Add headers
            for (key, value) in headers {
                let header_name = HeaderName::from_str(&key).map_err(|e| {
                    HttpError::Other(format!("Invalid header name '{}': {}", key, e))
                })?;
                req_builder = req_builder.header(header_name, value);
            }

            // Build the request body
            let body_bytes = body.unwrap_or_default();
            let request = req_builder
                .body(Full::new(Bytes::from(body_bytes)))
                .map_err(|e| HttpError::Other(format!("Failed to build request: {}", e)))?;

            // Create HTTP client with connector
            let client = Client::builder(TokioExecutor::new()).build_http();

            // Execute the request with timeout
            let response = tokio::time::timeout(timeout, client.request(request))
                .await
                .map_err(|_| HttpError::Timeout)?
                .map_err(|e| HttpError::ConnectionFailed(e.to_string()))?;

            // Extract status
            let status = response.status().as_u16();

            // Extract headers
            let mut response_headers = HashMap::new();
            for (name, value) in response.headers() {
                if let Ok(value_str) = value.to_str() {
                    response_headers.insert(name.to_string(), value_str.to_string());
                }
            }

            // Read the response body
            let body_bytes = response
                .into_body()
                .collect()
                .await
                .map_err(|e| HttpError::Other(format!("Failed to read response body: {}", e)))?
                .to_bytes()
                .to_vec();

            Ok(HttpResponse {
                status,
                headers: response_headers,
                body: body_bytes,
            })
        })
    }

    fn is_mock(&self) -> bool {
        false
    }
}

/// A rule for matching and responding to HTTP requests.
#[derive(Clone)]
pub struct MockHttpRule {
    /// HTTP method to match (None = any method).
    pub method: Option<String>,
    /// URL pattern (regex).
    pub url_pattern: Regex,
    /// Response to return.
    pub response: HttpResponse,
    /// Simulated latency.
    pub latency: Option<Duration>,
    /// Number of times this rule should match (None = unlimited).
    pub times: Option<usize>,
    /// Number of times this rule has matched.
    matched_count: usize,
}

impl MockHttpRule {
    /// Create a new rule matching any method.
    pub fn new(url_pattern: &str, response: HttpResponse) -> Self {
        Self {
            method: None,
            url_pattern: Regex::new(url_pattern).expect("Invalid URL regex pattern"),
            response,
            latency: None,
            times: None,
            matched_count: 0,
        }
    }

    /// Set the HTTP method to match.
    pub fn with_method(mut self, method: &str) -> Self {
        self.method = Some(method.to_uppercase());
        self
    }

    /// Set simulated latency.
    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = Some(latency);
        self
    }

    /// Set the number of times this rule should match.
    pub fn times(mut self, n: usize) -> Self {
        self.times = Some(n);
        self
    }

    /// Check if this rule matches the request.
    fn matches(&self, method: &str, url: &str) -> bool {
        // Check method
        if let Some(ref expected_method) = self.method {
            if expected_method != method.to_uppercase().as_str() {
                return false;
            }
        }

        // Check if we've exceeded the match limit
        if let Some(limit) = self.times {
            if self.matched_count >= limit {
                return false;
            }
        }

        // Check URL pattern
        self.url_pattern.is_match(url)
    }
}

impl std::fmt::Debug for MockHttpRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockHttpRule")
            .field("method", &self.method)
            .field("url_pattern", &self.url_pattern.as_str())
            .field("response_status", &self.response.status)
            .field("latency", &self.latency)
            .field("times", &self.times)
            .finish()
    }
}

/// Mock HTTP provider for testing.
///
/// Allows defining rules that match requests and return mock responses.
///
/// # Example
///
/// ```
/// use xerv_core::testing::{MockHttp, MockHttpRule, HttpResponse};
/// use serde_json::json;
///
/// let mock = MockHttp::new()
///     .rule(
///         MockHttpRule::new(r"^https://api\.example\.com/users/\d+$", HttpResponse::json(200, &json!({"name": "Alice"})))
///             .with_method("GET")
///     )
///     .rule(
///         MockHttpRule::new(r"^https://api\.example\.com/users$", HttpResponse::json(201, &json!({"id": 1})))
///             .with_method("POST")
///     );
/// ```
pub struct MockHttp {
    rules: RwLock<Vec<MockHttpRule>>,
    requests: RwLock<Vec<HttpRequest>>,
    fail_on_unmatched: bool,
}

impl MockHttp {
    /// Create a new mock HTTP provider.
    pub fn new() -> Self {
        Self {
            rules: RwLock::new(Vec::new()),
            requests: RwLock::new(Vec::new()),
            fail_on_unmatched: true,
        }
    }

    /// Add a rule.
    pub fn rule(self, rule: MockHttpRule) -> Self {
        self.rules.write().push(rule);
        self
    }

    /// Set whether to fail on unmatched requests.
    pub fn fail_on_unmatched(mut self, fail: bool) -> Self {
        self.fail_on_unmatched = fail;
        self
    }

    /// Fluent builder: start defining a GET rule.
    pub fn on_get(self, url_pattern: &str) -> MockHttpBuilder {
        MockHttpBuilder {
            mock: self,
            method: Some("GET".to_string()),
            url_pattern: url_pattern.to_string(),
            latency: None,
            times: None,
        }
    }

    /// Fluent builder: start defining a POST rule.
    pub fn on_post(self, url_pattern: &str) -> MockHttpBuilder {
        MockHttpBuilder {
            mock: self,
            method: Some("POST".to_string()),
            url_pattern: url_pattern.to_string(),
            latency: None,
            times: None,
        }
    }

    /// Fluent builder: start defining a PUT rule.
    pub fn on_put(self, url_pattern: &str) -> MockHttpBuilder {
        MockHttpBuilder {
            mock: self,
            method: Some("PUT".to_string()),
            url_pattern: url_pattern.to_string(),
            latency: None,
            times: None,
        }
    }

    /// Fluent builder: start defining a DELETE rule.
    pub fn on_delete(self, url_pattern: &str) -> MockHttpBuilder {
        MockHttpBuilder {
            mock: self,
            method: Some("DELETE".to_string()),
            url_pattern: url_pattern.to_string(),
            latency: None,
            times: None,
        }
    }

    /// Fluent builder: start defining a rule matching any method.
    pub fn on_any(self, url_pattern: &str) -> MockHttpBuilder {
        MockHttpBuilder {
            mock: self,
            method: None,
            url_pattern: url_pattern.to_string(),
            latency: None,
            times: None,
        }
    }

    /// Get all recorded requests.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.requests.read().clone()
    }

    /// Clear recorded requests.
    pub fn clear_requests(&self) {
        self.requests.write().clear();
    }

    /// Assert that a specific request was made.
    pub fn assert_request_made(&self, method: &str, url_pattern: &str) -> bool {
        let re = Regex::new(url_pattern).expect("Invalid URL pattern");
        let requests = self.requests.read();
        requests
            .iter()
            .any(|r| r.method.eq_ignore_ascii_case(method) && re.is_match(&r.url))
    }

    /// Get the number of requests made.
    pub fn request_count(&self) -> usize {
        self.requests.read().len()
    }
}

impl Default for MockHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpProvider for MockHttp {
    fn request(
        &self,
        method: &str,
        url: &str,
        headers: HashMap<String, String>,
        body: Option<Vec<u8>>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HttpResponse, HttpError>> + Send + '_>,
    > {
        // Record the request
        self.requests.write().push(HttpRequest {
            method: method.to_string(),
            url: url.to_string(),
            headers: headers.clone(),
            body: body.clone().unwrap_or_default(),
        });

        // Find a matching rule
        let mut rules = self.rules.write();
        let matched = rules.iter_mut().find(|rule| rule.matches(method, url));

        match matched {
            Some(rule) => {
                rule.matched_count += 1;
                let response = rule.response.clone();
                let latency = rule.latency;

                Box::pin(async move {
                    if let Some(delay) = latency {
                        tokio::time::sleep(delay).await;
                    }
                    Ok(response)
                })
            }
            None => {
                if self.fail_on_unmatched {
                    let method = method.to_string();
                    let url = url.to_string();
                    Box::pin(async move { Err(HttpError::NoMockMatch { method, url }) })
                } else {
                    // Return a 404 for unmatched requests
                    Box::pin(async move { Ok(HttpResponse::new(404, b"Not Found".to_vec())) })
                }
            }
        }
    }

    fn is_mock(&self) -> bool {
        true
    }
}

/// Builder for fluent mock HTTP rule creation.
pub struct MockHttpBuilder {
    mock: MockHttp,
    method: Option<String>,
    url_pattern: String,
    latency: Option<Duration>,
    times: Option<usize>,
}

impl MockHttpBuilder {
    /// Set simulated latency.
    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = latency.into();
        self
    }

    /// Set the number of times this rule should match.
    pub fn times(mut self, n: usize) -> Self {
        self.times = Some(n);
        self
    }

    /// Set the response to return.
    pub fn respond(self, response: HttpResponse) -> MockHttp {
        let mut rule = MockHttpRule::new(&self.url_pattern, response);
        rule.method = self.method;
        rule.latency = self.latency;
        rule.times = self.times;
        self.mock.rule(rule)
    }

    /// Set a JSON response.
    pub fn respond_json(self, status: u16, value: serde_json::Value) -> MockHttp {
        self.respond(HttpResponse::json(status, &value))
    }

    /// Set a plain text response.
    pub fn respond_text(self, status: u16, text: &str) -> MockHttp {
        let mut response = HttpResponse::new(status, text.as_bytes().to_vec());
        response
            .headers
            .insert("content-type".to_string(), "text/plain".to_string());
        self.respond(response)
    }

    /// Set an error response.
    pub fn respond_error(self, status: u16, message: &str) -> MockHttp {
        self.respond_json(status, serde_json::json!({"error": message}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn mock_http_matches_get() {
        let mock = MockHttp::new()
            .on_get(r"^https://api\.example\.com/users/\d+$")
            .respond_json(200, json!({"name": "Alice"}));

        let response = mock
            .request(
                "GET",
                "https://api.example.com/users/123",
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(response.status, 200);
        let body: serde_json::Value = response.body_json().unwrap();
        assert_eq!(body["name"], "Alice");
    }

    #[tokio::test]
    async fn mock_http_matches_post() {
        let mock = MockHttp::new()
            .on_post(r"^https://api\.example\.com/users$")
            .respond_json(201, json!({"id": 42}));

        let response = mock
            .request(
                "POST",
                "https://api.example.com/users",
                HashMap::new(),
                Some(b"{}".to_vec()),
            )
            .await
            .unwrap();

        assert_eq!(response.status, 201);
    }

    #[tokio::test]
    async fn mock_http_fails_on_unmatched() {
        let mock = MockHttp::new()
            .on_get(r"^https://api\.example\.com/users$")
            .respond_json(200, json!([]));

        let result = mock
            .request("GET", "https://api.example.com/other", HashMap::new(), None)
            .await;

        assert!(matches!(result, Err(HttpError::NoMockMatch { .. })));
    }

    #[tokio::test]
    async fn mock_http_records_requests() {
        let mock = MockHttp::new().on_get(r".*").respond_json(200, json!({}));

        mock.request("GET", "https://example.com/a", HashMap::new(), None)
            .await
            .unwrap();
        mock.request("GET", "https://example.com/b", HashMap::new(), None)
            .await
            .unwrap();

        assert_eq!(mock.request_count(), 2);
        assert!(mock.assert_request_made("GET", r"example\.com/a"));
        assert!(mock.assert_request_made("GET", r"example\.com/b"));
    }

    #[tokio::test]
    async fn mock_http_times_limit() {
        let mock = MockHttp::new()
            .on_get(r"^https://api\.example\.com/users$")
            .times(2)
            .respond_json(200, json!([]));

        // First two requests succeed
        mock.request("GET", "https://api.example.com/users", HashMap::new(), None)
            .await
            .unwrap();
        mock.request("GET", "https://api.example.com/users", HashMap::new(), None)
            .await
            .unwrap();

        // Third request fails (no matching rule)
        let result = mock
            .request("GET", "https://api.example.com/users", HashMap::new(), None)
            .await;
        assert!(matches!(result, Err(HttpError::NoMockMatch { .. })));
    }
}
