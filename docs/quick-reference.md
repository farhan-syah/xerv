# XERV Quick Reference

Fast lookup for common tasks and commands.

## Node Types

| Node | Purpose | Input Ports | Output Ports |
|------|---------|-------------|--------------|
| `std::merge` | Wait for multiple inputs | N/A | `out` |
| `std::split` | Fan-out to collection items | `in` | `item`, `done` |
| `std::switch` | Conditional routing | `in` | `true`, `false` |
| `std::loop` | Iteration with exit condition | `in` | `body`, `done` |
| `std::wait` | Human-in-the-loop approval | `in` | `resumed`, `timeout` |
| `std::map` | Field transformation | `in` | `out` |
| `std::concat` | String concatenation | `in` | `out` |
| `std::aggregate` | Numeric aggregation | `in` | `out` |
| `std::json_dynamic` | Dynamic JSON field access | `in` | `out` |

## Trigger Types

| Trigger | Purpose | Configuration |
|---------|---------|----------------|
| `webhook` | HTTP POST endpoint | `port`, `path` |
| `cron` | Scheduled execution | `schedule` (cron expr), `timezone` |
| `filesystem` | File system events | `paths`, `events`, `patterns` |
| `queue` | In-memory message queue | `name`, `capacity` |
| `kafka` | Kafka topic consumer | `brokers`, `topic`, `group_id` |
| `manual` | Manual trigger (testing) | None |
| `memory` | Direct memory injection (benchmarks) | None |

## REST API Endpoints

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `GET` | `/health` | Health check |
| `GET` | `/pipelines` | List pipelines |
| `GET` | `/pipelines/{id}` | Get pipeline details |
| `GET` | `/traces` | List traces (filterable) |
| `GET` | `/traces/{id}` | Get trace details |
| `GET` | `/traces/{id}/logs` | Get trace logs |
| `GET` | `/traces/{id}/logs/stream` | Stream trace logs (SSE) |
| `GET` | `/triggers` | List active triggers |
| `POST` | `/triggers/{id}/fire` | Manually fire trigger |
| `GET` | `/suspensions` | Get suspended traces |
| `POST` | `/suspensions/{id}/resume` | Resume suspended trace |

## YAML Flow Syntax

### Minimal Flow

```yaml
name: my_flow
version: "1.0"

triggers:
  webhook:
    type: webhook
    params:
      port: 8080
      path: /api

nodes:
  process:
    type: std::log

edges:
  - from: webhook
    to: process
```

### Node with Configuration

```yaml
nodes:
  route:
    type: std::switch
    config:
      condition:
        type: greater_than
        field: amount
        value: 1000
```

### Edge Between Conditional Ports

```yaml
edges:
  - from: route.true
    to: process_large

  - from: route.false
    to: process_small
```

### Selector Reference

```yaml
nodes:
  process:
    type: std::log
    config:
      message: "Order ${order.id} for ${order.amount}"
```

## Cron Expression Cheat Sheet

```
┌─────────────── second (0-59, optional)
│ ┌───────────── minute (0-59)
│ │ ┌─────────── hour (0-23)
│ │ │ ┌───────── day of month (1-31)
│ │ │ │ ┌─────── month (1-12)
│ │ │ │ │ ┌───── day of week (0-6, Sunday=0)
│ │ │ │ │ │
* * * * * *
```

### Common Expressions

| Expression | Meaning |
|-----------|---------|
| `0 12 * * *` | Every day at noon |
| `0 0 * * *` | Every day at midnight |
| `0 9 * * 1` | Every Monday at 9 AM |
| `0 0 1 * *` | First day of each month |
| `*/15 * * * *` | Every 15 minutes |
| `0 0 * * 0` | Every Sunday at midnight |

## Rust Code Patterns

### Create a Pipeline Programmatically

```rust
use xerv_executor::prelude::*;

let flow_def = FlowDefinition {
    name: "order_processing".into(),
    version: "1.0".into(),
    triggers: vec![/* ... */],
    nodes: vec![/* ... */],
    edges: vec![/* ... */],
};

let loaded = FlowBuilder::new()
    .from_definition(flow_def)
    .load(&LoaderConfig::default())?;

let pipeline = Pipeline::new(loaded)?;
```

### Load Flow from YAML

```rust
let yaml = std::fs::read_to_string("flow.yaml")?;
let flow_def: FlowDefinition = serde_yaml::from_str(&yaml)?;
let loaded = FlowBuilder::new()
    .from_definition(flow_def)
    .load(&LoaderConfig::default())?;
```

### Execute a Trace

```rust
let executor = pipeline.executor();
let input = serde_json::json!({"data": "value"});
let result = executor
    .execute_sync(serde_json::to_vec(&input)?)
    .await?;
```

### Test a Flow

```rust
#[tokio::test]
async fn test_flow() {
    let runner = FlowRunnerBuilder::new()
        .with_fixed_time("2024-01-15T10:00:00Z")
        .add_node(NodeId::new(0), "node_a", Box::new(SomeNode::default()))
        .set_entry_point(NodeId::new(0))
        .build()?;

    let result = runner.run(input_bytes).await?;
    assert!(result.is_success());
}
```

### Mock HTTP in Tests

```rust
let mock_http = MockHttp::new()
    .on_post("https://api.example.com/charge")
    .respond_json(json!({"id": "ch_123"}));

let runner = FlowRunnerBuilder::new()
    .with_mock_http(mock_http)
    .build()?;
```

## Troubleshooting

### Pipeline won't start

- Check YAML syntax: `serde_yaml::from_str(yaml)?`
- Verify trigger configuration (port accessible, path valid)
- Check logs: `RUST_LOG=xerv=debug cargo run`

### Trace hangs

- Check if trace is suspended: `GET /suspensions`
- View trace state: `GET /traces/{id}`
- Stream logs: `GET /traces/{id}/logs/stream`

### Node execution fails

- Verify selector references exist: `${node_name.field_name}`
- Check input data format matches node expectations
- Review trace logs for detailed error messages

### Arena corruption

```bash
# Clear arena files
rm -rf /var/lib/xerv/trace_*.bin
# Or set custom path: XERV_DATA_DIR=/tmp/xerv
```

## Performance Tips

1. **Use selectors efficiently** - They're resolved at link time, not runtime
2. **Minimize trace serialization** - Keep data small to fit in arena efficiently
3. **Batch operations** - Group operations in a single node when possible
4. **Use appropriate triggers** - Cron for periodic, webhook for event-driven
5. **Monitor metrics** - Use REST API to track execution latency
6. **Enable compression** - If large payloads (configure in ArenaConfig)

## Development Workflow

1. **Write YAML flow** - Define structure in `flows/my_flow.yaml`
2. **Build test flow** - Use `FlowRunnerBuilder` in tests
3. **Test with mocks** - Mock HTTP, time, filesystem, secrets
4. **Verify execution** - Assert trace completion and outputs
5. **Deploy** - Load flow in production service
6. **Monitor** - Use REST API to inspect traces

## Common Configurations

### Local Development

```rust
let server_config = ServerConfig {
    host: "127.0.0.1".into(),
    port: 8080,
};
```

### Production

```rust
let server_config = ServerConfig {
    host: "0.0.0.0".into(),
    port: 3000,
    // Add rate limiting, timeouts, etc.
};
```

### Testing

```rust
let runner = FlowRunnerBuilder::new()
    .with_fixed_time("2024-01-15T10:00:00Z")
    .with_seed(42)  // Deterministic RNG
    .build()?;
```
