# Writing Custom Nodes

XERV nodes are async functions that read from the arena, perform work, and write results back. This guide shows how to implement them in Rust and WebAssembly.

## Node Anatomy

Every node implements the `Node` trait:

```rust
pub trait Node: Send + Sync {
    /// Metadata about the node (ports, configuration)
    fn info(&self) -> NodeInfo;

    /// Execute the node logic
    fn execute(
        &self,
        input: RelPtr<Value>,
        ctx: &Context,
    ) -> NodeFuture;
}

pub type NodeFuture = Pin<Box<dyn Future<Output = Result<NodeOutput>> + Send>>;

pub struct NodeOutput {
    pub ptr: RelPtr<Value>,      // Data to pass downstream
    pub port: String,             // Which output port (default: "out")
}

pub struct NodeInfo {
    pub id: String,
    pub inputs: Vec<Port>,        // Input ports
    pub outputs: Vec<Port>,       // Output ports
    pub config: HashMap<String, Value>,
}

pub struct Port {
    pub name: String,
    pub direction: PortDirection,
    pub schema: String,           // Type name for validation
    pub required: bool,
    pub description: String,
}
```

## Simple Example: Validation Node

Let's build a node that validates an order:

```rust
use xerv_core::prelude::*;
use xerv_core::traits::{Node, NodeInfo, NodeOutput, Port, PortDirection};
use serde::{Deserialize, Serialize};
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};

#[derive(Serialize, Deserialize, RkyvSerialize, RkyvDeserialize, Archive)]
#[repr(C)]
pub struct Order {
    pub id: String,
    pub amount: f64,
}

#[derive(Serialize, Deserialize, RkyvSerialize, RkyvDeserialize, Archive)]
#[repr(C)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub reason: String,
}

pub struct ValidateOrderNode {
    min_amount: f64,
    max_amount: f64,
}

impl ValidateOrderNode {
    pub fn new(min_amount: f64, max_amount: f64) -> Self {
        Self {
            min_amount,
            max_amount,
        }
    }
}

impl Node for ValidateOrderNode {
    fn info(&self) -> NodeInfo {
        NodeInfo {
            id: "validate_order".into(),
            inputs: vec![
                Port::input("Order@v1"),
            ],
            outputs: vec![
                Port::output("ValidationResult@v1"),
            ],
            config: HashMap::new(),
        }
    }

    fn execute(
        &self,
        input: RelPtr<Value>,
        ctx: &Context,
    ) -> NodeFuture {
        Box::pin(async move {
            // 1. Read input from arena
            let order: &ArchivedOrder = ctx.arena.read(&input)?;

            // 2. Perform validation logic
            let is_valid = order.amount >= self.min_amount
                && order.amount <= self.max_amount;
            let reason = if is_valid {
                "Amount within bounds".into()
            } else {
                format!(
                    "Amount {} outside [{}, {}]",
                    order.amount, self.min_amount, self.max_amount
                )
            };

            // 3. Create result
            let result = ValidationResult { is_valid, reason };

            // 4. Write to arena
            let output_ptr = ctx.arena.write(&result)?;

            // 5. Return output
            Ok(NodeOutput {
                ptr: output_ptr,
                port: "out".into(),
            })
        })
    }
}
```

## Using Context

The `Context` provides access to external services:

```rust
pub trait Context: Send + Sync {
    // Arena for data storage
    fn arena(&self) -> &Arena;

    // HTTP requests
    fn http(&self) -> &dyn HttpProvider;

    // Time (useful for scheduling)
    fn clock(&self) -> &dyn ClockProvider;

    // Random number generation
    fn rng(&self) -> &dyn RngProvider;

    // Filesystem operations
    fn fs(&self) -> &dyn FsProvider;

    // Environment variables
    fn env(&self) -> &dyn EnvProvider;

    // Secret management
    fn secrets(&self) -> &dyn SecretsProvider;

    // UUID generation
    fn uuid(&self) -> &dyn UuidProvider;
}
```

Example: HTTP-enabled node

