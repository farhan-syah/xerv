# XERV Triggers

Triggers are event sources that initiate pipeline execution. XERV provides a standard library of triggers for common use cases.

## Overview

A trigger:
- Listens for external events (HTTP requests, file changes, scheduled times, queue messages)
- Serializes event data to JSON
- Injects the event into the pipeline as the initial trace input
- Can be enabled/disabled on the running pipeline

## Built-in Triggers

### Webhook Trigger

HTTP POST endpoint that accepts incoming events.

**Configuration:**

```yaml
triggers:
  api_webhook:
    type: webhook
    params:
      port: 8080
      path: /orders
      methods: [POST]
```

**Usage:**

```bash
curl -X POST http://localhost:8080/orders \
  -H "Content-Type: application/json" \
  -d '{"order_id": "ORD-123", "amount": 5000}'
```

**Rust:**

```rust
use xerv_nodes::prelude::*;

let trigger = WebhookTrigger::new(WebhookConfig {
    port: 8080,
    path: "/orders".into(),
    methods: vec!["POST".into()],
})?;

pipeline.add_trigger("api_webhook", Box::new(trigger)).await?;
```

### Cron Trigger

Scheduled execution using cron expressions.

**Configuration:**

```yaml
triggers:
  daily_report:
    type: cron
    params:
      schedule: "0 12 * * *"  # Noon daily
      timezone: "America/New_York"
```

**Cron Expression Format:**

```
┌─────────────── second (0 - 59, optional)
│ ┌───────────── minute (0 - 59)
│ │ ┌─────────── hour (0 - 23)
│ │ │ ┌───────── day of month (1 - 31)
│ │ │ │ ┌─────── month (1 - 12)
│ │ │ │ │ ┌───── day of week (0 - 6) (Sunday - Saturday)
│ │ │ │ │ │
* * * * * *
```

**Examples:**

- `"0 12 * * *"` - Every day at noon
- `"0 9 * * 1"` - Every Monday at 9 AM
- `"0 0 1 * *"` - First day of each month at midnight
- `"*/15 * * * *"` - Every 15 minutes
- `"0 0 * * *"` - Daily at midnight

**Rust:**

```rust
use xerv_nodes::prelude::*;

let trigger = CronTrigger::new(CronConfig {
    schedule: "0 12 * * *".into(),
    timezone: "America/New_York".into(),
})?;

pipeline.add_trigger("daily_report", Box::new(trigger)).await?;
```

### Filesystem Trigger

Watches directories for file creation, modification, or deletion.

**Configuration:**

```yaml
triggers:
  file_watcher:
    type: filesystem
    params:
      paths:
        - /data/input
        - /data/staging
      events: [created, modified]
      patterns:
        - "*.json"
        - "*.csv"
```

**Usage:**

When a file matching the pattern is created in a watched directory, the pipeline executes with:

```json
{
  "path": "/data/input/orders-2024-01-15.json",
  "event": "created",
  "size": 4096,
  "metadata": {
    "created_at": "2024-01-15T10:00:00Z",
    "permissions": "0644"
  }
}
```

**Rust:**

```rust
use xerv_nodes::prelude::*;

let trigger = FilesystemTrigger::new(FilesystemConfig {
    paths: vec!["/data/input".into()],
    events: vec!["created".into(), "modified".into()],
    patterns: vec!["*.json".into()],
})?;

pipeline.add_trigger("file_watcher", Box::new(trigger)).await?;
```

### Queue Trigger

In-memory message queue for inter-process communication.

**Configuration:**

```yaml
triggers:
  task_queue:
    type: queue
    params:
      name: "processing_queue"
      capacity: 1000
```

**Usage:**

```rust
use xerv_nodes::prelude::*;

// Get a queue handle from the trigger
let queue_handle = trigger.queue_handle();

// Producer: Send a message
queue_handle.send(QueueMessage {
    id: "msg-123".into(),
    payload: json!({
        "task_id": "task-001",
        "priority": "high"
    }),
}).await?;

// Consumer: The pipeline receives the message as trace input
```

**Rust:**

```rust
use xerv_nodes::prelude::*;

let trigger = QueueTrigger::new(QueueConfig {
    name: "processing_queue".into(),
    capacity: 1000,
})?;

let queue_handle = trigger.queue_handle();
pipeline.add_trigger("task_queue", Box::new(trigger)).await?;

// Send messages
queue_handle.send(message).await?;
```

### Kafka Trigger

Kafka topic consumer for distributed event streaming.

**Configuration:**

```yaml
triggers:
  kafka_consumer:
    type: kafka
    params:
      brokers:
        - "localhost:9092"
        - "localhost:9093"
      topic: "orders"
      group_id: "xerv-order-processor"
      auto_offset_reset: "earliest"
```

**Usage:**

Consumes messages from Kafka topic. Each message becomes a pipeline trace:

```json
{
  "key": "order-123",
  "value": {
    "order_id": "ORD-123",
    "amount": 5000,
    "customer_id": "CUST-999"
  },
  "partition": 0,
  "offset": 42,
  "timestamp": "2024-01-15T10:00:00Z"
}
```

**Rust:**

