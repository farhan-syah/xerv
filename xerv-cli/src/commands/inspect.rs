//! Inspect command - inspect a trace arena file with detailed node execution info.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use xerv_core::arena::Arena;
use xerv_core::types::TraceId;
use xerv_core::wal::{WalReader, WalRecordType};

/// Run the inspect command.
pub async fn run(trace: &str, detailed: bool) -> Result<()> {
    tracing::info!(trace = %trace, detailed = %detailed, "Inspecting trace");

    // Determine if trace is a UUID or a file path
    let (arena, wal_dir) = if let Ok(trace_id) = trace.parse::<uuid::Uuid>() {
        // Try to find the arena file for this trace ID
        let trace_id = TraceId::from_uuid(trace_id);
        let default_dir = Path::new("/tmp/xerv");
        let arena_path = default_dir.join(format!("trace_{}.bin", trace_id.as_uuid()));

        if !arena_path.exists() {
            anyhow::bail!(
                "Arena file not found for trace {}: {}",
                trace_id.as_uuid(),
                arena_path.display()
            );
        }

        let arena = Arena::open(&arena_path)
            .with_context(|| format!("Failed to open arena: {}", arena_path.display()))?;

        let wal_dir = default_dir.join("wal");
        (arena, wal_dir)
    } else {
        // Treat as a file path
        let path = Path::new(trace);
        if !path.exists() {
            anyhow::bail!("Arena file not found: {}", trace);
        }

        let arena =
            Arena::open(path).with_context(|| format!("Failed to open arena: {}", trace))?;

        // Try to find WAL directory relative to arena file
        let wal_dir = path
            .parent()
            .map(|p| p.join("wal"))
            .unwrap_or_else(|| Path::new("/tmp/xerv/wal").to_path_buf());

        (arena, wal_dir)
    };

    // Print trace information
    println!("Trace Information");
    println!("=================");
    println!("Trace ID:       {}", arena.trace_id().as_uuid());
    println!("Arena Path:     {}", arena.path().display());
    println!("Write Position: {} bytes", arena.write_position().as_u64());
    println!("Available:      {} bytes", arena.available_space());
    println!();

    if !detailed {
        println!("Use --detailed (-d) flag to see node execution timeline.");
        return Ok(());
    }

    // Detailed mode: read WAL records
    if !wal_dir.exists() {
        println!("WAL directory not found: {}", wal_dir.display());
        println!("Detailed trace inspection requires WAL records.");
        return Ok(());
    }

    let reader = WalReader::new(&wal_dir);
    let records = reader.read_all().map_err(|e| anyhow::anyhow!("{}", e))?;

    // Filter records for this trace
    let trace_id = arena.trace_id();
    let trace_records: Vec<_> = records.iter().filter(|r| r.trace_id == trace_id).collect();

    if trace_records.is_empty() {
        println!("No WAL records found for trace {}.", trace_id.as_uuid());
        return Ok(());
    }

    // Show execution timeline
    println!("Execution Timeline");
    println!("==================");

    let mut node_starts: HashMap<u32, u64> = HashMap::new();
    let mut trace_start: Option<u64> = None;
    let mut trace_end: Option<u64> = None;
    let mut total_errors = 0;
    let mut nodes_executed = 0;

    for record in &trace_records {
        let timestamp = format_timestamp(record.timestamp_ns);

        match record.record_type {
            WalRecordType::TraceStart => {
                trace_start = Some(record.timestamp_ns);
                println!("  {} ▶ Trace started", timestamp);
            }
            WalRecordType::NodeStart => {
                node_starts.insert(record.node_id.as_u32(), record.timestamp_ns);
                println!(
                    "  {}   ├─ Node {} started",
                    timestamp,
                    record.node_id.as_u32()
                );
            }
            WalRecordType::NodeDone => {
                nodes_executed += 1;
                let duration = if let Some(start) = node_starts.get(&record.node_id.as_u32()) {
                    format_duration(record.timestamp_ns.saturating_sub(*start))
                } else {
                    "?".to_string()
                };
                println!(
                    "  {}   ├─ Node {} completed ({}) @ offset {}, {} bytes",
                    timestamp,
                    record.node_id.as_u32(),
                    duration,
                    record.output_offset.as_u64(),
                    record.output_size
                );
            }
            WalRecordType::NodeError => {
                total_errors += 1;
                let error_msg = record.error_message.as_deref().unwrap_or("Unknown error");
                println!(
                    "  {}   ├─ Node {} FAILED: {}",
                    timestamp,
                    record.node_id.as_u32(),
                    error_msg
                );
            }
            WalRecordType::TraceComplete => {
                trace_end = Some(record.timestamp_ns);
                let duration = if let Some(start) = trace_start {
                    format_duration(record.timestamp_ns.saturating_sub(start))
                } else {
                    "?".to_string()
                };
                println!("  {} ✓ Trace completed ({})", timestamp, duration);
            }
            WalRecordType::TraceFailed => {
                trace_end = Some(record.timestamp_ns);
                let error_msg = record.error_message.as_deref().unwrap_or("Unknown error");
                let duration = if let Some(start) = trace_start {
                    format_duration(record.timestamp_ns.saturating_sub(start))
                } else {
                    "?".to_string()
                };
                println!(
                    "  {} ✗ Trace FAILED ({}) - {}",
                    timestamp, duration, error_msg
                );
            }
            WalRecordType::TraceSuspended => {
                println!(
                    "  {}   ├─ Trace suspended at node {}",
                    timestamp,
                    record.node_id.as_u32()
                );
            }
            WalRecordType::TraceResumed => {
                println!("  {}   ├─ Trace resumed", timestamp);
            }
            WalRecordType::LoopIteration => {
                println!(
                    "  {}   ├─ Loop iteration {} at node {}",
                    timestamp,
                    record.iteration,
                    record.node_id.as_u32()
                );
            }
            WalRecordType::LoopExit => {
                println!(
                    "  {}   ├─ Loop exited at node {}",
                    timestamp,
                    record.node_id.as_u32()
                );
            }
            WalRecordType::Checkpoint => {
                println!("  {}   ├─ Checkpoint marker", timestamp);
            }
        }
    }

    println!();

    // Summary
    println!("Summary");
    println!("=======");
    println!("  Total WAL records:  {}", trace_records.len());
    println!("  Nodes executed:     {}", nodes_executed);
    println!("  Errors:             {}", total_errors);

    if let (Some(start), Some(end)) = (trace_start, trace_end) {
        println!("  Total duration:     {}", format_duration(end - start));
    } else if trace_start.is_some() && trace_end.is_none() {
        println!("  Status:             IN PROGRESS or CRASHED");
    }

    // Show incomplete nodes (started but not completed)
    let incomplete_nodes: Vec<_> = trace_records
        .iter()
        .filter(|r| r.record_type == WalRecordType::NodeStart)
        .filter(|r| {
            !trace_records.iter().any(|done| {
                done.node_id == r.node_id
                    && matches!(
                        done.record_type,
                        WalRecordType::NodeDone | WalRecordType::NodeError
                    )
            })
        })
        .collect();

    if !incomplete_nodes.is_empty() {
        println!();
        println!("Incomplete Nodes (may indicate crash):");
        for record in incomplete_nodes {
            println!(
                "  - Node {} (started but not completed)",
                record.node_id.as_u32()
            );
        }
    }

    Ok(())
}

