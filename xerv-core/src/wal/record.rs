//! WAL record types and serialization.

use crate::types::{ArenaOffset, NodeId, TraceId};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Read, Write};

/// Minimum record size (header without payload).
pub const MIN_RECORD_SIZE: usize = 4 + 4 + 1 + 16 + 4; // 29 bytes

/// Type of WAL record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WalRecordType {
    /// Trace started.
    TraceStart = 0,
    /// Node execution started.
    NodeStart = 1,
    /// Node execution completed successfully.
    NodeDone = 2,
    /// Node execution failed.
    NodeError = 3,
    /// Trace completed successfully.
    TraceComplete = 4,
    /// Trace failed.
    TraceFailed = 5,
    /// Trace suspended (e.g., at a wait node).
    TraceSuspended = 6,
    /// Trace resumed.
    TraceResumed = 7,
    /// Checkpoint marker.
    Checkpoint = 8,
    /// Loop iteration started.
    LoopIteration = 9,
    /// Loop exited.
    LoopExit = 10,
}

impl TryFrom<u8> for WalRecordType {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::TraceStart),
            1 => Ok(Self::NodeStart),
            2 => Ok(Self::NodeDone),
            3 => Ok(Self::NodeError),
            4 => Ok(Self::TraceComplete),
            5 => Ok(Self::TraceFailed),
            6 => Ok(Self::TraceSuspended),
            7 => Ok(Self::TraceResumed),
            8 => Ok(Self::Checkpoint),
            9 => Ok(Self::LoopIteration),
            10 => Ok(Self::LoopExit),
            _ => Err("Unknown WAL record type"),
        }
    }
}

/// A single WAL record.
#[derive(Debug, Clone)]
pub struct WalRecord {
    /// Type of this record.
    pub record_type: WalRecordType,
    /// The trace this record belongs to.
    pub trace_id: TraceId,
    /// The node this record is about (if applicable).
    pub node_id: NodeId,
    /// Timestamp (Unix epoch nanoseconds).
    pub timestamp_ns: u64,
    /// Arena offset for output data (if applicable).
    pub output_offset: ArenaOffset,
    /// Output size in bytes.
    pub output_size: u32,
    /// Schema version hash.
    pub schema_hash: u64,
    /// Error message (if applicable).
    pub error_message: Option<String>,
    /// Loop iteration number (for loop records).
    pub iteration: u32,
    /// Additional metadata (JSON-encoded).
    pub metadata: Option<String>,
}

impl WalRecord {
    /// Create a trace start record.
    pub fn trace_start(trace_id: TraceId) -> Self {
        Self {
            record_type: WalRecordType::TraceStart,
            trace_id,
            node_id: NodeId::new(0),
            timestamp_ns: current_timestamp_ns(),
            output_offset: ArenaOffset::NULL,
            output_size: 0,
            schema_hash: 0,
            error_message: None,
            iteration: 0,
            metadata: None,
        }
    }

    /// Create a node start record.
    pub fn node_start(trace_id: TraceId, node_id: NodeId) -> Self {
        Self {
            record_type: WalRecordType::NodeStart,
            trace_id,
            node_id,
            timestamp_ns: current_timestamp_ns(),
            output_offset: ArenaOffset::NULL,
            output_size: 0,
            schema_hash: 0,
            error_message: None,
            iteration: 0,
            metadata: None,
        }
    }

    /// Create a node done record.
    pub fn node_done(
        trace_id: TraceId,
        node_id: NodeId,
        output_offset: ArenaOffset,
        output_size: u32,
        schema_hash: u64,
    ) -> Self {
        Self {
            record_type: WalRecordType::NodeDone,
            trace_id,
            node_id,
            timestamp_ns: current_timestamp_ns(),
            output_offset,
            output_size,
            schema_hash,
            error_message: None,
            iteration: 0,
            metadata: None,
        }
    }

    /// Create a node error record.
    pub fn node_error(trace_id: TraceId, node_id: NodeId, error: impl ToString) -> Self {
        Self {
            record_type: WalRecordType::NodeError,
            trace_id,
            node_id,
            timestamp_ns: current_timestamp_ns(),
            output_offset: ArenaOffset::NULL,
            output_size: 0,
            schema_hash: 0,
            error_message: Some(error.to_string()),
            iteration: 0,
            metadata: None,
        }
    }

    /// Create a trace complete record.
    pub fn trace_complete(trace_id: TraceId) -> Self {
        Self {
            record_type: WalRecordType::TraceComplete,
            trace_id,
            node_id: NodeId::new(0),
            timestamp_ns: current_timestamp_ns(),
            output_offset: ArenaOffset::NULL,
            output_size: 0,
            schema_hash: 0,
            error_message: None,
            iteration: 0,
            metadata: None,
        }
    }

