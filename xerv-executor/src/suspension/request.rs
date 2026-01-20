//! Suspension request and state types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use xerv_core::types::{NodeId, TraceId};

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

/// A request from a node to suspend trace execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspensionRequest {
    /// Unique hook ID for this suspension.
    pub hook_id: String,
    /// Timeout in seconds (None = no timeout).
    pub timeout_secs: Option<u64>,
    /// Action to take on timeout.
    pub timeout_action: TimeoutAction,
    /// Additional metadata for the approval UI.
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

/// State of a suspended trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspendedTraceState {
    /// The hook ID that triggered this suspension.
    pub hook_id: String,
    /// The trace that was suspended.
    pub trace_id: TraceId,
    /// The node where execution was suspended.
    pub suspended_at: NodeId,
    /// Path to the arena file for this trace.
    pub arena_path: PathBuf,
    /// When the suspension was created.
    pub created_at: DateTime<Utc>,
    /// When this suspension expires (None = never).
    pub expires_at: Option<DateTime<Utc>>,
    /// Action to take on timeout.
    pub timeout_action: TimeoutAction,
    /// Metadata associated with the suspension.
    pub metadata: serde_json::Value,
    /// The pipeline ID owning this trace.
    pub pipeline_id: String,
}

impl SuspendedTraceState {
    /// Create a new suspended trace state.
    pub fn new(
        hook_id: impl Into<String>,
        trace_id: TraceId,
        suspended_at: NodeId,
        arena_path: PathBuf,
        pipeline_id: impl Into<String>,
    ) -> Self {
        Self {
            hook_id: hook_id.into(),
            trace_id,
            suspended_at,
            arena_path,
            created_at: Utc::now(),
            expires_at: None,
            timeout_action: TimeoutAction::default(),
            metadata: serde_json::Value::Null,
            pipeline_id: pipeline_id.into(),
        }
    }

    /// Set expiration time.
    pub fn with_expiry(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Set expiration from timeout seconds.
    pub fn with_timeout(mut self, secs: u64, action: TimeoutAction) -> Self {
        self.expires_at = Some(Utc::now() + chrono::Duration::seconds(secs as i64));
        self.timeout_action = action;
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Check if this suspension has expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| Utc::now() >= exp)
            .unwrap_or(false)
    }

    /// Get time until expiration (None if no expiry or already expired).
    pub fn time_until_expiry(&self) -> Option<std::time::Duration> {
        self.expires_at.and_then(|exp| {
            let now = Utc::now();
            if now >= exp {
                None
            } else {
                Some(std::time::Duration::from_secs(
                    (exp - now).num_seconds() as u64
                ))
            }
        })
    }
}

/// Decision made when resuming a suspended trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResumeDecision {
    /// Approve and continue on "out" port.
    Approve {
        /// Optional response data to pass to the next node.
        response_data: Option<serde_json::Value>,
    },
    /// Reject and emit to "rejected" port.
    Reject {
        /// Optional reason for rejection.
        reason: Option<String>,
    },
    /// Escalate and emit to "escalated" port.
    Escalate {
        /// Optional escalation details.
        details: Option<serde_json::Value>,
    },
}

impl ResumeDecision {
    /// Create an approval decision.
    pub fn approve() -> Self {
        Self::Approve {
            response_data: None,
        }
    }

    /// Create an approval decision with response data.
    pub fn approve_with_data(data: serde_json::Value) -> Self {
        Self::Approve {
            response_data: Some(data),
        }
    }

    /// Create a rejection decision.
    pub fn reject() -> Self {
        Self::Reject { reason: None }
    }

    /// Create a rejection decision with reason.
    pub fn reject_with_reason(reason: impl Into<String>) -> Self {
        Self::Reject {
            reason: Some(reason.into()),
        }
    }

    /// Create an escalation decision.
    pub fn escalate() -> Self {
        Self::Escalate { details: None }
    }

    /// Create an escalation decision with details.
    pub fn escalate_with_details(details: serde_json::Value) -> Self {
        Self::Escalate {
            details: Some(details),
        }
    }

    /// Get the output port name for this decision.
    pub fn output_port(&self) -> &'static str {
        match self {
            Self::Approve { .. } => "out",
            Self::Reject { .. } => "rejected",
            Self::Escalate { .. } => "escalated",
        }
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
    fn suspended_state_expiry() {
        let state = SuspendedTraceState::new(
            "hook",
            TraceId::new(),
            NodeId::new(1),
            PathBuf::from("/tmp/test.bin"),
            "pipeline@v1",
        )
        .with_timeout(0, TimeoutAction::Reject);

        // Should be expired immediately
        assert!(state.is_expired());
    }

    #[test]
    fn suspended_state_not_expired() {
        let state = SuspendedTraceState::new(
            "hook",
            TraceId::new(),
            NodeId::new(1),
            PathBuf::from("/tmp/test.bin"),
            "pipeline@v1",
        )
        .with_timeout(3600, TimeoutAction::Reject);

        assert!(!state.is_expired());
        assert!(state.time_until_expiry().is_some());
    }

    #[test]
    fn resume_decision_ports() {
        assert_eq!(ResumeDecision::approve().output_port(), "out");
        assert_eq!(ResumeDecision::reject().output_port(), "rejected");
        assert_eq!(ResumeDecision::escalate().output_port(), "escalated");
    }
}
