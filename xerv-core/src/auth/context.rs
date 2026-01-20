//! Authentication context and scope definitions.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Authorization scopes for API access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthScope {
    /// Read pipeline information.
    PipelineRead,
    /// Create, modify, or delete pipelines.
    PipelineWrite,
    /// Read trace information.
    TraceRead,
    /// Create or modify traces.
    TraceWrite,
    /// Resume suspended traces.
    TraceResume,
    /// Administrative access.
    Admin,
}

impl AuthScope {
    /// Get all available scopes.
    pub fn all() -> HashSet<Self> {
        let mut scopes = HashSet::new();
        scopes.insert(Self::PipelineRead);
        scopes.insert(Self::PipelineWrite);
        scopes.insert(Self::TraceRead);
        scopes.insert(Self::TraceWrite);
        scopes.insert(Self::TraceResume);
        scopes.insert(Self::Admin);
        scopes
    }

    /// Get read-only scopes.
    pub fn read_only() -> HashSet<Self> {
        let mut scopes = HashSet::new();
        scopes.insert(Self::PipelineRead);
        scopes.insert(Self::TraceRead);
        scopes
    }

    /// Get standard operator scopes (read + trace resume).
    pub fn operator() -> HashSet<Self> {
        let mut scopes = Self::read_only();
        scopes.insert(Self::TraceResume);
        scopes
    }

    /// Parse scope from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "pipeline:read" | "pipeline_read" => Some(Self::PipelineRead),
            "pipeline:write" | "pipeline_write" => Some(Self::PipelineWrite),
            "trace:read" | "trace_read" => Some(Self::TraceRead),
            "trace:write" | "trace_write" => Some(Self::TraceWrite),
            "trace:resume" | "trace_resume" => Some(Self::TraceResume),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    /// Convert to string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PipelineRead => "pipeline:read",
            Self::PipelineWrite => "pipeline:write",
            Self::TraceRead => "trace:read",
            Self::TraceWrite => "trace:write",
            Self::TraceResume => "trace:resume",
            Self::Admin => "admin",
        }
    }
}

/// Authentication context for a validated request.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// Identity of the authenticated client.
    pub identity: String,
    /// Scopes granted to this client.
    pub scopes: HashSet<AuthScope>,
}

impl AuthContext {
    /// Create a new authentication context.
    pub fn new(identity: impl Into<String>, scopes: HashSet<AuthScope>) -> Self {
        Self {
            identity: identity.into(),
            scopes,
        }
    }

    /// Create an anonymous context (for exempt paths).
    pub fn anonymous() -> Self {
        Self {
            identity: "anonymous".to_string(),
            scopes: HashSet::new(),
        }
    }

    /// Check if this context has a specific scope.
    pub fn has_scope(&self, scope: AuthScope) -> bool {
        self.scopes.contains(&AuthScope::Admin) || self.scopes.contains(&scope)
    }

    /// Check if this context has all the required scopes.
    pub fn has_all_scopes(&self, scopes: &[AuthScope]) -> bool {
        if self.scopes.contains(&AuthScope::Admin) {
            return true;
        }
        scopes.iter().all(|s| self.scopes.contains(s))
    }

    /// Check if this context has any of the required scopes.
    pub fn has_any_scope(&self, scopes: &[AuthScope]) -> bool {
        if self.scopes.contains(&AuthScope::Admin) {
            return true;
        }
        scopes.iter().any(|s| self.scopes.contains(s))
    }

    /// Require a scope, returning an error if not present.
    pub fn require_scope(&self, scope: AuthScope) -> crate::error::Result<()> {
        if self.has_scope(scope) {
            Ok(())
        } else {
            Err(crate::error::XervError::AuthorizationDenied {
                identity: self.identity.clone(),
                required_scope: scope.as_str().to_string(),
                cause: format!(
                    "Identity '{}' does not have scope '{}'",
                    self.identity,
                    scope.as_str()
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_parsing() {
        assert_eq!(
            AuthScope::parse("pipeline:read"),
            Some(AuthScope::PipelineRead)
        );
        assert_eq!(
            AuthScope::parse("pipeline_read"),
            Some(AuthScope::PipelineRead)
        );
        assert_eq!(AuthScope::parse("admin"), Some(AuthScope::Admin));
        assert_eq!(AuthScope::parse("unknown"), None);
    }

    #[test]
    fn context_scope_check() {
        let ctx = AuthContext::new(
            "test",
            vec![AuthScope::PipelineRead, AuthScope::TraceRead]
                .into_iter()
                .collect(),
        );

        assert!(ctx.has_scope(AuthScope::PipelineRead));
        assert!(ctx.has_scope(AuthScope::TraceRead));
        assert!(!ctx.has_scope(AuthScope::PipelineWrite));
    }

    #[test]
    fn admin_has_all_scopes() {
        let ctx = AuthContext::new("admin", vec![AuthScope::Admin].into_iter().collect());

        assert!(ctx.has_scope(AuthScope::PipelineRead));
        assert!(ctx.has_scope(AuthScope::PipelineWrite));
        assert!(ctx.has_scope(AuthScope::TraceRead));
        assert!(ctx.has_scope(AuthScope::Admin));
    }

    #[test]
    fn require_scope_success() {
        let ctx = AuthContext::new("test", vec![AuthScope::PipelineRead].into_iter().collect());

        assert!(ctx.require_scope(AuthScope::PipelineRead).is_ok());
    }

    #[test]
    fn require_scope_failure() {
        let ctx = AuthContext::new("test", vec![AuthScope::PipelineRead].into_iter().collect());

        assert!(ctx.require_scope(AuthScope::PipelineWrite).is_err());
    }
}
