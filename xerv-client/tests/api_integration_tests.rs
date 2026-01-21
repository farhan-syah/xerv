//! Integration tests for xerv-client API operations.
//!
//! These tests use wiremock to simulate server responses and verify
//! that the client correctly handles various API scenarios.

use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use xerv_client::{Client, ClientError, TraceId};

#[tokio::test]
async fn test_deploy_pipeline_success() {
    let mock_server = MockServer::start().await;

    // Mock successful deployment response
    Mock::given(method("POST"))
        .and(path("/api/v1/pipelines"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "pipeline_id": "test-pipeline-123",
            "status": "deployed"
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let yaml = "name: test\nversion: 1.0.0";

    let result = client.deploy_pipeline(yaml).await;
    assert!(result.is_ok());

    let pipeline = result.unwrap();
    assert_eq!(pipeline.pipeline_id, "test-pipeline-123");
}

#[tokio::test]
async fn test_deploy_pipeline_server_error() {
    let mock_server = MockServer::start().await;

    // Mock server error
    Mock::given(method("POST"))
        .and(path("/api/v1/pipelines"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": "Internal server error"
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let yaml = "name: test\nversion: 1.0.0";

    let result = client.deploy_pipeline(yaml).await;
    assert!(result.is_err());

    match result {
        Err(ClientError::Api { status, message }) => {
            assert_eq!(status, 500);
            assert!(message.contains("Internal server error"));
        }
        _ => panic!("Expected API error"),
    }
}

#[tokio::test]
async fn test_list_pipelines() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pipelines"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "pipelines": [
                {
                    "pipeline_id": "pipeline-1",
                    "name": "Test Pipeline 1",
                    "version": "1.0.0",
                    "status": "running",
                    "trigger_count": 2,
                    "node_count": 5
                },
                {
                    "pipeline_id": "pipeline-2",
                    "name": "Test Pipeline 2",
                    "version": "2.0.0",
                    "status": "paused",
                    "trigger_count": 1,
                    "node_count": 3
                }
            ],
            "count": 2
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let result = client.list_pipelines().await;

    assert!(result.is_ok());
    let pipelines = result.unwrap();
    assert_eq!(pipelines.len(), 2);
    assert_eq!(pipelines[0].pipeline_id, "pipeline-1");
    assert_eq!(pipelines[1].pipeline_id, "pipeline-2");
}

#[tokio::test]
async fn test_get_pipeline() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pipelines/test-pipeline"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "pipeline_id": "test-pipeline",
            "name": "Test Pipeline",
            "version": "1.0.0",
            "status": "running",
            "trigger_count": 3,
            "node_count": 7
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let result = client.get_pipeline("test-pipeline").await;

    assert!(result.is_ok());
    let pipeline = result.unwrap();
    assert_eq!(pipeline.pipeline_id, "test-pipeline");
    assert_eq!(pipeline.name, "Test Pipeline");
    assert_eq!(pipeline.trigger_count, 3);
}

#[tokio::test]
async fn test_get_pipeline_not_found() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pipelines/nonexistent"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "error": "Pipeline not found"
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let result = client.get_pipeline("nonexistent").await;

    assert!(result.is_err());
    match result {
        Err(ClientError::Api { status, .. }) => assert_eq!(status, 404),
        _ => panic!("Expected 404 error"),
    }
}

#[tokio::test]
async fn test_delete_pipeline() {
    let mock_server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/api/v1/pipelines/test-pipeline"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let result = client.delete_pipeline("test-pipeline").await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_fire_trigger() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path(
            "/api/v1/pipelines/test-pipeline/triggers/webhook-1/fire",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "trace_id": "550e8400-e29b-41d4-a716-446655440000",
            "pipeline_id": "test-pipeline",
            "trigger_id": "webhook-1"
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let payload = json!({"user_id": "123", "action": "login"});

    let result = client
        .fire_trigger("test-pipeline", "webhook-1", &payload)
        .await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.pipeline_id, "test-pipeline");
    assert_eq!(response.trigger_id, "webhook-1");
}

#[tokio::test]
async fn test_list_traces() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/traces"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "traces": [
                {
                    "trace_id": "550e8400-e29b-41d4-a716-446655440000",
                    "pipeline_id": "test-pipeline",
                    "status": "running",
                    "started_at": "2024-01-15T10:00:00Z",
                    "completed_at": null
                }
            ],
            "count": 1
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let result = client.list_traces().await;

    assert!(result.is_ok());
    let traces = result.unwrap();
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].pipeline_id, "test-pipeline");
}

#[tokio::test]
async fn test_get_trace() {
    let mock_server = MockServer::start().await;
    let trace_id =
        TraceId::from_uuid(uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap());

    Mock::given(method("GET"))
        .and(path("/api/v1/traces/550e8400-e29b-41d4-a716-446655440000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "trace_id": "550e8400-e29b-41d4-a716-446655440000",
            "pipeline_id": "test-pipeline",
            "status": "completed",
            "started_at": "2024-01-15T10:00:00Z",
            "completed_at": "2024-01-15T10:05:00Z",
            "nodes_executed": 5,
            "error": null
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let result = client.get_trace(trace_id).await;

    assert!(result.is_ok());
    let trace = result.unwrap();
    assert_eq!(trace.pipeline_id, "test-pipeline");
    assert_eq!(trace.nodes_executed, 5);
    assert!(trace.error.is_none());
}

#[tokio::test]
async fn test_query_logs_with_filters() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/logs"))
        .and(query_param("level", "error"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "logs": [
                {
                    "id": 1,
                    "timestamp": "2024-01-15T10:30:00.000Z",
                    "level": "error",
                    "category": "system",
                    "message": "Test error message",
                    "trace_id": null,
                    "node_id": null,
                    "pipeline_id": null,
                    "fields": {}
                }
            ],
            "count": 1
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let result = client
        .query_logs(Some("error"), None, None, None, Some(10))
        .await;

    assert!(result.is_ok());
    let logs = result.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].level, "error");
}

#[tokio::test]
async fn test_get_recent_logs() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/logs/recent"))
        .and(query_param("limit", "20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "logs": [],
            "count": 0
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let result = client.get_recent_logs(20).await;

    assert!(result.is_ok());
    let logs = result.unwrap();
    assert_eq!(logs.len(), 0);
}

#[tokio::test]
async fn test_get_log_stats() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/logs/stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "total": 150,
            "capacity": 1000,
            "by_level": {
                "trace": 10,
                "debug": 20,
                "info": 100,
                "warn": 15,
                "error": 5
            }
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let result = client.get_log_stats().await;

    assert!(result.is_ok());
    let stats = result.unwrap();
    assert_eq!(stats.total, 150);
    assert_eq!(stats.capacity, 1000);
    assert_eq!(*stats.by_level.get("error").unwrap(), 5);
}

#[tokio::test]
async fn test_clear_logs() {
    let mock_server = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/api/v1/logs"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let result = client.clear_logs().await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_client_with_api_key() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pipelines"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "pipelines": [],
            "count": 0
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri())
        .unwrap()
        .with_api_key("test-api-key");

    let result = client.list_pipelines().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_client_authentication_failure() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pipelines"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": "Unauthorized"
        })))
        .mount(&mock_server)
        .await;

    let client = Client::new(mock_server.uri()).unwrap();
    let result = client.list_pipelines().await;

    assert!(result.is_err());
    match result {
        Err(ClientError::Api { status, message }) => {
            assert_eq!(status, 401);
            assert!(message.contains("Unauthorized"));
        }
        _ => panic!("Expected 401 authentication error"),
    }
}
