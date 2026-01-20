//! Dispatch backend configuration.
//!
//! This module provides configuration types for all dispatch backends:
//!
//! - [`MemoryConfig`] - In-memory dispatch (single-node, no persistence)
//! - [`RaftConfig`] - Raft consensus (multi-node, no external dependencies)
//! - [`RedisConfig`] - Redis-based dispatch (high throughput)
//! - [`NatsConfig`] - NATS-based dispatch (streaming, multi-region)
//!
//! # Example
//!
//! ```ignore
//! use xerv_core::dispatch::{DispatchConfig, RedisConfig};
//!
//! // Use environment variables
//! let config = DispatchConfig::from_env_or_default();
//!
//! // Or construct programmatically (requires dispatch-redis feature)
//! let config = DispatchConfig::redis(
//!     RedisConfig::new("redis://localhost:6379").with_streams()
//! );
//! ```

mod memory;
mod nats;
mod raft;
mod redis;

pub use memory::MemoryConfig;
pub use nats::{NatsConfig, NatsCredentials};
pub use raft::{RaftConfig, RaftConfigBuilder, RaftPeer};
pub use redis::RedisConfig;

use serde::{Deserialize, Serialize};

/// Configuration for the dispatch backend.
///
/// This enum determines which backend is used for trace dispatch.
/// The default is `Memory` for single-node deployments.
///
/// # Backend Selection
///
/// | Backend | Use Case | Dependencies |
/// |---------|----------|--------------|
/// | Memory | Development, testing, single-node | None |
/// | Raft | Edge, air-gapped, simple HA | None |
/// | Redis | High-throughput cloud | Redis 6+ |
/// | NATS | Multi-region, event-driven | NATS 2.2+ |
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum DispatchConfig {
    /// In-memory dispatch (single-node, no persistence).
    /// Best for development and testing.
    Memory(MemoryConfig),

    /// Built-in Raft consensus (multi-node, no external dependencies).
    /// Best for edge, air-gapped, and simple production deployments.
    #[cfg(feature = "dispatch-raft")]
    Raft(RaftConfig),

    /// Redis-based dispatch (high throughput, requires external Redis).
    /// Best for cloud-native, high-throughput deployments.
    #[cfg(feature = "dispatch-redis")]
    Redis(RedisConfig),

    /// NATS-based dispatch (streaming, requires external NATS).
    /// Best for multi-region, event-driven deployments.
    #[cfg(feature = "dispatch-nats")]
    Nats(NatsConfig),
}

impl Default for DispatchConfig {
    fn default() -> Self {
        Self::Memory(MemoryConfig::default())
    }
}

impl DispatchConfig {
    /// Create a memory dispatch config (for testing/development).
    pub fn memory() -> Self {
        Self::Memory(MemoryConfig::default())
    }

    /// Create a memory dispatch config with custom settings.
    pub fn memory_with(config: MemoryConfig) -> Self {
        Self::Memory(config)
    }

    /// Create a Raft dispatch config.
    #[cfg(feature = "dispatch-raft")]
    pub fn raft(config: RaftConfig) -> Self {
        Self::Raft(config)
    }

    /// Create a Redis dispatch config.
    #[cfg(feature = "dispatch-redis")]
    pub fn redis(config: RedisConfig) -> Self {
        Self::Redis(config)
    }

    /// Create a NATS dispatch config.
    #[cfg(feature = "dispatch-nats")]
    pub fn nats(config: NatsConfig) -> Self {
        Self::Nats(config)
    }

