# XERV REST API

The XERV Executor exposes a comprehensive REST API for pipeline management, trace execution, and monitoring.

## Starting the API Server

```rust
use xerv_executor::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let config = ServerConfig {
        host: "127.0.0.1".into(),
        port: 8080,
    };

    let app_state = AppState::new()?;
    let server = ApiServer::new(config, app_state);

    server.start().await?;

    Ok(())
}
```

Or via CLI:
```bash
xerv-cli serve --host 127.0.0.1 --port 8080
```

## Health Endpoint

### Check server health

```
GET /health
```

**Response (200 OK):**
```json
{
  "status": "healthy",
  "timestamp": "2024-01-15T10:00:00Z"
}
```

## Pipeline Endpoints

### List pipelines

```
GET /pipelines
```

**Response (200 OK):**
```json
{
  "pipelines": [
    {
      "id": "order-processing",
      "state": "running",
      "created_at": "2024-01-15T09:00:00Z",
      "active_traces": 5
    }
  ]
}
```

### Get pipeline details

```
GET /pipelines/{pipeline_id}
```

**Response (200 OK):**
```json
{
  "id": "order-processing",
  "name": "Order Processing",
  "state": "running",
  "nodes": ["validate", "fraud_check", "process"],
  "triggers": ["webhook"],
  "metrics": {
    "total_traces": 1250,
    "succeeded": 1200,
    "failed": 50,
    "avg_duration_ms": 245
  }
}
```

## Trace Endpoints

### Get trace history

```
GET /traces
```

**Query Parameters:**
- `pipeline_id` (optional) - Filter by pipeline
- `status` (optional) - Filter by status (running, completed, failed)
- `limit` (optional, default: 100) - Maximum results
- `offset` (optional, default: 0) - Pagination offset

**Response (200 OK):**
```json
{
  "traces": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "pipeline_id": "order-processing",
      "status": "completed",
      "started_at": "2024-01-15T10:00:00Z",
      "completed_at": "2024-01-15T10:00:2.453Z",
      "duration_ms": 2453,
      "nodes_completed": 5
    }
  ],
  "total": 42,
  "offset": 0,
  "limit": 100
}
```

### Get specific trace

```
GET /traces/{trace_id}
```