```rust
use xerv_nodes::prelude::*;

let trigger = KafkaTrigger::new(KafkaConfig {
    brokers: vec!["localhost:9092".into()],
    topic: "orders".into(),
    group_id: "xerv-order-processor".into(),
    auto_offset_reset: "earliest".into(),
})?;

pipeline.add_trigger("kafka_consumer", Box::new(trigger)).await?;
```

### Memory Trigger (Testing/Benchmarking)

Direct memory injection for testing and performance analysis.

**Usage:**

```rust
use xerv_nodes::prelude::*;

let trigger = MemoryTrigger::new();
let injector = trigger.injector();

pipeline.add_trigger("memory_trigger", Box::new(trigger)).await?;

// Inject events directly
for i in 0..1000 {
    injector.inject(json!({
        "batch": i,
        "count": 100
    })).await?;
}
```

### Manual Trigger (Testing)

Manual invocation for controlled testing.

**Rust:**

```rust
use xerv_nodes::prelude::*;

let trigger = ManualTrigger::new();
let fire_handle = trigger.fire_handle();

pipeline.add_trigger("manual", Box::new(trigger)).await?;

// Manually trigger execution
fire_handle.fire(json!({
    "test_data": "value"
})).await?;
```

## Implementing Custom Triggers

Implement the `Trigger` trait:

```rust
use xerv_core::prelude::*;
use async_trait::async_trait;

pub struct CustomTrigger {
    config: CustomTriggerConfig,
}

#[async_trait]
impl Trigger for CustomTrigger {
    async fn start(&mut self, callback: TriggerCallback) -> Result<()> {
        // Start listening for events
        loop {
            // When an event occurs
            let event_data = self.listen_for_event().await?;

            // Serialize to JSON
            let json_value = serde_json::to_value(&event_data)?;

            // Call the callback with the event
            callback(json_value).await?;
        }

        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        // Clean up resources
        Ok(())
    }

    fn info(&self) -> TriggerInfo {
        TriggerInfo {
            id: "custom_trigger".into(),
            trigger_type: "custom".into(),
        }
    }
}
```

Register your custom trigger:

```rust
let trigger = CustomTrigger::new(config);
pipeline.add_trigger("my_trigger", Box::new(trigger)).await?;
```

## Trigger Lifecycle

```
1. Pipeline starts
2. All enabled triggers call start()
3. Trigger listens for events in background task
4. When event occurs, trigger calls callback with JSON data
5. Pipeline creates new trace with event data as input
6. Trace executes through the DAG
7. When pipeline stops, all triggers call stop()
```

## Managing Triggers

### List active triggers

```rust
let triggers = pipeline.list_triggers().await?;
for trigger in triggers {
    println!("{}: {} (active: {})",
        trigger.id,
        trigger.trigger_type,
        trigger.is_active);
}
```

### Enable/disable triggers

```rust
// Disable a trigger (stop listening for events)
pipeline.disable_trigger("api_webhook").await?;

// Re-enable it
pipeline.enable_trigger("api_webhook").await?;
```

### Remove a trigger

```rust
pipeline.remove_trigger("api_webhook").await?;
```

## Testing with Triggers

Use `ManualTrigger` in tests:

```rust
#[tokio::test]
async fn test_order_processing_flow() {
    let runner = FlowRunnerBuilder::new()
        .add_trigger("manual", ManualTrigger::new())
        // ... add nodes
        .build()?;

    let fire_handle = runner.trigger_handle("manual")?;

    // Trigger execution with test data
    let trace_id = fire_handle.fire(json!({
        "order_id": "TEST-001",
        "amount": 1000
    })).await?;

    // Wait for execution
    let result = runner.wait_for_trace(trace_id).await?;
    assert!(result.is_success());
}
```

## Best Practices

1. **Choose the right trigger** - Use webhook for synchronous events, cron for scheduled tasks, kafka for event streams
2. **Handle errors gracefully** - Triggers should retry on transient failures
3. **Monitor trigger health** - Track trigger uptime and event processing latency
4. **Test trigger integration** - Use manual triggers in tests
5. **Document event format** - Clearly specify the JSON schema for trigger events
6. **Rate limit webhooks** - Consider implementing backpressure for high-volume triggers
7. **Secure webhooks** - Use API keys, IP whitelisting, or signature verification in production

## Trigger Patterns

### Fan-Out from Multiple Webhooks

```yaml
triggers:
  orders_webhook:
    type: webhook
    params:
      path: /orders
  shipments_webhook:
    type: webhook
    params:
      path: /shipments
  payments_webhook:
    type: webhook
    params:
      path: /payments

nodes:
  process:
    type: std::log
```

All webhooks feed into the same pipeline.

### Periodic Reconciliation

```yaml
triggers:
  hourly_check:
    type: cron
    params:
      schedule: "0 * * * *"

nodes:
  reconcile_database:
    type: custom::reconciliation
```

### Event-Driven with Kafka

```yaml
triggers:
  orders_stream:
    type: kafka
    params:
      brokers: [kafka:9092]
      topic: "orders"
      group_id: "xerv-processor"

nodes:
  validate:
    type: std::switch
  process:
    type: std::log
```
