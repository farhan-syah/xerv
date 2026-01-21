# XERV Architecture

This document explains how XERV executes traces with efficient memory usage, crash recovery, and deterministic scheduling.

## System Overview

XERV has three layers, with optional distributed clustering:

```mermaid
graph TB
    subgraph Cluster["CLUSTER PLANE (Optional)"]
        subgraph Raft["Raft Consensus"]
            R1["Leader election and failover"]
            R2["State replication via OpenRaft"]
            R3["gRPC network transport"]
        end
    end

    subgraph CP["CONTROL PLANE"]
        subgraph PipCon["Pipeline Controller"]
            PC1["Start/pause/stop/drain lifecycle"]
            PC2["Listener pool<br/>(webhooks, cron, fs watches)"]
            PC3["Trace queue management"]
            PC4["Circuit breaker<br/>(error rate management)"]
        end
    end

    subgraph EP["EXECUTION PLANE"]
        subgraph Exec["Executor (Topological Scheduler)"]
            E1["DAG validation<br/>(cycle detection)"]
            E2["Topological sort of nodes"]
            E3["Concurrent execution<br/>with async/await"]
            E4["Signal-based coordination<br/>(node completion)"]
        end
        subgraph Link["Linker (Selector Resolution)"]
            L1["Parse selectors:<br/>${node.field} → RelPtr&lt;T&gt;"]
            L2["Type-safe field access<br/>at link time"]
            L3["Pipeline config injection"]
        end
    end

    subgraph DP["DATA PLANE"]
        subgraph Arena["Arena (Memory-Mapped Append-Only Storage)"]
            A1["Rkyv-serialized data entries"]
            A2["Relative pointers (RelPtr&lt;T&gt;)<br/>for efficient access"]
            A3["One arena per trace<br/>(persistent across restarts)"]
            A4["Automatic cleanup of completed traces"]
        end
        subgraph WAL["Write-Ahead Log (WAL)"]
            W1["Records node completions"]
            W2["Enables crash recovery and replay"]
            W3["Automatic segment purging"]
        end
    end

    Cluster <--> CP
    CP <--> EP
    EP <--> DP
```

## Data Plane: Memory-Efficient Storage

### Arena File Format

Each trace has its own arena file at `/tmp/xerv/trace_{trace_id}.bin`:

```mermaid
graph TD
    subgraph Layout["Arena File Layout"]
        H["<b>Offset 0 (256 B)</b><br/>Header<br/>(metadata + pipeline config offset)"]
        C["<b>Offset 256 (N bytes)</b><br/>Pipeline Config<br/>(rkyv-serialized, read-only)"]
        D["<b>Offset 256+N (∞)</b><br/>Data Region<br/>(append-only)"]
    end

    H --> C
    C --> D

    style H fill:#e3f2fd
    style C fill:#f3e5f5
    style D fill:#e8f5e9
```

The **Header** contains:

- Trace ID (UUID)
- Current data offset (points to end of written data)
- Pipeline config offset and size
- WAL file path
- Timestamps (created, modified)

### Memory Mapping

The arena uses memory mapping via `memmap2`:

```rust
// Writer creates the file and mmaps it
let file = File::create("/tmp/xerv/trace_abc123.bin")?;
let mut arena = Arena::create(trace_id, config)?;

// Nodes write data to the mmap'd region
let ptr = arena.write(&my_data)?;  // Returns RelPtr<T>

// Other nodes access via offset (no allocation)
let read_guard = arena.read(ptr)?;  // Returns archived reference
```

Benefits:

- **Efficient data passing** between writes and reads
- **Persistent** across process restarts (pointer is just an offset)
- **Multi-reader safe** (OS handles concurrent access)
- **Optimized paging** - kernel page cache handles memory efficiently

### Rkyv Serialization

All data is serialized with **rkyv** (Relative Knowledge Virtual Machine), which enables efficient deserialization without allocations:

```rust
use rkyv::{Serialize, Archive, Deserialize};

#[derive(Serialize, Archive, Deserialize)]
pub struct Order {
    pub id: String,
    pub amount: f64,
    pub risk_score: f64,
}

// Writer: Serialize and write to arena
let order = Order { id: "123".into(), amount: 100.0, risk_score: 0.5 };
let bytes = rkyv::to_bytes::<_, 256>(&order)?;
let ptr = arena.write_bytes(&bytes)?;

// Reader: Access via archived reference
let archived: &ArchivedOrder = unsafe { arena.archived_ref::<Order>(ptr)? };
println!("Amount: {}", archived.amount);  // Efficient field access
```

