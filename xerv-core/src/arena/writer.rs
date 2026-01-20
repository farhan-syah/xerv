//! Arena read/write implementation.

use super::header::{ArenaHeader, HEADER_SIZE};
use crate::error::{Result, XervError};
use crate::types::{ArenaOffset, RelPtr, TraceId};
use fs2::FileExt;
use memmap2::{MmapMut, MmapOptions};
use parking_lot::RwLock;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Default arena size: 16 MB.
pub const DEFAULT_ARENA_SIZE: u64 = 16 * 1024 * 1024;

/// Maximum arena size: 4 GB.
pub const MAX_ARENA_SIZE: u64 = 4 * 1024 * 1024 * 1024;

/// Minimum entry alignment (8 bytes for rkyv).
pub const ENTRY_ALIGNMENT: usize = 8;

/// Configuration for arena creation.
#[derive(Debug, Clone)]
pub struct ArenaConfig {
    /// Initial capacity in bytes.
    pub capacity: u64,
    /// Directory for arena files.
    pub directory: PathBuf,
    /// Whether to sync writes to disk immediately.
    pub sync_on_write: bool,
}

impl Default for ArenaConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_ARENA_SIZE,
            directory: PathBuf::from("/tmp/xerv"),
            sync_on_write: false,
        }
    }
}

impl ArenaConfig {
    /// Create an in-memory configuration for testing.
    ///
    /// Uses a temporary directory with a unique name per invocation.
    pub fn in_memory() -> Self {
        Self {
            capacity: 4 * 1024 * 1024, // 4 MB for tests
            directory: std::env::temp_dir().join(format!("xerv_arena_{}", uuid::Uuid::new_v4())),
            sync_on_write: false,
        }
    }

    /// Create config with custom capacity.
    pub fn with_capacity(mut self, capacity: u64) -> Self {
        self.capacity = capacity.min(MAX_ARENA_SIZE);
        self
    }

    /// Create config with custom directory.
    pub fn with_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directory = directory.into();
        self
    }

    /// Enable sync on write for durability.
    pub fn with_sync(mut self, sync: bool) -> Self {
        self.sync_on_write = sync;
        self
    }
}

/// Shared arena state.
struct ArenaInner {
    /// The memory-mapped file.
    mmap: MmapMut,
    /// The underlying file handle.
    file: File,
    /// Path to the arena file.
    path: PathBuf,
    /// Current write position (atomic for lock-free reads).
    write_pos: AtomicU64,
    /// Header information.
    header: ArenaHeader,
    /// Whether to sync on write.
    sync_on_write: bool,
    /// Capacity.
    capacity: u64,
}

/// A memory-mapped arena for zero-copy data storage.
///
/// The `Arena` is the owner of the mmap and provides both read and write access.
/// For concurrent access patterns, use `ArenaReader` and `ArenaWriter`.
pub struct Arena {
    inner: Arc<RwLock<ArenaInner>>,
    trace_id: TraceId,
}

impl Arena {
    /// Create a new arena for the given trace.
    pub fn create(trace_id: TraceId, config: &ArenaConfig) -> Result<Self> {
        // Ensure directory exists
        std::fs::create_dir_all(&config.directory).map_err(|e| XervError::ArenaCreate {
            path: config.directory.clone(),
            cause: e.to_string(),
        })?;

        let filename = format!("trace_{}.bin", trace_id.as_uuid());
        let path = config.directory.join(&filename);

        // Create and size the file
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| XervError::ArenaCreate {
                path: path.clone(),
                cause: e.to_string(),
            })?;

        // Acquire exclusive lock
        file.try_lock_exclusive()
            .map_err(|e| XervError::ArenaCreate {
                path: path.clone(),
                cause: format!("Failed to lock file: {}", e),
            })?;

        // Set file size
        file.set_len(config.capacity)
            .map_err(|e| XervError::ArenaCreate {
                path: path.clone(),
                cause: e.to_string(),
            })?;

        // Memory map the file
        let mut mmap = unsafe {
            MmapOptions::new()
                .len(config.capacity as usize)
                .map_mut(&file)
                .map_err(|e| XervError::ArenaMmap {
                    path: path.clone(),
                    cause: e.to_string(),
                })?
        };

        // Write header
        let header = ArenaHeader::new(trace_id, config.capacity);
        let header_bytes = header.to_bytes().map_err(|e| XervError::ArenaCreate {
            path: path.clone(),
            cause: e.to_string(),
        })?;

