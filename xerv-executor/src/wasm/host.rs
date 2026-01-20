//! Host function bindings for WASM modules.
//!
//! Provides the `xerv` namespace imports that WASM modules can call
//! to interact with the host environment (arena, providers, logging).

use super::memory::{decode_arena_offset, encode_arena_offset};
use parking_lot::Mutex;
use std::sync::Arc;
use wasmtime::{Caller, Engine, Linker};
use xerv_core::arena::{ArenaReader, ArenaWriter};
use xerv_core::error::{Result, XervError};
use xerv_core::testing::providers::{
    ClockProvider, EnvProvider, RngProvider, SecretsProvider, UuidProvider,
};
use xerv_core::types::{NodeId, RelPtr, TraceId};

/// Log level for `__xerv_log`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Trace-level logging (most verbose).
    Trace = 0,
    /// Debug-level logging.
    Debug = 1,
    /// Info-level logging.
    Info = 2,
    /// Warning-level logging.
    Warn = 3,
    /// Error-level logging.
    Error = 4,
}

impl From<u32> for LogLevel {
    fn from(val: u32) -> Self {
        match val {
            0 => Self::Trace,
            1 => Self::Debug,
            2 => Self::Info,
            3 => Self::Warn,
            _ => Self::Error,
        }
    }
}

/// State provided to WASM host functions.
///
/// Contains all the context needed by host functions during execution:
/// - Identifiers (trace_id, node_id)
/// - Arena access (reader, writer)
/// - Providers (clock, RNG, UUID, env, secrets)
/// - Execution result storage
pub struct HostState {
    /// Current trace ID.
    pub trace_id: TraceId,
    /// Current node ID.
    pub node_id: NodeId,
    /// Arena reader for reading upstream data.
    pub reader: ArenaReader,
    /// Arena writer for storing outputs.
    pub writer: ArenaWriter,
    /// Clock provider.
    pub clock: Arc<dyn ClockProvider>,
    /// RNG provider.
    pub rng: Arc<dyn RngProvider>,
    /// UUID provider.
    pub uuid: Arc<dyn UuidProvider>,
    /// Environment variable provider.
    pub env: Arc<dyn EnvProvider>,
    /// Secrets provider.
    pub secrets: Arc<dyn SecretsProvider>,
    /// Result port name (set by WASM via `__xerv_set_result`).
    pub result_port: Mutex<Option<String>>,
    /// Result data pointer (set by WASM via `__xerv_set_result`).
    pub result_ptr: Mutex<Option<RelPtr<()>>>,
    /// Error message (set by WASM via `__xerv_set_error`).
    pub error_message: Mutex<Option<String>>,
}

impl HostState {
    /// Create new host state.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        trace_id: TraceId,
        node_id: NodeId,
        reader: ArenaReader,
        writer: ArenaWriter,
        clock: Arc<dyn ClockProvider>,
        rng: Arc<dyn RngProvider>,
        uuid: Arc<dyn UuidProvider>,
        env: Arc<dyn EnvProvider>,
        secrets: Arc<dyn SecretsProvider>,
    ) -> Self {
        Self {
            trace_id,
            node_id,
            reader,
            writer,
            clock,
            rng,
            uuid,
            env,
            secrets,
            result_port: Mutex::new(None),
            result_ptr: Mutex::new(None),
            error_message: Mutex::new(None),
        }
    }

    /// Check if the execution produced an error.
    pub fn has_error(&self) -> bool {
        self.error_message.lock().is_some()
    }

    /// Get the error message, if any.
    pub fn take_error(&self) -> Option<String> {
        self.error_message.lock().take()
    }

    /// Check if a result was set.
    pub fn has_result(&self) -> bool {
        self.result_port.lock().is_some()
    }

    /// Take the result port and pointer.
    pub fn take_result(&self) -> Option<(String, RelPtr<()>)> {
        let port = self.result_port.lock().take()?;
        let ptr = self.result_ptr.lock().take().unwrap_or(RelPtr::null());
        Some((port, ptr))
    }
}

/// Register all host functions with a Wasmtime Linker.
///
/// This registers the `xerv` namespace with all available host functions
/// that WASM modules can import.
pub fn register_host_functions(linker: &mut Linker<HostState>) -> Result<()> {
    // Arena access functions
    register_arena_functions(linker)?;

    // Provider functions
    register_provider_functions(linker)?;

    // Output functions
    register_output_functions(linker)?;

    // Logging functions
    register_logging_functions(linker)?;

    Ok(())
}