### Relative Pointers (RelPtr<T>)

A `RelPtr<T>` stores the offset from the arena base, not an absolute address:

```rust
pub struct RelPtr<T> {
    offset: u32,    // Relative to arena base (not memory address!)
    _phantom: PhantomData<T>,
}

impl<T> RelPtr<T> {
    pub fn offset(&self) -> u32 {
        self.offset
    }

    pub fn from_offset(offset: u32) -> Self {
        Self {
            offset,
            _phantom: PhantomData,
        }
    }

    // Access the data (requires arena context)
    pub fn deref(&self, arena: &Arena) -> Result<&ArchivedT> {
        let base = arena.data_region_ptr();
        unsafe {
            let addr = (base as usize + self.offset as usize) as *const ArchivedT;
            Ok(&*addr)
        }
    }
}
```

Why relative offsets?

- Survive process restarts (mmap may remap at different virtual address)
- Safe for serialization (not absolute memory addresses)
- Compact storage (u32 offset vs u64 pointer)

## Execution Plane: Scheduling

### DAG Construction

The **FlowBuilder** converts YAML to a **FlowGraph**:

```rust
pub struct FlowGraph {
    nodes: HashMap<NodeId, GraphNode>,
    edges: Vec<Edge>,
    entry_points: Vec<NodeId>,
}

pub struct GraphNode {
    id: NodeId,
    node_type: String,  // e.g., "std::switch", "std::merge"
}

pub struct Edge {
    from_node: NodeId,
    from_port: String,
    to_node: NodeId,
    to_port: String,
}
```

Example: Order processing flow

```yaml
nodes:
  fraud_check: # NodeId 1
    type: std::switch

  process_safe: # NodeId 2
    type: std::log

  process_risky: # NodeId 3
    type: std::log

  merge: # NodeId 4
    type: std::merge

edges:
  - from: fraud_check.false → process_safe
  - from: fraud_check.true → process_risky
  - from: process_safe → merge
  - from: process_risky → merge
```

Results in DAG:

```mermaid
graph LR
    FC["fraud_check<br/>(1)"]
    PS["process_safe<br/>(2)"]
    PR["process_risky<br/>(3)"]
    M["merge<br/>(4)"]

    FC -->|false| PS
    FC -->|true| PR
    PS --> M
    PR --> M
```

### DAG Validation and Topological Sorting

The Executor validates the DAG structure and stores a flattened topological order:

```rust
pub struct Executor {
    graph: Arc<FlowGraph>,
    execution_order: Vec<NodeId>,  // Flat topological order (not levels!)
    nodes: HashMap<NodeId, Arc<Box<dyn Node>>>,
}

impl Executor {
    pub fn new(
        graph: FlowGraph,
        nodes: HashMap<NodeId, Box<dyn Node>>
    ) -> Result<Self> {
        // 1. Validate DAG (cycle detection via graph validation)
        graph.validate()?;

        // 2. Topological sort (Kahn's algorithm) - returns flat order
        let execution_order = graph.topological_sort()?;

        // 3. Wrap nodes in Arc for concurrent task access
        let nodes: HashMap<NodeId, Arc<Box<dyn Node>>> = nodes
            .into_iter()
            .map(|(id, node)| (id, Arc::new(node)))
            .collect();

        Ok(Self {
            graph: Arc::new(graph),
            execution_order,
            nodes,
        })
    }
}
```

**Important:** The topological sort produces a valid execution order but does NOT enforce strict levels. Instead, the work-stealing scheduler at runtime dynamically finds ready nodes (all dependencies satisfied) and executes them concurrently.

For the order processing example:

```
Flat topological order: [fraud_check, process_safe, process_risky, merge]

Runtime execution (work-stealing):
┌─────────────────────────────────────────┐
│ Step 1: Find ready nodes = [fraud_check]│
│         Execute: fraud_check             │
├─────────────────────────────────────────┤
│ Step 2: fraud_check completes           │
│         Find ready = [process_safe,     │
│                       process_risky]    │
│         Execute both in parallel         │
├─────────────────────────────────────────┤
│ Step 3: Both complete                    │
│         Find ready = [merge]            │
│         Execute: merge                   │
├─────────────────────────────────────────┤
│ Step 4: All nodes done → trace complete │
└─────────────────────────────────────────┘
```

This approach automatically extracts maximum parallelism from any DAG without requiring explicit level definition.

