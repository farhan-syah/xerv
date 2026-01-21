//! WASM node implementation.
//!
//! Implements the `Node` trait for WASM modules, bridging the async
//! Node interface with synchronous WASM execution.

use super::host::{HostState, create_linker};
use super::memory::{MemoryBridge, encode_arena_offset};
use super::runtime::{CompiledModule, WasmRuntime};
use super::strategy::TransferStrategy;
use std::collections::HashMap;
use std::sync::Arc;
use wasmtime::{Store, TypedFunc};
use xerv_core::arena::{ArenaReader, ArenaWriter};
use xerv_core::error::{Result, XervError};
use xerv_core::testing::providers::{
    ClockProvider, EnvProvider, RealClock, RealEnv, RealRng, RealSecrets, RealUuid, RngProvider,
    SecretsProvider, UuidProvider,
};
use xerv_core::traits::{Context, Node, NodeFuture, NodeInfo, NodeOutput, Port};
use xerv_core::types::{NodeId, RelPtr, TraceId};

/// Configuration for a WASM node.
#[derive(Debug, Clone)]
pub struct WasmNodeConfig {
    /// Node name (e.g., "fraud_detector").
    pub name: String,
    /// Namespace (e.g., "plugins").
    pub namespace: String,
    /// Description of the node.
    pub description: String,
    /// Version string.
    pub version: String,
    /// Input port definitions.
    pub inputs: Vec<Port>,
    /// Output port definitions.
    pub outputs: Vec<Port>,
    /// Raw WASM module bytes.
    pub wasm_bytes: Vec<u8>,
    /// Whether this node has side effects.
    pub effectful: bool,
    /// Whether this node is deterministic.
    pub deterministic: bool,
}

impl WasmNodeConfig {
    /// Create a new WASM node configuration.
    pub fn new(namespace: impl Into<String>, name: impl Into<String>, wasm_bytes: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            namespace: namespace.into(),
            description: String::new(),
            version: "1.0.0".to_string(),
            inputs: vec![Port::input("Any")],
            outputs: vec![Port::output("Any"), Port::error()],
            wasm_bytes,
            effectful: false,
            deterministic: true,
        }
    }

    /// Set the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set the version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Set input ports.
    pub fn with_inputs(mut self, inputs: Vec<Port>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Set output ports.
    pub fn with_outputs(mut self, outputs: Vec<Port>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Mark as effectful.
    pub fn effectful(mut self) -> Self {
        self.effectful = true;
        self
    }

    /// Mark as non-deterministic.
    pub fn non_deterministic(mut self) -> Self {
        self.deterministic = false;
        self
    }
}

/// A WASM-based node that implements the Node trait.
pub struct WasmNode {
    /// Node configuration.
    config: WasmNodeConfig,
    /// Shared WASM runtime.
    runtime: Arc<WasmRuntime>,
    /// Compiled WASM module.
    module: Arc<CompiledModule>,
}

impl WasmNode {
    /// Create a new WASM node.
    pub fn new(config: WasmNodeConfig, runtime: Arc<WasmRuntime>) -> Result<Self> {
        // Compile the module
        let full_name = format!("{}::{}", config.namespace, config.name);
        let module = runtime.compile(&full_name, &config.wasm_bytes)?;

        Ok(Self {
            config,
            runtime,
            module,
        })
    }
}

impl Node for WasmNode {
    fn info(&self) -> NodeInfo {
        let mut info = NodeInfo::new(&self.config.namespace, &self.config.name)
            .with_description(&self.config.description)
            .with_version(&self.config.version)
            .with_inputs(self.config.inputs.clone())
            .with_outputs(self.config.outputs.clone());

        if self.config.effectful {
            info = info.effectful();
        }

        if !self.config.deterministic {
            info = info.non_deterministic();
        }

        info
    }

    fn execute<'a>(&'a self, ctx: Context, inputs: HashMap<String, RelPtr<()>>) -> NodeFuture<'a> {
        Box::pin(async move {
            let trace_id = ctx.trace_id();
            let node_id = ctx.node_id();

            // Clone reader and writer from context for the blocking task
            let reader = ArenaReader::clone(ctx.reader());
            let writer = ArenaWriter::clone(ctx.writer());

            // Get providers from context
            let clock: Arc<dyn ClockProvider> = Arc::new(RealClock::new());
            let rng: Arc<dyn RngProvider> = Arc::new(RealRng::new());
            let uuid: Arc<dyn UuidProvider> = Arc::new(RealUuid::new());
            let env: Arc<dyn EnvProvider> = Arc::new(RealEnv::new());
            let secrets: Arc<dyn SecretsProvider> = Arc::new(RealSecrets::default());

            // Clone self for the blocking task
            let runtime = Arc::clone(&self.runtime);
            let module = Arc::clone(&self.module);

            // Execute in blocking task to avoid blocking async runtime
            let result = tokio::task::spawn_blocking(move || {
                let executor = WasmNodeExecutor { runtime, module };
                executor.execute_sync(
                    trace_id, node_id, reader, writer, inputs, clock, rng, uuid, env, secrets,
                )
            })
            .await
            .map_err(|e| XervError::WasmExecution {
                node_id,
                cause: format!("Task join error: {}", e),
            })??;

            Ok(result)
        })
    }
}

