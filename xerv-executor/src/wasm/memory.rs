//! Memory bridge for WASM <-> host data transfer.
//!
//! Provides utilities for copying data between the host arena and WASM
//! linear memory, handling allocation and pointer conversion.

use wasmtime::{Memory, Store, TypedFunc};
use xerv_core::error::{Result, XervError};
use xerv_core::types::ArenaOffset;

/// A pointer within WASM linear memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmPtr {
    /// Offset within WASM linear memory.
    pub offset: u32,
    /// Size of the data in bytes.
    pub size: u32,
}

impl WasmPtr {
    /// Create a new WASM pointer.
    pub const fn new(offset: u32, size: u32) -> Self {
        Self { offset, size }
    }

    /// Create a null pointer.
    pub const fn null() -> Self {
        Self { offset: 0, size: 0 }
    }

    /// Check if this is a null pointer.
    pub const fn is_null(&self) -> bool {
        self.offset == 0 && self.size == 0
    }

    /// Get the end offset (offset + size).
    pub const fn end(&self) -> u32 {
        self.offset + self.size
    }
}

/// Bridge for memory operations between host and WASM.
///
/// Handles allocation within WASM linear memory and data transfer
/// in both directions.
pub struct MemoryBridge<T> {
    /// WASM linear memory.
    memory: Memory,
    /// WASM allocation function: `__xerv_alloc(size: u32) -> u32`.
    alloc_fn: TypedFunc<u32, u32>,
    /// Phantom data for store type.
    _marker: std::marker::PhantomData<T>,
}

impl<T> MemoryBridge<T> {
    /// Create a new memory bridge.
    pub fn new(memory: Memory, alloc_fn: TypedFunc<u32, u32>) -> Self {
        Self {
            memory,
            alloc_fn,
            _marker: std::marker::PhantomData,
        }
    }

    /// Get the WASM memory.
    pub fn memory(&self) -> &Memory {
        &self.memory
    }

    /// Allocate memory in WASM linear memory.
    ///
    /// Calls the WASM module's `__xerv_alloc` function to allocate
    /// a buffer of the specified size.
    pub fn allocate(&self, store: &mut Store<T>, size: u32) -> Result<u32> {
        self.alloc_fn
            .call(store, size)
            .map_err(|_| XervError::WasmMemoryAlloc {
                requested: size as u64,
            })
    }

    /// Copy bytes from host to WASM linear memory.
    ///
    /// Allocates space in WASM memory and copies the data there.
    ///
    /// # Returns
    /// A `WasmPtr` pointing to the copied data in WASM memory.
    pub fn copy_to_wasm(&self, store: &mut Store<T>, data: &[u8]) -> Result<WasmPtr> {
        if data.is_empty() {
            return Ok(WasmPtr::null());
        }

        let size = data.len() as u32;
        let offset = self.allocate(store, size)?;

        // Get mutable slice of WASM memory and copy data
        let mem_data = self.memory.data_mut(store);
        let dest = mem_data
            .get_mut(offset as usize..(offset + size) as usize)
            .ok_or_else(|| XervError::WasmMemoryAlloc {
                requested: size as u64,
            })?;
        dest.copy_from_slice(data);

        Ok(WasmPtr::new(offset, size))
    }

    /// Copy bytes from WASM linear memory to host.
    ///
    /// # Arguments
    /// * `store` - WASM store
    /// * `ptr` - Pointer to data in WASM memory
    ///
    /// # Returns
    /// A vector containing the copied bytes.
    pub fn copy_from_wasm(&self, store: &Store<T>, ptr: WasmPtr) -> Result<Vec<u8>> {
        if ptr.is_null() {
            return Ok(Vec::new());
        }

        let mem_data = self.memory.data(store);
        let src = mem_data
            .get(ptr.offset as usize..ptr.end() as usize)
            .ok_or_else(|| XervError::WasmExecution {
                node_id: xerv_core::types::NodeId::new(0),
                cause: format!(
                    "Invalid WASM memory access: offset={}, size={}",
                    ptr.offset, ptr.size
                ),
            })?;

        Ok(src.to_vec())
    }

    /// Write bytes to a specific location in WASM memory (without allocation).
    ///
    /// Used for writing to pre-allocated buffers.
    pub fn write_at(&self, store: &mut Store<T>, offset: u32, data: &[u8]) -> Result<()> {
        let end = offset as usize + data.len();
        let mem_data = self.memory.data_mut(store);

        let dest =
            mem_data
                .get_mut(offset as usize..end)
                .ok_or_else(|| XervError::WasmExecution {
                    node_id: xerv_core::types::NodeId::new(0),
                    cause: format!(
                        "Invalid WASM memory write: offset={}, size={}",
                        offset,
                        data.len()
                    ),
                })?;

        dest.copy_from_slice(data);
        Ok(())
    }

    /// Read bytes from a specific location in WASM memory.
    pub fn read_at(&self, store: &Store<T>, offset: u32, size: u32) -> Result<Vec<u8>> {
        self.copy_from_wasm(store, WasmPtr::new(offset, size))
    }

    /// Read a null-terminated string from WASM memory.
    pub fn read_string(&self, store: &Store<T>, ptr: u32, len: u32) -> Result<String> {
        let bytes = self.read_at(store, ptr, len)?;
        String::from_utf8(bytes).map_err(|e| XervError::WasmExecution {
            node_id: xerv_core::types::NodeId::new(0),
            cause: format!("Invalid UTF-8 string: {}", e),
        })
    }

    /// Get the current size of WASM linear memory in bytes.
    pub fn size(&self, store: &Store<T>) -> usize {
        self.memory.data_size(store)
    }
}

/// Encode an ArenaOffset as two u32 values (for WASM i32 ABI).
///
/// Arena offsets are 64-bit but WASM uses 32-bit integers, so we split
/// the offset into low and high parts.
#[inline]
pub fn encode_arena_offset(offset: ArenaOffset) -> (u32, u32) {
    let val = offset.as_u64();
    (val as u32, (val >> 32) as u32)
}

/// Decode two u32 values back into an ArenaOffset.
#[inline]
pub fn decode_arena_offset(lo: u32, hi: u32) -> ArenaOffset {
    let val = (hi as u64) << 32 | (lo as u64);
    ArenaOffset::new(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_ptr_basic() {
        let ptr = WasmPtr::new(100, 50);
        assert_eq!(ptr.offset, 100);
        assert_eq!(ptr.size, 50);
        assert_eq!(ptr.end(), 150);
        assert!(!ptr.is_null());
    }

    #[test]
    fn wasm_ptr_null() {
        let ptr = WasmPtr::null();
        assert!(ptr.is_null());
        assert_eq!(ptr.offset, 0);
        assert_eq!(ptr.size, 0);
    }

    #[test]
    fn arena_offset_encoding() {
        // Small offset (fits in u32)
        let offset = ArenaOffset::new(0x12345678);
        let (lo, hi) = encode_arena_offset(offset);
        assert_eq!(lo, 0x12345678);
        assert_eq!(hi, 0);
        assert_eq!(decode_arena_offset(lo, hi), offset);

        // Large offset (requires both u32s)
        let offset = ArenaOffset::new(0xABCD_1234_5678_9ABC);
        let (lo, hi) = encode_arena_offset(offset);
        assert_eq!(lo, 0x5678_9ABC);
        assert_eq!(hi, 0xABCD_1234);
        assert_eq!(decode_arena_offset(lo, hi), offset);
    }
}
