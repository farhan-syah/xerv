//! Wait node (human-in-the-loop).
//!
//! Pauses execution until an external signal is received.
//! Enables approval workflows and manual intervention patterns.

use std::collections::HashMap;
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput, Port, PortDirection};
use xerv_core::types::RelPtr;
use xerv_core::value::Value;

/// Configuration for how the wait state is persisted.
#[derive(Debug, Clone, Default)]
pub enum WaitPersistence {
    /// Store wait state in memory (development only).
    #[default]
    Memory,
    /// Store wait state in Redis.
    Redis {
        /// Key prefix for Redis storage.
        key_prefix: String,
        /// TTL for the wait state (seconds).
        ttl_seconds: u64,
    },
    /// Store wait state in a database.
    Database {
        /// Table name for wait states.
        table: String,
    },
}

/// Configuration for how the wait is resumed.
#[derive(Debug, Clone, Default)]
pub enum ResumeMethod {
    /// Resume via webhook callback.
    #[default]
    Webhook,
    /// Resume via API call with approval token.
    ApiApproval {
        /// Token required for approval.
        token_field: String,
    },
    /// Resume after a timeout (auto-approve or auto-reject).
    Timeout {
        /// Timeout duration in seconds.
        seconds: u64,
        /// What happens on timeout.
        on_timeout: TimeoutAction,
    },
}

/// Action to take when a timeout occurs.
#[derive(Debug, Clone, Copy, Default)]
pub enum TimeoutAction {
    /// Auto-approve and continue.
    Approve,
    /// Auto-reject and emit to error port.
    #[default]
    Reject,
    /// Escalate (emit to escalate port).
    Escalate,
}

/// Wait node - human-in-the-loop.
///
/// Pauses flow execution until an external signal (webhook, API call, etc.)
/// is received. The current state is checkpointed and can be resumed later.
///
/// # Ports
/// - Input: "in" - Data to preserve during wait
/// - Output: "out" - Emitted when approved/resumed
/// - Output: "rejected" - Emitted when rejected
/// - Output: "escalated" - Emitted when escalated
/// - Output: "error" - Emitted on errors
///
/// # Example Configuration
/// ```yaml
/// nodes:
///   await_approval:
///     type: std::wait
///     config:
///       hook_id: order_approval_${trace_id}
///       persistence: redis
///       resume_method: webhook
///       timeout_seconds: 86400  # 24 hours
///       on_timeout: reject
///       metadata:
///         approver_email: ${config.approval_email}
///         order_id: ${order.id}
///     inputs:
///       - from: validate_order.out -> in
///     outputs:
///       out: -> process_order.in
///       rejected: -> notify_rejection.in
/// ```
#[derive(Debug)]
pub struct WaitNode {
    /// Unique identifier for this wait hook.
    hook_id: String,
    /// How to persist the wait state.
    persistence: WaitPersistence,
    /// How the wait can be resumed.
    resume_method: ResumeMethod,
    /// Optional metadata to include with the wait notification.
    metadata_fields: Vec<String>,
}

impl WaitNode {
    /// Create a wait node with the given hook ID.
    pub fn new(hook_id: impl Into<String>) -> Self {
        Self {
            hook_id: hook_id.into(),
            persistence: WaitPersistence::Memory,
            resume_method: ResumeMethod::Webhook,
            metadata_fields: Vec::new(),
        }
    }

    /// Create a wait node with webhook resumption.
    pub fn webhook(hook_id: impl Into<String>) -> Self {
        Self::new(hook_id)
    }

    /// Create a wait node with Redis persistence.
    pub fn with_redis(
        hook_id: impl Into<String>,
        key_prefix: impl Into<String>,
        ttl_seconds: u64,
    ) -> Self {
        Self {
            hook_id: hook_id.into(),
            persistence: WaitPersistence::Redis {
                key_prefix: key_prefix.into(),
                ttl_seconds,
            },
            resume_method: ResumeMethod::Webhook,
            metadata_fields: Vec::new(),
        }
    }

    /// Set the persistence method.
    pub fn with_persistence(mut self, persistence: WaitPersistence) -> Self {
        self.persistence = persistence;
        self
    }

    /// Set the resume method.
    pub fn with_resume_method(mut self, method: ResumeMethod) -> Self {
        self.resume_method = method;
        self
    }

    /// Add a timeout with auto-action.
    pub fn with_timeout(mut self, seconds: u64, on_timeout: TimeoutAction) -> Self {
        self.resume_method = ResumeMethod::Timeout {
            seconds,
            on_timeout,
        };
        self
    }

    /// Add metadata fields to extract from input.
    pub fn with_metadata(mut self, fields: Vec<String>) -> Self {
        self.metadata_fields = fields;
        self
    }

    /// Extract metadata from input for the wait notification.
    fn extract_metadata(&self, input: &Value) -> serde_json::Map<String, serde_json::Value> {
        let mut metadata = serde_json::Map::new();
        for field in &self.metadata_fields {
            if let Some(value) = input.get_field(field) {
                let key = field.split('.').next_back().unwrap_or(field);
                metadata.insert(key.to_string(), value.into_inner());
            }
        }
        metadata
    }
}

