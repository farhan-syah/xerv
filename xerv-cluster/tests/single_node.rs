//! Single-node cluster tests.
//!
//! Tests basic cluster operations with a single node.

mod common;

use tempfile::TempDir;
use xerv_cluster::{ClusterCommand, ClusterConfig, ClusterNode, TraceStatus};
use xerv_core::types::{PipelineId, TraceId};

/// Test that a single node can start and initialize.
#[tokio::test]
async fn test_single_node_start_and_initialize() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let port = common::get_test_port();

    let config = ClusterConfig::builder()
        .node_id(1)
        .listen_addr(format!("127.0.0.1:{}", port))
        .data_dir(temp_dir.path())
        .build()
        .expect("Invalid config");

    let mut node = ClusterNode::start(config)
        .await
        .expect("Failed to start node");

    // Initialize the cluster
    node.initialize().await.expect("Failed to initialize");

    // Wait a bit for leader election
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Check that this node is the leader
    assert!(node.is_leader().await, "Single node should be leader");
    assert_eq!(node.leader().await, Some(1));

    node.shutdown().await.expect("Failed to shutdown");
}

/// Test deploying and managing a pipeline.
#[tokio::test]
async fn test_single_node_pipeline_lifecycle() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let port = common::get_test_port();

    let config = ClusterConfig::builder()
        .node_id(1)
        .listen_addr(format!("127.0.0.1:{}", port))
        .data_dir(temp_dir.path())
        .build()
        .expect("Invalid config");

    let mut node = ClusterNode::start(config)
        .await
        .expect("Failed to start node");
    node.initialize().await.expect("Failed to initialize");

    // Wait for leader election
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let pipeline_id = PipelineId::new("test-pipeline", 1);

    // Deploy pipeline
    let resp = node
        .execute(ClusterCommand::DeployPipeline {
            pipeline_id: pipeline_id.clone(),
            config: b"test config".to_vec(),
            deployed_at_ms: 1000,
        })
        .await
        .expect("Failed to deploy pipeline");
    assert!(resp.success, "Deploy should succeed: {:?}", resp.error);

    // Start pipeline
    let resp = node
        .execute(ClusterCommand::StartPipeline {
            pipeline_id: pipeline_id.clone(),
            started_at_ms: 2000,
        })
        .await
        .expect("Failed to start pipeline");
    assert!(resp.success, "Start should succeed: {:?}", resp.error);

    // Verify state
    {
        let state = node.state_machine().state().await;
        let pipeline = state
            .pipelines
            .get(&pipeline_id)
            .expect("Pipeline should exist");
        assert_eq!(
            pipeline.state,
            xerv_core::traits::PipelineState::Running,
            "Pipeline should be running"
        );
    }

    // Pause pipeline
    let resp = node
        .execute(ClusterCommand::PausePipeline {
            pipeline_id: pipeline_id.clone(),
            paused_at_ms: 3000,
        })
        .await
        .expect("Failed to pause pipeline");
    assert!(resp.success, "Pause should succeed");

    // Verify paused
    {
        let state = node.state_machine().state().await;
        let pipeline = state
            .pipelines
            .get(&pipeline_id)
            .expect("Pipeline should exist");
        assert_eq!(
            pipeline.state,
            xerv_core::traits::PipelineState::Paused,
            "Pipeline should be paused"
        );
    }

    // Undeploy
    let resp = node
        .execute(ClusterCommand::UndeployPipeline {
            pipeline_id: pipeline_id.clone(),
            undeployed_at_ms: 4000,
        })
        .await
        .expect("Failed to undeploy pipeline");
    assert!(resp.success, "Undeploy should succeed");

    // Verify removed
    {
        let state = node.state_machine().state().await;
        assert!(
            !state.pipelines.contains_key(&pipeline_id),
            "Pipeline should be removed"
        );
    }

    node.shutdown().await.expect("Failed to shutdown");
}

