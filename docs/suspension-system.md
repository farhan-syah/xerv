# XERV Suspension System

The suspension system enables **human-in-the-loop workflows** where trace execution can pause, wait for external decisions, and then resume with updated context.

## Overview

A suspension temporarily halts trace execution at a specific node, allowing external systems (approvers, users, manual processes) to interact with the suspended state before resuming.

### When to Suspend

- **Approval workflows** - Orders exceeding a threshold require manager approval
- **Manual intervention** - Payment authorization, identity verification
- **Data validation** - Human review of anomalous data before processing
- **External dependencies** - Waiting for third-party confirmation

## The WaitNode

The `std::wait` node suspends trace execution and waits for external decision:

```rust
pub struct WaitNode {
    pub reason: String,                    // Why suspension occurred
    pub timeout: Duration,                 // How long to wait
    pub timeout_action: TimeoutAction,     // What to do if timeout
    pub resume_method: ResumeMethod,       // How to resume
}

pub enum TimeoutAction {
    Fail,                                  // Fail the trace
    Retry,                                 // Retry from parent
    Skip,                                  // Skip to next node
}

pub enum ResumeMethod {
    Manual,                                // Via API call
    Webhook,                               // Via external HTTP request
}
```

### Example: Approval Workflow

```yaml
nodes:
  validate_order:
    type: std::switch
    config:
      condition:
        type: greater_than
        field: amount
        value: 10000

  request_approval:
    type: std::wait
    config:
      reason: "Order exceeds 10k - manager approval required"
      timeout: 3600              # 1 hour
      timeout_action: fail       # Fail if not approved within 1 hour
      resume_method: manual      # Resume via API

  process_approved:
    type: std::log
    config:
      message: "Order approved, processing..."

  process_standard:
    type: std::log
    config:
      message: "Standard order, processing..."

edges:
  - from: validate_order.true
    to: request_approval

  - from: validate_order.false
    to: process_standard

  - from: request_approval
    to: process_approved
```

## Suspension States

### States During Suspension

```
Normal Execution
      ↓
   [WaitNode executes]
      ↓
   Trace Suspended ← External system sees suspended trace
      ↓
User makes decision (approve/reject/timeout)
      ↓
   Resume API called with decision
      ↓
Execution resumes from checkpoint
      ↓
   Normal Execution
```

### State Diagram

```mermaid
stateDiagram-v2
    [*] --> Running
    Running --> WaitNode: Execution reaches std::wait
    WaitNode --> Suspended: Node suspends trace
    Suspended --> Timeout: Duration expires
    Suspended --> ResumeAPI: Resume decision received
    Timeout --> Completed: Timeout action executed
    Timeout --> Failed: If timeout_action=Fail
    ResumeAPI --> Resumed: Decision processed
    Resumed --> Running: Execution continues
    Running --> Completed: All nodes complete
    Running --> Failed: Node fails
    Completed --> [*]
    Failed --> [*]
```

## Working with Suspensions

### Querying Suspended Traces

Get all suspended traces:

```rust
let suspended = app_state.suspension_store()
    .all_suspended()
    .await?;

for suspended_trace in suspended {
    println!("Trace {}: {} (since {})",
        suspended_trace.trace_id,
        suspended_trace.reason,
        suspended_trace.suspended_at
    );
}
```

Filter by reason:

```rust
let approval_pending = app_state.suspension_store()
    .filter(|s| s.reason == "approval_pending")
    .await?;
```

### Resuming Execution

Resume a suspended trace via the REST API:

```bash
curl -X POST http://localhost:8080/suspensions/{trace_id}/resume \
  -H "Content-Type: application/json" \
  -d '{
    "decision": "approve",
    "context": {
      "approved_by": "manager@example.com",
      "approval_time": "2024-01-15T10:15:00Z"
    }
  }'
```

Or programmatically:

```rust
let decision = ResumeDecision {
    decision: "approve",
    context: json!({
        "approved_by": "manager@example.com",
        "approval_time": "2024-01-15T10:15:00Z"
    }),
};

executor
    .resume_suspended_trace(trace_id, decision)
    .await?;
```

### Handling Timeouts

If a trace exceeds the timeout without resumption, the `timeout_action` is executed:

```rust
pub enum TimeoutAction {
    Fail,     // Fail the trace with XervError::SuspensionTimeout
    Retry,    // Retry the entire wait node
    Skip,     // Skip to the next node (treat as success)
}
```

**Example: Implementing automatic escalation on timeout**

```yaml
request_approval:
  type: std::wait
  config:
    reason: "Manager approval needed"
    timeout: 3600        # 1 hour
    timeout_action: skip # Skip approval, proceed (or log for escalation)
```

## Building Suspension-Aware Nodes

Custom nodes can create suspension points:

```rust
pub struct CustomApprovalNode {
    reason: String,
    timeout: Duration,
}

impl Node for CustomApprovalNode {
    fn execute(
        &self,
        input: RelPtr<Value>,
        ctx: &Context,
    ) -> NodeFuture {
        Box::pin(async move {
            // Check if this is a resume from suspension
            if let Some(resume_context) = ctx.trace_state().resume_context() {
                // This trace is resuming - check the decision
                match resume_context.get("decision") {
                    Some("approve") => {
                        // Continue execution
                        Ok(NodeOutput {
                            ptr: input,
                            port: "approved".into(),
                        })
                    }
                    Some("reject") => {
                        // Reject execution
                        Err(XervError::Custom("Order rejected".into()))
                    }
                    _ => Err(XervError::Custom("Invalid decision".into())),
                }
            } else {
                // First execution - create suspension request
                let suspension = SuspensionRequest {
                    reason: self.reason.clone(),
                    timeout: self.timeout,
                    context: input,
                };

                ctx.trace_state()
                    .request_suspension(suspension)
                    .await?;

                // Return "waiting" port to indicate suspension
                Ok(NodeOutput {
                    ptr: input,
                    port: "waiting".into(),
                })
            }
        })
    }
}
```