        mmap[..HEADER_SIZE].copy_from_slice(&header_bytes);

        if config.sync_on_write {
            mmap.flush().map_err(|e| XervError::ArenaCreate {
                path: path.clone(),
                cause: e.to_string(),
            })?;
        }

        let inner = ArenaInner {
            mmap,
            file,
            path,
            write_pos: AtomicU64::new(header.write_pos.as_u64()),
            header,
            sync_on_write: config.sync_on_write,
            capacity: config.capacity,
        };

        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
            trace_id,
        })
    }

    /// Open an existing arena file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| XervError::ArenaCreate {
                path: path.clone(),
                cause: e.to_string(),
            })?;

        // Acquire exclusive lock
        file.try_lock_exclusive()
            .map_err(|e| XervError::ArenaCreate {
                path: path.clone(),
                cause: format!("Failed to lock file: {}", e),
            })?;

        let metadata = file.metadata().map_err(|e| XervError::ArenaCreate {
            path: path.clone(),
            cause: e.to_string(),
        })?;

        let capacity = metadata.len();

        // Memory map the file
        let mmap = unsafe {
            MmapOptions::new()
                .len(capacity as usize)
                .map_mut(&file)
                .map_err(|e| XervError::ArenaMmap {
                    path: path.clone(),
                    cause: e.to_string(),
                })?
        };

        // Read and validate header
        let header = ArenaHeader::from_bytes(&mmap[..HEADER_SIZE]).map_err(|e| {
            XervError::ArenaCorruption {
                offset: ArenaOffset::new(0),
                cause: e.to_string(),
            }
        })?;

        header.validate().map_err(|e| XervError::ArenaCorruption {
            offset: ArenaOffset::new(0),
            cause: e.to_string(),
        })?;

        let trace_id = header.trace_id;

        let inner = ArenaInner {
            mmap,
            file,
            path,
            write_pos: AtomicU64::new(header.write_pos.as_u64()),
            header,
            sync_on_write: false,
            capacity,
        };

        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
            trace_id,
        })
    }

    /// Get the trace ID for this arena.
    pub fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    /// Get the current write position.
    pub fn write_position(&self) -> ArenaOffset {
        let inner = self.inner.read();
        ArenaOffset::new(inner.write_pos.load(Ordering::Acquire))
    }

    /// Get available space in bytes.
    pub fn available_space(&self) -> u64 {
        let inner = self.inner.read();
        let write_pos = inner.write_pos.load(Ordering::Acquire);
        inner.capacity.saturating_sub(write_pos)
    }

    /// Get the path to the arena file.
    pub fn path(&self) -> PathBuf {
        self.inner.read().path.clone()
    }

    /// Write serialized bytes to the arena and return a relative pointer.
    pub fn write_bytes<T>(&self, bytes: &[u8]) -> Result<RelPtr<T>> {
        let mut inner = self.inner.write();

        // Calculate aligned size
        let size = bytes.len();
        let aligned_size = (size + ENTRY_ALIGNMENT - 1) & !(ENTRY_ALIGNMENT - 1);

        // Check capacity
        let write_pos = inner.write_pos.load(Ordering::Acquire);
        let new_pos = write_pos + aligned_size as u64;

        if new_pos > inner.capacity {
            return Err(XervError::ArenaCapacity {
                requested: aligned_size as u64,
                available: inner.capacity - write_pos,
            });
        }

        // Write the data
        let offset = ArenaOffset::new(write_pos);
        inner.mmap[write_pos as usize..write_pos as usize + size].copy_from_slice(bytes);

        // Zero padding for alignment
        if aligned_size > size {
            inner.mmap[write_pos as usize + size..write_pos as usize + aligned_size].fill(0);
        }

        // Update write position
        inner.write_pos.store(new_pos, Ordering::Release);

        // Sync if configured
        if inner.sync_on_write {
            inner.mmap.flush().map_err(|e| XervError::ArenaWrite {
                trace_id: self.trace_id,
                offset,
                cause: e.to_string(),
            })?;
        }

        Ok(RelPtr::new(offset, size as u32))
    }

    /// Read bytes from the arena.
    pub fn read_bytes(&self, offset: ArenaOffset, size: usize) -> Result<Vec<u8>> {
        let inner = self.inner.read();

        let off = offset.as_u64() as usize;
        let write_pos = inner.write_pos.load(Ordering::Acquire) as usize;

        // Validate offset and size
        if off + size > write_pos {
            return Err(XervError::ArenaInvalidOffset {
                offset,
                cause: format!(
                    "Offset {} + size {} exceeds write position {}",
                    off, size, write_pos
                ),
            });
        }

        Ok(inner.mmap[off..off + size].to_vec())
    }

    /// Flush all pending writes to disk.
    pub fn flush(&self) -> Result<()> {
        let inner = self.inner.read();
        inner.mmap.flush().map_err(|e| XervError::ArenaWrite {
            trace_id: self.trace_id,
            offset: self.write_position(),
            cause: e.to_string(),
        })
    }

    /// Update the header on disk (e.g., after writing config).
    fn update_header(&self) -> Result<()> {
        let mut inner = self.inner.write();
        inner.header.write_pos = ArenaOffset::new(inner.write_pos.load(Ordering::Acquire));

        let header_bytes = inner.header.to_bytes().map_err(|e| XervError::ArenaWrite {
            trace_id: self.trace_id,
            offset: ArenaOffset::new(0),
            cause: e.to_string(),
        })?;

        inner.mmap[..HEADER_SIZE].copy_from_slice(&header_bytes);

        if inner.sync_on_write {
            inner.mmap.flush().map_err(|e| XervError::ArenaWrite {
                trace_id: self.trace_id,
                offset: ArenaOffset::new(0),
                cause: e.to_string(),
            })?;
        }

        Ok(())
    }

    /// Create a reader handle for this arena.
    pub fn reader(&self) -> ArenaReader {
        ArenaReader {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Create a writer handle for this arena.
    pub fn writer(&self) -> ArenaWriter {
        ArenaWriter {
            inner: Arc::clone(&self.inner),
            trace_id: self.trace_id,
        }
    }

    /// Delete the arena file from disk.
    ///
    /// This should be called when a trace completes and the arena is no longer needed.
    /// The arena becomes unusable after this call.
    pub fn delete(self) -> Result<()> {
        let path = {
            let inner = self.inner.read();
            inner.path.clone()
        };

        // Drop self to release the mmap and file handle
        drop(self);

        // Now delete the file
        std::fs::remove_file(&path).map_err(|e| XervError::ArenaCreate {
            path,
            cause: format!("Failed to delete arena file: {}", e),
        })
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        // Update header with final write position before closing
        let _ = self.update_header();

        // Try to unlock the file
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            let inner = inner.get_mut();
            let _ = fs2::FileExt::unlock(&inner.file);
        }
    }
}