/// Format a nanosecond timestamp as HH:MM:SS.mmm.
fn format_timestamp(timestamp_ns: u64) -> String {
    use std::time::{Duration, UNIX_EPOCH};

    let duration = Duration::from_nanos(timestamp_ns);
    let datetime = UNIX_EPOCH + duration;

    // Use chrono for formatting if available, otherwise basic format
    match datetime.duration_since(UNIX_EPOCH) {
        Ok(since_epoch) => {
            let secs = since_epoch.as_secs();
            let millis = since_epoch.subsec_millis();

            let hours = (secs / 3600) % 24;
            let minutes = (secs / 60) % 60;
            let seconds = secs % 60;

            format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
        }
        Err(_) => "??:??:??.???".to_string(),
    }
}

/// Format a duration in nanoseconds as human-readable string.
fn format_duration(duration_ns: u64) -> String {
    if duration_ns < 1_000 {
        format!("{}ns", duration_ns)
    } else if duration_ns < 1_000_000 {
        format!("{:.2}µs", duration_ns as f64 / 1_000.0)
    } else if duration_ns < 1_000_000_000 {
        format!("{:.2}ms", duration_ns as f64 / 1_000_000.0)
    } else {
        format!("{:.2}s", duration_ns as f64 / 1_000_000_000.0)
    }
}
