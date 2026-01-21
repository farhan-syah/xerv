//! Prometheus metrics for XERV.
//!
//! Exports metrics compatible with Kubernetes HPA and Prometheus.
//!
//! # Metrics
//!
//! ## Counters
//! - `xerv_pending_traces_total` - Number of traces waiting to be processed
//! - `xerv_active_traces_total` - Number of currently executing traces
//! - `xerv_completed_traces_total` - Total number of completed traces
//! - `xerv_failed_traces_total` - Total number of failed traces
//!
//! ## Histograms
//! - `xerv_trace_duration_seconds` - Duration of trace executions
//! - `xerv_node_execution_seconds` - Duration of node executions
//!
//! ## Gauges
//! - `xerv_arena_size_bytes` - Current arena memory usage
//! - `xerv_dispatch_queue_depth` - Number of traces in dispatch queue
//! - `xerv_raft_leader` - Whether this node is the Raft leader (1 or 0)

use prometheus::{
    CounterVec, GaugeVec, HistogramOpts, HistogramVec, IntGauge, IntGaugeVec, Opts, Registry,
};
use std::sync::Arc;

/// Default histogram buckets for trace durations (in seconds).
const DURATION_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];

/// Default histogram buckets for node executions (in seconds).
const NODE_DURATION_BUCKETS: &[f64] = &[
    0.0001, 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0,
];

/// XERV metrics registry.
///
/// Contains all Prometheus metrics for the XERV executor.
pub struct Metrics {
    /// The Prometheus registry.
    registry: Registry,

    // Counters (with labels)
    /// Total completed traces by pipeline.
    pub completed_traces: CounterVec,
    /// Total failed traces by pipeline.
    pub failed_traces: CounterVec,
    /// Total nodes executed by node type.
    pub nodes_executed: CounterVec,

    // Gauges (with labels)
    /// Currently pending traces by pipeline.
    pub pending_traces: IntGaugeVec,
    /// Currently active traces by pipeline.
    pub active_traces: IntGaugeVec,
    /// Arena size in bytes by trace.
    pub arena_size: GaugeVec,

    // Global gauges
    /// Dispatch queue depth.
    pub dispatch_queue_depth: IntGauge,
    /// Whether this node is the Raft leader.
    pub raft_leader: IntGauge,
    /// Uptime in seconds.
    pub uptime_seconds: IntGauge,
    /// Number of active pipelines.
    pub active_pipelines: IntGauge,
    /// Number of registered triggers.
    pub registered_triggers: IntGauge,

    // Histograms
    /// Trace duration histogram by pipeline.
    pub trace_duration: HistogramVec,
    /// Node execution duration histogram by node type.
    pub node_duration: HistogramVec,
}