/// Register arena access functions.
fn register_arena_functions(linker: &mut Linker<HostState>) -> Result<()> {
    // __xerv_read_bytes(offset_lo: u32, offset_hi: u32, dest: u32, size: u32) -> i32
    // Reads bytes from arena into WASM memory. Returns 0 on success, -1 on error.
    linker
        .func_wrap(
            "xerv",
            "__xerv_read_bytes",
            |mut caller: Caller<'_, HostState>,
             offset_lo: u32,
             offset_hi: u32,
             dest: u32,
             size: u32|
             -> i32 {
                let offset = decode_arena_offset(offset_lo, offset_hi);
                let state = caller.data();

                // Read from arena
                let bytes = match state.reader.read_bytes(offset, size as usize) {
                    Ok(b) => b,
                    Err(_) => return -1,
                };

                // Get WASM memory
                let memory = match caller.get_export("memory") {
                    Some(wasmtime::Extern::Memory(m)) => m,
                    _ => return -1,
                };

                // Write to WASM memory
                let mem_data = memory.data_mut(&mut caller);
                let dest_slice = match mem_data.get_mut(dest as usize..(dest + size) as usize) {
                    Some(s) => s,
                    None => return -1,
                };
                dest_slice.copy_from_slice(&bytes);

                0
            },
        )
        .map_err(|e| XervError::WasmHostFunction {
            function: "__xerv_read_bytes".to_string(),
            cause: e.to_string(),
        })?;

    // __xerv_write_bytes(src: u32, size: u32, out_lo: *mut u32, out_hi: *mut u32) -> i32
    // Writes bytes from WASM memory to arena. Returns arena offset via out params.
    linker
        .func_wrap(
            "xerv",
            "__xerv_write_bytes",
            |mut caller: Caller<'_, HostState>,
             src: u32,
             size: u32,
             out_lo: u32,
             out_hi: u32|
             -> i32 {
                // Get WASM memory
                let memory = match caller.get_export("memory") {
                    Some(wasmtime::Extern::Memory(m)) => m,
                    _ => return -1,
                };

                // Read from WASM memory
                let mem_data = memory.data(&caller);
                let src_slice = match mem_data.get(src as usize..(src + size) as usize) {
                    Some(s) => s,
                    None => return -1,
                };
                let bytes = src_slice.to_vec();

                // Write to arena
                let state = caller.data();
                let ptr: RelPtr<()> = match state.writer.write_bytes(&bytes) {
                    Ok(p) => p,
                    Err(_) => return -1,
                };

                // Write offset back to WASM memory
                let (lo, hi) = encode_arena_offset(ptr.offset());
                let mem_data = memory.data_mut(&mut caller);

                if let Some(out) = mem_data.get_mut(out_lo as usize..(out_lo + 4) as usize) {
                    out.copy_from_slice(&lo.to_le_bytes());
                } else {
                    return -1;
                }

                if let Some(out) = mem_data.get_mut(out_hi as usize..(out_hi + 4) as usize) {
                    out.copy_from_slice(&hi.to_le_bytes());
                } else {
                    return -1;
                }

                ptr.size() as i32
            },
        )
        .map_err(|e| XervError::WasmHostFunction {
            function: "__xerv_write_bytes".to_string(),
            cause: e.to_string(),
        })?;

    Ok(())
}

