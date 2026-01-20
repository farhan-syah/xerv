# Getting Started with XERV

This guide covers basic setup for both embedding XERV in your application and running it as a standalone service.

## Installation

Add XERV to your `Cargo.toml`:

```toml
[dependencies]
xerv-core = "0.1"
xerv-nodes = "0.1"
xerv-executor = "0.1"
tokio = { version = "1.49", features = ["full"] }
serde_json = "1.0"
```

For WASM support:

```toml
[dependencies]
xerv-executor = { version = "0.1", features = ["wasm"] }
```

## Basic Example: Order Processing

Let's build a simple order processing pipeline that validates orders and routes them based on fraud risk.

### Define the Flow (YAML)

Create `flows/order_processing.yaml`:

```yaml
name: order_processing
version: "1.0"

triggers:
  api_webhook:
    type: webhook
    params:
      port: 8080
      path: /orders

nodes:
  validate:
    type: std::switch
    config:
      condition:
        type: greater_than
        field: amount
        value: 0

  process_safe:
    type: std::log
    config:
      message: "Processing low-risk order: ${validate.order_id}"

  process_risky:
    type: std::log
    config:
      message: "Processing high-risk order: ${validate.order_id}"

  merge:
    type: std::merge

edges:
  - from: api_webhook
    to: validate

  - from: validate.true
    to: process_safe

  - from: validate.false
    to: process_risky

  - from: process_safe
    to: merge

  - from: process_risky
    to: merge
```

### Run as a Service

Create `src/main.rs`:

```rust
use xerv_executor::prelude::*;
use xerv_nodes::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Load flow from YAML
    let flow_yaml = std::fs::read_to_string("flows/order_processing.yaml")?;
    let flow_def: xerv_core::FlowDefinition = serde_yaml::from_str(&flow_yaml)?;

    // Create loader and build the flow
    let loader_config = LoaderConfig::default();
    let loaded_flow = FlowBuilder::new()
        .from_definition(flow_def)
        .load(&loader_config)?;

    // Create pipeline
    let pipeline = Pipeline::new(loaded_flow)?;

    // Start the pipeline
    pipeline.start().await?;

    // Start REST API server
    let server_config = ServerConfig {
        host: "127.0.0.1".into(),
        port: 8080,
    };

    let app_state = AppState::new()?;
    let server = ApiServer::new(server_config, app_state);

    // Run the server
    server.start().await?;

    Ok(())
}
```

Run it:

```bash
cargo run
```

Send test requests:

```bash
# Low-risk order
curl -X POST http://localhost:8080/orders \
  -H "Content-Type: application/json" \
  -d '{
    "order_id": "ORD-001",
    "amount": 100,
    "customer_id": "CUST-123"
  }'

# High-risk order (negative amount as fraud indicator)
curl -X POST http://localhost:8080/orders \
  -H "Content-Type: application/json" \
  -d '{
    "order_id": "ORD-002",
    "amount": -500,
    "customer_id": "CUST-456"
  }'

# Check health
curl http://localhost:8080/health
```

## Embedded Usage

You can also embed XERV's pipeline execution in your application:

```rust
use xerv_executor::prelude::*;
use xerv_nodes::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load and build flow
    let flow_yaml = std::fs::read_to_string("flows/order_processing.yaml")?;
    let flow_def: xerv_core::FlowDefinition = serde_yaml::from_str(&flow_yaml)?;

    let loaded_flow = FlowBuilder::new()
        .from_definition(flow_def)
        .load(&LoaderConfig::default())?;

    let pipeline = Pipeline::new(loaded_flow)?;

    // Start pipeline (enables listeners)
    pipeline.start().await?;

    // Get the executor
    let executor = pipeline.executor();

    // Manually execute traces
    let input = serde_json::json!({
        "order_id": "ORD-001",
        "amount": 500
    });

    let input_bytes = serde_json::to_vec(&input)?;

    // Execute synchronously
    let result = executor
        .execute_sync(input_bytes)
        .await?;

    println!("Execution result: {:?}", result);

    // Gracefully shutdown
    pipeline.drain().await?;

    Ok(())
}
```

## Testing Your Flow

