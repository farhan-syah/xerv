//! Suspension store trait and implementations.

use super::request::{ResumeDecision, SuspendedTraceState};
use parking_lot::RwLock;
use std::collections::HashMap;
use xerv_core::error::{Result, XervError};

/// Trait for storing and retrieving suspended trace state.
pub trait SuspensionStore: Send + Sync {
    /// Store a suspended trace state.
    fn suspend(&self, state: SuspendedTraceState) -> Result<()>;

    /// Resume a suspended trace by hook ID.
    fn resume(&self, hook_id: &str, decision: ResumeDecision) -> Result<SuspendedTraceState>;

    /// Get suspended state by hook ID (without consuming it).
    fn get(&self, hook_id: &str) -> Result<SuspendedTraceState>;

    /// Check if a hook ID is currently suspended.
    fn is_suspended(&self, hook_id: &str) -> bool;

    /// Get all expired suspensions.
    fn get_expired(&self) -> Vec<(String, SuspendedTraceState)>;

    /// Remove a suspension by hook ID.
    fn remove(&self, hook_id: &str) -> Result<()>;

    /// List all suspended traces for a pipeline.
    fn list_by_pipeline(&self, pipeline_id: &str) -> Vec<SuspendedTraceState>;

    /// List all suspended traces.
    fn list_all(&self) -> Vec<SuspendedTraceState>;

    /// Get the count of suspended traces.
    fn count(&self) -> usize;
}

/// In-memory suspension store for testing and development.
#[derive(Debug, Default)]
pub struct MemorySuspensionStore {
    suspensions: RwLock<HashMap<String, SuspendedTraceState>>,
    /// Store of pending resume decisions (for async resume flow).
    pending_resumes: RwLock<HashMap<String, ResumeDecision>>,
}

impl MemorySuspensionStore {
    /// Create a new in-memory suspension store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a pending resume decision (if any).
    pub fn get_pending_resume(&self, hook_id: &str) -> Option<ResumeDecision> {
        self.pending_resumes.read().get(hook_id).cloned()
    }

    /// Check if any suspensions are waiting.
    pub fn has_suspensions(&self) -> bool {
        !self.suspensions.read().is_empty()
    }

    /// Consume and remove a pending resume decision.
    ///
    /// This should be called by the executor after processing a resume.
    pub fn consume_pending_resume(&self, hook_id: &str) -> Option<ResumeDecision> {
        self.pending_resumes.write().remove(hook_id)
    }

    /// Cleanup expired suspensions and return the count removed.
    ///
    /// This should be called periodically to prevent memory leaks.
    pub fn cleanup_expired(&self) -> usize {
        let expired_ids: Vec<String> = self
            .suspensions
            .read()
            .iter()
            .filter(|(_, state)| state.is_expired())
            .map(|(id, _)| id.clone())
            .collect();

        let count = expired_ids.len();
        if count > 0 {
            let mut suspensions = self.suspensions.write();
            for id in &expired_ids {
                suspensions.remove(id);
                tracing::warn!(hook_id = %id, "Expired suspension cleaned up");
            }
        }

        count
    }
}

impl SuspensionStore for MemorySuspensionStore {
    fn suspend(&self, state: SuspendedTraceState) -> Result<()> {
        let hook_id = state.hook_id.clone();

        // Check for duplicate
        if self.suspensions.read().contains_key(&hook_id) {
            return Err(XervError::SuspensionFailed {
                trace_id: state.trace_id,
                cause: format!("Hook ID '{}' already in use", hook_id),
            });
        }

        self.suspensions.write().insert(hook_id.clone(), state);
        tracing::info!(hook_id = %hook_id, "Trace suspended");
        Ok(())
    }

    fn resume(&self, hook_id: &str, decision: ResumeDecision) -> Result<SuspendedTraceState> {
        let mut suspensions = self.suspensions.write();

        let state = suspensions
            .remove(hook_id)
            .ok_or_else(|| XervError::HookNotFound {
                hook_id: hook_id.to_string(),
            })?;

        // Store the decision for the executor to pick up
        self.pending_resumes
            .write()
            .insert(hook_id.to_string(), decision);

        tracing::info!(
            hook_id = %hook_id,
            trace_id = %state.trace_id,
            "Trace resumed"
        );

        Ok(state)
    }