/// Internal executor that can be moved into spawn_blocking.
struct WasmNodeExecutor {
    runtime: Arc<WasmRuntime>,
    module: Arc<CompiledModule>,
}

impl WasmNodeExecutor {
    #[allow(clippy::too_many_arguments)]
    fn execute_sync(
        &self,
        trace_id: TraceId,
        node_id: NodeId,
        reader: ArenaReader,
        writer: ArenaWriter,
        inputs: HashMap<String, RelPtr<()>>,
        clock: Arc<dyn ClockProvider>,
        rng: Arc<dyn RngProvider>,
        uuid: Arc<dyn UuidProvider>,
        env: Arc<dyn EnvProvider>,
        secrets: Arc<dyn SecretsProvider>,
    ) -> Result<NodeOutput> {
        // Create host state
        let host_state = HostState::new(
            trace_id,
            node_id,
            reader.clone(),
            writer,
            clock,
            rng,
            uuid,
            env,
            secrets,
        );

        // Create store with host state
        let mut store = Store::new(self.runtime.engine(), host_state);

        // Set fuel if enabled
        if let Some(fuel) = self.runtime.initial_fuel() {
            store.set_fuel(fuel).map_err(|e| XervError::WasmExecution {
                node_id,
                cause: format!("Failed to set fuel: {}", e),
            })?;
        }

        // Create linker and instantiate module
        let linker = create_linker(self.runtime.engine())?;
        let instance = linker
            .instantiate(&mut store, self.module.module())
            .map_err(|e| XervError::WasmExecution {
                node_id,
                cause: format!("Failed to instantiate module: {}", e),
            })?;

        // Get required exports
        let memory =
            instance
                .get_memory(&mut store, "memory")
                .ok_or_else(|| XervError::WasmExecution {
                    node_id,
                    cause: "Module does not export 'memory'".to_string(),
                })?;

        let alloc_fn: TypedFunc<u32, u32> = instance
            .get_typed_func(&mut store, "__xerv_alloc")
            .map_err(|e| XervError::WasmExecution {
                node_id,
                cause: format!("Module does not export '__xerv_alloc': {}", e),
            })?;

        let execute_fn: TypedFunc<(u32, u32), i32> = instance
            .get_typed_func(&mut store, "__xerv_execute")
            .map_err(|e| XervError::WasmExecution {
                node_id,
                cause: format!("Module does not export '__xerv_execute': {}", e),
            })?;

        // Create memory bridge
        let bridge = MemoryBridge::new(memory, alloc_fn);

        // Prepare inputs in WASM memory
        let (inputs_ptr, input_count) = prepare_inputs(&bridge, &mut store, &reader, &inputs)?;

        // Call the execute function
        let result = execute_fn
            .call(&mut store, (inputs_ptr, input_count))
            .map_err(|e| XervError::WasmExecution {
                node_id,
                cause: format!("Execution failed: {}", e),
            })?;

        // Check for errors
        if store.data().has_error() {
            let error_msg = store
                .data()
                .take_error()
                .unwrap_or_else(|| "Unknown error".to_string());
            return Ok(NodeOutput::error_with_message(error_msg));
        }

        // Get result
        if let Some((port, ptr)) = store.data().take_result() {
            Ok(NodeOutput::new(port, ptr))
        } else if result == 0 {
            // Success but no explicit result - return empty output
            Ok(NodeOutput::out(RelPtr::<()>::null()))
        } else {
            Err(XervError::WasmExecution {
                node_id,
                cause: format!("Execution returned error code: {}", result),
            })
        }
    }
}

