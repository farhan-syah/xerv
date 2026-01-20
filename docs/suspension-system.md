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
    pub hook_id: String,                   // Unique ID to resume this trace
    pub timeout_secs: Option<u64>,         // Optional timeout in seconds
    pub timeout_action: TimeoutAction,     // What to do if timeout expires
    pub metadata_fields: Vec<String>,      // Fields to extract for approval UI
}

pub enum TimeoutAction {
    Approve,                               // Auto-approve on timeout (→ "out" port)
    Reject,                                // Auto-reject on timeout (→ "rejected" port)
    Escalate,                              // Auto-escalate on timeout (→ "escalated" port)
}
```

**Output Ports:**
- `out` - Activated when approved (manually or by timeout with `Approve` action)
- `rejected` - Activated when rejected (manually or by timeout with `Reject` action)
- `escalated` - Activated when escalated (manually or by timeout with `Escalate` action)

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
      hook_id: "order-approval-${trace_id}"
      timeout_secs: 3600         # 1 hour
      timeout_action: reject     # Auto-reject if not approved within 1 hour
      metadata_fields:           # Fields to show in approval UI
        - order_id
        - amount
        - customer_name

  process_approved:
    type: std::log
    config:
      message: "Order approved, processing..."

  process_rejected:
    type: std::log
    config:
      message: "Order rejected"

  process_standard:
    type: std::log
    config:
      message: "Standard order, processing..."

edges:
  - from: validate_order.true
    to: request_approval

  - from: validate_order.false
    to: process_standard

  - from: request_approval.out
    to: process_approved

  - from: request_approval.rejected
    to: process_rejected
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
    Timeout --> OutPort: timeout_action=Approve
    Timeout --> RejectedPort: timeout_action=Reject
    Timeout --> EscalatedPort: timeout_action=Escalate
    ResumeAPI --> OutPort: decision=approve
    ResumeAPI --> RejectedPort: decision=reject
    ResumeAPI --> EscalatedPort: decision=escalate
    OutPort --> Running: Execution continues
    RejectedPort --> Running: Execution continues
    EscalatedPort --> Running: Execution continues
    Running --> Completed: All nodes complete
    Running --> Failed: Node fails
    Completed --> [*]
    Failed --> [*]
```

## Working with Suspensions

### Querying Suspended Traces

Get all suspended traces:

```rust
let suspended = app_state.suspension_store.list_all();

for suspended_trace in suspended {
    println!("Trace {} (hook_id: {}) suspended at node {} (since {})",
        suspended_trace.trace_id,
        suspended_trace.hook_id,
        suspended_trace.suspended_at,
        suspended_trace.created_at
    );
}
```

Filter by pipeline:

```rust
let approval_pending = app_state.suspension_store
    .list_by_pipeline("order-processing");
```

### Resuming Execution

Resume a suspended trace via the REST API:

```bash
curl -X POST http://localhost:8080/api/v1/resume/{hook_id} \
  -H "Content-Type: application/json" \
  -d '{
    "decision": "approve",
    "response_data": {
      "approved_by": "manager@example.com",
      "approval_time": "2024-01-15T10:15:00Z"
    }
  }'
```

Or programmatically:

```rust
let decision = ResumeDecision::Approve {
    response_data: Some(json!({
        "approved_by": "manager@example.com",
        "approval_time": "2024-01-15T10:15:00Z"
    })),
};

executor
    .resume_suspended_trace(hook_id, decision)
    .await?;
```

**Resume decisions:**
- `ResumeDecision::Approve { response_data }` → activates `out` port
- `ResumeDecision::Reject { reason }` → activates `rejected` port
- `ResumeDecision::Escalate { details }` → activates `escalated` port

### Handling Timeouts

If a trace exceeds the timeout without resumption, the `timeout_action` determines which output port is activated:

```rust
pub enum TimeoutAction {
    Approve,   // Auto-approve → activates "out" port
    Reject,    // Auto-reject → activates "rejected" port
    Escalate,  // Auto-escalate → activates "escalated" port
}
```

**Example: Implementing automatic escalation on timeout**

```yaml
request_approval:
  type: std::wait
  config:
    hook_id: "manager-approval-${trace_id}"
    timeout_secs: 3600        # 1 hour
    timeout_action: escalate  # Auto-escalate if not approved within 1 hour
```

## Building Suspension-Aware Nodes

Custom nodes can create suspension points by returning `NodeOutput::Suspend`:

