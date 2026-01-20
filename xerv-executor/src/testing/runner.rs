//! Flow runner for testing with mock providers.

use crate::scheduler::{Edge, FlowGraph, GraphNode};
use std::collections::HashMap;
use std::sync::Arc;
use xerv_core::arena::{Arena, ArenaConfig};
use xerv_core::error::Result;
use xerv_core::testing::providers::{
    ClockProvider, EnvProvider, FsProvider, HttpProvider, MockClock, MockEnv, MockFs, MockHttp,
    MockRng, MockSecrets, MockUuid, RngProvider, SecretsProvider, UuidProvider,
};
use xerv_core::traits::{Context, Node, NodeOutput};
use xerv_core::types::{NodeId, RelPtr, TraceId};
use xerv_core::wal::{Wal, WalConfig, WalRecord};

/// Result of a flow execution.
#[derive(Debug)]
pub struct FlowResult {
    /// Trace ID of the executed flow.
    pub trace_id: TraceId,
    /// Outputs from each completed node.
    pub outputs: HashMap<NodeId, NodeOutput>,
    /// Nodes that completed successfully.
    pub completed_nodes: Vec<NodeId>,
    /// Error if the flow failed.
    pub error: Option<String>,
}

impl FlowResult {
    /// Check if the flow completed successfully.
    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }

    /// Get the output for a specific node.
    pub fn output(&self, node_id: NodeId) -> Option<&NodeOutput> {
        self.outputs.get(&node_id)
    }

    /// Check if a node completed.
    pub fn node_completed(&self, node_id: NodeId) -> bool {
        self.completed_nodes.contains(&node_id)
    }
}

/// Builder for configuring a FlowRunner with mock providers.
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

impl FlowRunnerBuilder {
    /// Create a new flow runner builder.
    pub fn new() -> Self {
        Self {
            graph: FlowGraph::new(),
            nodes: HashMap::new(),
            clock: None,
            http: None,
            rng: None,
            uuid: None,
            fs: None,
            env: None,
            secrets: None,
        }
    }

    /// Add a node to the flow.
    pub fn add_node(mut self, node_id: NodeId, name: &str, node: Box<dyn Node>) -> Self {
        self.graph.add_node(GraphNode::new(node_id, name));
        self.nodes.insert(node_id, node);
        self
    }

    /// Add an edge between nodes.
    pub fn add_edge(
        mut self,
        from_node: NodeId,
        from_port: &str,
        to_node: NodeId,
        to_port: &str,
    ) -> Self {
        self.graph
            .add_edge(Edge::new(from_node, from_port, to_node, to_port));
        self
    }

    /// Set entry point node.
    pub fn set_entry_point(mut self, node_id: NodeId) -> Self {
        self.graph.set_entry_point(node_id);
        self
    }

    /// Use a mock clock with fixed time.
    pub fn with_fixed_time(mut self, iso_time: &str) -> Self {
        self.clock = Some(Arc::new(MockClock::fixed(iso_time)));
        self
    }

    /// Use a mock clock.
    pub fn with_mock_clock(mut self, clock: MockClock) -> Self {
        self.clock = Some(Arc::new(clock));
        self
    }

    /// Use a mock HTTP provider.
    pub fn with_mock_http(mut self, http: MockHttp) -> Self {
        self.http = Some(Arc::new(http));
        self
    }

    /// Use a mock RNG with a seed.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.rng = Some(Arc::new(MockRng::seeded(seed)));
        self
    }

    /// Use sequential UUIDs for deterministic testing.
    pub fn with_sequential_uuids(mut self) -> Self {
        self.uuid = Some(Arc::new(MockUuid::sequential()));
        self
    }

    /// Use predetermined UUIDs.
    pub fn with_predetermined_uuids(mut self, uuids: Vec<uuid::Uuid>) -> Self {
        self.uuid = Some(Arc::new(MockUuid::predetermined(uuids)));
        self
    }

    /// Use a mock filesystem.
    pub fn with_mock_fs(mut self, fs: MockFs) -> Self {
        self.fs = Some(Arc::new(fs));
        self
    }

    /// Use mock environment variables.
    pub fn with_mock_env(mut self, env: MockEnv) -> Self {
        self.env = Some(Arc::new(env));
        self
    }

    /// Use mock secrets.
    pub fn with_mock_secrets(mut self, secrets: MockSecrets) -> Self {
        self.secrets = Some(Arc::new(secrets));
        self
    }

    /// Build the flow runner.
    pub fn build(self) -> Result<FlowRunner> {
        // Validate graph
        self.graph.validate()?;

        // Create WAL
        let wal = Arc::new(Wal::open(WalConfig::in_memory())?);

        // Get execution order
        let execution_order = self.graph.topological_sort()?;

        // Use defaults for any unspecified providers
        let clock = self
            .clock
            .unwrap_or_else(|| Arc::new(MockClock::fixed("2024-01-01T00:00:00Z")));
        let http = self.http.unwrap_or_else(|| Arc::new(MockHttp::new()));
        let rng = self.rng.unwrap_or_else(|| Arc::new(MockRng::seeded(42)));
        let uuid = self
            .uuid
            .unwrap_or_else(|| Arc::new(MockUuid::sequential()));
        let fs = self.fs.unwrap_or_else(|| Arc::new(MockFs::new()));
        let env = self.env.unwrap_or_else(|| Arc::new(MockEnv::new()));
        let secrets = self
            .secrets
            .unwrap_or_else(|| Arc::new(MockSecrets::default()));

        Ok(FlowRunner {
            graph: Arc::new(self.graph),
            nodes: Arc::new(self.nodes),
            wal,
            execution_order,
            clock,
            http,
            rng,
            uuid,
            fs,
            env,
            secrets,
        })
    }
}

