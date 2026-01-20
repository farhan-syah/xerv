# Testing XERV Workflows

XERV provides a testing framework with mocked external dependencies (time, HTTP, filesystem, RNG) for reliable workflow testing without side effects.

## Core Principle

Tests should be **repeatable, isolated, and fast**. By providing mocks for all I/O, you get:

- **Predictable execution** - Same input produces consistent output
- **No flakiness** - No timing issues, network timeouts, or random failures
- **Quick feedback** - No real HTTP calls, filesystem I/O, or delays
- **Test isolation** - Tests don't interfere with each other

## Quick Example

```rust
#[tokio::test]
async fn test_order_fraud_detection() {
    use xerv_executor::testing::FlowRunnerBuilder;
    use xerv_core::testing::MockClock;
    use xerv_core::types::NodeId;

    let mut runner = FlowRunnerBuilder::new()
        .with_fixed_time("2024-01-15T10:00:00Z")
        .add_node(NodeId::new(0), "fraud_check", Box::new(FraudCheckNode::default()))
        .set_entry_point(NodeId::new(0))
        .build()
        .unwrap();

    let input = serde_json::json!({ "amount": 5000.0, "risk_score": 0.95 });

    let result = runner
        .run(serde_json::to_vec(&input).unwrap())
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.completed_nodes.len(), 1);
}
```

## FlowRunner Setup

The `FlowRunnerBuilder` constructs a test execution environment:

```rust
pub struct FlowRunnerBuilder {
    graph: FlowGraph,
    nodes: HashMap<NodeId, Box<dyn Node>>,
    clock: Option<Arc<dyn ClockProvider>>,
    http: Option<Arc<dyn HttpProvider>>,
    rng: Option<Arc<dyn RngProvider>>,
    uuid: Option<Arc<dyn UuidProvider>>,
    fs: Option<Arc<dyn FsProvider>>,
    env: Option<Arc<dyn EnvProvider>>,
    secrets: Option<Arc<dyn SecretsProvider>>,
}
```

### Building a Basic Flow

```rust
let runner = FlowRunnerBuilder::new()
    // Add nodes
    .add_node(
        NodeId::new(0),
        "node_a",
        Box::new(SomeNode::default())
    )
    .add_node(
        NodeId::new(1),
        "node_b",
        Box::new(AnotherNode::default())
    )
    // Connect nodes
    .add_edge(NodeId::new(0), "out", NodeId::new(1), "in")
    // Set entry point
    .set_entry_point(NodeId::new(0))
    // Build
    .build()?;

// Execute
let input = vec![/* bytes */];
let result = runner.run(input).await?;

// Inspect results
assert!(result.is_success());
assert_eq!(result.completed_nodes.len(), 2);

// Get specific node output
if let Some(output) = result.output(NodeId::new(1)) {
    // Process output...
}
```

## FlowResult

After execution, inspect the result:

```rust
pub struct FlowResult {
    pub trace_id: TraceId,
    pub outputs: HashMap<NodeId, NodeOutput>,
    pub completed_nodes: Vec<NodeId>,
    pub error: Option<String>,
}

impl FlowResult {
    pub fn is_success(&self) -> bool;
    pub fn output(&self, node_id: NodeId) -> Option<&NodeOutput>;
    pub fn node_completed(&self, node_id: NodeId) -> bool;
}
```

Example assertions:

```rust
// Check overall success
assert!(result.is_success());

// Check specific nodes completed
assert!(result.node_completed(NodeId::new(0)));
assert!(result.node_completed(NodeId::new(1)));

// Check node count
assert_eq!(result.completed_nodes.len(), 3);

// Get and inspect output
let output = result.output(NodeId::new(2)).expect("node should complete");
// Now read from arena using output.ptr
```

## Mocking Time

Use `MockClock` to test time-dependent logic:

```rust
#[tokio::test]
async fn test_with_fixed_time() {
    let runner = FlowRunnerBuilder::new()
        .with_fixed_time("2024-01-15T10:30:45Z")
        .build()
        .unwrap();

    // All nodes see the same fixed time
}
```

Advanced clock usage:

```rust
#[tokio::test]
async fn test_with_advancing_time() {
    use xerv_core::testing::MockClock;

    let clock = MockClock::new();
    clock.advance_to("2024-01-15T10:00:00Z");

    let runner = FlowRunnerBuilder::new()
        .with_mock_clock(clock.clone())
        .build()
        .unwrap();

    // Inside node execution:
    // - ctx.clock().now() returns 2024-01-15T10:00:00Z

    // Advance time in test
    clock.advance_by(Duration::from_secs(3600));

    // Now ctx.clock().now() returns 2024-01-15T11:00:00Z
}
```