    /// Create a trace failed record.
    pub fn trace_failed(trace_id: TraceId, error: impl ToString) -> Self {
        Self {
            record_type: WalRecordType::TraceFailed,
            trace_id,
            node_id: NodeId::new(0),
            timestamp_ns: current_timestamp_ns(),
            output_offset: ArenaOffset::NULL,
            output_size: 0,
            schema_hash: 0,
            error_message: Some(error.to_string()),
            iteration: 0,
            metadata: None,
        }
    }

    /// Create a trace suspended record.
    pub fn trace_suspended(trace_id: TraceId, node_id: NodeId) -> Self {
        Self {
            record_type: WalRecordType::TraceSuspended,
            trace_id,
            node_id,
            timestamp_ns: current_timestamp_ns(),
            output_offset: ArenaOffset::NULL,
            output_size: 0,
            schema_hash: 0,
            error_message: None,
            iteration: 0,
            metadata: None,
        }
    }

    /// Create a trace resumed record (for crash recovery or wait resumption).
    pub fn trace_resumed(trace_id: TraceId) -> Self {
        Self {
            record_type: WalRecordType::TraceResumed,
            trace_id,
            node_id: NodeId::new(0),
            timestamp_ns: current_timestamp_ns(),
            output_offset: ArenaOffset::NULL,
            output_size: 0,
            schema_hash: 0,
            error_message: None,
            iteration: 0,
            metadata: None,
        }
    }

    /// Create a loop iteration record.
    pub fn loop_iteration(trace_id: TraceId, node_id: NodeId, iteration: u32) -> Self {
        Self {
            record_type: WalRecordType::LoopIteration,
            trace_id,
            node_id,
            timestamp_ns: current_timestamp_ns(),
            output_offset: ArenaOffset::NULL,
            output_size: 0,
            schema_hash: 0,
            error_message: None,
            iteration,
            metadata: None,
        }
    }

    /// Set metadata on this record.
    pub fn with_metadata(mut self, metadata: impl ToString) -> Self {
        self.metadata = Some(metadata.to_string());
        self
    }

    /// Serialize the record to bytes.
    pub fn to_bytes(&self) -> io::Result<Vec<u8>> {
        let mut payload = Vec::new();

        // Write fixed fields
        payload.write_u64::<LittleEndian>(self.timestamp_ns)?;
        payload.write_u64::<LittleEndian>(self.output_offset.as_u64())?;
        payload.write_u32::<LittleEndian>(self.output_size)?;
        payload.write_u64::<LittleEndian>(self.schema_hash)?;
        payload.write_u32::<LittleEndian>(self.iteration)?;

        // Write error message (length-prefixed)
        if let Some(ref msg) = self.error_message {
            let bytes = msg.as_bytes();
            payload.write_u32::<LittleEndian>(bytes.len() as u32)?;
            payload.write_all(bytes)?;
        } else {
            payload.write_u32::<LittleEndian>(0)?;
        }

        // Write metadata (length-prefixed)
        if let Some(ref meta) = self.metadata {
            let bytes = meta.as_bytes();
            payload.write_u32::<LittleEndian>(bytes.len() as u32)?;
            payload.write_all(bytes)?;
        } else {
            payload.write_u32::<LittleEndian>(0)?;
        }

        // Calculate CRC32 of the payload
        let crc = crc32fast::hash(&payload);

        // Build final record
        let total_len = MIN_RECORD_SIZE + payload.len();
        let mut record = Vec::with_capacity(total_len);

        record.write_u32::<LittleEndian>(total_len as u32)?;
        record.write_u32::<LittleEndian>(crc)?;
        record.write_u8(self.record_type as u8)?;
        record.write_all(self.trace_id.as_uuid().as_bytes())?;
        record.write_u32::<LittleEndian>(self.node_id.as_u32())?;
        record.write_all(&payload)?;

        Ok(record)
    }

