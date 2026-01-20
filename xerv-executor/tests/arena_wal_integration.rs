//! Integration tests for Arena and WAL operations.
//!
//! Tests data persistence, recovery, and zero-copy semantics.

use xerv_core::arena::{Arena, ArenaConfig};
use xerv_core::types::{ArenaOffset, NodeId, RelPtr, TraceId};
use xerv_core::wal::{Wal, WalConfig, WalReader, WalRecord};

mod common;

use common::{test_arena_config, test_wal_config};

// Arena tests

#[test]
fn arena_create_and_access() {
    let trace_id = TraceId::new();
    let arena = Arena::create(trace_id, &test_arena_config()).unwrap();

    assert_eq!(arena.trace_id(), trace_id);
    assert!(arena.available_space() > 0);
}

#[test]
fn arena_write_position_increases() {
    let trace_id = TraceId::new();
    let arena = Arena::create(trace_id, &test_arena_config()).unwrap();
    let initial_pos = arena.write_position().as_u64();

    let writer = arena.writer();
    let _ptr: RelPtr<()> = writer.write_bytes(&[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();

    assert!(arena.write_position().as_u64() > initial_pos);
}

#[test]
fn arena_multiple_writes_sequential() {
    let trace_id = TraceId::new();
    let arena = Arena::create(trace_id, &test_arena_config()).unwrap();
    let writer = arena.writer();

    let ptr1: RelPtr<()> = writer.write_bytes(&[1u8; 100]).unwrap();
    let ptr2: RelPtr<()> = writer.write_bytes(&[2u8; 200]).unwrap();
    let ptr3: RelPtr<()> = writer.write_bytes(&[3u8; 300]).unwrap();

    // Pointers should be different and sequential
    assert!(ptr2.offset().as_u64() > ptr1.offset().as_u64());
    assert!(ptr3.offset().as_u64() > ptr2.offset().as_u64());
}

#[test]
fn arena_config_in_memory_unique() {
    let config1 = ArenaConfig::in_memory();
    let config2 = ArenaConfig::in_memory();
    assert_ne!(config1.directory, config2.directory);
}

// WAL tests

#[test]
fn wal_write_and_read_records() {
    let config = test_wal_config();
    let wal = Wal::open(config.clone()).unwrap();

    let trace_id = TraceId::new();
    let node_id = NodeId::new(1);

    wal.write(&WalRecord::trace_start(trace_id)).unwrap();
    wal.write(&WalRecord::node_done(
        trace_id,
        node_id,
        ArenaOffset::new(100),
        256,
        1234,
    ))
    .unwrap();
    wal.write(&WalRecord::trace_complete(trace_id)).unwrap();
    wal.flush().unwrap();

    let reader = WalReader::new(config.directory);
    let records = reader.read_all().unwrap();
    assert_eq!(records.len(), 3);
}

#[test]
fn wal_sync_on_write() {
    let mut config = test_wal_config();
    config.sync_on_write = true;

    let wal = Wal::open(config).unwrap();
    let record = WalRecord::trace_start(TraceId::new());

    let result = wal.write(&record);
    assert!(result.is_ok());
}

#[test]
fn wal_error_record() {
    let config = test_wal_config();
    let wal = Wal::open(config.clone()).unwrap();

    let trace_id = TraceId::new();
    let node_id = NodeId::new(2);

    wal.write(&WalRecord::node_error(trace_id, node_id, "Test error"))
        .unwrap();
    wal.flush().unwrap();

    let reader = WalReader::new(config.directory);
    let records = reader.read_all().unwrap();
    assert_eq!(records.len(), 1);
}

#[test]
fn wal_config_in_memory_unique() {
    let config1 = WalConfig::in_memory();
    let config2 = WalConfig::in_memory();
    assert_ne!(config1.directory, config2.directory);
}

// Type tests

#[test]
fn rel_ptr_null() {
    let ptr: RelPtr<()> = RelPtr::null();
    assert!(ptr.is_null());
    assert!(ptr.offset().is_null());
}

#[test]
fn trace_id_uniqueness() {
    let id1 = TraceId::new();
    let id2 = TraceId::new();
    assert_ne!(id1, id2);
}

#[test]
fn node_id_display_format() {
    let id = NodeId::new(42);
    assert_eq!(format!("{}", id), "node_42");
}