    fn get(&self, hook_id: &str) -> Result<SuspendedTraceState> {
        self.suspensions
            .read()
            .get(hook_id)
            .cloned()
            .ok_or_else(|| XervError::HookNotFound {
                hook_id: hook_id.to_string(),
            })
    }

    fn is_suspended(&self, hook_id: &str) -> bool {
        self.suspensions.read().contains_key(hook_id)
    }

    fn get_expired(&self) -> Vec<(String, SuspendedTraceState)> {
        self.suspensions
            .read()
            .iter()
            .filter(|(_, state)| state.is_expired())
            .map(|(id, state)| (id.clone(), state.clone()))
            .collect()
    }

    fn remove(&self, hook_id: &str) -> Result<()> {
        self.suspensions.write().remove(hook_id);
        self.pending_resumes.write().remove(hook_id);
        Ok(())
    }

    fn list_by_pipeline(&self, pipeline_id: &str) -> Vec<SuspendedTraceState> {
        self.suspensions
            .read()
            .values()
            .filter(|state| state.pipeline_id == pipeline_id)
            .cloned()
            .collect()
    }

    fn list_all(&self) -> Vec<SuspendedTraceState> {
        self.suspensions.read().values().cloned().collect()
    }

    fn count(&self) -> usize {
        self.suspensions.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suspension::TimeoutAction;
    use std::path::PathBuf;
    use xerv_core::types::{NodeId, TraceId};

    fn test_state(hook_id: &str) -> SuspendedTraceState {
        SuspendedTraceState::new(
            hook_id,
            TraceId::new(),
            NodeId::new(1),
            PathBuf::from("/tmp/test.bin"),
            "test-pipeline@v1",
        )
    }

    #[test]
    fn suspend_and_resume() {
        let store = MemorySuspensionStore::new();

        let state = test_state("hook-1");
        let trace_id = state.trace_id;

        store.suspend(state).unwrap();
        assert!(store.is_suspended("hook-1"));

        let resumed = store.resume("hook-1", ResumeDecision::approve()).unwrap();
        assert_eq!(resumed.trace_id, trace_id);
        assert!(!store.is_suspended("hook-1"));
    }

    #[test]
    fn duplicate_hook_id_fails() {
        let store = MemorySuspensionStore::new();

        store.suspend(test_state("hook-1")).unwrap();
        let result = store.suspend(test_state("hook-1"));

        assert!(result.is_err());
    }

    #[test]
    fn resume_nonexistent_hook_fails() {
        let store = MemorySuspensionStore::new();

        let result = store.resume("nonexistent", ResumeDecision::approve());
        assert!(result.is_err());
    }

    #[test]
    fn list_by_pipeline() {
        let store = MemorySuspensionStore::new();

        let mut state1 = test_state("hook-1");
        state1.pipeline_id = "pipeline-a@v1".to_string();

        let mut state2 = test_state("hook-2");
        state2.pipeline_id = "pipeline-b@v1".to_string();

        let mut state3 = test_state("hook-3");
        state3.pipeline_id = "pipeline-a@v1".to_string();

        store.suspend(state1).unwrap();
        store.suspend(state2).unwrap();
        store.suspend(state3).unwrap();

        let pipeline_a = store.list_by_pipeline("pipeline-a@v1");
        assert_eq!(pipeline_a.len(), 2);

        let pipeline_b = store.list_by_pipeline("pipeline-b@v1");
        assert_eq!(pipeline_b.len(), 1);
    }

    #[test]
    fn get_expired() {
        let store = MemorySuspensionStore::new();

        // Non-expiring suspension
        store.suspend(test_state("hook-1")).unwrap();

        // Expired suspension
        let state2 = test_state("hook-2").with_timeout(0, TimeoutAction::Reject);
        store.suspend(state2).unwrap();

        let expired = store.get_expired();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].0, "hook-2");
    }
}