/// A read-only handle to an arena.
///
/// Multiple readers can exist concurrently.
#[derive(Clone)]
pub struct ArenaReader {
    inner: Arc<RwLock<ArenaInner>>,
}

impl ArenaReader {
    /// Read bytes from the arena.
    pub fn read_bytes(&self, offset: ArenaOffset, size: usize) -> Result<Vec<u8>> {
        let inner = self.inner.read();

        let off = offset.as_u64() as usize;
        let write_pos = inner.write_pos.load(Ordering::Acquire) as usize;

        if off + size > write_pos {
            return Err(XervError::ArenaInvalidOffset {
                offset,
                cause: format!(
                    "Offset {} + size {} exceeds write position {}",
                    off, size, write_pos
                ),
            });
        }

        Ok(inner.mmap[off..off + size].to_vec())
    }

    /// Get the current write position.
    pub fn write_position(&self) -> ArenaOffset {
        let inner = self.inner.read();
        ArenaOffset::new(inner.write_pos.load(Ordering::Acquire))
    }
}

/// A write handle to an arena.
///
/// Only one writer should be active at a time for correctness.
pub struct ArenaWriter {
    inner: Arc<RwLock<ArenaInner>>,
    trace_id: TraceId,
}

impl ArenaWriter {
    /// Write bytes to the arena.
    pub fn write_bytes<T>(&self, bytes: &[u8]) -> Result<RelPtr<T>> {
        let mut inner = self.inner.write();

        let size = bytes.len();
        let aligned_size = (size + ENTRY_ALIGNMENT - 1) & !(ENTRY_ALIGNMENT - 1);

        let write_pos = inner.write_pos.load(Ordering::Acquire);
        let new_pos = write_pos + aligned_size as u64;

        if new_pos > inner.capacity {
            return Err(XervError::ArenaCapacity {
                requested: aligned_size as u64,
                available: inner.capacity - write_pos,
            });
        }

        let offset = ArenaOffset::new(write_pos);
        inner.mmap[write_pos as usize..write_pos as usize + size].copy_from_slice(bytes);

        if aligned_size > size {
            inner.mmap[write_pos as usize + size..write_pos as usize + aligned_size].fill(0);
        }

        inner.write_pos.store(new_pos, Ordering::Release);

        if inner.sync_on_write {
            inner.mmap.flush().map_err(|e| XervError::ArenaWrite {
                trace_id: self.trace_id,
                offset,
                cause: e.to_string(),
            })?;
        }

        Ok(RelPtr::new(offset, size as u32))
    }