### Concurrent Execution with Work-Stealing

The Executor uses a **work-stealing approach** for maximum concurrency while respecting DAG dependencies:

```rust
pub struct ExecutorConfig {
    /// Maximum concurrent node executions per trace
    pub max_concurrent_nodes: usize,
}

impl Executor {
    pub async fn execute_trace(&self, trace_id: TraceId) -> Result<()> {
        // Semaphore limits concurrent node executions
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_nodes));

        // JoinSet to track spawned tasks
        let mut tasks: JoinSet<NodeExecutionResult> = JoinSet::new();

        // Track spawned nodes to prevent double-spawning
        let mut spawned: HashSet<NodeId> = HashSet::new();

        loop {
            // Find all ready nodes (dependencies satisfied, not yet executing)
            let ready_nodes = self.find_ready_nodes(trace_id, &spawned)?;

            // Spawn tasks for ready nodes
            for node_id in ready_nodes {
                spawned.insert(node_id);

                // Prepare execution context
                let exec_ctx = self.prepare_node_execution(trace_id, node_id)?;
                let node = Arc::clone(self.nodes.get(&node_id)?);

                // Mark node as executing
                if let Some(mut trace) = self.traces.get_mut(&trace_id) {
                    trace.mark_executing(node_id);
                }

                // Spawn task with semaphore-based backpressure
                let semaphore = Arc::clone(&semaphore);
                tasks.spawn(async move {
                    // Acquire permit (blocks if at max_concurrent_nodes)
                    let _permit = semaphore.acquire().await?;

                    // Execute the node
                    let result = node.execute(exec_ctx).await;

                    NodeExecutionResult { node_id, result }
                });
            }

            // If no tasks running and no ready nodes, we're done
            if tasks.is_empty() {
                break;
            }

            // Wait for any task to complete
            if let Some(join_result) = tasks.join_next().await {
                let execution_result = join_result?;

                // Process result and update trace state
                self.process_node_result(trace_id, execution_result).await?;
            }
        }

        Ok(())
    }
}
```

**Key Features:**

- **Work-stealing scheduler** - Finds all "ready" nodes (dependencies met) and spawns tasks up to `max_concurrent_nodes`
- **Semaphore-based backpressure** - Limits concurrent node executions to prevent resource exhaustion
- **Dynamic task spawning** - No pre-defined levels; nodes execute as soon as dependencies are satisfied
- **Arc-wrapped nodes** - All nodes wrapped in `Arc<Box<dyn Node>>` for safe concurrent access across async tasks
- **JoinSet management** - Uses `tokio::JoinSet` for efficient task lifecycle management

**Concurrency Control:**

```rust
// Configure in ExecutorConfig or environment variable
pub max_concurrent_nodes: usize,  // Default: 16

// From environment:
// XERV_MAX_CONCURRENT_NODES=32 ./xerv serve
```

This design maximizes parallelism by allowing nodes from different levels of the DAG to execute concurrently, as long as their individual dependencies are satisfied.

## Linker: Selector Resolution

The **Linker** resolves selector expressions to arena offsets at link time:

### Parsing

```rust
pub struct Selector {
    raw: String,           // "${fraud_check.score}"
    root: String,          // "fraud_check"
    path: Vec<String>,     // ["score"]
}

pub fn parse(input: &str) -> Result<Vec<Selector>> {
    // Regex: \$\{([a-z_]+)(?:\.([a-z_.]+))?\}
    // Returns all selectors in the string
}
```

### Resolution

```rust
pub struct CompiledSelector {
    node_id: NodeId,
    field_offset: u32,     // Offset within the rkyv struct
    schema_name: String,   // For type checking
}

pub struct Linker {
    compiled: HashMap<String, CompiledSelector>,
}

impl Linker {
    pub fn compile(flow: &FlowDefinition) -> Result<Self> {
        let mut compiled = HashMap::new();

        // For each node with selectors in config
        for (node_id, node) in &flow.nodes {
            let selectors = SelectorParser::parse_all(&node.config)?;

            for selector in selectors {
                // Look up the source node's schema
                let source_node = flow.nodes.get(&selector.root)?;
                let schema = Schema::for_type(&source_node.output_type)?;

                // Find field offset in rkyv struct
                let field_offset = schema.field_offset(&selector.path)?;

                compiled.insert(selector.raw, CompiledSelector {
                    node_id: get_node_id(&selector.root),
                    field_offset,
                    schema_name: source_node.output_type.clone(),
                });
            }
        }

        Ok(Self { compiled })
    }

    pub fn resolve(&self, selector: &str, arena: &Arena) -> Result<RelPtr<Value>> {
        let compiled = self.compiled.get(selector)?;

        // Get the base pointer for this node's output
        let base_ptr = arena.node_output(compiled.node_id)?;

        // Calculate the field offset
        let field_ptr = RelPtr::<Value>::new(
            base_ptr.offset() + compiled.field_offset
        );

        Ok(field_ptr)
    }
}
```

