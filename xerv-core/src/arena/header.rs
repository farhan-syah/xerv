//! Arena header structure.

use crate::types::{ArenaOffset, TraceId};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Read, Write};

/// Magic number for XERV arena files.
pub const ARENA_MAGIC: u64 = 0x5845_5256_4152_454E; // "XERVARENA" in hex

/// Current arena format version.
pub const ARENA_VERSION: u32 = 1;

/// Fixed size of the arena header in bytes.
pub const HEADER_SIZE: usize = 128;

/// Arena file header.
///
/// This is stored at the beginning of every arena file and contains
/// metadata needed to interpret the rest of the file.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ArenaHeader {
    /// Magic number for file identification.
    pub magic: u64,
    /// Arena format version.
    pub version: u32,
    /// Flags (reserved for future use).
    pub flags: u32,
    /// Trace ID that owns this arena.
    pub trace_id: TraceId,
    /// Offset to the pipeline configuration data.
    pub config_offset: ArenaOffset,
    /// Size of the pipeline configuration in bytes.
    pub config_size: u32,
    /// Offset to the start of the data region.
    pub data_offset: ArenaOffset,
    /// Current write position (end of valid data).
    pub write_pos: ArenaOffset,
    /// Total capacity of the arena file.
    pub capacity: u64,
    /// Creation timestamp (Unix epoch seconds).
    pub created_at: u64,
    /// Schema version string hash (for compatibility checking).
    pub schema_hash: u64,
    /// Pipeline version number.
    pub pipeline_version: u32,
    /// Reserved for alignment and future use.
    pub _reserved: [u8; 32],
}

impl ArenaHeader {
    /// Create a new arena header for the given trace.
    pub fn new(trace_id: TraceId, capacity: u64) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self {
            magic: ARENA_MAGIC,
            version: ARENA_VERSION,
            flags: 0,
            trace_id,
            config_offset: ArenaOffset::new(HEADER_SIZE as u64),
            config_size: 0,
            data_offset: ArenaOffset::new(HEADER_SIZE as u64),
            write_pos: ArenaOffset::new(HEADER_SIZE as u64),
            capacity,
            created_at: now,
            schema_hash: 0,
            pipeline_version: 0,
            _reserved: [0u8; 32],
        }
    }

    /// Validate the header.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.magic != ARENA_MAGIC {
            return Err("Invalid magic number");
        }
        if self.version != ARENA_VERSION {
            return Err("Unsupported arena version");
        }
        if self.write_pos.as_u64() > self.capacity {
            return Err("Write position exceeds capacity");
        }
        Ok(())
    }

    /// Read header from a byte slice.
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < HEADER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Buffer too small for header",
            ));
        }

        let mut cursor = io::Cursor::new(bytes);

        let magic = cursor.read_u64::<LittleEndian>()?;
        let version = cursor.read_u32::<LittleEndian>()?;
        let flags = cursor.read_u32::<LittleEndian>()?;

        // Read trace_id (16 bytes UUID)
        let mut uuid_bytes = [0u8; 16];
        cursor.read_exact(&mut uuid_bytes)?;
        let trace_id = TraceId::from_uuid(uuid::Uuid::from_bytes(uuid_bytes));

        let config_offset = ArenaOffset::new(cursor.read_u64::<LittleEndian>()?);
        let config_size = cursor.read_u32::<LittleEndian>()?;
        let _padding1 = cursor.read_u32::<LittleEndian>()?; // alignment padding
        let data_offset = ArenaOffset::new(cursor.read_u64::<LittleEndian>()?);
        let write_pos = ArenaOffset::new(cursor.read_u64::<LittleEndian>()?);
        let capacity = cursor.read_u64::<LittleEndian>()?;
        let created_at = cursor.read_u64::<LittleEndian>()?;
        let schema_hash = cursor.read_u64::<LittleEndian>()?;
        let pipeline_version = cursor.read_u32::<LittleEndian>()?;
        let _padding2 = cursor.read_u32::<LittleEndian>()?; // alignment padding after pipeline_version

        let mut reserved = [0u8; 32];
        cursor.read_exact(&mut reserved)?;

        Ok(Self {
            magic,
            version,
            flags,
            trace_id,
            config_offset,
            config_size,
            data_offset,
            write_pos,
            capacity,
            created_at,
            schema_hash,
            pipeline_version,
            _reserved: reserved,
        })
    }

    /// Write header to a byte buffer.
    pub fn to_bytes(&self) -> io::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(HEADER_SIZE);

        buf.write_u64::<LittleEndian>(self.magic)?;
        buf.write_u32::<LittleEndian>(self.version)?;
        buf.write_u32::<LittleEndian>(self.flags)?;

        // Write trace_id (16 bytes UUID)
        buf.write_all(self.trace_id.as_uuid().as_bytes())?;

        buf.write_u64::<LittleEndian>(self.config_offset.as_u64())?;
        buf.write_u32::<LittleEndian>(self.config_size)?;
        buf.write_u32::<LittleEndian>(0)?; // alignment padding
        buf.write_u64::<LittleEndian>(self.data_offset.as_u64())?;
        buf.write_u64::<LittleEndian>(self.write_pos.as_u64())?;
        buf.write_u64::<LittleEndian>(self.capacity)?;
        buf.write_u64::<LittleEndian>(self.created_at)?;
        buf.write_u64::<LittleEndian>(self.schema_hash)?;
        buf.write_u32::<LittleEndian>(self.pipeline_version)?;
        buf.write_u32::<LittleEndian>(0)?; // alignment padding after pipeline_version

        buf.write_all(&self._reserved)?;

        // Ensure we wrote exactly HEADER_SIZE bytes
        debug_assert_eq!(buf.len(), HEADER_SIZE);

        Ok(buf)
    }

    /// Get available space for data.
    pub fn available_space(&self) -> u64 {
        self.capacity.saturating_sub(self.write_pos.as_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let trace_id = TraceId::new();
        let header = ArenaHeader::new(trace_id, 1024 * 1024);

        let bytes = header.to_bytes().unwrap();
        assert_eq!(bytes.len(), HEADER_SIZE);

        let restored = ArenaHeader::from_bytes(&bytes).unwrap();
        assert_eq!(restored.magic, ARENA_MAGIC);
        assert_eq!(restored.version, ARENA_VERSION);
        assert_eq!(restored.trace_id, trace_id);
        assert_eq!(restored.capacity, 1024 * 1024);
    }

    #[test]
    fn header_validation() {
        let header = ArenaHeader::new(TraceId::new(), 1024);
        assert!(header.validate().is_ok());

        let mut bad_magic = header;
        bad_magic.magic = 0xDEADBEEF;
        assert!(bad_magic.validate().is_err());

        let mut bad_pos = header;
        bad_pos.write_pos = ArenaOffset::new(2048);
        assert!(bad_pos.validate().is_err());
    }

    #[test]
    fn header_size_is_128() {
        // Ensure header fits in exactly 128 bytes for alignment
        assert_eq!(HEADER_SIZE, 128);
    }
}