    /// Flush all pending writes to disk.
    pub fn flush(&self) -> Result<()> {
        let inner = self.inner.read();
        inner.mmap.flush().map_err(|e| XervError::ArenaWrite {
            trace_id: self.trace_id,
            offset: ArenaOffset::new(inner.write_pos.load(Ordering::Acquire)),
            cause: e.to_string(),
        })
    }

    /// Get the current write position.
    pub fn write_position(&self) -> ArenaOffset {
        let inner = self.inner.read();
        ArenaOffset::new(inner.write_pos.load(Ordering::Acquire))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn arena_create_and_write() {
        let dir = tempdir().unwrap();
        let config = ArenaConfig::default()
            .with_capacity(1024 * 1024)
            .with_directory(dir.path());

        let trace_id = TraceId::new();
        let arena = Arena::create(trace_id, &config).unwrap();

        assert!(arena.path().exists());
        assert_eq!(arena.trace_id(), trace_id);
    }

    #[test]
    fn arena_write_and_read_bytes() {
        let dir = tempdir().unwrap();
        let config = ArenaConfig::default()
            .with_capacity(1024 * 1024)
            .with_directory(dir.path());

        let arena = Arena::create(TraceId::new(), &config).unwrap();

        let data = b"Hello, XERV!";
        let ptr: RelPtr<()> = arena.write_bytes(data).unwrap();
        assert!(!ptr.is_null());

        let read_back = arena.read_bytes(ptr.offset(), ptr.size() as usize).unwrap();
        assert_eq!(&read_back, data);
    }

    #[test]
    fn arena_multiple_writes() {
        let dir = tempdir().unwrap();
        let config = ArenaConfig::default()
            .with_capacity(1024 * 1024)
            .with_directory(dir.path());

        let arena = Arena::create(TraceId::new(), &config).unwrap();

        let mut ptrs = Vec::new();
        for i in 0..100 {
            let data = format!("item_{}", i);
            let ptr: RelPtr<()> = arena.write_bytes(data.as_bytes()).unwrap();
            ptrs.push((ptr, data));
        }

        // Verify all data can be read back
        for (ptr, expected) in &ptrs {
            let read_back = arena.read_bytes(ptr.offset(), ptr.size() as usize).unwrap();
            assert_eq!(String::from_utf8(read_back).unwrap(), *expected);
        }
    }

    #[test]
    fn arena_capacity_check() {
        let dir = tempdir().unwrap();
        let config = ArenaConfig::default()
            .with_capacity(256) // Very small
            .with_directory(dir.path());

        let arena = Arena::create(TraceId::new(), &config).unwrap();

        // This should eventually fail due to capacity
        let data = "a".repeat(100);

        // Keep writing until we run out of space
        let mut writes = 0;
        while arena.write_bytes::<()>(data.as_bytes()).is_ok() {
            writes += 1;
            if writes > 10 {
                break; // Prevent infinite loop in case of bug
            }
        }

        // Should have failed at some point
        assert!(writes < 10);
    }

    #[test]
    fn arena_reader_writer() {
        let dir = tempdir().unwrap();
        let config = ArenaConfig::default()
            .with_capacity(1024 * 1024)
            .with_directory(dir.path());

        let arena = Arena::create(TraceId::new(), &config).unwrap();
        let writer = arena.writer();
        let reader = arena.reader();

        let data = b"shared data";
        let ptr: RelPtr<()> = writer.write_bytes(data).unwrap();

        let read_back = reader
            .read_bytes(ptr.offset(), ptr.size() as usize)
            .unwrap();
        assert_eq!(&read_back, data);
    }
}