Use the testing framework for deterministic tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use xerv_executor::prelude::*;
    use xerv_nodes::prelude::*;
    use xerv_core::types::NodeId;

    #[tokio::test]
    async fn test_order_routing() {
        // Build a test flow
        let runner = FlowRunnerBuilder::new()
            .with_fixed_time("2024-01-15T10:00:00Z")
            .add_node(
                NodeId::new(0),
                "validate",
                Box::new(SwitchNode::new(SwitchCondition::GreaterThan {
                    field: "amount".into(),
                    value: 0.0,
                })),
            )
            .add_node(
                NodeId::new(1),
                "process_safe",
                Box::new(LogNode::new("Low risk")),
            )
            .add_node(
                NodeId::new(2),
                "process_risky",
                Box::new(LogNode::new("High risk")),
            )
            .add_edge(NodeId::new(0), "true", NodeId::new(1), "in")
            .add_edge(NodeId::new(0), "false", NodeId::new(2), "in")
            .set_entry_point(NodeId::new(0))
            .build()
            .expect("flow should build");

        // Test low-risk order
        let input = serde_json::json!({
            "order_id": "ORD-001",
            "amount": 100
        });

        let result = runner
            .run(serde_json::to_vec(&input).unwrap())
            .await
            .expect("execution should succeed");

        assert!(result.is_success());
        assert!(result.node_completed(NodeId::new(1)));  // safe path
        assert!(!result.node_completed(NodeId::new(2))); // risky path not taken

        // Test high-risk order
        let input = serde_json::json!({
            "order_id": "ORD-002",
            "amount": -500
        });

        let result = runner
            .run(serde_json::to_vec(&input).unwrap())
            .await
            .expect("execution should succeed");

        assert!(result.is_success());
        assert!(!result.node_completed(NodeId::new(1))); // safe path not taken
        assert!(result.node_completed(NodeId::new(2)));  // risky path
    }
}
```

## Configuration

### Server Configuration

```rust
use xerv_executor::prelude::*;

let config = ServerConfig {
    host: "0.0.0.0".into(),  // Bind to all interfaces
    port: 3000,              // Custom port
    // Additional options:
    // - timeout: Duration,
    // - max_connections: usize,
    // - rate_limit: Option<RateLimitConfig>,
};

let server = ApiServer::new(config, app_state);
```

### Arena Configuration

```rust
use xerv_core::prelude::*;

let arena_config = ArenaConfig {
    data_dir: "/var/lib/xerv".into(),
    max_size: 1024 * 1024 * 100,  // 100 MB
    // Additional options:
    // - checkpoint_interval: usize,
    // - enable_compression: bool,
};

let arena = Arena::create(trace_id, &arena_config)?;
```

### Trigger Configuration

Triggers are configured in the YAML flow definition:

```yaml
triggers:
  webhook:
    type: webhook
    params:
      port: 8080
      path: /api/events

  cron:
    type: cron
    params:
      schedule: "0 12 * * *"
      timezone: "UTC"

  kafka:
    type: kafka
    params:
      brokers: ["kafka:9092"]
      topic: "events"
      group_id: "xerv"
```

## Monitoring and Debugging

### View Logs

XERV uses structured logging with `tracing`. View logs with:

```bash
RUST_LOG=xerv=debug cargo run
```

### Inspect Traces

Get all completed traces:

```bash
curl http://localhost:8080/traces
```

Get specific trace details:

```bash
curl http://localhost:8080/traces/{trace_id}
```

Stream trace logs:

```bash
curl http://localhost:8080/traces/{trace_id}/logs/stream
```

### Health Checks

Monitor pipeline health:

```bash
curl http://localhost:8080/health
```

Get pipeline metrics:

```bash
curl http://localhost:8080/pipelines/{pipeline_id}
```

## Next Steps

- **[Architecture](architecture.md)** - Deep dive into internals
- **[Triggers](triggers.md)** - Configure event sources
- **[Suspension System](suspension-system.md)** - Add approvals and human workflows
- **[Custom Nodes](nodes.md)** - Write your own node types
- **[REST API Reference](api.md)** - Full API documentation
- **[Testing](testing.md)** - Comprehensive testing guide

## Common Issues

### Port Already in Use

```
Error: Address already in use
```

Change the port in your configuration:

```rust
let config = ServerConfig {
    port: 3001,  // Use different port
    ..Default::default()
};
```

### Arena Corruption

If arena files become corrupted:

```bash
# Remove the arena directory to start fresh
rm -rf /var/lib/xerv/trace_*.bin

# Or configure a different path
export XERV_DATA_DIR=/tmp/xerv
```

### Trace Hangs

If a trace seems to hang:

1. Check if it's suspended: `curl http://localhost:8080/suspensions`
2. Check if it's waiting on a node: `curl http://localhost:8080/traces/{trace_id}`
3. Review logs: `curl http://localhost:8080/traces/{trace_id}/logs`

## Example Flows

See the `examples/` directory for more complex flows:

- `order-processing/` - End-to-end order workflow
- `approval-workflow/` - Multi-stage approvals with suspension
- `data-pipeline/` - Batch data processing with cron trigger
- `kafka-consumer/` - Event streaming from Kafka
