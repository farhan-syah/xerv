//! Integration tests for xerv-client.
//!
//! These tests verify the client API surface without requiring a running server.

use xerv_client::{Client, ClientError};

#[test]
fn test_client_construction() {
    // Valid URL
    let client = Client::new("http://localhost:8080");
    assert!(client.is_ok());

    // HTTPS URL
    let client = Client::new("https://api.example.com");
    assert!(client.is_ok());
}

#[test]
fn test_client_invalid_url() {
    // Missing protocol
    let result = Client::new("localhost:8080");
    assert!(result.is_err());

    match result {
        Err(ClientError::InvalidUrl(msg)) => {
            assert!(msg.contains("http://"));
        }
        _ => panic!("Expected InvalidUrl error"),
    }
}

#[test]
fn test_client_with_api_key() {
    let client = Client::new("http://localhost:8080")
        .unwrap()
        .with_api_key("test-key");

    // Client should be successfully created with API key
    // (we can't inspect the internal state, but we verify no panic)
    drop(client);
}

#[test]
fn test_client_builder_pattern() {
    use std::time::Duration;

    let client = Client::new("http://localhost:8080")
        .unwrap()
        .with_api_key("my-secret-key")
        .with_timeout(Duration::from_secs(60));

    assert!(client.is_ok());
}

#[test]
fn test_url_normalization() {
    // Trailing slash should be handled
    let client1 = Client::new("http://localhost:8080");
    let client2 = Client::new("http://localhost:8080/");

    assert!(client1.is_ok());
    assert!(client2.is_ok());
}

#[test]
fn test_https_urls() {
    let client = Client::new("https://secure.example.com");
    assert!(client.is_ok());
}

#[test]
fn test_error_display() {
    let error = ClientError::InvalidUrl("test error".to_string());
    let display = format!("{}", error);
    assert!(display.contains("Invalid URL"));
    assert!(display.contains("test error"));
}

#[test]
fn test_api_error_display() {
    let error = ClientError::Api {
        status: 404,
        message: "Not found".to_string(),
    };

    let display = format!("{}", error);
    assert!(display.contains("404"));
    assert!(display.contains("Not found"));
}

// Note: Actual HTTP tests would require a running server or mocking.
// These tests focus on client construction and error handling.
// For end-to-end tests, use the test infrastructure in xerv-executor.
