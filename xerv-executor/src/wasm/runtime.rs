//! WASM runtime management using Wasmtime.
//!
//! Provides engine configuration, module compilation, and caching
//! for efficient WASM execution.

use dashmap::DashMap;
use std::sync::Arc;
use wasmtime::{Config, Engine, Module};
use xerv_core::error::{Result, XervError};

/// Default maximum memory pages (64 KB per page).
const DEFAULT_MAX_MEMORY_PAGES: u32 = 1024; // 64 MB

/// Default fuel amount for execution limiting.
const DEFAULT_FUEL: u64 = 10_000_000;

/// Configuration for the WASM runtime.
#[derive(Debug, Clone)]
pub struct WasmRuntimeConfig {
    /// Maximum memory pages allowed (64 KB per page).
    pub max_memory_pages: u32,
    /// Whether to enable fuel-based execution limiting.
    pub fuel_enabled: bool,
    /// Initial fuel amount when fuel is enabled.
    pub fuel_amount: u64,
    /// Whether to enable epoch-based interruption for timeouts.
    pub epoch_interruption: bool,
    /// Whether to cache compiled modules.
    pub cache_modules: bool,
    /// Enable debug info in compiled modules.
    pub debug_info: bool,
}

impl Default for WasmRuntimeConfig {
    fn default() -> Self {
        Self {
            max_memory_pages: DEFAULT_MAX_MEMORY_PAGES,
            fuel_enabled: false,
            fuel_amount: DEFAULT_FUEL,
            epoch_interruption: true,
            cache_modules: true,
            debug_info: false,
        }
    }
}

impl WasmRuntimeConfig {
    /// Create a configuration optimized for production use.
    pub fn production() -> Self {
        Self {
            max_memory_pages: DEFAULT_MAX_MEMORY_PAGES,
            fuel_enabled: false,
            fuel_amount: DEFAULT_FUEL,
            epoch_interruption: true,
            cache_modules: true,
            debug_info: false,
        }
    }

    /// Create a configuration for testing with stricter limits.
    pub fn testing() -> Self {
        Self {
            max_memory_pages: 256, // 16 MB
            fuel_enabled: true,
            fuel_amount: 1_000_000,
            epoch_interruption: true,
            cache_modules: false,
            debug_info: true,
        }
    }

    /// Set maximum memory pages.
    pub fn with_max_memory_pages(mut self, pages: u32) -> Self {
        self.max_memory_pages = pages;
        self
    }

    /// Enable or disable fuel-based limiting.
    pub fn with_fuel(mut self, enabled: bool, amount: u64) -> Self {
        self.fuel_enabled = enabled;
        self.fuel_amount = amount;
        self
    }

    /// Enable or disable epoch interruption.
    pub fn with_epoch_interruption(mut self, enabled: bool) -> Self {
        self.epoch_interruption = enabled;
        self
    }

    /// Enable or disable module caching.
    pub fn with_cache(mut self, enabled: bool) -> Self {
        self.cache_modules = enabled;
        self
    }

    /// Create a Wasmtime Config from this configuration.
    fn to_wasmtime_config(&self) -> Config {
        let mut config = Config::new();

        // Enable epoch interruption for timeout support
        config.epoch_interruption(self.epoch_interruption);

        // Enable fuel consumption for execution limiting
        config.consume_fuel(self.fuel_enabled);

        // Enable debug info if requested
        config.debug_info(self.debug_info);

        // Use Cranelift for compilation
        config.strategy(wasmtime::Strategy::Cranelift);

        config
    }
}

/// A compiled WASM module ready for instantiation.
pub struct CompiledModule {
    /// The compiled Wasmtime module.
    module: Module,
    /// Hash of the original WASM bytes (for caching).
    hash: u64,
}

impl CompiledModule {
    /// Get the underlying Wasmtime module.
    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Get the hash of this module.
    pub fn hash(&self) -> u64 {
        self.hash
    }
}

/// WASM runtime managing Wasmtime engine and compiled modules.
pub struct WasmRuntime {
    /// The Wasmtime engine (thread-safe, can be shared).
    engine: Engine,
    /// Configuration for this runtime.
    config: WasmRuntimeConfig,
    /// Cache of compiled modules by their content hash.
    module_cache: DashMap<u64, Arc<CompiledModule>>,
}