    /// Deserialize a record from bytes.
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < MIN_RECORD_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Record too small",
            ));
        }

        let mut cursor = io::Cursor::new(bytes);

        let total_len = cursor.read_u32::<LittleEndian>()? as usize;
        let stored_crc = cursor.read_u32::<LittleEndian>()?;
        let record_type_byte = cursor.read_u8()?;

        let record_type = WalRecordType::try_from(record_type_byte)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let mut uuid_bytes = [0u8; 16];
        cursor.read_exact(&mut uuid_bytes)?;
        let trace_id = TraceId::from_uuid(uuid::Uuid::from_bytes(uuid_bytes));

        let node_id = NodeId::new(cursor.read_u32::<LittleEndian>()?);

        // Read payload
        let payload_start = MIN_RECORD_SIZE;
        let payload_end = total_len;

        if bytes.len() < payload_end {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Record truncated: expected {} bytes, got {}",
                    total_len,
                    bytes.len()
                ),
            ));
        }

        let payload = &bytes[payload_start..payload_end];

        // Verify CRC
        let computed_crc = crc32fast::hash(payload);
        if computed_crc != stored_crc {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "CRC mismatch: expected {}, got {}",
                    stored_crc, computed_crc
                ),
            ));
        }

        // Parse payload
        let mut payload_cursor = io::Cursor::new(payload);

        let timestamp_ns = payload_cursor.read_u64::<LittleEndian>()?;
        let output_offset = ArenaOffset::new(payload_cursor.read_u64::<LittleEndian>()?);
        let output_size = payload_cursor.read_u32::<LittleEndian>()?;
        let schema_hash = payload_cursor.read_u64::<LittleEndian>()?;
        let iteration = payload_cursor.read_u32::<LittleEndian>()?;

        // Read error message
        let error_len = payload_cursor.read_u32::<LittleEndian>()? as usize;
        let error_message = if error_len > 0 {
            let mut buf = vec![0u8; error_len];
            payload_cursor.read_exact(&mut buf)?;
            Some(String::from_utf8_lossy(&buf).into_owned())
        } else {
            None
        };

        // Read metadata
        let meta_len = payload_cursor.read_u32::<LittleEndian>()? as usize;
        let metadata = if meta_len > 0 {
            let mut buf = vec![0u8; meta_len];
            payload_cursor.read_exact(&mut buf)?;
            Some(String::from_utf8_lossy(&buf).into_owned())
        } else {
            None
        };

        Ok(Self {
            record_type,
            trace_id,
            node_id,
            timestamp_ns,
            output_offset,
            output_size,
            schema_hash,
            error_message,
            iteration,
            metadata,
        })
    }

    /// Get the total serialized size of this record.
    pub fn serialized_size(&self) -> usize {
        let mut size = MIN_RECORD_SIZE;
        size += 8 + 8 + 4 + 8 + 4; // Fixed payload fields
        size += 4; // Error message length
        if let Some(ref msg) = self.error_message {
            size += msg.len();
        }
        size += 4; // Metadata length
        if let Some(ref meta) = self.metadata {
            size += meta.len();
        }
        size
    }
}

/// Get current timestamp in nanoseconds since Unix epoch.
fn current_timestamp_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_roundtrip() {
        let trace_id = TraceId::new();
        let node_id = NodeId::new(42);

        let record =
            WalRecord::node_done(trace_id, node_id, ArenaOffset::new(0x1000), 256, 0xDEADBEEF);

        let bytes = record.to_bytes().unwrap();
        let restored = WalRecord::from_bytes(&bytes).unwrap();

        assert_eq!(restored.record_type, WalRecordType::NodeDone);
        assert_eq!(restored.trace_id, trace_id);
        assert_eq!(restored.node_id, node_id);
        assert_eq!(restored.output_offset, ArenaOffset::new(0x1000));
        assert_eq!(restored.output_size, 256);
        assert_eq!(restored.schema_hash, 0xDEADBEEF);
    }

    #[test]
    fn record_with_error() {
        let trace_id = TraceId::new();
        let node_id = NodeId::new(1);

        let record = WalRecord::node_error(trace_id, node_id, "Something went wrong");

        let bytes = record.to_bytes().unwrap();
        let restored = WalRecord::from_bytes(&bytes).unwrap();

        assert_eq!(restored.record_type, WalRecordType::NodeError);
        assert_eq!(
            restored.error_message.as_deref(),
            Some("Something went wrong")
        );
    }

    #[test]
    fn record_with_metadata() {
        let trace_id = TraceId::new();

        let record =
            WalRecord::trace_start(trace_id).with_metadata(r#"{"pipeline": "order_processing"}"#);

        let bytes = record.to_bytes().unwrap();
        let restored = WalRecord::from_bytes(&bytes).unwrap();

        assert_eq!(
            restored.metadata.as_deref(),
            Some(r#"{"pipeline": "order_processing"}"#)
        );
    }

    #[test]
    fn crc_verification() {
        let record = WalRecord::trace_start(TraceId::new());
        let mut bytes = record.to_bytes().unwrap();

        // Corrupt a byte in the payload
        if bytes.len() > MIN_RECORD_SIZE {
            bytes[MIN_RECORD_SIZE] ^= 0xFF;
        }

        // Should fail CRC check
        assert!(WalRecord::from_bytes(&bytes).is_err());
    }
}
