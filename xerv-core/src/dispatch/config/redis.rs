//! Redis-based dispatch configuration.

use serde::{Deserialize, Serialize};

/// Configuration for Redis-based dispatch.
///
/// Uses Redis for high-throughput trace dispatch with optional Redis Streams
/// for reliable delivery. Best for cloud-native, high-throughput deployments.
///
/// # Example
///
/// ```
/// use xerv_core::dispatch::RedisConfig;
///
/// let config = RedisConfig::new("redis://redis-cluster:6379")
///     .with_streams()
///     .pool_size(20)
///     .consumer_group("my-workers");
///
/// assert!(config.use_streams);
/// assert_eq!(config.pool_size, 20);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    /// Redis URL (e.g., "redis://localhost:6379").
    pub url: String,

    /// Connection pool size.
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,

    /// Queue key prefix.
    #[serde(default = "default_redis_prefix")]
    pub key_prefix: String,

    /// Use Redis Streams (reliable delivery) vs List (simple FIFO).
    #[serde(default = "default_use_streams")]
    pub use_streams: bool,

    /// Consumer group name (for streams).
    #[serde(default = "default_consumer_group")]
    pub consumer_group: String,

    /// Consumer name within the group.
    #[serde(default)]
    pub consumer_name: Option<String>,

    /// Message timeout in milliseconds (for claiming stale messages).
    #[serde(default = "default_message_timeout")]
    pub message_timeout_ms: u64,

    /// Enable TLS.
    #[serde(default)]
    pub tls: bool,

    /// TLS CA certificate path.
    #[serde(default)]
    pub tls_ca_cert: Option<String>,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: "redis://localhost:6379".to_string(),
            pool_size: default_pool_size(),
            key_prefix: default_redis_prefix(),
            use_streams: default_use_streams(),
            consumer_group: default_consumer_group(),
            consumer_name: None,
            message_timeout_ms: default_message_timeout(),
            tls: false,
            tls_ca_cert: None,
        }
    }
}

impl RedisConfig {
    /// Create a new Redis config with the given URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    /// Enable Redis Streams for reliable delivery.
    pub fn with_streams(mut self) -> Self {
        self.use_streams = true;
        self
    }

    /// Disable Redis Streams (use simple lists).
    pub fn without_streams(mut self) -> Self {
        self.use_streams = false;
        self
    }

    /// Alias for `with_streams()` for API consistency.
    pub fn use_streams(self) -> Self {
        self.with_streams()
    }

    /// Set the consumer group name.
    pub fn consumer_group(mut self, group: impl Into<String>) -> Self {
        self.consumer_group = group.into();
        self
    }

    /// Set the consumer name.
    pub fn consumer_name(mut self, name: impl Into<String>) -> Self {
        self.consumer_name = Some(name.into());
        self
    }

    /// Set the pool size.
    pub fn pool_size(mut self, size: usize) -> Self {
        self.pool_size = size;
        self
    }

    /// Set the key prefix.
    pub fn key_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.key_prefix = prefix.into();
        self
    }

    /// Set the message timeout in milliseconds.
    pub fn message_timeout_ms(mut self, ms: u64) -> Self {
        self.message_timeout_ms = ms;
        self
    }

    /// Enable TLS.
    pub fn with_tls(mut self) -> Self {
        self.tls = true;
        self
    }

    /// Set the TLS CA certificate path.
    pub fn tls_ca_cert(mut self, path: impl Into<String>) -> Self {
        self.tls_ca_cert = Some(path.into());
        self.tls = true;
        self
    }
}

fn default_pool_size() -> usize {
    10
}

fn default_redis_prefix() -> String {
    "xerv:dispatch".to_string()
}

fn default_use_streams() -> bool {
    true
}

fn default_consumer_group() -> String {
    "xerv-workers".to_string()
}

fn default_message_timeout() -> u64 {
    60_000 // 1 minute
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let config = RedisConfig::default();
        assert_eq!(config.url, "redis://localhost:6379");
        assert_eq!(config.pool_size, 10);
        assert!(config.use_streams);
    }

    #[test]
    fn builder_pattern() {
        let config = RedisConfig::new("redis://redis-cluster:6379")
            .with_streams()
            .pool_size(20)
            .consumer_group("my-workers");

        assert!(config.use_streams);
        assert_eq!(config.pool_size, 20);
        assert_eq!(config.consumer_group, "my-workers");
    }

    #[test]
    fn tls_configuration() {
        let config = RedisConfig::new("rediss://secure-redis:6379")
            .with_tls()
            .tls_ca_cert("/etc/ssl/ca.crt");

        assert!(config.tls);
        assert_eq!(config.tls_ca_cert, Some("/etc/ssl/ca.crt".to_string()));
    }
}