impl Metrics {
    /// Create a new metrics registry with all XERV metrics.
    pub fn new() -> Self {
        let registry = Registry::new();

        // Completed traces counter
        let completed_traces = CounterVec::new(
            Opts::new("xerv_completed_traces_total", "Total completed traces")
                .namespace("xerv")
                .const_label("service", "executor"),
            &["pipeline", "cluster"],
        )
        .expect("metric creation should not fail");

        // Failed traces counter
        let failed_traces = CounterVec::new(
            Opts::new("xerv_failed_traces_total", "Total failed traces")
                .namespace("xerv")
                .const_label("service", "executor"),
            &["pipeline", "cluster"],
        )
        .expect("metric creation should not fail");

        // Nodes executed counter
        let nodes_executed = CounterVec::new(
            Opts::new("xerv_nodes_executed_total", "Total nodes executed")
                .namespace("xerv")
                .const_label("service", "executor"),
            &["pipeline", "node_type", "cluster"],
        )
        .expect("metric creation should not fail");

        // Pending traces gauge
        let pending_traces = IntGaugeVec::new(
            Opts::new("xerv_pending_traces", "Number of pending traces")
                .namespace("xerv")
                .const_label("service", "executor"),
            &["pipeline", "cluster"],
        )
        .expect("metric creation should not fail");

        // Active traces gauge
        let active_traces = IntGaugeVec::new(
            Opts::new("xerv_active_traces", "Number of active traces")
                .namespace("xerv")
                .const_label("service", "executor"),
            &["pipeline", "cluster"],
        )
        .expect("metric creation should not fail");

        // Arena size gauge
        let arena_size = GaugeVec::new(
            Opts::new("xerv_arena_size_bytes", "Arena memory usage in bytes")
                .namespace("xerv")
                .const_label("service", "executor"),
            &["pipeline", "cluster"],
        )
        .expect("metric creation should not fail");

        // Dispatch queue depth
        let dispatch_queue_depth = IntGauge::with_opts(
            Opts::new(
                "xerv_dispatch_queue_depth",
                "Number of traces in dispatch queue",
            )
            .namespace("xerv")
            .const_label("service", "executor"),
        )
        .expect("metric creation should not fail");

        // Raft leader indicator
        let raft_leader = IntGauge::with_opts(
            Opts::new(
                "xerv_raft_leader",
                "Whether this node is the Raft leader (1 or 0)",
            )
            .namespace("xerv")
            .const_label("service", "executor"),
        )
        .expect("metric creation should not fail");

        // Uptime
        let uptime_seconds = IntGauge::with_opts(
            Opts::new("xerv_uptime_seconds", "Server uptime in seconds")
                .namespace("xerv")
                .const_label("service", "executor"),
        )
        .expect("metric creation should not fail");

        // Active pipelines
        let active_pipelines = IntGauge::with_opts(
            Opts::new("xerv_active_pipelines", "Number of active pipelines")
                .namespace("xerv")
                .const_label("service", "executor"),
        )
        .expect("metric creation should not fail");

        // Registered triggers
        let registered_triggers = IntGauge::with_opts(
            Opts::new("xerv_registered_triggers", "Number of registered triggers")
                .namespace("xerv")
                .const_label("service", "executor"),
        )
        .expect("metric creation should not fail");

        // Trace duration histogram
        let trace_duration = HistogramVec::new(
            HistogramOpts::new(
                "xerv_trace_duration_seconds",
                "Duration of trace executions in seconds",
            )
            .namespace("xerv")
            .const_label("service", "executor")
            .buckets(DURATION_BUCKETS.to_vec()),
            &["pipeline", "cluster"],
        )
        .expect("metric creation should not fail");

        // Node duration histogram
        let node_duration = HistogramVec::new(
            HistogramOpts::new(
                "xerv_node_execution_seconds",
                "Duration of node executions in seconds",
            )
            .namespace("xerv")
            .const_label("service", "executor")
            .buckets(NODE_DURATION_BUCKETS.to_vec()),
            &["pipeline", "node_type", "cluster"],
        )
        .expect("metric creation should not fail");

        // Register all metrics
        registry
            .register(Box::new(completed_traces.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(failed_traces.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(nodes_executed.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(pending_traces.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(active_traces.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(arena_size.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(dispatch_queue_depth.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(raft_leader.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(uptime_seconds.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(active_pipelines.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(registered_triggers.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(trace_duration.clone()))
            .expect("registration should not fail");
        registry
            .register(Box::new(node_duration.clone()))
            .expect("registration should not fail");

        Self {
            registry,
            completed_traces,
            failed_traces,
            nodes_executed,
            pending_traces,
            active_traces,
            arena_size,
            dispatch_queue_depth,
            raft_leader,
            uptime_seconds,
            active_pipelines,
            registered_triggers,
            trace_duration,
            node_duration,
        }
    }

    /// Get the Prometheus registry.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Record a completed trace.
    pub fn record_trace_completed(&self, pipeline: &str, cluster: &str, duration_secs: f64) {
        self.completed_traces
            .with_label_values(&[pipeline, cluster])
            .inc();
        self.trace_duration
            .with_label_values(&[pipeline, cluster])
            .observe(duration_secs);
    }

    /// Record a failed trace.
    pub fn record_trace_failed(&self, pipeline: &str, cluster: &str, duration_secs: f64) {
        self.failed_traces
            .with_label_values(&[pipeline, cluster])
            .inc();
        self.trace_duration
            .with_label_values(&[pipeline, cluster])
            .observe(duration_secs);
    }

    /// Record a node execution.
    pub fn record_node_executed(
        &self,
        pipeline: &str,
        node_type: &str,
        cluster: &str,
        duration_secs: f64,
    ) {
        self.nodes_executed
            .with_label_values(&[pipeline, node_type, cluster])
            .inc();
        self.node_duration
            .with_label_values(&[pipeline, node_type, cluster])
            .observe(duration_secs);
    }

    /// Update pending trace count.
    pub fn set_pending_traces(&self, pipeline: &str, cluster: &str, count: i64) {
        self.pending_traces
            .with_label_values(&[pipeline, cluster])
            .set(count);
    }

    /// Update active trace count.
    pub fn set_active_traces(&self, pipeline: &str, cluster: &str, count: i64) {
        self.active_traces
            .with_label_values(&[pipeline, cluster])
            .set(count);
    }

    /// Update arena size.
    pub fn set_arena_size(&self, pipeline: &str, cluster: &str, bytes: f64) {
        self.arena_size
            .with_label_values(&[pipeline, cluster])
            .set(bytes);
    }

    /// Update dispatch queue depth.
    pub fn set_dispatch_queue_depth(&self, depth: i64) {
        self.dispatch_queue_depth.set(depth);
    }

    /// Update raft leader status.
    pub fn set_raft_leader(&self, is_leader: bool) {
        self.raft_leader.set(if is_leader { 1 } else { 0 });
    }

    /// Update uptime.
    pub fn set_uptime(&self, seconds: i64) {
        self.uptime_seconds.set(seconds);
    }

    /// Update active pipeline count.
    pub fn set_active_pipelines(&self, count: i64) {
        self.active_pipelines.set(count);
    }

    /// Update registered trigger count.
    pub fn set_registered_triggers(&self, count: i64) {
        self.registered_triggers.set(count);
    }

    /// Encode all metrics in Prometheus text format.
    pub fn encode(&self) -> String {
        use prometheus::Encoder;

        let encoder = prometheus::TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();

        encoder
            .encode(&metric_families, &mut buffer)
            .expect("encoding should not fail");

        String::from_utf8(buffer).expect("metrics should be valid UTF-8")
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Global metrics instance for convenience.
static GLOBAL_METRICS: std::sync::OnceLock<Arc<Metrics>> = std::sync::OnceLock::new();

/// Initialize the global metrics instance.
pub fn init_global_metrics() -> Arc<Metrics> {
    GLOBAL_METRICS
        .get_or_init(|| Arc::new(Metrics::new()))
        .clone()
}

/// Get the global metrics instance.
///
/// Panics if `init_global_metrics` has not been called.
pub fn global_metrics() -> Arc<Metrics> {
    GLOBAL_METRICS
        .get()
        .expect("global metrics not initialized")
        .clone()
}

/// Try to get the global metrics instance, returning None if not initialized.
pub fn try_global_metrics() -> Option<Arc<Metrics>> {
    GLOBAL_METRICS.get().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_creation() {
        let metrics = Metrics::new();
        assert!(metrics.encode().contains("xerv_"));
    }

    #[test]
    fn record_trace_completed() {
        let metrics = Metrics::new();
        metrics.record_trace_completed("test_pipeline", "default", 1.5);

        let output = metrics.encode();
        assert!(output.contains("xerv_completed_traces_total"));
        assert!(output.contains("xerv_trace_duration_seconds"));
    }

    #[test]
    fn record_node_executed() {
        let metrics = Metrics::new();
        metrics.record_node_executed("test_pipeline", "std::http", "default", 0.05);

        let output = metrics.encode();
        assert!(output.contains("xerv_nodes_executed_total"));
        assert!(output.contains("xerv_node_execution_seconds"));
    }

    #[test]
    fn gauge_updates() {
        let metrics = Metrics::new();
        metrics.set_dispatch_queue_depth(42);
        metrics.set_raft_leader(true);
        metrics.set_uptime(3600);

        let output = metrics.encode();
        // Check metric names and values (labels are included in output)
        assert!(output.contains("xerv_dispatch_queue_depth"));
        assert!(output.contains("} 42")); // value after labels
        assert!(output.contains("xerv_raft_leader"));
        assert!(output.contains("} 1")); // value after labels
        assert!(output.contains("xerv_uptime_seconds"));
        assert!(output.contains("} 3600")); // value after labels
    }

    #[test]
    fn labeled_gauges() {
        let metrics = Metrics::new();
        metrics.set_active_traces("my_pipeline", "us-east", 10);
        metrics.set_pending_traces("my_pipeline", "us-east", 5);

        let output = metrics.encode();
        assert!(output.contains("xerv_active_traces"));
        assert!(output.contains("xerv_pending_traces"));
    }
}
