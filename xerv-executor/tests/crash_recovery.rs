//! Integration tests for crash recovery from WAL.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use xerv_core::logging::BufferedCollector;
use xerv_core::traits::Node;
use xerv_core::types::{NodeId, TraceId};
use xerv_core::wal::{TraceRecoveryState, Wal, WalConfig};
use xerv_executor::recovery::{CrashReplayer, RecoveryAction, RecoveryReport};
use xerv_executor::scheduler::{Executor, ExecutorConfig, FlowGraph, GraphNode};

#[allow(unused_imports)]
use common::{PassthroughNode, test_executor_config};

/// Create a simple replayer for testing action determination.
fn create_test_replayer() -> CrashReplayer {
    let wal = Arc::new(Wal::open(WalConfig::in_memory()).unwrap());
    let mut graph = FlowGraph::new();
    graph.add_node(GraphNode::new(NodeId::new(0), "test::passthrough"));
    let nodes: HashMap<NodeId, Box<dyn Node>> = HashMap::new();
    let log_collector = Arc::new(BufferedCollector::with_default_capacity());
    let executor = Arc::new(
        Executor::new(
            ExecutorConfig::default(),
            graph,
            nodes,
            Arc::clone(&wal),
            log_collector,
            Some("test_pipeline".to_string()),
        )
        .unwrap(),
    );
    CrashReplayer::new(wal, executor)
}

#[tokio::test]
async fn recovery_no_incomplete_traces() {
    let replayer = create_test_replayer();

    assert!(!replayer.has_incomplete_traces().unwrap());

    let report = replayer.recover_all().await.unwrap();
    assert_eq!(report.total_processed(), 0);
    assert!(report.recovered.is_empty());
    assert!(report.skipped.is_empty());
    assert!(report.awaiting_resume.is_empty());
}

#[tokio::test]
async fn recovery_action_await_resume_for_suspended() {
    let state = TraceRecoveryState {
        trace_id: TraceId::new(),
        last_completed_node: Some(NodeId::new(1)),
        suspended_at: Some(NodeId::new(2)),
        started_nodes: Vec::new(),
        completed_nodes: HashMap::new(),
        suspension_metadata: None,
    };

    let replayer = create_test_replayer();
    let action = replayer.determine_action(&state);

    match action {
        RecoveryAction::AwaitResume { suspended_at } => {
            assert_eq!(suspended_at, NodeId::new(2));
        }
        _ => panic!("Expected AwaitResume action"),
    }
}

#[tokio::test]
async fn recovery_action_retry_nodes_for_started() {
    let state = TraceRecoveryState {
        trace_id: TraceId::new(),
        last_completed_node: Some(NodeId::new(1)),
        suspended_at: None,
        started_nodes: vec![NodeId::new(2), NodeId::new(3)],
        completed_nodes: HashMap::new(),
        suspension_metadata: None,
    };

    let replayer = create_test_replayer();
    let action = replayer.determine_action(&state);

    match action {
        RecoveryAction::RetryNodes { nodes } => {
            assert_eq!(nodes, vec![NodeId::new(2), NodeId::new(3)]);
        }
        _ => panic!("Expected RetryNodes action"),
    }
}

#[tokio::test]
async fn recovery_action_resume_from_for_completed() {
    let state = TraceRecoveryState {
        trace_id: TraceId::new(),
        last_completed_node: Some(NodeId::new(5)),
        suspended_at: None,
        started_nodes: Vec::new(),
        completed_nodes: HashMap::new(),
        suspension_metadata: None,
    };

    let replayer = create_test_replayer();
    let action = replayer.determine_action(&state);

    match action {
        RecoveryAction::ResumeFrom { node_id } => {
            assert_eq!(node_id, NodeId::new(5));
        }
        _ => panic!("Expected ResumeFrom action"),
    }
}

#[tokio::test]
async fn recovery_action_restart_for_no_progress() {
    let state = TraceRecoveryState {
        trace_id: TraceId::new(),
        last_completed_node: None,
        suspended_at: None,
        started_nodes: Vec::new(),
        completed_nodes: HashMap::new(),
        suspension_metadata: None,
    };

    let replayer = create_test_replayer();
    let action = replayer.determine_action(&state);

    match action {
        RecoveryAction::ResumeFrom { node_id } => {
            // Should restart from beginning (node 0)
            assert_eq!(node_id, NodeId::new(0));
        }
        _ => panic!("Expected ResumeFrom action for restart"),
    }
}

