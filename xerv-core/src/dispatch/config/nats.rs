//! NATS-based dispatch configuration.

use serde::{Deserialize, Serialize};

/// Configuration for NATS-based dispatch.
///
/// Uses NATS JetStream for persistent, exactly-once delivery with support for
/// multi-region deployments. Best for event-driven and globally distributed systems.
///
/// # Example
///
/// ```
/// use xerv_core::dispatch::NatsConfig;
///
/// let config = NatsConfig::new("nats://nats:4222")
///     .with_jetstream()
///     .stream_name("MY_STREAM")
///     .durable("worker-1");
///
/// assert!(config.use_jetstream);
/// assert_eq!(config.stream_name, "MY_STREAM");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsConfig {
    /// NATS server URL (e.g., "nats://localhost:4222").
    pub url: String,

    /// Subject prefix for trace queues.
    #[serde(default = "default_nats_prefix")]
    pub subject_prefix: String,

    /// Use JetStream for persistence (recommended).
    #[serde(default = "default_use_jetstream")]
    pub use_jetstream: bool,

    /// Stream name (for JetStream).
    #[serde(default = "default_stream_name")]
    pub stream_name: String,

    /// Consumer name.
    #[serde(default)]
    pub consumer_name: Option<String>,

    /// Durable consumer name (survives restarts).
    #[serde(default)]
    pub durable_name: Option<String>,

    /// Ack wait timeout in milliseconds.
    #[serde(default = "default_ack_wait")]
    pub ack_wait_ms: u64,

    /// Max in-flight messages per consumer.
    #[serde(default = "default_max_ack_pending")]
    pub max_ack_pending: u64,

    /// Authentication credentials.
    #[serde(default)]
    pub credentials: Option<NatsCredentials>,

    /// Enable TLS.
    #[serde(default)]
    pub tls: bool,
}

/// NATS authentication credentials.
///
/// Supports multiple authentication methods:
/// - Username/password
/// - Token
/// - NKey (Ed25519 key pair)
/// - Credentials file
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NatsCredentials {
    /// Username/password authentication.
    UserPass {
        /// Username.
        username: String,
        /// Password.
        password: String,
    },
    /// Token authentication.
    Token {
        /// Authentication token.
        token: String,
    },
    /// NKey authentication.
    NKey {
        /// NKey seed (private key).
        seed: String,
    },
    /// Credentials file authentication.
    CredsFile {
        /// Path to credentials file.
        path: String,
    },
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self {
            url: "nats://localhost:4222".to_string(),
            subject_prefix: default_nats_prefix(),
            use_jetstream: default_use_jetstream(),
            stream_name: default_stream_name(),
            consumer_name: None,
            durable_name: None,
            ack_wait_ms: default_ack_wait(),
            max_ack_pending: default_max_ack_pending(),
            credentials: None,
            tls: false,
        }
    }
}

impl NatsConfig {
    /// Create a new NATS config with the given URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    /// Enable JetStream for persistence.
    pub fn with_jetstream(mut self) -> Self {
        self.use_jetstream = true;
        self
    }

    /// Disable JetStream (use core NATS).
    pub fn without_jetstream(mut self) -> Self {
        self.use_jetstream = false;
        self
    }

    /// Set the stream name.
    pub fn stream_name(mut self, name: impl Into<String>) -> Self {
        self.stream_name = name.into();
        self
    }

    /// Set the subject prefix.
    pub fn subject_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.subject_prefix = prefix.into();
        self
    }

    /// Set a durable consumer name.
    pub fn durable(mut self, name: impl Into<String>) -> Self {
        self.durable_name = Some(name.into());
        self
    }

    /// Set the consumer name.
    pub fn consumer_name(mut self, name: impl Into<String>) -> Self {
        self.consumer_name = Some(name.into());
        self
    }

    /// Set the ack wait timeout in milliseconds.
    pub fn ack_wait_ms(mut self, ms: u64) -> Self {
        self.ack_wait_ms = ms;
        self
    }

    /// Set the max ack pending limit.
    pub fn max_ack_pending(mut self, limit: u64) -> Self {
        self.max_ack_pending = limit;
        self
    }

    /// Set username/password credentials.
    pub fn with_user_pass(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.credentials = Some(NatsCredentials::UserPass {
            username: username.into(),
            password: password.into(),
        });
        self
    }

    /// Set token credentials.
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.credentials = Some(NatsCredentials::Token {
            token: token.into(),
        });
        self
    }

    /// Set NKey credentials.
    pub fn with_nkey(mut self, seed: impl Into<String>) -> Self {
        self.credentials = Some(NatsCredentials::NKey { seed: seed.into() });
        self
    }

    /// Set credentials file.
    pub fn with_creds_file(mut self, path: impl Into<String>) -> Self {
        self.credentials = Some(NatsCredentials::CredsFile { path: path.into() });
        self
    }

    /// Enable TLS.
    pub fn with_tls(mut self) -> Self {
        self.tls = true;
        self
    }
}

fn default_nats_prefix() -> String {
    "xerv.dispatch".to_string()
}

fn default_use_jetstream() -> bool {
    true
}

fn default_stream_name() -> String {
    "XERV_TRACES".to_string()
}

fn default_ack_wait() -> u64 {
    30_000 // 30 seconds
}

fn default_max_ack_pending() -> u64 {
    1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let config = NatsConfig::default();
        assert_eq!(config.url, "nats://localhost:4222");
        assert!(config.use_jetstream);
        assert_eq!(config.stream_name, "XERV_TRACES");
    }

    #[test]
    fn builder_pattern() {
        let config = NatsConfig::new("nats://nats:4222")
            .with_jetstream()
            .stream_name("MY_STREAM")
            .durable("worker-1");

        assert!(config.use_jetstream);
        assert_eq!(config.stream_name, "MY_STREAM");
        assert_eq!(config.durable_name, Some("worker-1".to_string()));
    }

    #[test]
    fn credentials_user_pass() {
        let config = NatsConfig::new("nats://nats:4222").with_user_pass("user", "pass");

        assert!(matches!(
            config.credentials,
            Some(NatsCredentials::UserPass { .. })
        ));
    }

    #[test]
    fn credentials_token() {
        let config = NatsConfig::new("nats://nats:4222").with_token("secret-token");

        assert!(matches!(
            config.credentials,
            Some(NatsCredentials::Token { .. })
        ));
    }
}
