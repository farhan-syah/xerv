//! Authentication and authorization module.
//!
//! Provides API key-based authentication with scope-based authorization.

mod api_key;
mod config;
mod context;
mod middleware;

pub use api_key::{ApiKeyBuilder, ApiKeyHash, ApiKeyValidator};
pub use config::{ApiKeyConfig, ApiKeyEntry, AuthConfig};
pub use context::{AuthContext, AuthScope};
pub use middleware::{AuthMiddleware, HeaderAccess, SimpleHeaders};