**Response (200 OK):**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "pipeline_id": "order-processing",
  "status": "completed",
  "started_at": "2024-01-15T10:00:00Z",
  "completed_at": "2024-01-15T10:00:02.453Z",
  "nodes": [
    {
      "id": 0,
      "name": "validate",
      "status": "completed",
      "started_at": "2024-01-15T10:00:00.100Z",
      "completed_at": "2024-01-15T10:00:00.250Z"
    },
    {
      "id": 1,
      "name": "fraud_check",
      "status": "completed",
      "started_at": "2024-01-15T10:00:00.260Z",
      "completed_at": "2024-01-15T10:00:02.300Z"
    }
  ]
}
```

## Logs Endpoint

### Get trace logs

```
GET /traces/{trace_id}/logs
```

**Query Parameters:**
- `level` (optional) - Filter by level (debug, info, warn, error)
- `node_id` (optional) - Filter by node

**Response (200 OK):**
```json
{
  "logs": [
    {
      "timestamp": "2024-01-15T10:00:00.100Z",
      "level": "info",
      "node_id": 0,
      "node_name": "validate",
      "message": "Validating order with ID ORD-123"
    },
    {
      "timestamp": "2024-01-15T10:00:00.250Z",
      "level": "info",
      "node_id": 1,
      "node_name": "fraud_check",
      "message": "Fraud score: 0.95 (threshold: 0.8)"
    }
  ]
}
```

## Suspension Endpoints

### Get suspended traces

```
GET /suspensions
```

**Query Parameters:**
- `reason` (optional) - Filter by suspension reason
- `limit` (optional, default: 100) - Maximum results

**Response (200 OK):**
```json
{
  "suspended_traces": [
    {
      "trace_id": "550e8400-e29b-41d4-a716-446655440000",
      "pipeline_id": "order-processing",
      "suspended_at": "2024-01-15T10:05:00Z",
      "reason": "approval_pending",
      "context": {
        "amount": 5000,
        "order_id": "ORD-123"
      }
    }
  ]
}
```

### Resume suspended trace

```
POST /suspensions/{trace_id}/resume
```

**Request Body:**
```json
{
  "decision": "approve",
  "context": {
    "approved_by": "manager@example.com",
    "approval_time": "2024-01-15T10:15:00Z"
  }
}
```

**Response (200 OK):**
```json
{
  "trace_id": "550e8400-e29b-41d4-a716-446655440000",
  "resumed_at": "2024-01-15T10:15:01Z",
  "status": "resumed"
}
```

## Trigger Endpoints

### List active triggers

```
GET /triggers
```

**Response (200 OK):**
```json
{
  "triggers": [
    {
      "id": "webhook-001",
      "type": "webhook",
      "pipeline_id": "order-processing",
      "endpoint": "http://localhost:8080/webhooks/webhook-001",
      "active": true
    },
    {
      "id": "cron-001",
      "type": "cron",
      "pipeline_id": "daily-report",
      "schedule": "0 12 * * *",
      "active": true,
      "last_fired": "2024-01-15T12:00:00Z"
    }
  ]
}
```

### Manually trigger a pipeline

```
POST /triggers/{trigger_id}/fire
```

**Request Body (optional):**
```json
{
  "payload": {
    "order_id": "ORD-456",
    "amount": 2500
  }
}
```

**Response (200 OK):**
```json
{
  "trace_id": "550e8400-e29b-41d4-a716-446655440001",
  "triggered_at": "2024-01-15T10:20:00Z"
}
```

## Error Responses

All error responses follow this format:

```json
{
  "error": "error_code",
  "message": "Human-readable error description",
  "details": {
    "field": "value"
  }
}
```

### Common Error Codes

| Code | HTTP Status | Description |
|------|------------|-------------|
| `pipeline_not_found` | 404 | Pipeline does not exist |
| `trace_not_found` | 404 | Trace does not exist |
| `invalid_payload` | 400 | Request body is malformed |
| `execution_failed` | 500 | Trace execution failed |
| `suspended_trace` | 409 | Trace is suspended and cannot execute |

**Example Error Response:**
```json
{
  "error": "pipeline_not_found",
  "message": "Pipeline 'unknown-pipeline' not found",
  "details": {
    "pipeline_id": "unknown-pipeline"
  }
}
```

## Streaming Responses

Certain endpoints support Server-Sent Events (SSE) for real-time updates.

### Stream trace execution logs

```
GET /traces/{trace_id}/logs/stream
```

**Response:** Streaming JSON objects, one per line
```
{"timestamp": "...", "level": "info", "message": "..."}
{"timestamp": "...", "level": "info", "message": "..."}
```

## Rate Limiting

The API does not enforce rate limits by default. Configure limits in `ServerConfig`:

```rust
let config = ServerConfig {
    host: "127.0.0.1".into(),
    port: 8080,
    rate_limit: Some(RateLimitConfig {
        requests_per_second: 1000,
        burst_size: 100,
    }),
};
```

## Authentication

Currently, XERV does not provide built-in authentication. Recommendations:

1. **Run behind a proxy** (nginx, Caddy) with auth middleware
2. **Use network segmentation** to restrict API access
3. **Implement custom authentication** in your application layer

Future versions will support JWT and API key authentication.

## Examples

### Trigger a pipeline via webhook

```bash
curl -X POST http://localhost:8080/webhooks/webhook-001 \
  -H "Content-Type: application/json" \
  -d '{"order_id": "ORD-123", "amount": 5000}'
```

### Check pipeline health

```bash
curl http://localhost:8080/health
```

### Monitor active traces

```bash
curl "http://localhost:8080/traces?status=running&limit=10"
```

### Stream trace logs in real-time

```bash
curl http://localhost:8080/traces/550e8400-e29b-41d4-a716-446655440000/logs/stream
```

### Resume a suspended trace

```bash
curl -X POST http://localhost:8080/suspensions/550e8400-e29b-41d4-a716-446655440000/resume \
  -H "Content-Type: application/json" \
  -d '{"decision": "approve", "context": {"approved_by": "manager@example.com"}}'
```