```rust
pub struct FraudCheckNode {
    api_url: String,
}

impl Node for FraudCheckNode {
    fn execute(
        &self,
        input: RelPtr<Value>,
        ctx: &Context,
    ) -> NodeFuture {
        Box::pin(async move {
            // Read order
            let order: &ArchivedOrder = ctx.arena.read(&input)?;

            // Make HTTP request to fraud service
            let response = ctx.http()
                .post(&self.api_url)
                .json(&serde_json::json!({ "order_id": &order.id }))
                .send()
                .await?;

            let fraud_score: f64 = response.json().await?;

            // Write result
            let result = FraudResult { score: fraud_score };
            let output_ptr = ctx.arena.write(&result)?;

            Ok(NodeOutput {
                ptr: output_ptr,
                port: "out".into(),
            })
        })
    }
}
```

## Conditional Nodes (Switch/Fork)

Nodes can have multiple output ports for branching:

```rust
pub struct SwitchNode {
    condition: Condition,
}

#[derive(Clone)]
pub enum Condition {
    GreaterThan { field: String, value: f64 },
    LessThan { field: String, value: f64 },
    Equals { field: String, value: String },
}

impl Node for SwitchNode {
    fn info(&self) -> NodeInfo {
        NodeInfo {
            id: "switch".into(),
            inputs: vec![Port::input("Any@v1")],
            outputs: vec![
                Port::named("true", PortDirection::Output, "Any@v1"),
                Port::named("false", PortDirection::Output, "Any@v1"),
            ],
            config: HashMap::new(),
        }
    }

    fn execute(
        &self,
        input: RelPtr<Value>,
        ctx: &Context,
    ) -> NodeFuture {
        Box::pin(async move {
            let value = ctx.arena.read(&input)?;

            // Evaluate condition
            let result = match &self.condition {
                Condition::GreaterThan { field, value: threshold } => {
                    let actual = value.get_field(field)
                        .and_then(|v| v.as_f64())
                        .ok_or(XervError::FieldNotFound {
                            field: field.clone()
                        })?;
                    actual > *threshold
                }
                // ... other conditions
            };

            // Output goes to "true" or "false" port based on result
            let port = if result { "true" } else { "false" };

            Ok(NodeOutput {
                ptr: input,  // Pass input through
                port: port.into(),
            })
        })
    }
}
```

Flow configuration:

```yaml
nodes:
  fraud_check:
    type: my_nodes::switch
    config:
      condition:
        type: greater_than
        field: fraud_score
        value: 0.8

edges:
  - from: fraud_check.true
    to: flag_suspicious

  - from: fraud_check.false
    to: process_order
```

## Merge/Barrier Nodes

Nodes can have multiple inputs:

```rust
pub struct MergeNode {
    expected_inputs: usize,
}

impl Node for MergeNode {
    fn info(&self) -> NodeInfo {
        let inputs = (0..self.expected_inputs)
            .map(|i| Port::named(
                format!("in_{}", i),
                PortDirection::Input,
                "Any@v1"
            ))
            .collect();

        NodeInfo {
            id: "merge".into(),
            inputs,
            outputs: vec![Port::output("Array@v1")],
            config: HashMap::new(),
        }
    }

    fn execute(
        &self,
        input: RelPtr<Value>,
        ctx: &Context,
    ) -> NodeFuture {
        Box::pin(async move {
            // The scheduler collects all inputs before calling execute
            // They're available in a special "merged" format

            let merged = ctx.arena.read(&input)?;
            // merged now contains all N inputs as an array

            Ok(NodeOutput {
                ptr: input,
                port: "out".into(),
            })
        })
    }
}
```

## Fan-Out Nodes

Split a collection into multiple executions:

```rust
pub struct SplitNode;

impl Node for SplitNode {
    fn info(&self) -> NodeInfo {
        NodeInfo {
            id: "split".into(),
            inputs: vec![Port::input("Array@v1")],
            outputs: vec![
                Port::named(
                    "item",
                    PortDirection::Output,
                    "Any@v1"
                ),
                Port::named(
                    "done",
                    PortDirection::Output,
                    "Nil@v1"
                ),
            ],
            config: HashMap::new(),
        }
    }

    fn execute(
        &self,
        input: RelPtr<Value>,
        ctx: &Context,
    ) -> NodeFuture {
        Box::pin(async move {
            let array = ctx.arena.read(&input)?;

            // For each item in array, scheduler will:
            // 1. Create new trace with item as input
            // 2. Route to downstream node via "item" port
            // After all items processed:
            // 3. Route to "done" port

            Ok(NodeOutput {
                ptr: input,
                port: "item".into(),  // Signal: iterate mode
            })
        })
    }
}
```