### Example

Node config:

```yaml
check_limit:
  type: std::switch
  config:
    condition:
      type: greater_than
      field: amount
      value: ${pipeline.config.limit}
```

At link time:

```
Selector: "${pipeline.config.limit}"
├─ Root: "pipeline"
├─ Path: ["config", "limit"]
├─ Source schema: PipelineConfig
├─ Field offset in rkyv struct: 24
└─ Compiled selector: {
     node_id: 0,
     field_offset: 24,
     schema: "PipelineConfig"
   }
```

At runtime:

```
arena.read(compiled_selector.resolve(arena))
├─ base_ptr = arena.node_output(pipeline_node)
├─ field_ptr = base_ptr + 24
└─ value = 1000.0  (from archived struct)
```

## Control Plane: Pipeline Lifecycle

The **Pipeline** manages the overall lifecycle:

```rust
pub enum PipelineState {
    Stopped,
    Running,
    Paused,
    Draining,
}

pub struct Pipeline {
    state: Arc<Mutex<PipelineState>>,
    listeners: ListenerPool,
    executor: Arc<Executor>,
    metrics: Arc<PipelineMetrics>,
}

impl Pipeline {
    pub async fn start(&self) -> Result<()> {
        // 1. Start all enabled listeners (webhooks, cron, fs)
        self.listeners.start_all().await?;

        // 2. Begin accepting traces
        self.state.lock().await.transition(PipelineState::Running)?;

        Ok(())
    }

    pub async fn pause(&self) -> Result<()> {
        // Stop accepting new traces
        // In-flight traces continue
        self.state.lock().await.transition(PipelineState::Paused)?;
        Ok(())
    }

    pub async fn drain(&self) -> Result<()> {
        // Stop accepting new traces
        // Wait for all in-flight traces to complete
        self.state.lock().await.transition(PipelineState::Draining)?;
        self.listeners.stop_all().await?;

        loop {
            if self.executor.active_traces() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Ok(())
    }
}
```

## Crash Recovery: Write-Ahead Log

The **WAL** ensures durability:

### Writing

```rust
pub struct Wal {
    file: File,
    checkpoint_interval: usize,
}

impl Wal {
    pub fn record_node_completion(
        &mut self,
        trace_id: TraceId,
        node_id: NodeId,
        output_ptr: RelPtr<Value>,
    ) -> Result<()> {
        let record = WalRecord {
            timestamp: now(),
            trace_id,
            event: WalEvent::NodeCompleted {
                node_id,
                output_ptr,
            },
        };

        // Write to file synchronously
        let bytes = rkyv::to_bytes::<_, 256>(&record)?;
        self.file.write_all(&bytes)?;
        self.file.flush()?;

        Ok(())
    }
}
```

### Recovery

```rust
pub struct CrashReplayer {
    wal: Wal,
}

impl CrashReplayer {
    pub async fn replay(&self, trace_id: TraceId) -> Result<TraceState> {
        let records = self.wal.read_records(trace_id)?;

        let mut state = TraceState::new();

        for record in records {
            match record.event {
                WalEvent::NodeCompleted { node_id, output_ptr } => {
                    // Mark node as complete, skip re-execution
                    state.mark_complete(node_id, output_ptr);
                }
                WalEvent::TraceFailed { error } => {
                    // Trace previously failed
                    state.mark_failed(error);
                    break;
                }
            }
        }

        Ok(state)
    }

    pub async fn execute_with_recovery(
        &self,
        trace: Trace,
    ) -> Result<TraceResult> {
        // Replay completed nodes from WAL
        let previous_state = self.replay(trace.id)?;

        // Execute only incomplete nodes
        for node in self.executor.execution_order {
            if previous_state.is_complete(node.id) {
                // Reuse output from arena
                continue;
            }

            // Execute and WAL record
            let output = node.execute(trace.context).await?;
            self.wal.record_node_completion(trace.id, node.id, output_ptr)?;
        }

        Ok(trace.finalize()?)
    }
}
```