```rust
use xerv_core::suspension::{SuspensionRequest, TimeoutAction};
use xerv_core::traits::{Node, NodeOutput, Context, NodeFuture};

pub struct CustomApprovalNode {
    hook_id: String,
    timeout_secs: Option<u64>,
}

impl Node for CustomApprovalNode {
    fn info(&self) -> NodeInfo {
        NodeInfo::new("custom", "approval")
    }

    fn execute<'a>(
        &'a self,
        ctx: Context,
        inputs: HashMap<String, RelPtr<()>>,
    ) -> NodeFuture<'a> {
        Box::pin(async move {
            let input = inputs.get("in").copied().unwrap_or_else(RelPtr::null);

            // Build suspension request with metadata
            let request = SuspensionRequest::new(&self.hook_id)
                .with_metadata(json!({
                    "approval_type": "custom",
                    "resume_url": format!("/api/v1/resume/{}", self.hook_id),
                }));

            // Apply timeout if configured
            let request = if let Some(secs) = self.timeout_secs {
                request.with_timeout(secs, TimeoutAction::Reject)
            } else {
                request
            };

            // Return suspension - executor will handle persisting state
            Ok(NodeOutput::suspend(request, input))
        })
    }
}
```

The executor handles suspension automatically:
1. Flushes the arena to disk
2. Stores trace state in the suspension store
3. Removes trace from active memory
4. Writes WAL record for crash recovery

When resumed via API, the trace continues from the wait node with the decision determining which output port activates.

## Testing Suspended Traces

Use the test framework to simulate suspension workflows:

```rust
use xerv_core::suspension::TimeoutAction;
use xerv_executor::suspension::ResumeDecision;

#[tokio::test]
async fn test_approval_workflow() {
    let runner = FlowRunnerBuilder::new()
        .add_node(NodeId::new(0), "validate", Box::new(ValidateNode::default()))
        .add_node(NodeId::new(1), "request_approval", Box::new(WaitNode::new(
            "test-hook-123",                      // hook_id
            Some(3600),                           // timeout_secs
            TimeoutAction::Reject,                // timeout_action
        )))
        .add_node(NodeId::new(2), "process", Box::new(ProcessNode::default()))
        .add_node(NodeId::new(3), "handle_reject", Box::new(RejectHandler::default()))
        .add_edge(NodeId::new(0), "out", NodeId::new(1), "in")
        .add_edge(NodeId::new(1), "out", NodeId::new(2), "in")       // approved path
        .add_edge(NodeId::new(1), "rejected", NodeId::new(3), "in") // rejected path
        .set_entry_point(NodeId::new(0))
        .build()?;

    // Run the flow - it will suspend at the approval node
    let result = runner.run(input).await?;

    // Trace is now suspended
    assert!(result.node_completed(NodeId::new(0)));  // validate completed
    assert!(result.is_suspended());                   // trace is suspended

    // Now resume it programmatically with approval
    let decision = ResumeDecision::Approve {
        response_data: Some(json!({"approved_by": "test"})),
    };

    // Resume the trace
    runner.resume_suspended("test-hook-123", decision).await?;

    // Get the final result
    let final_result = runner.get_trace_result(result.trace_id).await?;
    assert!(final_result.is_success());
    assert!(final_result.node_completed(NodeId::new(2)));  // process executed (approved path)
}
```

## Persistence

Suspended traces are persisted to disk so they survive process restarts:

```rust
pub trait SuspensionStore: Send + Sync {
    /// Store a suspended trace state.
    fn suspend(&self, state: SuspendedTraceState) -> Result<()>;

    /// Get a suspended trace by hook_id.
    fn get(&self, hook_id: &str) -> Result<SuspendedTraceState>;

    /// Resume a trace with a decision (removes from store).
    fn resume(&self, hook_id: &str, decision: ResumeDecision) -> Result<SuspendedTraceState>;

    /// List all suspended traces.
    fn list_all(&self) -> Vec<SuspendedTraceState>;

    /// List suspended traces for a specific pipeline.
    fn list_by_pipeline(&self, pipeline_id: &str) -> Vec<SuspendedTraceState>;
}
```

**SuspendedTraceState fields:**
```rust
pub struct SuspendedTraceState {
    pub hook_id: String,           // Unique ID for resuming
    pub trace_id: TraceId,         // The suspended trace
    pub suspended_at: NodeId,      // Which node triggered suspension
    pub arena_path: PathBuf,       // Path to persisted arena
    pub pipeline_id: String,       // Owning pipeline
    pub created_at: DateTime<Utc>, // When suspended
    pub expires_at: Option<DateTime<Utc>>,  // Timeout expiration
    pub metadata: Value,           // Custom metadata for approval UI
}
```

