//! Error types for the XERV client.

use thiserror::Error;

/// Errors that can occur when using the XERV client.
#[derive(Debug, Error)]
pub enum ClientError {
    /// HTTP request failed.
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// Server returned an error response.
    #[error("API error (status {status}): {message}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Error message from server.
        message: String,
    },

    /// Failed to deserialize response.
    #[error("Failed to deserialize response: {0}")]
    Deserialize(#[from] serde_json::Error),

    /// Invalid URL provided.
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// Invalid response format.
    #[error("Invalid response format: {0}")]
    InvalidResponse(String),

    /// Authentication failed.
    #[error("Authentication failed: {0}")]
    Auth(String),
}

/// Result type for client operations.
pub type Result<T> = std::result::Result<T, ClientError>;