## Putting It Together: A Trace Execution

Request arrives at webhook:

```mermaid
sequenceDiagram
    participant Webhook as HTTP Request
    participant Arena as Arena Storage
    participant Executor as Executor/Scheduler
    participant Sem as Semaphore<br/>(backpressure)
    participant FC as fraud_check<br/>(task)
    participant PS as process_safe<br/>(task)
    participant PR as process_risky<br/>(task)
    participant Merge as merge<br/>(task)
    participant WAL as Write-Ahead Log
    participant Pipeline as Pipeline

    Webhook->>Arena: 1. POST /orders<br/>Serializes input → entry 0
    Webhook->>Pipeline: Create new trace
    Pipeline->>Executor: 2. Queue trace for execution

    Executor->>Executor: 3. Find ready nodes:<br/>[FC] (no dependencies)

    par Spawn FC task (acquire semaphore permit)
        Executor->>Sem: Acquire permit
        Sem-->>Executor: Permit acquired
        Executor->>+FC: Spawn async task
        FC->>Arena: Read input from arena
    end

    Note over FC,Merge: fraud_check executing...
    FC->>FC: Evaluate: 0.95 > 0.8? → true
    FC->>Arena: Write output → entry 1
    FC->>WAL: NodeCompleted(fraud_check)

    Executor->>Executor: 4. Find ready nodes:<br/>[PS, PR] (FC done)

    par Spawn PS and PR tasks (acquire 2 permits)
        Executor->>Sem: Acquire permit #1
        Sem-->>Executor: Permit acquired
        Executor->>Sem: Acquire permit #2
        Sem-->>Executor: Permit acquired
        Executor->>+PS: Spawn task (async)
        Executor->>+PR: Spawn task (async)
    end

    par Execute PS and PR in parallel
        PS->>Arena: Read FC output
        PS->>Arena: Write output → entry 2
        PS->>WAL: NodeCompleted(process_safe)
    and
        PR->>Arena: Read FC output
        PR->>Arena: Write output → entry 3
        PR->>WAL: NodeCompleted(process_risky)
    end

    Executor->>Executor: 5. Find ready nodes:<br/>[Merge] (PS and PR done)

    par Spawn Merge task (acquire 1 permit)
        Executor->>Sem: Acquire permit #3
        Sem-->>Executor: Permit acquired
        Executor->>+Merge: Spawn async task
    end

    Merge->>Arena: Read PS output
    Merge->>Arena: Read PR output
    Merge->>Arena: Write final output → entry 4
    Merge->>WAL: NodeCompleted(merge)

    Executor->>Executor: 6. No ready nodes<br/>All tasks done → exit

    Merge-->>-Pipeline: Return trace result
    Pipeline-->>Webhook: HTTP 200 with trace ID

    Note over Executor,Sem: Semaphore released as tasks complete
```

**Execution Timeline (Concurrent):**

```
Time:     0ms      10ms     20ms      30ms     40ms     50ms     60ms
          │        │        │         │        │        │        │

fraud_check:
          ├────────┤ (10ms, writes entry 1)

process_safe:
          │        ├────────────────┤ (20ms, waits for FC)

process_risky:
          │        ├────────────────┤ (20ms, parallel with PS)

merge:
          │        │                 ├────────┤ (10ms, waits for PS+PR)
          │        │                 │        │
          └────────────────────────────────────┘
          Total execution: ~50ms (vs ~70ms if sequential)
```

**With Semaphore (max_concurrent_nodes = 2):**

When `max_concurrent_nodes = 2`, only 2 nodes execute concurrently. The semaphore blocks task spawning until a permit is available.

- Time 0-10ms: FC executes (1 permit held)
- Time 10-30ms: PS and PR execute in parallel (2 permits held)
- Time 30-40ms: Merge executes (1 permit held)

This backpressure prevents resource exhaustion and allows fine-grained control over executor memory usage.

## Dispatch Backends and Scaling

XERV supports multiple dispatch backends to handle different deployment scenarios and scale profiles. The **dispatch backend** is responsible for queuing and coordinating trace execution across nodes.

### Backend Options

