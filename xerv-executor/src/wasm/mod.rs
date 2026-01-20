//! WASM integration for XERV nodes.
//!
//! This module provides support for executing WASM modules as XERV nodes,
//! enabling nodes written in any language that compiles to WASM (Rust, Go,
//! C/C++, AssemblyScript) while maintaining zero-copy semantics where possible.
//!
//! # Architecture
//!
//! The WASM integration consists of several components:
//!
//! - **WasmRuntime**: Manages the Wasmtime engine and compiled modules
//! - **WasmNode**: Implements the `Node` trait for WASM modules
//! - **HostState**: State provided to WASM host functions
//! - **MemoryBridge**: Handles data transfer between host and WASM memory
//! - **TransferStrategy**: Selects copy vs host-api based on payload size
//!
//! # WASM Module ABI Contract
//!
//! WASM modules must export:
//!
//! ```text
//! memory: Memory
//! __xerv_alloc(size: u32) -> u32           // Allocate buffer, return ptr
//! __xerv_execute(inputs: u32, count: u32) -> i32  // Main entry point
//! ```
//!
//! Modules can import from the `xerv` namespace:
//!
//! ```text
//! // Arena access
//! __xerv_read_bytes(offset_lo, offset_hi, dest, size) -> i32
//! __xerv_write_bytes(src, size, out_lo, out_hi) -> i32
//!
//! // Providers
//! __xerv_clock_now() -> u64
//! __xerv_rng_next_u64() -> u64
//! __xerv_uuid_new_v4(dest) -> i32
//! __xerv_env_get(key_ptr, key_len, val_ptr, val_cap) -> i32
//! __xerv_secret_get(key_ptr, key_len, val_ptr, val_cap) -> i32
//!
//! // Output
//! __xerv_set_result(port_ptr, port_len, data_ptr, data_size) -> i32
//! __xerv_set_error(msg_ptr, msg_len) -> i32
//!
//! // Logging
//! __xerv_log(level, msg_ptr, msg_len) -> i32
//! ```
//!
//! # Data Transfer Strategies
//!
//! | Payload Size | Strategy | Mechanism |
//! |:-------------|:---------|:----------|
//! | < 1 MB | SliceCopy | Copy bytes to WASM linear memory |
//! | >= 1 MB | HostApi | WASM calls host functions for access |
//!
//! # Example
//!
//! ```ignore
//! use xerv_executor::wasm::{WasmRuntime, WasmRuntimeConfig, WasmNode, WasmNodeConfig};
//! use std::sync::Arc;
//!
//! // Create runtime
//! let runtime = Arc::new(WasmRuntime::new(WasmRuntimeConfig::default())?);
//!
//! // Load WASM module
//! let wasm_bytes = std::fs::read("my_node.wasm")?;
//!
//! // Create node
//! let config = WasmNodeConfig::new("plugins", "my_node", wasm_bytes)
//!     .with_description("My custom WASM node");
//! let node = WasmNode::new(config, runtime)?;
//!
//! // Use node in a flow...
//! ```

mod host;
mod memory;
mod node;
mod runtime;
mod strategy;

// Re-export public types
pub use host::{HostState, LogLevel, create_linker, register_host_functions};
pub use memory::{MemoryBridge, WasmPtr, decode_arena_offset, encode_arena_offset};
pub use node::{WasmNode, WasmNodeConfig, WasmNodeFactory};
pub use runtime::{CompiledModule, WasmRuntime, WasmRuntimeConfig};
pub use strategy::TransferStrategy;