/// Prepare input data in WASM memory.
///
/// Returns a pointer to the input array and the number of inputs.
fn prepare_inputs<T>(
    bridge: &MemoryBridge<T>,
    store: &mut Store<T>,
    reader: &ArenaReader,
    inputs: &HashMap<String, RelPtr<()>>,
) -> Result<(u32, u32)> {
    if inputs.is_empty() {
        return Ok((0, 0));
    }

    // Each input entry: [name_ptr: u32, name_len: u32, offset_lo: u32, offset_hi: u32, size: u32]
    const ENTRY_SIZE: usize = 20; // 5 * 4 bytes
    let count = inputs.len();
    let array_size = count * ENTRY_SIZE;

    // Allocate the input array
    let array_ptr =
        bridge
            .allocate(store, array_size as u32)
            .map_err(|_| XervError::WasmMemoryAlloc {
                requested: array_size as u64,
            })?;

    let mut offset = array_ptr;
    for (name, ptr) in inputs {
        // Copy name to WASM memory
        let name_wasm = bridge.copy_to_wasm(store, name.as_bytes())?;

        // Encode arena offset
        let (offset_lo, offset_hi) = encode_arena_offset(ptr.offset());

        // Build entry
        let mut entry = [0u8; ENTRY_SIZE];
        entry[0..4].copy_from_slice(&name_wasm.offset.to_le_bytes());
        entry[4..8].copy_from_slice(&name_wasm.size.to_le_bytes());
        entry[8..12].copy_from_slice(&offset_lo.to_le_bytes());
        entry[12..16].copy_from_slice(&offset_hi.to_le_bytes());
        entry[16..20].copy_from_slice(&ptr.size().to_le_bytes());

        bridge.write_at(store, offset, &entry)?;
        offset += ENTRY_SIZE as u32;

        // For SliceCopy strategy, copy input data to WASM memory
        let strategy = TransferStrategy::select(ptr.size() as usize);
        if strategy.requires_copy() && ptr.size() > 0 {
            let data = reader.read_bytes(ptr.offset(), ptr.size() as usize)?;
            let _data_wasm = bridge.copy_to_wasm(store, &data)?;
        }
    }

    Ok((array_ptr, count as u32))
}

/// Factory for creating WASM nodes.
pub struct WasmNodeFactory {
    /// Shared WASM runtime.
    runtime: Arc<WasmRuntime>,
}

impl WasmNodeFactory {
    /// Create a new WASM node factory.
    pub fn new(runtime: Arc<WasmRuntime>) -> Self {
        Self { runtime }
    }

    /// Create a WASM node from configuration.
    pub fn create(&self, config: WasmNodeConfig) -> Result<WasmNode> {
        WasmNode::new(config, Arc::clone(&self.runtime))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_node_config_builder() {
        let config = WasmNodeConfig::new("plugins", "test_node", vec![0x00, 0x61, 0x73, 0x6D])
            .with_description("A test node")
            .with_version("2.0.0")
            .effectful()
            .non_deterministic();

        assert_eq!(config.namespace, "plugins");
        assert_eq!(config.name, "test_node");
        assert_eq!(config.description, "A test node");
        assert_eq!(config.version, "2.0.0");
        assert!(config.effectful);
        assert!(!config.deterministic);
    }
}