## Loop Nodes

Control repeated execution:

```rust
pub struct LoopNode {
    max_iterations: usize,
}

impl Node for LoopNode {
    fn info(&self) -> NodeInfo {
        NodeInfo {
            id: "loop".into(),
            inputs: vec![Port::input("Any@v1")],
            outputs: vec![
                Port::named("body", PortDirection::Output, "Any@v1"),
                Port::named("done", PortDirection::Output, "Any@v1"),
            ],
            config: HashMap::new(),
        }
    }

    fn execute(
        &self,
        input: RelPtr<Value>,
        ctx: &Context,
    ) -> NodeFuture {
        Box::pin(async move {
            // Read iteration count from context
            let iteration = ctx.trace_state().iteration_count();

            if iteration >= self.max_iterations {
                // Exit loop
                Ok(NodeOutput {
                    ptr: input,
                    port: "done".into(),
                })
            } else {
                // Continue loop
                Ok(NodeOutput {
                    ptr: input,
                    port: "body".into(),
                })
            }
        })
    }
}
```

Flow:

```yaml
nodes:
  process_loop:
    type: my_nodes::loop
    config:
      max_iterations: 10

  validate_item:
    type: std::log

edges:
  - from: process_loop.body
    to: validate_item

  - from: validate_item
    to: process_loop  # Back to loop for next iteration
```

## Testing Your Node

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use xerv_core::testing::{TestContextBuilder, MockClock};

    #[tokio::test]
    async fn test_validation_passes() {
        // Build a test context
        let ctx = TestContextBuilder::new()
            .with_fixed_time("2024-01-15T10:00:00Z")
            .build()
            .unwrap();

        // Create node
        let node = ValidateOrderNode::new(100.0, 5000.0);

        // Create test input
        let order = Order {
            id: "ORD-123".into(),
            amount: 1000.0,
        };
        let input_ptr = ctx.arena.write(&order).unwrap();

        // Execute
        let output = node.execute(input_ptr, &ctx).await.unwrap();

        // Assert
        let result: &ArchivedValidationResult = ctx.arena.read(&output.ptr).unwrap();
        assert!(result.is_valid);
    }

    #[tokio::test]
    async fn test_validation_fails_too_high() {
        let ctx = TestContextBuilder::new()
            .with_fixed_time("2024-01-15T10:00:00Z")
            .build()
            .unwrap();

        let node = ValidateOrderNode::new(100.0, 5000.0);

        let order = Order {
            id: "ORD-456".into(),
            amount: 10000.0,  // Exceeds max
        };
        let input_ptr = ctx.arena.write(&order).unwrap();

        let output = node.execute(input_ptr, &ctx).await.unwrap();

        let result: &ArchivedValidationResult = ctx.arena.read(&output.ptr).unwrap();
        assert!(!result.is_valid);
    }
}
```

## Integration Test: Full Flow

```rust
#[tokio::test]
async fn test_order_flow() {
    use xerv_executor::prelude::*;

    let mut runner = FlowRunnerBuilder::new()
        .with_fixed_time("2024-01-15T10:00:00Z")
        .add_node(
            NodeId::new(0),
            "validate_order",
            Box::new(ValidateOrderNode::new(100.0, 5000.0)),
        )
        .add_node(
            NodeId::new(1),
            "fraud_check",
            Box::new(SwitchNode {
                condition: Condition::GreaterThan {
                    field: "is_valid".into(),
                    value: 0.0,
                },
            }),
        )
        .add_edge(NodeId::new(0), "out", NodeId::new(1), "in")
        .set_entry_point(NodeId::new(0))
        .build()
        .unwrap();

    let input = serde_json::json!({
        "id": "ORD-123",
        "amount": 1500.0
    });

    let result = runner
        .run(serde_json::to_vec(&input).unwrap())
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.completed_nodes.len(), 2);
}
```

## WebAssembly Nodes

For custom logic in WASM:

```rust
// src/lib.rs (compiled to WASM)
#[no_mangle]
pub extern "C" fn process(input: u32) -> u32 {
    // input is a RelPtr offset
    // Unsafe WASM code to read/write arena

    let value = unsafe {
        // Read from shared memory at offset
        let ptr = input as *const f64;
        *ptr
    };

    // Do computation
    let result = value * 2.0;

    // Write back
    unsafe {
        let output_ptr = output_buffer as *mut f64;
        *output_ptr = result;
    }

    output_ptr as u32
}
```

Compile with:
```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
```

Use in YAML:
```yaml
nodes:
  compute:
    type: wasm
    config:
      module: ./nodes/compute.wasm
      function: process