/// Test trace lifecycle (start, complete).
#[tokio::test]
async fn test_single_node_trace_lifecycle() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let port = common::get_test_port();

    let config = ClusterConfig::builder()
        .node_id(1)
        .listen_addr(format!("127.0.0.1:{}", port))
        .data_dir(temp_dir.path())
        .build()
        .expect("Invalid config");

    let mut node = ClusterNode::start(config)
        .await
        .expect("Failed to start node");
    node.initialize().await.expect("Failed to initialize");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let pipeline_id = PipelineId::new("test-pipeline", 1);
    let trace_id = TraceId::new();

    // First deploy and start a pipeline
    node.execute(ClusterCommand::DeployPipeline {
        pipeline_id: pipeline_id.clone(),
        config: Vec::new(),
        deployed_at_ms: 1000,
    })
    .await
    .expect("Deploy failed");

    node.execute(ClusterCommand::StartPipeline {
        pipeline_id: pipeline_id.clone(),
        started_at_ms: 1500,
    })
    .await
    .expect("Start failed");

    // Start a trace
    let resp = node
        .execute(ClusterCommand::StartTrace {
            trace_id,
            pipeline_id: pipeline_id.clone(),
            trigger_data: b"trigger payload".to_vec(),
            started_at_ms: 2000,
        })
        .await
        .expect("Failed to start trace");
    assert!(resp.success, "StartTrace should succeed: {:?}", resp.error);

    // Verify trace state
    {
        let state = node.state_machine().state().await;
        let trace = state.traces.get(&trace_id).expect("Trace should exist");
        assert_eq!(
            trace.status,
            TraceStatus::Running,
            "Trace should be running"
        );
        let pipeline = state.pipelines.get(&pipeline_id).unwrap();
        assert_eq!(pipeline.active_traces, 1, "Should have 1 active trace");
    }

    // Complete the trace
    let resp = node
        .execute(ClusterCommand::CompleteTrace {
            trace_id,
            completed_at_ms: 3000,
        })
        .await
        .expect("Failed to complete trace");
    assert!(resp.success, "CompleteTrace should succeed");

    // Verify completed state
    {
        let state = node.state_machine().state().await;
        let trace = state.traces.get(&trace_id).expect("Trace should exist");
        assert_eq!(
            trace.status,
            TraceStatus::Completed,
            "Trace should be completed"
        );
        let pipeline = state.pipelines.get(&pipeline_id).unwrap();
        assert_eq!(pipeline.active_traces, 0, "Should have 0 active traces");
    }

    node.shutdown().await.expect("Failed to shutdown");
}

/// Test that starting a trace on a non-existent pipeline fails.
#[tokio::test]
async fn test_single_node_trace_without_pipeline_fails() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let port = common::get_test_port();

    let config = ClusterConfig::builder()
        .node_id(1)
        .listen_addr(format!("127.0.0.1:{}", port))
        .data_dir(temp_dir.path())
        .build()
        .expect("Invalid config");

    let mut node = ClusterNode::start(config)
        .await
        .expect("Failed to start node");
    node.initialize().await.expect("Failed to initialize");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Try to start a trace without deploying a pipeline
    let resp = node
        .execute(ClusterCommand::StartTrace {
            trace_id: TraceId::new(),
            pipeline_id: PipelineId::new("nonexistent", 1),
            trigger_data: Vec::new(),
            started_at_ms: 1000,
        })
        .await
        .expect("Execute should work");

    assert!(!resp.success, "StartTrace should fail without pipeline");
    assert!(
        resp.error.as_ref().unwrap().contains("not found"),
        "Error should mention not found"
    );

    node.shutdown().await.expect("Failed to shutdown");
}

/// Test snapshot creation.
#[tokio::test]
async fn test_single_node_snapshot() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let port = common::get_test_port();

    let config = ClusterConfig::builder()
        .node_id(1)
        .listen_addr(format!("127.0.0.1:{}", port))
        .data_dir(temp_dir.path())
        .snapshot_threshold(5) // Low threshold for testing
        .build()
        .expect("Invalid config");

    let mut node = ClusterNode::start(config)
        .await
        .expect("Failed to start node");
    node.initialize().await.expect("Failed to initialize");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Deploy several pipelines to create log entries
    for i in 0..10 {
        let pipeline_id = PipelineId::new(format!("pipeline-{}", i), 1);
        node.execute(ClusterCommand::DeployPipeline {
            pipeline_id,
            config: Vec::new(),
            deployed_at_ms: i as u64 * 100,
        })
        .await
        .expect("Deploy failed");
    }

    // Trigger snapshot manually
    node.trigger_snapshot().await.expect("Snapshot should work");

    // Verify metrics show snapshot
    let metrics = node.metrics();
    assert!(metrics.snapshot.is_some(), "Should have a snapshot");

    node.shutdown().await.expect("Failed to shutdown");
}