Time-dependent node example:

```rust
pub struct CronTriggerNode {
    cron_expr: String,
}

impl Node for CronTriggerNode {
    fn execute(&self, input: RelPtr<Value>, ctx: &Context) -> NodeFuture {
        Box::pin(async move {
            let now = ctx.clock().now();

            // Check if cron should fire at this time
            if self.should_fire_at(&now) {
                Ok(NodeOutput { ptr: input, port: "triggered".into() })
            } else {
                Ok(NodeOutput { ptr: input, port: "skipped".into() })
            }
        })
    }
}

#[tokio::test]
async fn test_cron_fires_at_correct_time() {
    let clock = MockClock::new();
    let runner = FlowRunnerBuilder::new()
        .with_mock_clock(clock.clone())
        .add_node(NodeId::new(0), "cron", Box::new(CronTriggerNode {
            cron_expr: "0 12 * * *".into(),  // Noon daily
        }))
        .set_entry_point(NodeId::new(0))
        .build()
        .unwrap();

    // Set time to 11:59 AM
    clock.advance_to("2024-01-15T11:59:00Z");
    let result = runner.run(vec![]).await.unwrap();
    // Should not trigger

    // Set time to 12:00 PM
    clock.advance_to("2024-01-15T12:00:00Z");
    let result = runner.run(vec![]).await.unwrap();
    // Should trigger
}
```

## Mocking HTTP

Use `MockHttp` to simulate external API calls:

```rust
#[tokio::test]
async fn test_with_http() {
    use xerv_core::testing::MockHttp;

    let runner = FlowRunnerBuilder::new()
        .with_mock_http(
            MockHttp::new()
                .on_post("https://api.stripe.com/charges")
                .respond_json(serde_json::json!({
                    "id": "ch_123",
                    "status": "succeeded"
                }))
        )
        .build()
        .unwrap();

    // Any node calling ctx.http().post() gets mocked response
}
```

Multiple endpoints:

```rust
let mock_http = MockHttp::new()
    .on_get("https://api.user.com/users/123")
    .respond_json(serde_json::json!({
        "id": "123",
        "name": "Alice",
        "email": "alice@example.com"
    }))
    .on_post("https://api.payment.com/charges")
    .respond_json(serde_json::json!({
        "id": "ch_456",
        "amount": 5000,
        "status": "succeeded"
    }))
    .on_get("https://api.fraud.com/score")
    .respond_with_status(500)  // Simulate error

let runner = FlowRunnerBuilder::new()
    .with_mock_http(mock_http)
    .build()
    .unwrap();
```

Error scenarios:

```rust
#[tokio::test]
async fn test_http_failure_handling() {
    let mock_http = MockHttp::new()
        .on_post("https://api.payment.com/charge")
        .respond_with_status(500)
        .with_body("Payment service unavailable");

    let runner = FlowRunnerBuilder::new()
        .with_mock_http(mock_http)
        .add_node(NodeId::new(0), "charge", Box::new(ChargeNode::default()))
        .set_entry_point(NodeId::new(0))
        .build()
        .unwrap();

    let result = runner.run(vec![]).await.unwrap();

    // Node should handle the error gracefully
    assert!(result.error.is_some());
}
```

Asserting on requests:

```rust
#[tokio::test]
async fn test_correct_http_calls_made() {
    let mock_http = MockHttp::new()
        .on_post("https://api.stripe.com/charges")
        .respond_json(serde_json::json!({ "id": "ch_123" }));

    let runner = FlowRunnerBuilder::new()
        .with_mock_http(mock_http.clone())
        // ... setup nodes
        .build()
        .unwrap();

    runner.run(vec![]).await.unwrap();

    // Assert that the request was made
    assert!(mock_http.request_made("POST", "https://api.stripe.com/charges"));

    // Get request details
    let requests = mock_http.requests("POST", "https://api.stripe.com/charges");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, r#"{"amount":5000}"#);
}
```

## Mocking Random Numbers

Deterministic RNG:

```rust
#[tokio::test]
async fn test_with_seeded_rng() {
    let runner = FlowRunnerBuilder::new()
        .with_seed(42)  // Same seed = same sequence
        .build()
        .unwrap();

    // First execution
    let result1 = runner.run(input.clone()).await.unwrap();

    // Second execution with same seed
    let runner2 = FlowRunnerBuilder::new()
        .with_seed(42)
        .build()
        .unwrap();
    let result2 = runner2.run(input.clone()).await.unwrap();

    // Results should be identical
    assert_eq!(result1.output(NodeId::new(0)), result2.output(NodeId::new(0)));
}
```