/// Register provider functions.
fn register_provider_functions(linker: &mut Linker<HostState>) -> Result<()> {
    // __xerv_clock_now() -> u64
    // Returns current monotonic time in nanoseconds.
    linker
        .func_wrap(
            "xerv",
            "__xerv_clock_now",
            |caller: Caller<'_, HostState>| -> u64 { caller.data().clock.now() },
        )
        .map_err(|e| XervError::WasmHostFunction {
            function: "__xerv_clock_now".to_string(),
            cause: e.to_string(),
        })?;

    // __xerv_rng_next_u64() -> u64
    // Returns a random u64.
    linker
        .func_wrap(
            "xerv",
            "__xerv_rng_next_u64",
            |caller: Caller<'_, HostState>| -> u64 { caller.data().rng.next_u64() },
        )
        .map_err(|e| XervError::WasmHostFunction {
            function: "__xerv_rng_next_u64".to_string(),
            cause: e.to_string(),
        })?;

    // __xerv_uuid_new_v4(dest: u32) -> i32
    // Generates a new UUID and writes 16 bytes to dest. Returns 0 on success.
    linker
        .func_wrap(
            "xerv",
            "__xerv_uuid_new_v4",
            |mut caller: Caller<'_, HostState>, dest: u32| -> i32 {
                let uuid = caller.data().uuid.new_v4();
                let bytes = uuid.as_bytes();

                let memory = match caller.get_export("memory") {
                    Some(wasmtime::Extern::Memory(m)) => m,
                    _ => return -1,
                };

                let mem_data = memory.data_mut(&mut caller);
                if let Some(out) = mem_data.get_mut(dest as usize..(dest + 16) as usize) {
                    out.copy_from_slice(bytes);
                    0
                } else {
                    -1
                }
            },
        )
        .map_err(|e| XervError::WasmHostFunction {
            function: "__xerv_uuid_new_v4".to_string(),
            cause: e.to_string(),
        })?;

    // __xerv_env_get(key_ptr: u32, key_len: u32, val_ptr: u32, val_cap: u32) -> i32
    // Gets an environment variable. Returns length written, or -1 if not found.
    linker
        .func_wrap(
            "xerv",
            "__xerv_env_get",
            |mut caller: Caller<'_, HostState>,
             key_ptr: u32,
             key_len: u32,
             val_ptr: u32,
             val_cap: u32|
             -> i32 {
                let memory = match caller.get_export("memory") {
                    Some(wasmtime::Extern::Memory(m)) => m,
                    _ => return -1,
                };

                // Read key from WASM memory
                let mem_data = memory.data(&caller);
                let key_slice = match mem_data.get(key_ptr as usize..(key_ptr + key_len) as usize) {
                    Some(s) => s,
                    None => return -1,
                };
                let key = match std::str::from_utf8(key_slice) {
                    Ok(s) => s.to_string(),
                    Err(_) => return -1,
                };

                // Get the value
                let value = match caller.data().env.var(&key) {
                    Some(v) => v,
                    None => return -1,
                };

                // Write value to WASM memory
                let len = value.len().min(val_cap as usize);
                let mem_data = memory.data_mut(&mut caller);
                if let Some(out) = mem_data.get_mut(val_ptr as usize..(val_ptr as usize + len)) {
                    out.copy_from_slice(&value.as_bytes()[..len]);
                    len as i32
                } else {
                    -1
                }
            },
        )
        .map_err(|e| XervError::WasmHostFunction {
            function: "__xerv_env_get".to_string(),
            cause: e.to_string(),
        })?;

    // __xerv_secret_get(key_ptr: u32, key_len: u32, val_ptr: u32, val_cap: u32) -> i32
    // Gets a secret. Returns length written, or -1 if not found.
    linker
        .func_wrap(
            "xerv",
            "__xerv_secret_get",
            |mut caller: Caller<'_, HostState>,
             key_ptr: u32,
             key_len: u32,
             val_ptr: u32,
             val_cap: u32|
             -> i32 {
                let memory = match caller.get_export("memory") {
                    Some(wasmtime::Extern::Memory(m)) => m,
                    _ => return -1,
                };

                // Read key from WASM memory
                let mem_data = memory.data(&caller);
                let key_slice = match mem_data.get(key_ptr as usize..(key_ptr + key_len) as usize) {
                    Some(s) => s,
                    None => return -1,
                };
                let key = match std::str::from_utf8(key_slice) {
                    Ok(s) => s.to_string(),
                    Err(_) => return -1,
                };

                // Get the secret
                let value = match caller.data().secrets.get(&key) {
                    Some(v) => v,
                    None => return -1,
                };

                // Write value to WASM memory
                let len = value.len().min(val_cap as usize);
                let mem_data = memory.data_mut(&mut caller);
                if let Some(out) = mem_data.get_mut(val_ptr as usize..(val_ptr as usize + len)) {
                    out.copy_from_slice(&value.as_bytes()[..len]);
                    len as i32
                } else {
                    -1
                }
            },
        )
        .map_err(|e| XervError::WasmHostFunction {
            function: "__xerv_secret_get".to_string(),
            cause: e.to_string(),
        })?;

    Ok(())
}