```

See the [architecture.md](architecture.md) section on WASM integration for details on memory sharing and host bindings.

## Node Registration

To use your node in a flow, register it:

```rust
use xerv_nodes::prelude::*;

pub struct MyNodeFactory;

impl NodeFactory for MyNodeFactory {
    fn create(&self, node_type: &str, config: &Value) -> Result<Box<dyn Node>> {
        match node_type {
            "my_nodes::validate_order" => {
                let min = config.get("min_amount")?.as_f64()?;
                let max = config.get("max_amount")?.as_f64()?;
                Ok(Box::new(ValidateOrderNode::new(min, max)))
            }
            "my_nodes::fraud_check" => {
                Ok(Box::new(FraudCheckNode {
                    api_url: config.get("api_url")?.as_str()?.into(),
                }))
            }
            _ => Err(XervError::UnknownNodeType(node_type.into())),
        }
    }
}

// Register when loading flows
let factory = MyNodeFactory;
let flow = FlowLoader::load_file("flow.yaml", config)?;
let pipeline = Pipeline::new(flow, factory)?;
```

## Best Practices

1. **Keep nodes focused** - One responsibility per node
2. **Use schemas** - Define input/output types with rkyv Archive
3. **Handle errors** - Propagate meaningful errors, avoid unwrap()
4. **Test thoroughly** - Unit test node logic, integration test flows
5. **Document ports** - Describe what each input/output expects
6. **Minimize allocation** - Reference arena data, avoid unnecessary clones
7. **Use async** - Make HTTP calls, file I/O, etc. truly async
8. **Cache config** - Parse YAML config once in constructor

## Common Patterns

### Transformation Node

```rust
pub struct TransformNode<F> {
    transform_fn: F,
}

impl<F> Node for TransformNode<F>
where
    F: Fn(&ArchivedValue) -> Result<Value> + Send + Sync,
{
    fn execute(&self, input: RelPtr<Value>, ctx: &Context) -> NodeFuture {
        Box::pin(async move {
            let archived = ctx.arena.read(&input)?;
            let transformed = (self.transform_fn)(archived)?;
            let output_ptr = ctx.arena.write(&transformed)?;
            Ok(NodeOutput { ptr: output_ptr, port: "out".into() })
        })
    }
}
```

### Aggregation Node

```rust
pub struct AggregateNode {
    operation: AggregateOp,
}

enum AggregateOp {
    Sum,
    Average,
    Max,
}

impl Node for AggregateNode {
    fn execute(&self, input: RelPtr<Value>, ctx: &Context) -> NodeFuture {
        Box::pin(async move {
            let array = ctx.arena.read(&input)?;
            let result = match self.operation {
                AggregateOp::Sum => array.iter().sum::<f64>(),
                AggregateOp::Average => {
                    array.iter().sum::<f64>() / array.len() as f64
                }
                AggregateOp::Max => {
                    array.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                }
            };
            let output = Value::Number(result);
            let output_ptr = ctx.arena.write(&output)?;
            Ok(NodeOutput { ptr: output_ptr, port: "out".into() })
        })
    }
}
```

### Batching Node

```rust
pub struct BatchNode {
    batch_size: usize,
}

impl Node for BatchNode {
    fn execute(&self, input: RelPtr<Value>, ctx: &Context) -> NodeFuture {
        Box::pin(async move {
            let trace_state = ctx.trace_state();
            let batch_num = trace_state.batch_index();

            if batch_num < self.batch_size {
                // Accumulate
                trace_state.accumulate(input)?;
                Ok(NodeOutput {
                    ptr: input,
                    port: "accumulating".into(),
                })
            } else {
                // Emit batch
                let batch = trace_state.take_batch()?;
                let output_ptr = ctx.arena.write(&batch)?;
                Ok(NodeOutput {
                    ptr: output_ptr,
                    port: "batch".into(),
                })
            }
        })
    }
}
```