#[tokio::test]
async fn recovery_report_totals() {
    let report = RecoveryReport {
        recovered: vec![TraceId::new(), TraceId::new()],
        skipped: vec![
            (TraceId::new(), "Test skip 1".to_string()),
            (TraceId::new(), "Test skip 2".to_string()),
        ],
        awaiting_resume: vec![TraceId::new()],
    };

    assert_eq!(report.total_processed(), 5);
    assert_eq!(report.recovered.len(), 2);
    assert_eq!(report.skipped.len(), 2);
    assert_eq!(report.awaiting_resume.len(), 1);
}

#[tokio::test]
async fn recovery_empty_report() {
    let report = RecoveryReport::default();

    assert_eq!(report.total_processed(), 0);
    assert!(report.recovered.is_empty());
    assert!(report.skipped.is_empty());
    assert!(report.awaiting_resume.is_empty());
}

#[tokio::test]
async fn recovery_parses_valid_suspension_metadata() {
    let metadata_json =
        r#"{"hook_id":"webhook_123","timeout_ms":30000,"reason":"waiting_for_webhook"}"#;

    let state = TraceRecoveryState {
        trace_id: TraceId::new(),
        last_completed_node: Some(NodeId::new(1)),
        suspended_at: Some(NodeId::new(2)),
        started_nodes: Vec::new(),
        completed_nodes: HashMap::new(),
        suspension_metadata: Some(metadata_json.to_string()),
    };

    let replayer = create_test_replayer();
    let action = replayer.determine_action(&state);

    // Should recognize suspension and await resume
    match action {
        RecoveryAction::AwaitResume { suspended_at } => {
            assert_eq!(suspended_at, NodeId::new(2));
        }
        _ => panic!("Expected AwaitResume action with valid metadata"),
    }
}

#[tokio::test]
async fn recovery_handles_malformed_json_metadata_gracefully() {
    // Invalid JSON should not cause panic - should fall back to default behavior
    let invalid_json = r#"{"hook_id":"webhook_123",invalid json here}"#;

    let state = TraceRecoveryState {
        trace_id: TraceId::new(),
        last_completed_node: Some(NodeId::new(1)),
        suspended_at: Some(NodeId::new(2)),
        started_nodes: Vec::new(),
        completed_nodes: HashMap::new(),
        suspension_metadata: Some(invalid_json.to_string()),
    };

    let replayer = create_test_replayer();
    let action = replayer.determine_action(&state);

    // Even with malformed metadata, should still recognize suspension
    match action {
        RecoveryAction::AwaitResume { suspended_at } => {
            assert_eq!(suspended_at, NodeId::new(2));
        }
        _ => panic!("Expected AwaitResume action even with malformed metadata"),
    }
}

#[tokio::test]
async fn recovery_handles_missing_suspension_metadata() {
    // None suspension_metadata should be handled gracefully
    let state = TraceRecoveryState {
        trace_id: TraceId::new(),
        last_completed_node: Some(NodeId::new(1)),
        suspended_at: Some(NodeId::new(2)),
        started_nodes: Vec::new(),
        completed_nodes: HashMap::new(),
        suspension_metadata: None,
    };

    let replayer = create_test_replayer();
    let action = replayer.determine_action(&state);

    // Should still handle suspension correctly with minimal metadata
    match action {
        RecoveryAction::AwaitResume { suspended_at } => {
            assert_eq!(suspended_at, NodeId::new(2));
        }
        _ => panic!("Expected AwaitResume action even without metadata"),
    }
}

#[tokio::test]
async fn recovery_metadata_with_empty_string() {
    // Empty string metadata should be handled gracefully
    let state = TraceRecoveryState {
        trace_id: TraceId::new(),
        last_completed_node: Some(NodeId::new(1)),
        suspended_at: Some(NodeId::new(2)),
        started_nodes: Vec::new(),
        completed_nodes: HashMap::new(),
        suspension_metadata: Some(String::new()),
    };

    let replayer = create_test_replayer();
    let action = replayer.determine_action(&state);

    // Empty string should still recognize suspension
    match action {
        RecoveryAction::AwaitResume { suspended_at } => {
            assert_eq!(suspended_at, NodeId::new(2));
        }
        _ => panic!("Expected AwaitResume action with empty metadata string"),
    }
}
