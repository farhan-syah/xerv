//! Core suspension types for human-in-the-loop workflows.
//!
//! This module provides the fundamental types for trace suspension,
//! which are used by nodes to signal that execution should pause
//! until an external action is taken.

use serde::{Deserialize, Serialize};

/// Action to take when a suspension times out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TimeoutAction {
    /// Auto-approve and continue execution on "out" port.
    Approve,
    /// Auto-reject and emit to "rejected" port.
    #[default]
    Reject,
    /// Escalate and emit to "escalated" port.
    Escalate,
}

impl TimeoutAction {
    /// Get the output port name for this timeout action.
    pub fn output_port(&self) -> &'static str {
        match self {
            Self::Approve => "out",
            Self::Reject => "rejected",
            Self::Escalate => "escalated",
        }
    }
}

/// A request from a node to suspend trace execution.
///
/// This is returned by nodes (like `WaitNode`) that need to pause
/// execution until an external action is taken (human approval,
/// webhook callback, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspensionRequest {
    /// Unique hook ID for this suspension.
    ///
    /// This ID is used to resume the trace via the API.
    /// It should be unique within the pipeline context.
    pub hook_id: String,

    /// Timeout in seconds (None = no timeout).
    pub timeout_secs: Option<u64>,

    /// Action to take on timeout.
    pub timeout_action: TimeoutAction,

    /// Additional metadata for the approval UI.
    ///
    /// This can include information about what's being approved,
    /// links to relevant data, approver information, etc.
    pub metadata: serde_json::Value,
}

impl SuspensionRequest {
    /// Create a new suspension request with the given hook ID.
    pub fn new(hook_id: impl Into<String>) -> Self {
        Self {
            hook_id: hook_id.into(),
            timeout_secs: None,
            timeout_action: TimeoutAction::default(),
            metadata: serde_json::Value::Null,
        }
    }

    /// Set the timeout.
    pub fn with_timeout(mut self, secs: u64, action: TimeoutAction) -> Self {
        self.timeout_secs = Some(secs);
        self.timeout_action = action;
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspension_request_builder() {
        let req = SuspensionRequest::new("test-hook")
            .with_timeout(300, TimeoutAction::Approve)
            .with_metadata(serde_json::json!({"key": "value"}));

        assert_eq!(req.hook_id, "test-hook");
        assert_eq!(req.timeout_secs, Some(300));
        assert_eq!(req.timeout_action, TimeoutAction::Approve);
    }

    #[test]
    fn timeout_action_output_ports() {
        assert_eq!(TimeoutAction::Approve.output_port(), "out");
        assert_eq!(TimeoutAction::Reject.output_port(), "rejected");
        assert_eq!(TimeoutAction::Escalate.output_port(), "escalated");
    }

    #[test]
    fn timeout_action_default_is_reject() {
        assert_eq!(TimeoutAction::default(), TimeoutAction::Reject);
    }
}
