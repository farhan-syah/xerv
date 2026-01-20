//! Recovery report types.

use xerv_core::types::TraceId;

/// Result of crash recovery operation.
#[derive(Debug, Default)]
pub struct RecoveryReport {
    /// Traces that were successfully recovered and resumed.
    pub recovered: Vec<TraceId>,
    /// Traces that were skipped with reasons.
    pub skipped: Vec<(TraceId, String)>,
    /// Traces that are awaiting manual resume (suspended at wait nodes).
    pub awaiting_resume: Vec<TraceId>,
}

impl RecoveryReport {
    /// Create a new empty recovery report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a recovered trace.
    pub fn add_recovered(&mut self, trace_id: TraceId) {
        self.recovered.push(trace_id);
    }

    /// Add a skipped trace with reason.
    pub fn add_skipped(&mut self, trace_id: TraceId, reason: impl Into<String>) {
        self.skipped.push((trace_id, reason.into()));
    }

    /// Add a trace awaiting resume.
    pub fn add_awaiting_resume(&mut self, trace_id: TraceId) {
        self.awaiting_resume.push(trace_id);
    }

    /// Get total number of traces processed.
    pub fn total_processed(&self) -> usize {
        self.recovered.len() + self.skipped.len() + self.awaiting_resume.len()
    }

    /// Check if all traces were successfully recovered.
    pub fn all_recovered(&self) -> bool {
        self.skipped.is_empty() && self.awaiting_resume.is_empty()
    }
}

impl std::fmt::Display for RecoveryReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RecoveryReport {{ recovered: {}, skipped: {}, awaiting_resume: {} }}",
            self.recovered.len(),
            self.skipped.len(),
            self.awaiting_resume.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_report_basic() {
        let mut report = RecoveryReport::new();

        let trace1 = TraceId::new();
        let trace2 = TraceId::new();
        let trace3 = TraceId::new();

        report.add_recovered(trace1);
        report.add_skipped(trace2, "Node implementation missing");
        report.add_awaiting_resume(trace3);

        assert_eq!(report.total_processed(), 3);
        assert!(!report.all_recovered());
        assert_eq!(report.recovered.len(), 1);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.awaiting_resume.len(), 1);
    }

    #[test]
    fn recovery_report_all_recovered() {
        let mut report = RecoveryReport::new();

        report.add_recovered(TraceId::new());
        report.add_recovered(TraceId::new());

        assert!(report.all_recovered());
    }
}
