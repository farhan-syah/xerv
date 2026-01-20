//! Multi-node cluster tests.
//!
//! Tests cluster operations with multiple nodes.

mod common;

use common::TestCluster;
use xerv_cluster::ClusterCommand;
use xerv_core::types::PipelineId;

/// Test that a 3-node cluster can start and elect a leader.
#[tokio::test]
async fn test_multi_node_leader_election() {
    let mut cluster = TestCluster::new(3).await;

    // Initialize the cluster and add all nodes
    cluster.initialize().await;
    cluster.add_all_nodes().await;

    // Wait for leader election
    let leader = cluster
        .wait_for_leader(3000)
        .await
        .expect("Should elect a leader");

    assert_eq!(leader, 1, "First node should be leader after initialize");

    // Give followers time to stabilize
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Verify all nodes agree on the leader
    for (i, node) in cluster.nodes.iter().enumerate() {
        let node_leader = node.leader().await;
        assert_eq!(
            node_leader,
            Some(1),
            "Node {} should agree on leader, got {:?}",
            i + 1,
            node_leader
        );
    }

    cluster.shutdown().await;
}

/// Test adding nodes to a cluster.
#[tokio::test]
async fn test_multi_node_add_nodes() {
    let mut cluster = TestCluster::new(3).await;

    // Initialize with just the first node
    cluster.initialize().await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Add remaining nodes
    cluster.add_all_nodes().await;

    // Wait for cluster to stabilize
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Verify membership on all nodes
    for (i, node) in cluster.nodes.iter().enumerate() {
        let metrics = node.metrics();
        let member_count = metrics.membership_config.voter_ids().count();
        assert_eq!(
            member_count,
            3,
            "Node {} should see 3 members, saw {}",
            i + 1,
            member_count
        );
    }

    cluster.shutdown().await;
}

/// Test that commands are replicated across nodes.
#[tokio::test]
async fn test_multi_node_replication() {
    let mut cluster = TestCluster::new(3).await;
    cluster.initialize().await;
    cluster.add_all_nodes().await;

    // Wait for cluster to stabilize
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let pipeline_id = PipelineId::new("replicated-pipeline", 1);

    // Execute command on leader (node 1)
    let resp = cluster.nodes[0]
        .execute(ClusterCommand::DeployPipeline {
            pipeline_id: pipeline_id.clone(),
            config: b"test config".to_vec(),
            deployed_at_ms: 1000,
        })
        .await
        .expect("Deploy should work");
    assert!(resp.success, "Deploy should succeed");

    // Wait for replication
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Verify all nodes have the pipeline
    for (i, node) in cluster.nodes.iter().enumerate() {
        let state = node.state_machine().state().await;
        assert!(
            state.pipelines.contains_key(&pipeline_id),
            "Node {} should have the pipeline",
            i + 1
        );
    }

    cluster.shutdown().await;
}

/// Test leader forwarding - commands sent to followers are forwarded.
#[tokio::test]
async fn test_multi_node_leader_forwarding() {
    let mut cluster = TestCluster::new(3).await;
    cluster.initialize().await;
    cluster.add_all_nodes().await;

    // Wait for cluster to stabilize
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let pipeline_id = PipelineId::new("forwarded-pipeline", 1);

    // Execute command on a follower (node 2, which is index 1)
    // This should be forwarded to the leader
    let resp = cluster.nodes[1]
        .execute(ClusterCommand::DeployPipeline {
            pipeline_id: pipeline_id.clone(),
            config: b"forwarded config".to_vec(),
            deployed_at_ms: 2000,
        })
        .await
        .expect("Deploy via follower should work");

    assert!(
        resp.success,
        "Deploy via follower should succeed: {:?}",
        resp.error
    );

    // Wait for replication
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Verify the pipeline exists on all nodes
    for (i, node) in cluster.nodes.iter().enumerate() {
        let state = node.state_machine().state().await;
        assert!(
            state.pipelines.contains_key(&pipeline_id),
            "Node {} should have the pipeline after forwarding",
            i + 1
        );
    }

    cluster.shutdown().await;
}

/// Test cluster metrics.
#[tokio::test]
async fn test_multi_node_metrics() {
    let mut cluster = TestCluster::new(3).await;
    cluster.initialize().await;
    cluster.add_all_nodes().await;

    // Wait for cluster to stabilize
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Check leader metrics
    let leader_metrics = cluster.nodes[0].metrics();
    assert_eq!(
        leader_metrics.state,
        openraft::ServerState::Leader,
        "Node 1 should be leader"
    );
    assert_eq!(
        leader_metrics.current_leader,
        Some(1),
        "Leader should know it's the leader"
    );

    // Check follower metrics
    for i in 1..cluster.nodes.len() {
        let metrics = cluster.nodes[i].metrics();
        assert_eq!(
            metrics.state,
            openraft::ServerState::Follower,
            "Node {} should be follower",
            i + 1
        );
        assert_eq!(
            metrics.current_leader,
            Some(1),
            "Follower {} should know leader is 1",
            i + 1
        );
    }

    cluster.shutdown().await;
}
