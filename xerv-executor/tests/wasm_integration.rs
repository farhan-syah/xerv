//! Integration tests for WASM node support.
//!
//! These tests verify the WASM runtime, module compilation, and basic
//! functionality of WASM nodes.

#![cfg(feature = "wasm")]

use std::sync::Arc;
use xerv_executor::wasm::{
    TransferStrategy, WasmNodeConfig, WasmNodeFactory, WasmRuntime, WasmRuntimeConfig,
};

/// Create a minimal valid WASM module with required exports.
/// Uses wasmtime's built-in WAT parser for correctness.
fn create_xerv_module() -> Vec<u8> {
    let wat = r#"
        (module
            (memory (export "memory") 1)
            (func (export "__xerv_alloc") (param i32) (result i32)
                i32.const 1024)
            (func (export "__xerv_execute") (param i32 i32) (result i32)
                i32.const 0))
    "#;
    wat::parse_str(wat).expect("Failed to parse WAT")
}

#[test]
fn runtime_creation_default() {
    let runtime = WasmRuntime::with_defaults().expect("Failed to create runtime");
    assert_eq!(runtime.cache_size(), 0);
}

#[test]
fn runtime_creation_custom_config() {
    let config = WasmRuntimeConfig::default()
        .with_max_memory_pages(512)
        .with_fuel(true, 500_000)
        .with_epoch_interruption(true)
        .with_cache(true);

    let runtime = WasmRuntime::new(config).expect("Failed to create runtime");
    assert_eq!(runtime.initial_fuel(), Some(500_000));
}

#[test]
fn runtime_creation_testing_config() {
    let config = WasmRuntimeConfig::testing();
    let runtime = WasmRuntime::new(config).expect("Failed to create runtime");
    assert!(runtime.config().fuel_enabled);
    assert!(runtime.config().debug_info);
}

#[test]
fn module_compilation_valid() {
    let runtime = WasmRuntime::with_defaults().expect("Failed to create runtime");
    let wasm_bytes = create_xerv_module();
    let result = runtime.compile("test_module", &wasm_bytes);
    assert!(
        result.is_ok(),
        "Failed to compile module: {:?}",
        result.err()
    );
}

#[test]
fn module_compilation_invalid() {
    let runtime = WasmRuntime::with_defaults().expect("Failed to create runtime");
    let invalid_wasm = b"not a wasm module";
    let result = runtime.compile("invalid", invalid_wasm);
    assert!(result.is_err());
}

#[test]
fn module_caching() {
    let config = WasmRuntimeConfig::default().with_cache(true);
    let runtime = WasmRuntime::new(config).expect("Failed to create runtime");
    let wasm_bytes = create_xerv_module();

    // First compilation
    let module1 = runtime
        .compile("test1", &wasm_bytes)
        .expect("First compile failed");
    assert_eq!(runtime.cache_size(), 1);

    // Second compilation with same bytes should hit cache
    let module2 = runtime
        .compile("test2", &wasm_bytes)
        .expect("Second compile failed");
    assert_eq!(runtime.cache_size(), 1);
    assert_eq!(module1.hash(), module2.hash());

    // Clear cache
    runtime.clear_cache();
    assert_eq!(runtime.cache_size(), 0);
}

#[test]
fn strategy_selection() {
    // Small payloads use SliceCopy
    assert_eq!(TransferStrategy::select(0), TransferStrategy::SliceCopy);
    assert_eq!(TransferStrategy::select(1024), TransferStrategy::SliceCopy);
    assert_eq!(
        TransferStrategy::select(1024 * 1024 - 1),
        TransferStrategy::SliceCopy
    );

    // Large payloads use HostApi
    assert_eq!(
        TransferStrategy::select(1024 * 1024),
        TransferStrategy::HostApi
    );
    assert_eq!(
        TransferStrategy::select(10 * 1024 * 1024),
        TransferStrategy::HostApi
    );
}

#[test]
fn node_config_builder() {
    let wasm_bytes = create_xerv_module();
    let config = WasmNodeConfig::new("plugins", "fraud_detector", wasm_bytes)
        .with_description("Fraud detection using ML model")
        .with_version("2.0.0")
        .effectful()
        .non_deterministic();

    assert_eq!(config.namespace, "plugins");
    assert_eq!(config.name, "fraud_detector");
    assert_eq!(config.description, "Fraud detection using ML model");
    assert_eq!(config.version, "2.0.0");
    assert!(config.effectful);
    assert!(!config.deterministic);
}

#[test]
fn node_factory_creation() {
    let runtime = Arc::new(WasmRuntime::with_defaults().expect("Failed to create runtime"));
    let factory = WasmNodeFactory::new(Arc::clone(&runtime));
    let wasm_bytes = create_xerv_module();

    let config = WasmNodeConfig::new("test", "passthrough", wasm_bytes);
    let node = factory.create(config);
    assert!(node.is_ok(), "Failed to create node: {:?}", node.err());
}

#[test]
fn node_factory_invalid_module() {
    let runtime = Arc::new(WasmRuntime::with_defaults().expect("Failed to create runtime"));
    let factory = WasmNodeFactory::new(Arc::clone(&runtime));

    let config = WasmNodeConfig::new("test", "invalid", b"not wasm".to_vec());
    let node = factory.create(config);
    assert!(node.is_err());
}

#[test]
fn node_info_from_config() {
    let runtime = Arc::new(WasmRuntime::with_defaults().expect("Failed to create runtime"));
    let wasm_bytes = create_xerv_module();
    let config = WasmNodeConfig::new("custom", "my_node", wasm_bytes)
        .with_description("A custom node")
        .with_version("1.2.3")
        .effectful();

    let node = xerv_executor::wasm::WasmNode::new(config, runtime).expect("Failed to create node");

    use xerv_core::traits::Node;
    let info = node.info();
    assert_eq!(info.name, "custom::my_node");
    assert_eq!(info.namespace, "custom");
    assert_eq!(info.short_name, "my_node");
    assert_eq!(info.description, "A custom node");
    assert_eq!(info.version, "1.2.3");
    assert!(info.effectful);
    assert!(info.deterministic); // Still deterministic unless marked otherwise
}