    /// Get the backend name.
    pub fn backend_name(&self) -> &'static str {
        match self {
            Self::Memory(_) => "memory",
            #[cfg(feature = "dispatch-raft")]
            Self::Raft(_) => "raft",
            #[cfg(feature = "dispatch-redis")]
            Self::Redis(_) => "redis",
            #[cfg(feature = "dispatch-nats")]
            Self::Nats(_) => "nats",
        }
    }

    /// Get the memory configuration.
    ///
    /// Returns the configured `MemoryConfig` if this is a Memory dispatch,
    /// otherwise returns a default configuration for fallback use.
    pub fn memory_config(&self) -> MemoryConfig {
        match self {
            Self::Memory(config) => config.clone(),
            #[cfg(feature = "dispatch-raft")]
            Self::Raft(_) => MemoryConfig::default(),
            #[cfg(feature = "dispatch-redis")]
            Self::Redis(_) => MemoryConfig::default(),
            #[cfg(feature = "dispatch-nats")]
            Self::Nats(_) => MemoryConfig::default(),
        }
    }

    /// Create a dispatch config from environment variables.
    ///
    /// # Environment Variables
    ///
    /// - `XERV_DISPATCH_BACKEND`: Backend type (`memory`, `raft`, `redis`, `nats`)
    /// - `XERV_DISPATCH_REDIS_URL`: Redis connection URL (for redis backend)
    /// - `XERV_DISPATCH_NATS_URL`: NATS connection URL (for nats backend)
    /// - `XERV_DISPATCH_RAFT_NODE_ID`: Node ID (for raft backend)
    ///
    /// # Returns
    ///
    /// Returns `None` if `XERV_DISPATCH_BACKEND` is not set, otherwise returns
    /// the configured dispatch config or falls back to memory if the backend
    /// is not available.
    pub fn from_env() -> Option<Self> {
        let backend = std::env::var("XERV_DISPATCH_BACKEND").ok()?;

        Some(match backend.to_lowercase().as_str() {
            "memory" => Self::Memory(MemoryConfig::default()),

            #[cfg(feature = "dispatch-raft")]
            "raft" => {
                let node_id = std::env::var("XERV_DISPATCH_RAFT_NODE_ID")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                Self::Raft(RaftConfig::new(node_id))
            }
            #[cfg(not(feature = "dispatch-raft"))]
            "raft" => {
                tracing::warn!(
                    "Raft backend requested but dispatch-raft feature not enabled, falling back to memory"
                );
                Self::Memory(MemoryConfig::default())
            }

            #[cfg(feature = "dispatch-redis")]
            "redis" => {
                let url = std::env::var("XERV_DISPATCH_REDIS_URL")
                    .unwrap_or_else(|_| "redis://localhost:6379".to_string());
                Self::Redis(RedisConfig::new(&url).use_streams())
            }
            #[cfg(not(feature = "dispatch-redis"))]
            "redis" => {
                tracing::warn!(
                    "Redis backend requested but dispatch-redis feature not enabled, falling back to memory"
                );
                Self::Memory(MemoryConfig::default())
            }

            #[cfg(feature = "dispatch-nats")]
            "nats" => {
                let url = std::env::var("XERV_DISPATCH_NATS_URL")
                    .unwrap_or_else(|_| "nats://localhost:4222".to_string());
                Self::Nats(NatsConfig::new(&url).with_jetstream())
            }
            #[cfg(not(feature = "dispatch-nats"))]
            "nats" => {
                tracing::warn!(
                    "NATS backend requested but dispatch-nats feature not enabled, falling back to memory"
                );
                Self::Memory(MemoryConfig::default())
            }

            unknown => {
                tracing::warn!(backend = %unknown, "Unknown dispatch backend, falling back to memory");
                Self::Memory(MemoryConfig::default())
            }
        })
    }

    /// Create a dispatch config from environment variables, or use the default.
    ///
    /// This is a convenience method that returns `from_env()` if environment
    /// variables are set, otherwise returns the default (memory) config.
    pub fn from_env_or_default() -> Self {
        Self::from_env().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_memory() {
        let config = DispatchConfig::default();
        assert_eq!(config.backend_name(), "memory");
    }

    #[test]
    fn config_serialization() {
        let config = DispatchConfig::Memory(MemoryConfig::default());
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("memory"));

        let parsed: DispatchConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.backend_name(), "memory");
    }

    #[test]
    fn from_env_returns_none_when_not_set() {
        // Clear the env var if it exists
        // SAFETY: Tests run single-threaded with --test-threads=1 or serially
        unsafe { std::env::remove_var("XERV_DISPATCH_BACKEND") };
        assert!(DispatchConfig::from_env().is_none());
    }

    #[test]
    fn from_env_or_default_returns_memory() {
        // SAFETY: Tests run single-threaded with --test-threads=1 or serially
        unsafe { std::env::remove_var("XERV_DISPATCH_BACKEND") };
        let config = DispatchConfig::from_env_or_default();
        assert_eq!(config.backend_name(), "memory");
    }

    #[test]
    fn from_env_memory_backend() {
        // SAFETY: Tests run single-threaded with --test-threads=1 or serially
        unsafe { std::env::set_var("XERV_DISPATCH_BACKEND", "memory") };
        let config = DispatchConfig::from_env().unwrap();
        assert_eq!(config.backend_name(), "memory");
        unsafe { std::env::remove_var("XERV_DISPATCH_BACKEND") };
    }

    #[test]
    fn from_env_unknown_backend_falls_back_to_memory() {
        // SAFETY: Tests run single-threaded with --test-threads=1 or serially
        unsafe { std::env::set_var("XERV_DISPATCH_BACKEND", "unknown_backend") };
        let config = DispatchConfig::from_env().unwrap();
        assert_eq!(config.backend_name(), "memory");
        unsafe { std::env::remove_var("XERV_DISPATCH_BACKEND") };
    }
}
