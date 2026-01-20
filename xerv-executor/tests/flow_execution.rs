//! Integration tests for flow execution.
//!
//! Tests topological ordering and graph construction.

mod common;

use std::sync::Arc;
use xerv_core::types::NodeId;
use xerv_core::wal::Wal;
use xerv_executor::scheduler::Executor;

use common::{
    build_diamond_flow, build_linear_flow, test_executor_config, test_log_collector,
    test_wal_config,
};

#[test]
fn topological_order_single_node() {
    let (graph, nodes) = build_linear_flow(1);
    let wal = Arc::new(Wal::open(test_wal_config()).unwrap());
    let log_collector = test_log_collector();

    let executor = Executor::new(
        test_executor_config(),
        graph,
        nodes,
        wal,
        log_collector,
        Some("test_pipeline".to_string()),
    )
    .unwrap();

    let order = executor.execution_order();
    assert_eq!(order.len(), 2); // trigger + 1 node
    assert_eq!(order[0], NodeId::new(0)); // trigger first
    assert_eq!(order[1], NodeId::new(1));
}

#[test]
fn topological_order_linear_flow() {
    let (graph, nodes) = build_linear_flow(5);
    let wal = Arc::new(Wal::open(test_wal_config()).unwrap());
    let log_collector = test_log_collector();

    let executor = Executor::new(
        test_executor_config(),
        graph,
        nodes,
        wal,
        log_collector,
        Some("test_pipeline".to_string()),
    )
    .unwrap();

    let order = executor.execution_order();
    assert_eq!(order.len(), 6); // trigger + 5 nodes

    for (i, &node_id) in order.iter().enumerate() {
        assert_eq!(node_id, NodeId::new(i as u32));
    }
}

#[test]
fn topological_order_diamond_flow() {
    let (graph, nodes) = build_diamond_flow();
    let wal = Arc::new(Wal::open(test_wal_config()).unwrap());
    let log_collector = test_log_collector();

    let executor = Executor::new(
        test_executor_config(),
        graph,
        nodes,
        wal,
        log_collector,
        Some("test_pipeline".to_string()),
    )
    .unwrap();

    let order = executor.execution_order();
    assert_eq!(order.len(), 5); // trigger, A, B, C, D

    let pos = |id: u32| order.iter().position(|n| *n == NodeId::new(id)).unwrap();

    // Trigger must come first
    assert_eq!(pos(0), 0);

    // A comes after trigger
    assert!(pos(1) > pos(0));

    // B and C come after A
    assert!(pos(2) > pos(1));
    assert!(pos(3) > pos(1));

    // D comes after both B and C
    assert!(pos(4) > pos(2));
    assert!(pos(4) > pos(3));
}

#[test]
fn executor_tracks_active_traces() {
    let (graph, nodes) = build_linear_flow(2);
    let wal = Arc::new(Wal::open(test_wal_config()).unwrap());
    let log_collector = test_log_collector();

    let executor = Executor::new(
        test_executor_config(),
        graph,
        nodes,
        wal,
        log_collector,
        Some("test_pipeline".to_string()),
    )
    .unwrap();

    // Initially no active traces
    assert_eq!(executor.active_trace_count(), 0);
}