## Testing Suspended Traces

Use the test framework to simulate suspension workflows:

```rust
#[tokio::test]
async fn test_approval_workflow() {
    let runner = FlowRunnerBuilder::new()
        .add_node(NodeId::new(0), "validate", Box::new(ValidateNode::default()))
        .add_node(NodeId::new(1), "request_approval", Box::new(WaitNode::new(
            "approval_pending".into(),
            Duration::from_secs(3600),
            TimeoutAction::Fail,
            ResumeMethod::Manual,
        )))
        .add_node(NodeId::new(2), "process", Box::new(ProcessNode::default()))
        .add_edge(NodeId::new(0), "out", NodeId::new(1), "in")
        .add_edge(NodeId::new(1), "resumed", NodeId::new(2), "in")
        .set_entry_point(NodeId::new(0))
        .build()?;

    // Run the flow - it will suspend at the approval node
    let result = runner.run(input).await?;

    // Trace is now suspended
    assert!(result.node_completed(NodeId::new(0)));  // validate completed
    assert!(result.node_completed(NodeId::new(1)));  // wait node executed

    // Now resume it programmatically
    let decision = ResumeDecision {
        decision: "approve",
        context: json!({}),
    };

    // Resume the trace
    runner.resume_suspended(result.trace_id, decision).await?;

    // Get the final result
    let final_result = runner.get_trace_result(result.trace_id).await?;
    assert!(final_result.is_success());
    assert!(final_result.node_completed(NodeId::new(2)));  // process executed
}
```

## Persistence

Suspended traces are persisted to disk so they survive process restarts:

```rust
pub trait SuspensionStore: Send + Sync {
    async fn store(&self, suspension: SuspendedTraceState) -> Result<()>;
    async fn retrieve(&self, trace_id: TraceId) -> Result<Option<SuspendedTraceState>>;
    async fn all_suspended(&self) -> Result<Vec<SuspendedTraceState>>;
    async fn delete(&self, trace_id: TraceId) -> Result<()>;
}
```

Default implementation: `MemorySuspensionStore` (in-memory storage for testing)

For production, implement a custom store backed by your database:

```rust
pub struct DatabaseSuspensionStore {
    db: Arc<sqlx::Pool<sqlx::Postgres>>,
}

impl SuspensionStore for DatabaseSuspensionStore {
    async fn store(&self, suspension: SuspendedTraceState) -> Result<()> {
        sqlx::query(
            "INSERT INTO suspended_traces (trace_id, pipeline_id, reason, context)
             VALUES ($1, $2, $3, $4)"
        )
        .bind(suspension.trace_id.to_string())
        .bind(suspension.pipeline_id)
        .bind(suspension.reason)
        .bind(serde_json::to_string(&suspension.context)?)
        .execute(&**self.db)
        .await?;

        Ok(())
    }

    async fn retrieve(&self, trace_id: TraceId) -> Result<Option<SuspendedTraceState>> {
        let row = sqlx::query_as::<_, SuspendedTraceState>(
            "SELECT trace_id, pipeline_id, reason, context FROM suspended_traces
             WHERE trace_id = $1"
        )
        .bind(trace_id.to_string())
        .fetch_optional(&**self.db)
        .await?;

        Ok(row)
    }

    // ... implement other methods
}
```

## Common Patterns

### Auto-Approval for Small Orders

```yaml
nodes:
  check_amount:
    type: std::switch
    config:
      condition:
        type: less_than
        field: amount
        value: 1000

  request_approval:
    type: std::wait
    config:
      reason: "Large order approval needed"
      timeout: 3600
      timeout_action: fail

  process_order:
    type: std::log

edges:
  - from: check_amount.true
    to: process_order        # Small orders bypass approval

  - from: check_amount.false
    to: request_approval     # Large orders require approval

  - from: request_approval
    to: process_order
```

### Escalation Chain

```yaml
nodes:
  request_approval:
    type: std::wait
    config:
      reason: "Manager approval needed"
      timeout: 3600          # 1 hour
      timeout_action: skip   # Skip approval

  request_escalation:
    type: std::wait
    config:
      reason: "Approval timeout - escalating to director"
      timeout: 1800          # 30 minutes
      timeout_action: fail   # Fail if director doesn't respond

  process:
    type: std::log
```

### Webhook Resume

For integrations that can push completion:

```yaml
request_approval:
  type: std::wait
  config:
    reason: "Waiting for webhook callback"
    timeout: 86400         # 24 hours
    timeout_action: fail
    resume_method: webhook # Will receive POST to /webhooks/{trace_id}/resume
```

External system pushes completion:

```bash
curl -X POST http://localhost:8080/webhooks/{trace_id}/resume \
  -H "Content-Type: application/json" \
  -d '{"decision": "approve", "context": {"webhook_id": "wh_123"}}'
```

## Monitoring Suspensions

Via REST API:

```bash
# Get all suspended traces
curl http://localhost:8080/suspensions

# Get suspended traces for a pipeline
curl "http://localhost:8080/suspensions?pipeline_id=order-processing"
```

Or programmatically with metrics:

```rust
let metrics = app_state.pipeline_metrics(pipeline_id).await?;
println!("Suspended traces: {}", metrics.suspended_count);
println!("Avg suspension duration: {:?}", metrics.avg_suspension_time);
```
