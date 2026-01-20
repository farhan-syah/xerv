//! Timeout processor for suspended traces.

use super::request::{ResumeDecision, TimeoutAction};
use super::store::SuspensionStore;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

/// Message sent when a suspension times out.
#[derive(Debug, Clone)]
pub struct TimeoutEvent {
    /// The hook ID that timed out.
    pub hook_id: String,
    /// The trace ID that was suspended.
    pub trace_id: xerv_core::types::TraceId,
    /// The action to take.
    pub action: TimeoutAction,
    /// The pipeline ID.
    pub pipeline_id: String,
}

/// Background processor for handling suspension timeouts.
pub struct TimeoutProcessor {
    store: Arc<dyn SuspensionStore>,
    check_interval: Duration,
    running: Arc<AtomicBool>,
    event_tx: Option<mpsc::Sender<TimeoutEvent>>,
}

impl TimeoutProcessor {
    /// Create a new timeout processor.
    pub fn new(store: Arc<dyn SuspensionStore>) -> Self {
        Self {
            store,
            check_interval: Duration::from_secs(5),
            running: Arc::new(AtomicBool::new(false)),
            event_tx: None,
        }
    }

    /// Set the check interval.
    pub fn with_check_interval(mut self, interval: Duration) -> Self {
        self.check_interval = interval;
        self
    }

    /// Set the event sender for timeout notifications.
    pub fn with_event_sender(mut self, tx: mpsc::Sender<TimeoutEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Check if the processor is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Run the timeout processor.
    ///
    /// This method runs until `stop()` is called.
    pub async fn run(&self) {
        self.running.store(true, Ordering::SeqCst);
        tracing::info!(
            interval_secs = self.check_interval.as_secs(),
            "Timeout processor started"
        );

        while self.running.load(Ordering::SeqCst) {
            self.process_expired().await;
            tokio::time::sleep(self.check_interval).await;
        }

        tracing::info!("Timeout processor stopped");
    }

    /// Process all expired suspensions.
    async fn process_expired(&self) {
        let expired = self.store.get_expired();

        for (hook_id, state) in expired {
            tracing::info!(
                hook_id = %hook_id,
                trace_id = %state.trace_id,
                action = ?state.timeout_action,
                "Processing expired suspension"
            );

            // Determine the decision based on timeout action
            let decision = match state.timeout_action {
                TimeoutAction::Approve => ResumeDecision::approve(),
                TimeoutAction::Reject => {
                    ResumeDecision::reject_with_reason("Timeout: auto-rejected")
                }
                TimeoutAction::Escalate => ResumeDecision::escalate_with_details(
                    serde_json::json!({"reason": "Timeout: auto-escalated"}),
                ),
            };

            // Resume the trace with the timeout decision
            match self.store.resume(&hook_id, decision) {
                Ok(resumed_state) => {
                    // Send timeout event if channel is configured
                    if let Some(tx) = &self.event_tx {
                        let event = TimeoutEvent {
                            hook_id: hook_id.clone(),
                            trace_id: resumed_state.trace_id,
                            action: state.timeout_action,
                            pipeline_id: resumed_state.pipeline_id.clone(),
                        };

                        if tx.send(event).await.is_err() {
                            tracing::warn!(
                                hook_id = %hook_id,
                                "Failed to send timeout event - channel closed"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        hook_id = %hook_id,
                        error = %e,
                        "Failed to process expired suspension"
                    );
                }
            }
        }
    }

    /// Stop the timeout processor.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suspension::MemorySuspensionStore;
    use crate::suspension::SuspendedTraceState;
    use std::path::PathBuf;
    use xerv_core::types::NodeId;

    #[tokio::test]
    async fn process_expired_suspension() {
        let store = Arc::new(MemorySuspensionStore::new());

        // Create an already-expired suspension
        let state = SuspendedTraceState::new(
            "timeout-hook",
            xerv_core::types::TraceId::new(),
            NodeId::new(1),
            PathBuf::from("/tmp/test.bin"),
            "test-pipeline@v1",
        )
        .with_timeout(0, TimeoutAction::Reject);

        store.suspend(state).unwrap();
        assert_eq!(store.count(), 1);

        let (tx, mut rx) = mpsc::channel(10);
        let processor = TimeoutProcessor::new(store.clone()).with_event_sender(tx);

        // Process expired
        processor.process_expired().await;

        // Should have been removed
        assert_eq!(store.count(), 0);

        // Should have received event
        let event = rx.try_recv().unwrap();
        assert_eq!(event.hook_id, "timeout-hook");
        assert_eq!(event.action, TimeoutAction::Reject);
    }
}