Default implementation: `MemorySuspensionStore` (in-memory storage for testing)

For production, implement a custom store backed by your database:

```rust
pub struct DatabaseSuspensionStore {
    db: Arc<sqlx::Pool<sqlx::Postgres>>,
}

impl SuspensionStore for DatabaseSuspensionStore {
    fn suspend(&self, state: SuspendedTraceState) -> Result<()> {
        // Use blocking task for sync database operations
        tokio::task::block_in_place(|| {
            sqlx::query(
                "INSERT INTO suspended_traces
                 (hook_id, trace_id, pipeline_id, suspended_at, arena_path, metadata, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)"
            )
            .bind(&state.hook_id)
            .bind(state.trace_id.to_string())
            .bind(&state.pipeline_id)
            .bind(state.suspended_at.as_u32() as i32)
            .bind(state.arena_path.to_string_lossy().to_string())
            .bind(serde_json::to_string(&state.metadata)?)
            .bind(state.expires_at)
            .execute(&**self.db)
        })?;

        Ok(())
    }

    fn get(&self, hook_id: &str) -> Result<SuspendedTraceState> {
        // ... implement query by hook_id
    }

    fn resume(&self, hook_id: &str, _decision: ResumeDecision) -> Result<SuspendedTraceState> {
        // Retrieve and delete in a transaction
        // ...
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
      hook_id: "order-approval-${trace_id}"
      timeout_secs: 3600
      timeout_action: reject

  process_order:
    type: std::log

  handle_rejected:
    type: std::log
    config:
      message: "Order was rejected"

edges:
  - from: check_amount.true
    to: process_order        # Small orders bypass approval

  - from: check_amount.false
    to: request_approval     # Large orders require approval

  - from: request_approval.out
    to: process_order        # Approved orders

  - from: request_approval.rejected
    to: handle_rejected      # Rejected orders
```

### Escalation Chain

```yaml
nodes:
  request_approval:
    type: std::wait
    config:
      hook_id: "manager-approval-${trace_id}"
      timeout_secs: 3600          # 1 hour
      timeout_action: escalate    # Auto-escalate on timeout

  request_escalation:
    type: std::wait
    config:
      hook_id: "director-approval-${trace_id}"
      timeout_secs: 1800          # 30 minutes
      timeout_action: reject      # Auto-reject if director doesn't respond

  process:
    type: std::log
    config:
      message: "Order approved and processing"

  handle_rejected:
    type: std::log
    config:
      message: "Order rejected after escalation"

edges:
  - from: request_approval.out
    to: process                   # Manager approved

  - from: request_approval.escalated
    to: request_escalation        # Escalate to director

  - from: request_escalation.out
    to: process                   # Director approved

  - from: request_escalation.rejected
    to: handle_rejected           # Director rejected or timeout
```

### External System Callback

For integrations that push completion via webhook:

```yaml
request_approval:
  type: std::wait
  config:
    hook_id: "external-callback-${trace_id}"
    timeout_secs: 86400    # 24 hours
    timeout_action: reject # Reject if no callback within 24h
    metadata_fields:
      - callback_url       # For external system reference
```

External system posts completion to the resume endpoint:

```bash
curl -X POST http://localhost:8080/api/v1/resume/external-callback-abc123 \
  -H "Content-Type: application/json" \
  -d '{
    "decision": "approve",
    "response_data": {"webhook_id": "wh_123", "external_ref": "ext-456"}
  }'
```

## Monitoring Suspensions

Via REST API:

```bash
# Get all suspended traces
curl http://localhost:8080/api/v1/suspensions

# Get suspended traces for a specific pipeline
curl http://localhost:8080/api/v1/pipelines/order-processing/suspensions

# Get details for a specific suspended trace
curl http://localhost:8080/api/v1/resume/hook-id-123
```

Response format:
```json
{
  "suspensions": [
    {
      "hook_id": "order-approval-abc123",
      "trace_id": "550e8400-e29b-41d4-a716-446655440000",
      "pipeline_id": "order-processing",
      "created_at": "2024-01-15T10:00:00Z",
      "expires_at": "2024-01-15T11:00:00Z",
      "is_expired": false
    }
  ],
  "count": 1
}
```