impl Node for WaitNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("std", "wait")
            .with_description("Pause execution for human-in-the-loop approval")
            .with_inputs(vec![Port::input("Any")])
            .with_outputs(vec![
                Port::named("out", PortDirection::Output, "Any")
                    .with_description("Emitted when approved"),
                Port::named("rejected", PortDirection::Output, "Any")
                    .with_description("Emitted when rejected"),
                Port::named("escalated", PortDirection::Output, "Any")
                    .with_description("Emitted when escalated"),
                Port::error(),
            ])
    }

    fn execute<'a>(&'a self, ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a> {
        Box::pin(async move {
            let input = inputs.get("in").copied().unwrap_or_else(RelPtr::null);

            // Read input for metadata extraction
            let value = if input.is_null() {
                Value::null()
            } else {
                match ctx.read_bytes(input) {
                    Ok(bytes) => Value::from_bytes(&bytes).unwrap_or_else(|_| Value::null()),
                    Err(_) => Value::null(),
                }
            };

            // Extract metadata
            let metadata = self.extract_metadata(&value);

            // In a full implementation, this would:
            // 1. Flush the arena to disk
            // 2. Write trace_id + hook_id to persistence store
            // 3. Send notification (email, Slack, etc.) with approval links
            // 4. Return a "waiting" status to the executor
            // 5. The executor would unload the trace from memory
            //
            // Resumption would be handled by:
            // 1. External webhook/API call with hook_id
            // 2. Executor loads arena back from disk
            // 3. Replays from WAL to get to this node
            // 4. Continues execution based on approval decision

            tracing::info!(
                hook_id = %self.hook_id,
                trace_id = %ctx.trace_id(),
                persistence = ?self.persistence,
                resume_method = ?self.resume_method,
                metadata = ?metadata,
                "Wait: pausing for approval"
            );

            // For now, we simulate immediate approval for testing
            // In production, this would return a "waiting" signal
            // that the executor handles specially.
            //
            // The wait node output includes:
            // - Original input data
            // - Wait state information
            // - Metadata for the approval UI
            let wait_state = Value::from(serde_json::json!({
                "hook_id": self.hook_id,
                "trace_id": ctx.trace_id().to_string(),
                "status": "waiting",
                "metadata": metadata,
                "resume_url": format!("/api/v1/resume/{}", self.hook_id)
            }));

            // Write wait state to arena
            let state_bytes = match wait_state.to_bytes() {
                Ok(bytes) => bytes,
                Err(e) => {
                    return Ok(NodeOutput::error_with_message(format!(
                        "Failed to serialize wait state: {}",
                        e
                    )));
                }
            };

            let state_ptr = match ctx.write_bytes(&state_bytes) {
                Ok(ptr) => ptr,
                Err(e) => {
                    return Ok(NodeOutput::error_with_message(format!(
                        "Failed to write wait state: {}",
                        e
                    )));
                }
            };

            // In a real implementation, we would return a special "waiting" output
            // that tells the executor to suspend this trace.
            // For now, we emit on "out" to simulate immediate approval.
            Ok(NodeOutput::out(state_ptr))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wait_node_info() {
        let node = WaitNode::new("test_hook");
        let info = node.info();

        assert_eq!(info.name, "std::wait");
        assert_eq!(info.inputs.len(), 1);
        assert_eq!(info.outputs.len(), 4);
        assert_eq!(info.outputs[0].name, "out");
        assert_eq!(info.outputs[1].name, "rejected");
        assert_eq!(info.outputs[2].name, "escalated");
        assert_eq!(info.outputs[3].name, "error");
    }

    #[test]
    fn wait_persistence_default() {
        let persistence = WaitPersistence::default();
        assert!(matches!(persistence, WaitPersistence::Memory));
    }

    #[test]
    fn wait_resume_method_default() {
        let method = ResumeMethod::default();
        assert!(matches!(method, ResumeMethod::Webhook));
    }

    #[test]
    fn wait_timeout_action_default() {
        let action = TimeoutAction::default();
        assert!(matches!(action, TimeoutAction::Reject));
    }

    #[test]
    fn wait_with_redis() {
        let node = WaitNode::with_redis("hook", "xerv:wait", 3600);
        assert!(matches!(
            node.persistence,
            WaitPersistence::Redis {
                ttl_seconds: 3600,
                ..
            }
        ));
    }

    #[test]
    fn wait_with_timeout() {
        let node = WaitNode::new("hook").with_timeout(300, TimeoutAction::Approve);
        assert!(matches!(
            node.resume_method,
            ResumeMethod::Timeout {
                seconds: 300,
                on_timeout: TimeoutAction::Approve
            }
        ));
    }

    #[test]
    fn wait_extract_metadata() {
        let node = WaitNode::new("hook")
            .with_metadata(vec!["user.name".to_string(), "order.id".to_string()]);

        let input = Value::from(json!({
            "user": {"name": "Alice"},
            "order": {"id": 12345}
        }));

        let metadata = node.extract_metadata(&input);

        assert_eq!(
            metadata.get("name"),
            Some(&serde_json::Value::String("Alice".to_string()))
        );
        assert_eq!(
            metadata.get("id"),
            Some(&serde_json::Value::Number(12345.into()))
        );
    }

    #[test]
    fn wait_builder_chain() {
        let node = WaitNode::new("approval_hook")
            .with_persistence(WaitPersistence::Redis {
                key_prefix: "xerv".to_string(),
                ttl_seconds: 86400,
            })
            .with_metadata(vec!["order_id".to_string()]);

        assert_eq!(node.hook_id, "approval_hook");
        assert!(matches!(node.persistence, WaitPersistence::Redis { .. }));
        assert_eq!(node.metadata_fields.len(), 1);
    }
}