| Backend    | Best For                                             | Throughput        | Scaling                | Zero-Deps | HA  |
| ---------- | ---------------------------------------------------- | ----------------- | ---------------------- | --------- | --- |
| **Memory** | Development, single-machine edge deployments         | 10k+/sec          | Vertical only          | ✅        | ❌  |
| **Raft**   | Multi-node consensus, on-premises, zero dependencies | Consensus limited | Horizontal (odd count) | ✅        | ✅  |
| **Redis**  | Cloud-native, high throughput, stateless workers     | 50k+/sec          | Horizontal (unlimited) | ❌        | ✅  |
| **NATS**   | Streaming, multi-region, scale-to-zero               | 50k+/sec          | Horizontal (unlimited) | ❌        | ✅  |

**Configuration:**

```yaml
# Memory (default, no config needed)
dispatch:
  backend: memory

# Raft with 3 replicas
dispatch:
  backend: raft
  raft:
    nodeId: 1
    peers:
      - { id: 2, addr: "node2:5000" }
      - { id: 3, addr: "node3:5000" }

# Redis with connection pooling
dispatch:
  backend: redis
  redis:
    url: "redis://redis-cluster:6379"
    poolSize: 20

# NATS with JetStream persistence
dispatch:
  backend: nats
  nats:
    url: "nats://nats:4222"
    jetstream: true
```