Sequential UUIDs (for repeatable tests):

```rust
#[tokio::test]
async fn test_with_sequential_uuids() {
    let runner = FlowRunnerBuilder::new()
        .with_sequential_uuids()
        .build()
        .unwrap();

    // Each call to ctx.uuid().new_v4() returns:
    // 00000000-0000-4000-8000-000000000000
    // 00000000-0000-4000-8000-000000000001
    // 00000000-0000-4000-8000-000000000002
    // etc.
}
```

## Mocking Filesystem

```rust
#[tokio::test]
async fn test_file_operations() {
    use xerv_core::testing::MockFs;

    let mock_fs = MockFs::new()
        .with_file("/etc/config.json", r#"{"limit": 1000}"#)
        .with_directory("/tmp/data");

    let runner = FlowRunnerBuilder::new()
        .with_mock_fs(mock_fs)
        .build()
        .unwrap();

    // Nodes can read files without real filesystem access
}
```

Test writing files:

```rust
#[tokio::test]
async fn test_writes_audit_log() {
    let mock_fs = MockFs::new();

    let runner = FlowRunnerBuilder::new()
        .with_mock_fs(mock_fs.clone())
        .add_node(NodeId::new(0), "audit", Box::new(AuditLogNode::default()))
        .set_entry_point(NodeId::new(0))
        .build()
        .unwrap();

    runner.run(vec![]).await.unwrap();

    // Assert file was written
    let written = mock_fs.file_contents("/var/log/audit.log");
    assert!(written.contains("Order processed"));
}
```

## Mocking Environment Variables

```rust
#[tokio::test]
async fn test_reads_config_from_env() {
    let mock_env = MockEnv::new()
        .with_var("API_KEY", "secret_key_123")
        .with_var("API_URL", "https://api.example.com");

    let runner = FlowRunnerBuilder::new()
        .with_mock_env(mock_env)
        .build()
        .unwrap();

    // Nodes can read env vars without real environment
}
```

## Mocking Secrets

```rust
#[tokio::test]
async fn test_with_secrets() {
    let mock_secrets = MockSecrets::new()
        .with_secret("db_password", "super_secret_123")
        .with_secret("api_token", "token_abc_xyz");

    let runner = FlowRunnerBuilder::new()
        .with_mock_secrets(mock_secrets)
        .build()
        .unwrap();

    // Nodes can retrieve secrets securely
}
```

## Testing Complex Flows

Multi-node flow with multiple branches:

```rust
#[tokio::test]
async fn test_order_processing_flow() {
    let runner = FlowRunnerBuilder::new()
        .with_fixed_time("2024-01-15T10:00:00Z")
        .with_mock_http(
            MockHttp::new()
                .on_post("https://fraud-api.com/check")
                .respond_json(serde_json::json!({ "score": 0.92 }))
        )
        // Validate node
        .add_node(NodeId::new(0), "validate", Box::new(ValidateOrderNode::new(10.0, 50000.0)))
        // Fraud check node (conditionally branches)
        .add_node(NodeId::new(1), "fraud_check", Box::new(FraudCheckNode::new("https://fraud-api.com/check")))
        // Processing nodes
        .add_node(NodeId::new(2), "process_safe", Box::new(ProcessSafeNode::default()))
        .add_node(NodeId::new(3), "process_risky", Box::new(ProcessRiskyNode::default()))
        // Merge barrier
        .add_node(NodeId::new(4), "merge", Box::new(MergeNode::new(2)))
        // Edges
        .add_edge(NodeId::new(0), "out", NodeId::new(1), "in")
        .add_edge(NodeId::new(1), "true", NodeId::new(3), "in")   // High fraud risk
        .add_edge(NodeId::new(1), "false", NodeId::new(2), "in")  // Low fraud risk
        .add_edge(NodeId::new(2), "out", NodeId::new(4), "in_0")
        .add_edge(NodeId::new(3), "out", NodeId::new(4), "in_1")
        .set_entry_point(NodeId::new(0))
        .build()
        .unwrap();

    let input = serde_json::json!({
        "id": "ORD-12345",
        "amount": 5000.0,
        "customer_id": "CUST-999"
    });

    let result = runner.run(serde_json::to_vec(&input).unwrap()).await.unwrap();

    // Assert structure
    assert!(result.is_success());
    assert_eq!(result.completed_nodes.len(), 5);

    // Assert execution path
    assert!(result.node_completed(NodeId::new(1)));  // fraud_check ran
    assert!(result.node_completed(NodeId::new(3)));  // routed to risky (score 0.92)
    assert!(!result.node_completed(NodeId::new(2)));  // safe path not taken
    assert!(result.node_completed(NodeId::new(4)));  // merge completed

    // HTTP call assertions
    // (would need to expose mock_http from runner to check)
}
```