impl WasmRuntime {
    /// Create a new WASM runtime with the given configuration.
    pub fn new(config: WasmRuntimeConfig) -> Result<Self> {
        let wasmtime_config = config.to_wasmtime_config();
        let engine = Engine::new(&wasmtime_config).map_err(|e| XervError::WasmLoad {
            module: "engine".to_string(),
            cause: e.to_string(),
        })?;

        Ok(Self {
            engine,
            config,
            module_cache: DashMap::new(),
        })
    }

    /// Create a new runtime with default configuration.
    pub fn with_defaults() -> Result<Self> {
        Self::new(WasmRuntimeConfig::default())
    }

    /// Get the Wasmtime engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Get the runtime configuration.
    pub fn config(&self) -> &WasmRuntimeConfig {
        &self.config
    }

    /// Compile WASM bytes into a module.
    ///
    /// If caching is enabled and the module was previously compiled,
    /// returns the cached version.
    pub fn compile(&self, name: &str, wasm_bytes: &[u8]) -> Result<Arc<CompiledModule>> {
        let hash = hash_bytes(wasm_bytes);

        // Check cache first
        if self.config.cache_modules {
            if let Some(cached) = self.module_cache.get(&hash) {
                return Ok(Arc::clone(&cached));
            }
        }

        // Compile the module
        let module = Module::new(&self.engine, wasm_bytes).map_err(|e| XervError::WasmLoad {
            module: name.to_string(),
            cause: e.to_string(),
        })?;

        let compiled = Arc::new(CompiledModule { module, hash });

        // Cache if enabled
        if self.config.cache_modules {
            self.module_cache.insert(hash, Arc::clone(&compiled));
        }

        Ok(compiled)
    }

    /// Compile WASM bytes from a file.
    pub fn compile_file(&self, path: &std::path::Path) -> Result<Arc<CompiledModule>> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let wasm_bytes = std::fs::read(path).map_err(|e| XervError::WasmLoad {
            module: name.to_string(),
            cause: e.to_string(),
        })?;

        self.compile(name, &wasm_bytes)
    }

    /// Validate WASM bytes without compiling.
    pub fn validate(&self, wasm_bytes: &[u8]) -> Result<()> {
        self.engine
            .precompile_module(wasm_bytes)
            .map_err(|e| XervError::WasmLoad {
                module: "validation".to_string(),
                cause: e.to_string(),
            })?;
        Ok(())
    }

    /// Clear the module cache.
    pub fn clear_cache(&self) {
        self.module_cache.clear();
    }

    /// Get the number of cached modules.
    pub fn cache_size(&self) -> usize {
        self.module_cache.len()
    }

    /// Get the initial fuel amount for new stores.
    pub fn initial_fuel(&self) -> Option<u64> {
        if self.config.fuel_enabled {
            Some(self.config.fuel_amount)
        } else {
            None
        }
    }
}

/// Compute a hash of bytes (for cache key).
fn hash_bytes(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};

    // Use FxHash for speed (not cryptographic, but good for caching)
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_config_default() {
        let config = WasmRuntimeConfig::default();
        assert_eq!(config.max_memory_pages, DEFAULT_MAX_MEMORY_PAGES);
        assert!(!config.fuel_enabled);
        assert!(config.epoch_interruption);
        assert!(config.cache_modules);
    }

    #[test]
    fn runtime_config_testing() {
        let config = WasmRuntimeConfig::testing();
        assert!(config.fuel_enabled);
        assert!(config.debug_info);
        assert!(!config.cache_modules);
    }

    #[test]
    fn runtime_creation() {
        let runtime = WasmRuntime::with_defaults().expect("Failed to create runtime");
        assert_eq!(runtime.cache_size(), 0);
    }

    #[test]
    fn hash_bytes_consistency() {
        let data = b"test data for hashing";
        let hash1 = hash_bytes(data);
        let hash2 = hash_bytes(data);
        assert_eq!(hash1, hash2);

        let different = b"different data";
        let hash3 = hash_bytes(different);
        assert_ne!(hash1, hash3);
    }
}