impl Default for FlowRunnerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Flow runner for executing flows with mock providers.
///
/// This is the primary entry point for testing flows with deterministic behavior.
pub struct FlowRunner {
    graph: Arc<FlowGraph>,
    nodes: Arc<HashMap<NodeId, Box<dyn Node>>>,
    wal: Arc<Wal>,
    execution_order: Vec<NodeId>,

    // Providers (public for assertions)
    /// Mock clock provider.
    pub clock: Arc<dyn ClockProvider>,
    /// Mock HTTP provider.
    pub http: Arc<dyn HttpProvider>,
    /// Mock RNG provider.
    pub rng: Arc<dyn RngProvider>,
    /// Mock UUID provider.
    pub uuid: Arc<dyn UuidProvider>,
    /// Mock filesystem provider.
    pub fs: Arc<dyn FsProvider>,
    /// Mock environment provider.
    pub env: Arc<dyn EnvProvider>,
    /// Mock secrets provider.
    pub secrets: Arc<dyn SecretsProvider>,
}

impl FlowRunner {
    /// Create a new flow runner builder.
    pub fn builder() -> FlowRunnerBuilder {
        FlowRunnerBuilder::new()
    }

    /// Run the flow with the given initial input.
    pub async fn run(&self, initial_input: &[u8]) -> Result<FlowResult> {
        let trace_id = TraceId::new();

        // Create arena for this trace
        let arena = Arena::create(trace_id, &ArenaConfig::in_memory())?;

        // Write initial input to arena
        let initial_ptr: RelPtr<()> = arena.write_bytes(initial_input)?;

        // Track state
        let mut outputs: HashMap<NodeId, NodeOutput> = HashMap::new();
        let mut completed_nodes: Vec<NodeId> = Vec::new();

        // Log trace start
        let record = WalRecord::trace_start(trace_id);
        self.wal.write(&record)?;

        // Execute nodes in topological order
        for &node_id in &self.execution_order {
            // Get the node implementation
            let node = match self.nodes.get(&node_id) {
                Some(n) => n,
                None => {
                    return Ok(FlowResult {
                        trace_id,
                        outputs,
                        completed_nodes,
                        error: Some(format!("Node {} not found", node_id)),
                    });
                }
            };

            // Collect inputs from upstream nodes
            let inputs = self.collect_inputs(node_id, &outputs, initial_ptr);

            // Check if this node should execute (has required inputs)
            if !self.should_execute(node_id, &outputs)
                && !self.graph.entry_points().contains(&node_id)
            {
                continue;
            }

            // Create execution context with mock providers
            let (reader, writer) = (arena.reader(), arena.writer());
            let ctx = Context::with_providers(
                trace_id,
                node_id,
                reader,
                writer,
                Arc::clone(&self.wal),
                Arc::clone(&self.clock),
                Arc::clone(&self.http),
                Arc::clone(&self.rng),
                Arc::clone(&self.uuid),
                Arc::clone(&self.fs),
                Arc::clone(&self.env),
                Arc::clone(&self.secrets),
            );

            // Log node start
            let record = WalRecord::node_start(trace_id, node_id);
            self.wal.write(&record)?;

            // Execute the node
            match node.execute(ctx, inputs).await {
                Ok(output) => {
                    // Get arena location
                    let (offset, size) = output.arena_location();
                    let schema_hash = output.schema_hash;

                    // Log completion
                    let record = WalRecord::node_done(trace_id, node_id, offset, size, schema_hash);
                    self.wal.write(&record)?;

                    outputs.insert(node_id, output);
                    completed_nodes.push(node_id);
                }
                Err(e) => {
                    // Log error
                    let record = WalRecord::node_error(trace_id, node_id, e.to_string());
                    self.wal.write(&record)?;

                    return Ok(FlowResult {
                        trace_id,
                        outputs,
                        completed_nodes,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        // Log trace complete
        let record = WalRecord::trace_complete(trace_id);
        self.wal.write(&record)?;

        Ok(FlowResult {
            trace_id,
            outputs,
            completed_nodes,
            error: None,
        })
    }

    /// Collect inputs for a node from upstream outputs.
    fn collect_inputs(
        &self,
        node_id: NodeId,
        outputs: &HashMap<NodeId, NodeOutput>,
        initial_input: RelPtr<()>,
    ) -> HashMap<String, RelPtr<()>> {
        let mut inputs = HashMap::new();

        // For entry points, use initial input
        if self.graph.entry_points().contains(&node_id) {
            inputs.insert("in".to_string(), initial_input);
            return inputs;
        }

        // Collect from upstream nodes
        for edge in self.graph.incoming_edges(node_id) {
            if let Some(output) = outputs.get(&edge.from_node) {
                if output.port == edge.from_port {
                    inputs.insert(edge.to_port.clone(), output.data);
                }
            }
        }

        inputs
    }

    /// Check if a node should execute based on upstream completion.
    fn should_execute(&self, node_id: NodeId, outputs: &HashMap<NodeId, NodeOutput>) -> bool {
        // Check if all predecessors have completed
        for edge in self.graph.incoming_edges(node_id) {
            if !outputs.contains_key(&edge.from_node) {
                return false;
            }
        }
        true
    }

    /// Get the execution order.
    pub fn execution_order(&self) -> &[NodeId] {
        &self.execution_order
    }

    /// Get the WAL for assertions.
    pub fn wal(&self) -> &Wal {
        &self.wal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xerv_core::traits::{NodeFuture, NodeInfo};

    struct PassthroughNode;

    impl Node for PassthroughNode {
        fn info(&self) -> NodeInfo {
            NodeInfo::new("test", "passthrough")
        }

        fn execute<'a>(
            &'a self,
            _ctx: Context,
            inputs: HashMap<String, RelPtr<()>>,
        ) -> NodeFuture<'a> {
            Box::pin(async move {
                let input = inputs.get("in").cloned().unwrap_or_else(RelPtr::null);
                Ok(NodeOutput::out(input))
            })
        }
    }

    struct DoubleNode;

    impl Node for DoubleNode {
        fn info(&self) -> NodeInfo {
            NodeInfo::new("test", "double")
        }

        fn execute<'a>(
            &'a self,
            ctx: Context,
            inputs: HashMap<String, RelPtr<()>>,
        ) -> NodeFuture<'a> {
            Box::pin(async move {
                let input = inputs.get("in").cloned().unwrap_or_else(RelPtr::null);

                // Read input and duplicate it
                if !input.is_null() {
                    let data = ctx.read_bytes(input)?;
                    let doubled: Vec<u8> = data.iter().chain(data.iter()).copied().collect();
                    let ptr = ctx.write_bytes(&doubled)?;
                    Ok(NodeOutput::out(ptr))
                } else {
                    Ok(NodeOutput::out(RelPtr::<()>::null()))
                }
            })
        }
    }

    #[tokio::test]
    async fn flow_runner_single_node() {
        let runner = FlowRunner::builder()
            .add_node(
                NodeId::new(0),
                "test::passthrough",
                Box::new(PassthroughNode),
            )
            .set_entry_point(NodeId::new(0))
            .build()
            .unwrap();

        let result = runner.run(b"hello").await.unwrap();

        assert!(result.is_success());
        assert_eq!(result.completed_nodes.len(), 1);
        assert!(result.node_completed(NodeId::new(0)));
    }

    #[tokio::test]
    async fn flow_runner_linear_chain() {
        let runner = FlowRunner::builder()
            .add_node(
                NodeId::new(0),
                "test::passthrough",
                Box::new(PassthroughNode),
            )
            .add_node(NodeId::new(1), "test::double", Box::new(DoubleNode))
            .add_edge(NodeId::new(0), "out", NodeId::new(1), "in")
            .set_entry_point(NodeId::new(0))
            .build()
            .unwrap();

        let result = runner.run(b"test").await.unwrap();

        assert!(result.is_success());
        assert_eq!(result.completed_nodes.len(), 2);
        assert!(result.node_completed(NodeId::new(0)));
        assert!(result.node_completed(NodeId::new(1)));
    }

    #[tokio::test]
    async fn flow_runner_with_fixed_time() {
        let runner = FlowRunner::builder()
            .add_node(
                NodeId::new(0),
                "test::passthrough",
                Box::new(PassthroughNode),
            )
            .set_entry_point(NodeId::new(0))
            .with_fixed_time("2024-06-15T14:30:00Z")
            .build()
            .unwrap();

        let result = runner.run(b"data").await.unwrap();
        assert!(result.is_success());

        // The clock should be at fixed time
        assert!(runner.clock.is_mock());
    }

    #[tokio::test]
    async fn flow_runner_with_mock_http() {
        let http = MockHttp::new()
            .on_get("/api/test")
            .respond_json(200, serde_json::json!({"status": "ok"}));

        let runner = FlowRunner::builder()
            .add_node(
                NodeId::new(0),
                "test::passthrough",
                Box::new(PassthroughNode),
            )
            .set_entry_point(NodeId::new(0))
            .with_mock_http(http)
            .build()
            .unwrap();

        let result = runner.run(b"input").await.unwrap();
        assert!(result.is_success());

        // HTTP mock should be available
        assert!(runner.http.is_mock());
    }
}