## Testing Error Cases

```rust
#[tokio::test]
async fn test_handles_missing_field() {
    let runner = FlowRunnerBuilder::new()
        .add_node(NodeId::new(0), "extract", Box::new(ExtractFieldNode::new("price")))
        .set_entry_point(NodeId::new(0))
        .build()
        .unwrap();

    let input = serde_json::json!({
        "id": "ORD-123"
        // Missing "price" field
    });

    let result = runner.run(serde_json::to_vec(&input).unwrap()).await.unwrap();

    // Should fail gracefully
    assert!(!result.is_success());
    assert!(result.error.is_some());
    assert!(result.error.as_ref().unwrap().contains("price"));
}

#[tokio::test]
async fn test_handles_network_timeout() {
    let mock_http = MockHttp::new()
        .on_get("https://slow-api.com")
        .delay(Duration::from_secs(10));

    let runner = FlowRunnerBuilder::new()
        .with_mock_http(mock_http)
        // ... timeout configured in node or executor
        .build()
        .unwrap();

    let result = runner.run(vec![]).await.unwrap();

    assert!(!result.is_success());
    // Should timeout gracefully
}
```

## Testing Idempotency

Ensure operations are safe to retry:

```rust
#[tokio::test]
async fn test_charge_is_idempotent() {
    let mock_http = MockHttp::new()
        .on_post("https://api.stripe.com/charges")
        .respond_json(serde_json::json!({
            "id": "ch_123",
            "idempotency_key": "key_abc",
            "amount": 5000
        }));

    let runner = FlowRunnerBuilder::new()
        .with_mock_http(mock_http.clone())
        .add_node(NodeId::new(0), "charge", Box::new(ChargeNode::default()))
        .set_entry_point(NodeId::new(0))
        .build()
        .unwrap();

    let input = serde_json::json!({
        "idempotency_key": "key_abc",
        "amount": 5000
    });

    // Execute twice with same idempotency key
    let result1 = runner.run(serde_json::to_vec(&input).unwrap()).await.unwrap();
    let result2 = runner.run(serde_json::to_vec(&input).unwrap()).await.unwrap();

    // Should get same charge ID both times
    // (verify by checking HTTP calls had same key)
}
```

## Snapshot Testing

Compare against known-good outputs:

```rust
#[tokio::test]
async fn test_order_flow_snapshot() {
    let runner = FlowRunnerBuilder::new()
        // ... setup
        .build()
        .unwrap();

    let result = runner.run(test_input).await.unwrap();

    // Compare against snapshot
    insta::assert_json_snapshot!(result_to_json(&result));
}
```

## Performance Testing

```rust
#[tokio::test]
async fn bench_flow_execution() {
    use std::time::Instant;

    let runner = FlowRunnerBuilder::new()
        // ... setup
        .build()
        .unwrap();

    let start = Instant::now();
    for _ in 0..1000 {
        runner.run(test_input.clone()).await.unwrap();
    }
    let elapsed = start.elapsed();

    println!("1000 executions: {:?} ({:.2} ms/exec)",
        elapsed,
        elapsed.as_secs_f64() / 1000.0 * 1000.0);

    // Assert performance
    assert!(elapsed.as_secs() < 10);  // Should complete in < 10s
}
```

## Best Practices

1. **One assertion per test** (or tightly related group)
   - Tests are clearer and easier to debug

2. **Use descriptive names**
   - `test_order_flow_with_high_fraud_score()` not `test_flow_1()`

3. **Arrange-Act-Assert pattern**
   ```rust
   // Arrange: Set up test fixtures
   let runner = FlowRunnerBuilder::new().build()?;

   // Act: Execute the test
   let result = runner.run(input).await?;

   // Assert: Verify outcomes
   assert!(result.is_success());
   ```

4. **Mock all I/O** - No real HTTP, files, or external services
5. **Test edge cases** - Empty inputs, null values, invalid data
6. **Test error paths** - Not just the happy path
7. **Use fixed time** - Avoid flakiness from system clock
8. **Parameterized tests** - Test multiple inputs with one test
   ```rust
   #[tokio::test]
   async fn test_various_amounts() {
       for amount in [10.0, 100.0, 5000.0, 10000.0] {
           // Test each amount
       }
   }
   ```

9. **Clear test data** - Use realistic examples
10. **Organize tests** - Group by feature or node type in test modules