For Kubernetes deployments, see the [Helm Chart Documentation](../charts/xerv/README.md#configuration) for complete configuration options.

## Execution Modes

XERV supports different execution modes depending on your deployment backend. Choose based on your requirements:

### Local Mode (Single-Node, Memory Backend)

**Best for:** High-throughput, low-latency workloads on a single machine.

```
┌─────────────────────────────────────────────┐
│              XERV Local Mode                │
│                                             │
│  ┌─────────┐   ┌─────────┐   ┌─────────┐   │
│  │ Trigger │──▶│Executor │──▶│  Arena  │   │
│  └─────────┘   └─────────┘   └─────────┘   │
│                     │                       │
│                     ▼                       │
│                ┌─────────┐                  │
│                │   WAL   │                  │
│                └─────────┘                  │
└─────────────────────────────────────────────┘
```

**Characteristics:**

- **Zero-copy data passing** - Nodes share data via RelPtr offsets in memory-mapped arena
- **Maximum throughput** - No network serialization overhead
- **Lowest latency** - Direct memory access, no consensus delays
- **Single point of failure** - No automatic failover

**When to use:**

- Edge deployments with local data processing
- High-frequency pipelines (1000+ traces/second)
- Latency-sensitive workloads (sub-millisecond node execution)
- Development and testing

**Configuration:**

```rust
// Local mode is the default - no cluster config needed
let executor = Executor::new(flow_graph)?;
executor.run(trace).await?;
```

### Cloud-Native Mode (Raft/Redis/NATS Backend)

**Best for:** High availability, horizontal scaling, and cloud deployments.

**With Raft Backend:**

```
┌─────────────────────────────────────────────────────────┐
│                  XERV Raft Cluster                       │
│                                                          │
│  ┌──────────┐    ┌──────────┐    ┌──────────┐          │
│  │  Node 1  │◄──▶│  Node 2  │◄──▶│  Node 3  │          │
│  │ (Leader) │    │(Follower)│    │(Follower)│          │
│  └────┬─────┘    └────┬─────┘    └────┬─────┘          │
│       │               │               │                 │
│       └───────────────┼───────────────┘                 │
│                       │                                 │
│               ┌───────▼───────┐                         │
│               │ Raft Consensus│                         │
│               │ (OpenRaft)    │                         │
│               └───────────────┘                         │
└─────────────────────────────────────────────────────────┘
```

**With Redis/NATS Backend:**

```
┌─────────────────────────────────────────────────────────┐
│              XERV Stateless Workers                       │
│                                                          │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐              │
│  │ Worker 1 │  │ Worker 2 │  │ Worker N │              │
│  │(Stateless)  │(Stateless)  │(Stateless)              │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘              │
│       │             │             │                     │
│       └─────────────┼─────────────┘                     │
│                     │                                   │
│        ┌────────────▼────────────┐                      │
│        │ Redis Streams or        │                      │
│        │ NATS JetStream          │                      │
│        │ (Shared Queue)          │                      │
│        └─────────────────────────┘                      │
└─────────────────────────────────────────────────────────┘
```

**Characteristics:**

- **Automatic failover** - Raft: leader election; Redis/NATS: consumer groups handle it
- **State replication** - Pipeline definitions synced across nodes
- **Horizontal scaling** - Easily add/remove workers (Redis/NATS) or carefully manage cluster membership (Raft)
- **Network efficiency** - Minimal overhead with Redis/NATS; gRPC only for Raft peers

**When to use each backend:**

- **Raft** - Production on-premises, zero external dependencies, high availability required
- **Redis** - Cloud deployments, AWS, Google Cloud, Azure; high throughput needed; want managed service
- **NATS** - Multi-region deployments, streaming architecture, scale-to-zero with KEDA

**Configuration for Raft:**

```rust
use xerv_cluster::{ClusterConfig, ClusterNode};

let config = ClusterConfig::builder()
    .node_id(1)
    .listen_addr("127.0.0.1:5000")
    .peers(vec![
        (2, "127.0.0.1:5001".to_string()),
        (3, "127.0.0.1:5002".to_string()),
    ])
    .build();

let node = ClusterNode::start(config).await?;
```

**Configuration for Redis/NATS:**

See [Helm Chart Documentation](../charts/xerv/README.md#configuration) for production setups with example values files.

### Execution Mode Comparison

| Aspect           | Local (Memory) | Raft Cluster  | Redis Backend | NATS Backend |
| ---------------- | -------------- | ------------- | ------------- | ------------ |
| **Throughput**   | 10k+/sec       | Consensus     | 50k+/sec      | 50k+/sec     |
| **Latency**      | Sub-ms         | ~10-50ms      | ~5-20ms       | ~5-20ms      |
| **Availability** | Single point   | Automatic     | Automatic     | Automatic    |
| **Zero-copy**    | ✅ RelPtr      | ✅ RelPtr     | Network I/O   | Network I/O  |
| **Scaling**      | Vertical only  | Horizontal    | Horizontal    | Horizontal   |
| **Zero-deps**    | ✅             | ✅            | ❌            | ❌           |
| **Cloud-native** | ⚠️ Manual      | ⚠️ Complex    | ✅ Simple     | ✅ Simple    |
| **Use case**     | Dev, edge      | Production HA | Cloud, scale  | Streaming    |

### Architectural Trade-offs

**Zero-copy vs. Distribution:**

The RelPtr mechanism that enables zero-copy data passing is inherently local - offsets reference positions in a memory-mapped file on a single machine. XERV's dispatch backend architecture handles this elegantly:

**Memory Backend (Local Mode):**

1. **Zero-copy** - All data passes via RelPtr offsets
2. **Maximum performance** - Sub-millisecond node execution
3. **Trade-off** - Single point of failure

**Raft Backend (Consensus Mode):**

1. **Trace affinity** - A trace executes entirely on one node (arena is local to that node)
2. **Work distribution** - Leader assigns new traces to nodes via Raft
3. **Metadata replication** - Pipeline definitions and state replicated via Raft consensus
4. **Data remains local** - Arena files not replicated (preserve zero-copy)
5. **Trade-off** - Consensus overhead, must maintain odd quorum size

**Redis Backend (Stateless Mode):**

1. **Stateless workers** - Any worker can execute any trace
2. **Shared queue** - Redis Streams coordinate work distribution
3. **Automatic load balancing** - Consumer groups balance traces across workers
4. **Trade-off** - Network I/O for queue coordination; external dependency

**NATS Backend (Streaming Mode):**

1. **Stateless workers** - Any worker can execute any trace
2. **Event streaming** - NATS JetStream provides persistent queue
3. **Multi-region ready** - Super-clusters enable geographic distribution
4. **Scale-to-zero** - KEDA compatible for serverless deployments
5. **Trade-off** - Network I/O for queue coordination; external dependency

Each backend preserves the core XERV design while enabling different deployment scenarios. Choose based on your infrastructure and scale requirements.

## Performance Characteristics

### Memory

- **Efficient node communication** - RelPtr-based offset arithmetic
- **Append-only arena** - predictable allocation without garbage collection
- **Memory-mapped I/O** - kernel page cache optimization

### Latency

- **Async execution** - non-blocking node execution with tokio
- **Efficient data access** - rkyv archived reference without reallocation
- **Parallel scheduling** - concurrent node execution at each topological level

### Durability

- **Write-ahead log** - node completions recorded before continuing execution
- **Crash recovery** - replay incomplete nodes from previous execution state
- **Persistent offsets** - RelPtr survives process restarts

### Resource Management

- **Automatic cleanup** - timed-out traces, completed arena files, WAL segments
- **Graceful shutdown** - in-flight traces complete before termination
- **Circuit breaker** - prevents cascading failures during high error rates
- **Bounded memory** - log subscribers and suspension state are properly cleaned

## Reliability Features

### Circuit Breaker

The circuit breaker protects pipelines from cascading failures:

```rust
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,        // Failures before opening (default: 5)
    pub failure_window_secs: u64,      // Time window for counting (default: 60)
    pub recovery_timeout_secs: u64,    // Wait before retry (default: 30)
    pub success_threshold: u32,        // Successes to close circuit (default: 3)
}
```

**States:**

- **Closed** - Normal operation, all requests allowed
- **Open** - Circuit tripped, requests rejected immediately
- **HalfOpen** - Testing recovery, limited requests allowed

**Operation:**

1. Pipeline tracks failures within sliding window
2. When threshold exceeded, circuit opens (rejects all traces)
3. After recovery timeout, enters half-open state
4. If next N requests succeed, circuit closes
5. If any fail in half-open, returns to open state

### Distributed Clustering (xerv-cluster)

For high availability and horizontal scaling, XERV supports multi-node deployment:

**Architecture:**

```
┌─────────────────────────────────────────────────────────────┐
│                    XERV Cluster                             │
│                                                             │
│  ┌─────────────┐   ┌─────────────┐   ┌─────────────┐       │
│  │   Node 1    │   │   Node 2    │   │   Node 3    │       │
│  │  (Leader)   │   │ (Follower)  │   │ (Follower)  │       │
│  │             │   │             │   │             │       │
│  │ ┌─────────┐ │   │ ┌─────────┐ │   │ ┌─────────┐ │       │
│  │ │  Raft   │◄┼───┼─┤  Raft   │◄┼───┼─┤  Raft   │ │       │
│  │ │ Engine  │ │   │ │ Engine  │ │   │ │ Engine  │ │       │
│  │ └────┬────┘ │   │ └────┬────┘ │   │ └────┬────┘ │       │
│  │      │      │   │      │      │   │      │      │       │
│  │ ┌────▼────┐ │   │ ┌────▼────┐ │   │ ┌────▼────┐ │       │
│  │ │  State  │ │   │ │  State  │ │   │ │  State  │ │       │
│  │ │ Machine │ │   │ │ Machine │ │   │ │ Machine │ │       │
│  │ └─────────┘ │   │ └─────────┘ │   │ └─────────┘ │       │
│  └─────────────┘   └─────────────┘   └─────────────┘       │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Features:**

- **Raft Consensus** - Leader election and log replication via OpenRaft
- **Automatic Failover** - New leader elected if current leader fails
- **State Replication** - Trace execution state replicated across nodes
- **gRPC Transport** - Efficient inter-node communication
- **Membership Management** - Dynamic adding/removing of cluster nodes

**Usage:**

```rust
use xerv_cluster::{ClusterConfig, ClusterNode};

// Configure the cluster
let config = ClusterConfig::builder()
    .node_id(1)
    .listen_addr("127.0.0.1:5000")
    .peers(vec![
        (2, "127.0.0.1:5001".to_string()),
        (3, "127.0.0.1:5002".to_string()),
    ])
    .build();

// Start the node
let node = ClusterNode::start(config).await?;

// Commands are automatically routed to the leader
node.execute(ClusterCommand::StartTrace { ... }).await?;
```

**When to Use Clustering:**

- **High availability** - Survive node failures without downtime
- **Horizontal scaling** - Distribute trace execution across multiple nodes
- **Geographic distribution** - Place nodes closer to data sources
- **Load balancing** - Leader distributes work to followers

## Advanced Topics

### Custom Schemas

Define strongly-typed schemas for your data:

```rust
#[derive(Serialize, Archive, Deserialize, Schema)]
#[repr(C)]
pub struct Order {
    pub id: String,
    pub amount: f64,
    pub items: Vec<OrderItem>,
}

// Linker can now resolve selectors like ${order.items[0].sku}
```

### Declared Loops

Use `std::loop` to handle repeated execution:

```yaml
nodes:
  process_items:
    type: std::loop
    config:
      collection: ${order.items}
      exit_condition:
        field: count
        value: 100

edges:
  - from: process_items.loop_body
    to: validate_item
  - from: validate_item
    to: process_items.next_iteration
```

### WASM Nodes

Custom logic in WebAssembly:

```rust
let wasm_node = WasmNode::new(
    WasmNodeConfig {
        module_path: "nodes/my_logic.wasm",
        function: "process",
    }
)?;
```

See [nodes.md](nodes.md) for implementation details.
