//! Integration tests for pipeline lifecycle management.
//!
//! Tests state transitions and metrics tracking.

mod common;

use std::sync::atomic::Ordering;
use xerv_core::traits::{PipelineConfig, PipelineState};
use xerv_core::types::PipelineId;
use xerv_executor::pipeline::{Pipeline, PipelineBuilder, PipelineController, PipelineMetrics};

use common::{build_linear_flow, test_executor_config, test_wal_config};

fn create_test_pipeline(name: &str) -> Pipeline {
    let (graph, nodes) = build_linear_flow(2);

    let id = PipelineId {
        name: name.to_string(),
        version: 1,
    };

    let config = PipelineConfig::new(name);

    let mut builder = PipelineBuilder::new(id, config)
        .with_graph(graph)
        .with_wal_config(test_wal_config())
        .with_executor_config(test_executor_config());

    for (node_id, node) in nodes {
        builder = builder.with_node(node_id, node);
    }

    builder.build().unwrap()
}

// Metrics tests (fast, synchronous)

#[test]
fn metrics_initial_values() {
    let metrics = PipelineMetrics::default();
    assert_eq!(metrics.traces_started.load(Ordering::Relaxed), 0);
    assert_eq!(metrics.traces_completed.load(Ordering::Relaxed), 0);
    assert_eq!(metrics.traces_failed.load(Ordering::Relaxed), 0);
    assert_eq!(metrics.traces_active.load(Ordering::Relaxed), 0);
    assert_eq!(metrics.error_rate(), 0.0);
}

#[test]
fn metrics_tracking() {
    let metrics = PipelineMetrics::default();

    metrics.record_trace_start();
    metrics.record_trace_start();
    metrics.record_trace_start();
    assert_eq!(metrics.traces_started.load(Ordering::Relaxed), 3);
    assert_eq!(metrics.traces_active.load(Ordering::Relaxed), 3);

    metrics.record_trace_complete();
    assert_eq!(metrics.traces_completed.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.traces_active.load(Ordering::Relaxed), 2);

    metrics.record_trace_failed();
    assert_eq!(metrics.traces_failed.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.traces_active.load(Ordering::Relaxed), 1);

    // Error rate = 1 failed / 3 started = 0.333...
    let error_rate = metrics.error_rate();
    assert!((error_rate - 0.333).abs() < 0.01);
}

// Controller tests (fast, no execution)

#[test]
fn controller_deploy_and_list() {
    let controller = PipelineController::new();
    assert!(controller.list().is_empty());

    controller
        .deploy(create_test_pipeline("pipeline-1"))
        .unwrap();
    controller
        .deploy(create_test_pipeline("pipeline-2"))
        .unwrap();

    let list = controller.list();
    assert_eq!(list.len(), 2);
    assert!(list.contains(&"pipeline-1@v1".to_string()));
    assert!(list.contains(&"pipeline-2@v1".to_string()));
}

#[test]
fn controller_get() {
    let controller = PipelineController::new();
    controller.deploy(create_test_pipeline("get-test")).unwrap();

    assert!(controller.get("get-test@v1").is_some());
    assert!(controller.get("nonexistent@v1").is_none());
}

#[test]
fn controller_rejects_duplicate() {
    let controller = PipelineController::new();
    controller.deploy(create_test_pipeline("dup-test")).unwrap();

    let result = controller.deploy(create_test_pipeline("dup-test"));
    assert!(result.is_err());
}

// Pipeline state tests (async but fast - no execution)

#[tokio::test]
async fn initial_state_is_initializing() {
    let pipeline = create_test_pipeline("test-initial");
    assert_eq!(pipeline.state().await, PipelineState::Initializing);
}

#[tokio::test]
async fn start_transitions_to_running() {
    let pipeline = create_test_pipeline("test-start");
    pipeline.start().await.unwrap();
    assert_eq!(pipeline.state().await, PipelineState::Running);
}

#[tokio::test]
async fn pause_resume_cycle() {
    let pipeline = create_test_pipeline("test-pause-resume");

    pipeline.start().await.unwrap();
    assert_eq!(pipeline.state().await, PipelineState::Running);

    pipeline.pause().await.unwrap();
    assert_eq!(pipeline.state().await, PipelineState::Paused);

    pipeline.resume().await.unwrap();
    assert_eq!(pipeline.state().await, PipelineState::Running);
}

#[tokio::test]
async fn cannot_pause_when_not_running() {
    let pipeline = create_test_pipeline("test-pause-fail");
    let result = pipeline.pause().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn cannot_resume_when_not_paused() {
    let pipeline = create_test_pipeline("test-resume-fail");
    pipeline.start().await.unwrap();
    let result = pipeline.resume().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn stop_transitions_to_stopped() {
    let pipeline = create_test_pipeline("test-stop");
    pipeline.start().await.unwrap();
    pipeline.stop().await.unwrap();
    assert_eq!(pipeline.state().await, PipelineState::Stopped);
}