/// Register output functions.
fn register_output_functions(linker: &mut Linker<HostState>) -> Result<()> {
    // __xerv_set_result(port_ptr: u32, port_len: u32, data_ptr: u32, data_size: u32) -> i32
    // Sets the output port and data. Returns 0 on success.
    linker
        .func_wrap(
            "xerv",
            "__xerv_set_result",
            |mut caller: Caller<'_, HostState>,
             port_ptr: u32,
             port_len: u32,
             data_ptr: u32,
             data_size: u32|
             -> i32 {
                let memory = match caller.get_export("memory") {
                    Some(wasmtime::Extern::Memory(m)) => m,
                    _ => return -1,
                };

                // Read port name
                let mem_data = memory.data(&caller);
                let port_slice =
                    match mem_data.get(port_ptr as usize..(port_ptr + port_len) as usize) {
                        Some(s) => s,
                        None => return -1,
                    };
                let port = match std::str::from_utf8(port_slice) {
                    Ok(s) => s.to_string(),
                    Err(_) => return -1,
                };

                // Read data from WASM memory
                let data_slice =
                    match mem_data.get(data_ptr as usize..(data_ptr + data_size) as usize) {
                        Some(s) => s,
                        None => return -1,
                    };
                let data = data_slice.to_vec();

                // Write data to arena
                let state = caller.data();
                let ptr: RelPtr<()> = match state.writer.write_bytes(&data) {
                    Ok(p) => p,
                    Err(_) => return -1,
                };

                // Store result
                *state.result_port.lock() = Some(port);
                *state.result_ptr.lock() = Some(ptr);

                0
            },
        )
        .map_err(|e| XervError::WasmHostFunction {
            function: "__xerv_set_result".to_string(),
            cause: e.to_string(),
        })?;

    // __xerv_set_error(msg_ptr: u32, msg_len: u32) -> i32
    // Sets an error message. Returns 0 on success.
    linker
        .func_wrap(
            "xerv",
            "__xerv_set_error",
            |mut caller: Caller<'_, HostState>, msg_ptr: u32, msg_len: u32| -> i32 {
                let memory = match caller.get_export("memory") {
                    Some(wasmtime::Extern::Memory(m)) => m,
                    _ => return -1,
                };

                // Read error message
                let mem_data = memory.data(&caller);
                let msg_slice = match mem_data.get(msg_ptr as usize..(msg_ptr + msg_len) as usize) {
                    Some(s) => s,
                    None => return -1,
                };
                let msg = match std::str::from_utf8(msg_slice) {
                    Ok(s) => s.to_string(),
                    Err(_) => return -1,
                };

                *caller.data().error_message.lock() = Some(msg);
                0
            },
        )
        .map_err(|e| XervError::WasmHostFunction {
            function: "__xerv_set_error".to_string(),
            cause: e.to_string(),
        })?;

    Ok(())
}

/// Register logging functions.
fn register_logging_functions(linker: &mut Linker<HostState>) -> Result<()> {
    // __xerv_log(level: u32, msg_ptr: u32, msg_len: u32) -> i32
    // Logs a message. Returns 0 on success.
    linker
        .func_wrap(
            "xerv",
            "__xerv_log",
            |mut caller: Caller<'_, HostState>, level: u32, msg_ptr: u32, msg_len: u32| -> i32 {
                let memory = match caller.get_export("memory") {
                    Some(wasmtime::Extern::Memory(m)) => m,
                    _ => return -1,
                };

                // Read log message
                let mem_data = memory.data(&caller);
                let msg_slice = match mem_data.get(msg_ptr as usize..(msg_ptr + msg_len) as usize) {
                    Some(s) => s,
                    None => return -1,
                };
                let msg = match std::str::from_utf8(msg_slice) {
                    Ok(s) => s,
                    Err(_) => return -1,
                };

                let state = caller.data();
                let level = LogLevel::from(level);

                match level {
                    LogLevel::Trace => tracing::trace!(
                        trace_id = %state.trace_id,
                        node_id = %state.node_id,
                        "[WASM] {}",
                        msg
                    ),
                    LogLevel::Debug => tracing::debug!(
                        trace_id = %state.trace_id,
                        node_id = %state.node_id,
                        "[WASM] {}",
                        msg
                    ),
                    LogLevel::Info => tracing::info!(
                        trace_id = %state.trace_id,
                        node_id = %state.node_id,
                        "[WASM] {}",
                        msg
                    ),
                    LogLevel::Warn => tracing::warn!(
                        trace_id = %state.trace_id,
                        node_id = %state.node_id,
                        "[WASM] {}",
                        msg
                    ),
                    LogLevel::Error => tracing::error!(
                        trace_id = %state.trace_id,
                        node_id = %state.node_id,
                        "[WASM] {}",
                        msg
                    ),
                }

                0
            },
        )
        .map_err(|e| XervError::WasmHostFunction {
            function: "__xerv_log".to_string(),
            cause: e.to_string(),
        })?;

    Ok(())
}

/// Create a linker with all host functions registered.
pub fn create_linker(engine: &Engine) -> Result<Linker<HostState>> {
    let mut linker = Linker::new(engine);
    register_host_functions(&mut linker)?;
    Ok(linker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_conversion() {
        assert_eq!(LogLevel::from(0), LogLevel::Trace);
        assert_eq!(LogLevel::from(1), LogLevel::Debug);
        assert_eq!(LogLevel::from(2), LogLevel::Info);
        assert_eq!(LogLevel::from(3), LogLevel::Warn);
        assert_eq!(LogLevel::from(4), LogLevel::Error);
        assert_eq!(LogLevel::from(99), LogLevel::Error); // Default to error
    }
}
