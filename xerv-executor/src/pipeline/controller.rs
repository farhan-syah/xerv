//! Pipeline lifecycle controller implementation.

use crate::scheduler::{Executor, ExecutorConfig, FlowGraph};
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{RwLock, broadcast};
use xerv_core::error::{Result, XervError};
use xerv_core::logging::{BufferedCollector, LogCategory, LogCollector, LogEvent};
use xerv_core::traits::{
    Node, PipelineConfig, PipelineHook, PipelineSettings, PipelineState, Trigger, TriggerEvent,
};
use xerv_core::types::{NodeId, PipelineId, TraceId};
use xerv_core::wal::{Wal, WalConfig};

/// Metrics for a pipeline.
#[derive(Debug, Default)]
pub struct PipelineMetrics {
    /// Total traces started.
    pub traces_started: AtomicU64,
    /// Total traces completed successfully.
    pub traces_completed: AtomicU64,
    /// Total traces failed.
    pub traces_failed: AtomicU64,
    /// Currently active traces.
    pub traces_active: AtomicU64,
    /// Total node executions.
    pub node_executions: AtomicU64,
    /// Total execution time in microseconds.
    pub total_execution_us: AtomicU64,
}

impl PipelineMetrics {
    /// Record a trace start.
    pub fn record_trace_start(&self) {
        self.traces_started.fetch_add(1, Ordering::Relaxed);
        self.traces_active.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a trace completion.
    pub fn record_trace_complete(&self) {
        self.traces_completed.fetch_add(1, Ordering::Relaxed);
        self.traces_active.fetch_sub(1, Ordering::Relaxed);
    }

    /// Record a trace failure.
    pub fn record_trace_failed(&self) {
        self.traces_failed.fetch_add(1, Ordering::Relaxed);
        self.traces_active.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get the current error rate (failures / started).
    pub fn error_rate(&self) -> f64 {
        let started = self.traces_started.load(Ordering::Relaxed);
        let failed = self.traces_failed.load(Ordering::Relaxed);
        if started == 0 {
            0.0
        } else {
            failed as f64 / started as f64
        }
    }
}

/// A running pipeline instance.
pub struct Pipeline {
    /// Pipeline ID.
    pub id: PipelineId,
    /// Current state.
    state: RwLock<PipelineState>,
    /// Settings.
    settings: PipelineSettings,
    /// The executor.
    executor: Arc<Executor>,
    /// Active triggers.
    triggers: Vec<Box<dyn Trigger>>,
    /// Lifecycle hooks.
    hooks: Option<Box<dyn PipelineHook>>,
    /// Metrics.
    metrics: Arc<PipelineMetrics>,
    /// Shutdown signal.
    shutdown_tx: broadcast::Sender<()>,
    /// Log collector for structured logging with correlation IDs.
    log_collector: Arc<BufferedCollector>,
}

impl Pipeline {
    /// Get the current state.
    pub async fn state(&self) -> PipelineState {
        *self.state.read().await
    }

    /// Get metrics.
    pub fn metrics(&self) -> &PipelineMetrics {
        &self.metrics
    }

    /// Start the pipeline.
    pub async fn start(&self) -> Result<()> {
        let mut state = self.state.write().await;

        if *state != PipelineState::Initializing && *state != PipelineState::Stopped {
            return Err(XervError::CircuitBreakerOpen {
                pipeline_id: self.id.to_string(),
                error_rate: 0.0,
            });
        }

        *state = PipelineState::Running;

        // Call lifecycle hook
        if let Some(hooks) = &self.hooks {
            let ctx = xerv_core::traits::PipelineCtx::new(&self.id.name, self.id.version);
            hooks.on_start(ctx).await;
        }

        // Log pipeline start
        self.log_collector.collect(
            LogEvent::info(LogCategory::Pipeline, "Pipeline started")
                .with_pipeline_id(self.id.to_string()),
        );

        tracing::info!(
            pipeline_id = %self.id,
            "Pipeline started"
        );

        Ok(())
    }

    /// Pause the pipeline (stop accepting new events).
    pub async fn pause(&self) -> Result<()> {
        let mut state = self.state.write().await;

        if *state != PipelineState::Running {
            return Err(XervError::CircuitBreakerOpen {
                pipeline_id: self.id.to_string(),
                error_rate: 0.0,
            });
        }

        *state = PipelineState::Paused;

        // Pause all triggers
        for trigger in &self.triggers {
            trigger.pause().await?;
        }

        // Log pipeline pause
        self.log_collector.collect(
            LogEvent::info(LogCategory::Pipeline, "Pipeline paused")
                .with_pipeline_id(self.id.to_string()),
        );

        tracing::info!(
            pipeline_id = %self.id,
            "Pipeline paused"
        );

        Ok(())
    }

    /// Resume the pipeline.
    pub async fn resume(&self) -> Result<()> {
        let mut state = self.state.write().await;

        if *state != PipelineState::Paused {
            return Err(XervError::CircuitBreakerOpen {
                pipeline_id: self.id.to_string(),
                error_rate: 0.0,
            });
        }

        *state = PipelineState::Running;

        // Resume all triggers
        for trigger in &self.triggers {
            trigger.resume().await?;
        }

        // Log pipeline resume
        self.log_collector.collect(
            LogEvent::info(LogCategory::Pipeline, "Pipeline resumed")
                .with_pipeline_id(self.id.to_string()),
        );

        tracing::info!(
            pipeline_id = %self.id,
            "Pipeline resumed"
        );

        Ok(())
    }

    /// Drain the pipeline (stop new events, wait for in-flight).
    pub async fn drain(&self) -> Result<()> {
        let mut state = self.state.write().await;

        if *state != PipelineState::Running && *state != PipelineState::Paused {
            return Ok(());
        }

        *state = PipelineState::Draining;

        // Call lifecycle hook
        if let Some(hooks) = &self.hooks {
            let ctx = xerv_core::traits::PipelineCtx::new(&self.id.name, self.id.version);
            hooks.on_drain(ctx).await;
        }

        // Stop all triggers
        for trigger in &self.triggers {
            trigger.stop().await?;
        }

        let active_traces = self.metrics.traces_active.load(Ordering::Relaxed);

        // Log draining start
        self.log_collector.collect(
            LogEvent::info(LogCategory::Pipeline, "Pipeline draining")
                .with_pipeline_id(self.id.to_string())
                .with_field_i64("active_traces", active_traces as i64),
        );

        tracing::info!(
            pipeline_id = %self.id,
            active_traces = %active_traces,
            "Pipeline draining"
        );

        // Wait for active traces to complete
        while self.metrics.traces_active.load(Ordering::Relaxed) > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        *state = PipelineState::Stopped;

        // Log drain complete
        self.log_collector.collect(
            LogEvent::info(LogCategory::Pipeline, "Pipeline drained and stopped")
                .with_pipeline_id(self.id.to_string()),
        );

        tracing::info!(
            pipeline_id = %self.id,
            "Pipeline drained and stopped"
        );

        Ok(())
    }

    /// Stop the pipeline immediately.
    pub async fn stop(&self) -> Result<()> {
        let mut state = self.state.write().await;

        // Stop all triggers
        for trigger in &self.triggers {
            trigger.stop().await?;
        }

        // Signal shutdown
        let _ = self.shutdown_tx.send(());

        *state = PipelineState::Stopped;

        // Call lifecycle hook
        if let Some(hooks) = &self.hooks {
            let ctx = xerv_core::traits::PipelineCtx::new(&self.id.name, self.id.version);
            hooks.on_stop(ctx).await;
        }

        // Log pipeline stop
        self.log_collector.collect(
            LogEvent::info(LogCategory::Pipeline, "Pipeline stopped")
                .with_pipeline_id(self.id.to_string()),
        );

        tracing::info!(
            pipeline_id = %self.id,
            "Pipeline stopped"
        );

        Ok(())
    }

    /// Handle a trigger event.
    pub async fn handle_event(&self, event: TriggerEvent) -> Result<TraceId> {
        let state = self.state.read().await;

        // Check if pipeline is accepting events
        if *state != PipelineState::Running {
            return Err(XervError::CircuitBreakerOpen {
                pipeline_id: self.id.to_string(),
                error_rate: self.metrics.error_rate(),
            });
        }

        // Check circuit breaker
        if self.metrics.error_rate() > self.settings.circuit_breaker_threshold {
            return Err(XervError::CircuitBreakerOpen {
                pipeline_id: self.id.to_string(),
                error_rate: self.metrics.error_rate(),
            });
        }

        drop(state); // Release read lock

        // Start trace
        self.metrics.record_trace_start();
        let trace_id = self.executor.start_trace(event).await?;

        // Execute trace in background
        let executor = Arc::clone(&self.executor);
        let metrics = Arc::clone(&self.metrics);
        let log_collector = Arc::clone(&self.log_collector);
        let pipeline_id_str = self.id.to_string();

        tokio::spawn(async move {
            let result = executor.execute_trace(trace_id).await;

            match result {
                Ok(()) => {
                    metrics.record_trace_complete();
                }
                Err(e) => {
                    metrics.record_trace_failed();

                    // Log trace failure
                    log_collector.collect(
                        LogEvent::error(LogCategory::Trace, "Trace failed")
                            .with_trace_id(trace_id)
                            .with_pipeline_id(&pipeline_id_str)
                            .with_field("error", e.to_string()),
                    );

                    tracing::error!(
                        trace_id = %trace_id,
                        error = %e,
                        "Trace failed"
                    );
                }
            }
        });

        Ok(trace_id)
    }

    /// Resume a suspended trace.
    ///
    /// This is called when the external API receives a resume signal
    /// for a trace that was suspended at a wait node.
    pub async fn resume_suspended_trace(
        &self,
        hook_id: &str,
        decision: crate::suspension::ResumeDecision,
    ) -> Result<TraceId> {
        let state = self.state.read().await;

        // Check if pipeline is running
        if *state != PipelineState::Running {
            return Err(XervError::CircuitBreakerOpen {
                pipeline_id: self.id.to_string(),
                error_rate: 0.0,
            });
        }

        drop(state);

        tracing::info!(
            pipeline_id = %self.id,
            hook_id = %hook_id,
            "Resuming suspended trace"
        );

        // Resume the trace through the executor
        self.executor
            .resume_suspended_trace(hook_id, decision)
            .await
    }
}

/// Builder for creating pipelines.
pub struct PipelineBuilder {
    id: PipelineId,
    #[allow(dead_code)] // Stored for future use when config is passed to executor
    config: PipelineConfig,
    settings: PipelineSettings,
    graph: FlowGraph,
    nodes: HashMap<NodeId, Box<dyn Node>>,
    triggers: Vec<Box<dyn Trigger>>,
    hooks: Option<Box<dyn PipelineHook>>,
    wal_config: WalConfig,
    executor_config: ExecutorConfig,
    log_collector: Option<Arc<BufferedCollector>>,
}

impl PipelineBuilder {
    /// Create a new pipeline builder.
    pub fn new(id: PipelineId, config: PipelineConfig) -> Self {
        Self {
            id,
            config,
            settings: PipelineSettings::default(),
            graph: FlowGraph::new(),
            nodes: HashMap::new(),
            triggers: Vec::new(),
            hooks: None,
            wal_config: WalConfig::from_env_or_default(),
            executor_config: ExecutorConfig::from_env_or_default(),
            log_collector: None,
        }
    }

    /// Set pipeline settings.
    pub fn with_settings(mut self, settings: PipelineSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Set the flow graph.
    pub fn with_graph(mut self, graph: FlowGraph) -> Self {
        self.graph = graph;
        self
    }

    /// Add a node.
    pub fn with_node(mut self, id: NodeId, node: Box<dyn Node>) -> Self {
        self.nodes.insert(id, node);
        self
    }

    /// Add a trigger.
    pub fn with_trigger(mut self, trigger: Box<dyn Trigger>) -> Self {
        self.triggers.push(trigger);
        self
    }

    /// Set lifecycle hooks.
    pub fn with_hooks(mut self, hooks: Box<dyn PipelineHook>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Set WAL configuration.
    pub fn with_wal_config(mut self, config: WalConfig) -> Self {
        self.wal_config = config;
        self
    }

    /// Set executor configuration.
    pub fn with_executor_config(mut self, config: ExecutorConfig) -> Self {
        self.executor_config = config;
        self
    }

    /// Set the log collector for structured logging.
    pub fn with_log_collector(mut self, collector: Arc<BufferedCollector>) -> Self {
        self.log_collector = Some(collector);
        self
    }

    /// Build the pipeline.
    pub fn build(self) -> Result<Pipeline> {
        let wal = Arc::new(Wal::open(self.wal_config.clone())?);

        // Use provided log collector or create a new one
        let log_collector = self
            .log_collector
            .unwrap_or_else(|| Arc::new(BufferedCollector::with_default_capacity()));

        let pipeline_id_str = self.id.to_string();
        let executor = Executor::new(
            self.executor_config,
            self.graph,
            self.nodes,
            wal,
            Arc::clone(&log_collector),
            Some(pipeline_id_str),
        )?;

        let (shutdown_tx, _) = broadcast::channel(1);

        Ok(Pipeline {
            id: self.id,
            state: RwLock::new(PipelineState::Initializing),
            settings: self.settings,
            executor: Arc::new(executor),
            triggers: self.triggers,
            hooks: self.hooks,
            metrics: Arc::new(PipelineMetrics::default()),
            shutdown_tx,
            log_collector,
        })
    }
}

/// Controller for managing multiple pipelines.
pub struct PipelineController {
    /// Active pipelines by ID.
    pipelines: DashMap<String, Arc<Pipeline>>,
}

impl PipelineController {
    /// Create a new pipeline controller.
    pub fn new() -> Self {
        Self {
            pipelines: DashMap::new(),
        }
    }

    /// Deploy a new pipeline.
    pub fn deploy(&self, pipeline: Pipeline) -> Result<()> {
        let key = pipeline.id.to_string();

        if self.pipelines.contains_key(&key) {
            return Err(XervError::PipelineExists { pipeline_id: key });
        }

        self.pipelines.insert(key, Arc::new(pipeline));
        Ok(())
    }

    /// Get a pipeline by ID.
    pub fn get(&self, id: &str) -> Option<Arc<Pipeline>> {
        self.pipelines.get(id).map(|r| Arc::clone(&r))
    }

    /// List all pipeline IDs.
    pub fn list(&self) -> Vec<String> {
        self.pipelines.iter().map(|r| r.key().clone()).collect()
    }

    /// Remove a pipeline.
    pub async fn remove(&self, id: &str) -> Result<()> {
        let pipeline = self.pipelines.remove(id);

        if let Some((_, pipeline)) = pipeline {
            pipeline.stop().await?;
        }

        Ok(())
    }

    /// Drain and remove a pipeline.
    pub async fn drain_and_remove(&self, id: &str) -> Result<()> {
        if let Some(pipeline) = self.get(id) {
            pipeline.drain().await?;
            self.pipelines.remove(id);
        }

        Ok(())
    }
}

impl Default for PipelineController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_tracking() {
        let metrics = PipelineMetrics::default();

        metrics.record_trace_start();
        metrics.record_trace_start();
        metrics.record_trace_complete();
        metrics.record_trace_failed();

        assert_eq!(metrics.traces_started.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.traces_completed.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.traces_failed.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.traces_active.load(Ordering::Relaxed), 0);
        assert!((metrics.error_rate() - 0.5).abs() < 0.001);
    }

    #[test]
    fn pipeline_controller_operations() {
        let controller = PipelineController::new();
        assert!(controller.list().is_empty());
    }
}
