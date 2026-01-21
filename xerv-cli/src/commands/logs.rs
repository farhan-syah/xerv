//! Log streaming and query CLI commands.
//!
//! Provides commands for querying and streaming logs from a running XERV server.

use anyhow::{Context, Result};
use xerv_client::{Client, LogEntry, TraceId};

/// Options for querying logs.
#[derive(Debug, Default)]
pub struct LogQueryOptions<'a> {
    /// Server host.
    pub host: &'a str,
    /// Server port.
    pub port: u16,
    /// Filter by log level.
    pub level: Option<&'a str>,
    /// Filter by trace ID.
    pub trace_id: Option<&'a str>,
    /// Filter by pipeline ID.
    pub pipeline_id: Option<&'a str>,
    /// Filter by node ID.
    pub node_id: Option<u32>,
    /// Filter by message content.
    pub contains: Option<&'a str>,
    /// Maximum number of logs to return.
    pub limit: Option<usize>,
    /// Whether to follow (stream) logs.
    pub follow: bool,
}

impl<'a> LogQueryOptions<'a> {
    /// Create new query options with host and port.
    pub fn new(host: &'a str, port: u16) -> Self {
        Self {
            host,
            port,
            ..Default::default()
        }
    }

    /// Set the log level filter.
    #[allow(dead_code)]
    pub fn with_level(mut self, level: &'a str) -> Self {
        self.level = Some(level);
        self
    }

    /// Set the trace ID filter.
    #[allow(dead_code)]
    pub fn with_trace_id(mut self, trace_id: &'a str) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Set the pipeline ID filter.
    #[allow(dead_code)]
    pub fn with_pipeline_id(mut self, pipeline_id: &'a str) -> Self {
        self.pipeline_id = Some(pipeline_id);
        self
    }

    /// Set the node ID filter.
    #[allow(dead_code)]
    pub fn with_node_id(mut self, node_id: u32) -> Self {
        self.node_id = Some(node_id);
        self
    }

    /// Set the message content filter.
    #[allow(dead_code)]
    pub fn with_contains(mut self, contains: &'a str) -> Self {
        self.contains = Some(contains);
        self
    }

    /// Set the limit.
    #[allow(dead_code)]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Enable follow mode.
    #[allow(dead_code)]
    pub fn follow(mut self) -> Self {
        self.follow = true;
        self
    }
}

/// Query logs from the server.
pub async fn query(options: LogQueryOptions<'_>) -> Result<()> {
    let base_url = format!("http://{}:{}", options.host, options.port);
    let client = Client::new(&base_url)?;

    // Parse trace_id if provided
    let trace_id = if let Some(tid_str) = options.trace_id {
        Some(TraceId::from_uuid(
            tid_str
                .parse::<uuid::Uuid>()
                .context("Invalid trace ID format")?,
        ))
    } else {
        None
    };

    if options.follow {
        // Poll for logs continuously
        follow_logs(&client, trace_id, options.level, options.limit).await
    } else {
        // Single query
        query_logs_once(
            &client,
            options.level,
            trace_id,
            options.pipeline_id,
            options.contains,
            options.limit,
        )
        .await
    }
}

/// Query logs once and display.
async fn query_logs_once(
    client: &Client,
    level: Option<&str>,
    trace_id: Option<TraceId>,
    pipeline_id: Option<&str>,
    contains: Option<&str>,
    limit: Option<usize>,
) -> Result<()> {
    let logs = client
        .query_logs(level, trace_id, pipeline_id, contains, limit)
        .await
        .context("Failed to query logs")?;

    if logs.is_empty() {
        println!("No logs found matching the filter.");
        return Ok(());
    }

    for log in &logs {
        print_log_entry(log);
    }

    println!();
    println!("Showing {} logs", logs.len());

    Ok(())
}

/// Follow logs continuously.
async fn follow_logs(
    client: &Client,
    _trace_id: Option<TraceId>,
    _level: Option<&str>,
    _limit: Option<usize>,
) -> Result<()> {
    use std::collections::HashSet;
    use tokio::time::{Duration, sleep};

    let mut seen_ids: HashSet<u64> = HashSet::new();

    println!("Following logs (Ctrl+C to stop)...");
    println!();

    // First, show recent logs
    if let Ok(logs) = client.get_recent_logs(50).await {
        for log in logs {
            seen_ids.insert(log.id);
        }
    }

    loop {
        match client.query_logs(None, None, None, None, Some(100)).await {
            Ok(logs) => {
                for log in logs {
                    if !seen_ids.contains(&log.id) {
                        seen_ids.insert(log.id);
                        print_log_entry(&log);
                    }
                }
            }
            Err(e) => {
                eprintln!("Error fetching logs: {}", e);
            }
        }

        sleep(Duration::from_secs(1)).await;
    }
}

/// Get logs for a specific trace.
pub async fn by_trace(host: &str, port: u16, trace_id: &str) -> Result<()> {
    let base_url = format!("http://{}:{}", host, port);
    let client = Client::new(&base_url)?;

    let trace_id = TraceId::from_uuid(
        trace_id
            .parse::<uuid::Uuid>()
            .context("Invalid trace ID format")?,
    );

    let logs = client
        .get_trace_logs(trace_id)
        .await
        .context("Failed to fetch trace logs")?;

    if logs.is_empty() {
        println!("No logs found for trace: {}", trace_id.as_uuid());
        return Ok(());
    }

    println!("Logs for trace: {}", trace_id.as_uuid());
    println!("{:-<80}", "");
    println!();

    for log in &logs {
        print_log_entry(log);
    }

    println!();
    println!("Total: {} logs", logs.len());

    Ok(())
}

/// Get log statistics.
pub async fn stats(host: &str, port: u16) -> Result<()> {
    let base_url = format!("http://{}:{}", host, port);
    let client = Client::new(&base_url)?;

    let stats = client
        .get_log_stats()
        .await
        .context("Failed to fetch log statistics")?;

    println!("Log Statistics");
    println!("{:-<40}", "");

    println!("  Total Events: {}", stats.total);
    println!("  Buffer Capacity: {}", stats.capacity);
    if stats.capacity > 0 {
        println!(
            "  Buffer Usage: {:.1}%",
            (stats.total as f64 / stats.capacity as f64) * 100.0
        );
    }
    println!();

    println!("  By Level:");
    let levels = ["trace", "debug", "info", "warn", "error"];
    for level in levels {
        let count = stats.by_level.get(level).copied().unwrap_or(0);
        if count > 0 || stats.total == 0 {
            let bar_len = if stats.total > 0 {
                ((count as f64 / stats.total as f64) * 20.0) as usize
            } else {
                0
            };
            let bar = "█".repeat(bar_len);
            println!("    {:>5}: {:>5} {}", level, count, bar);
        }
    }

    Ok(())
}

/// Clear all logs on the server.
pub async fn clear(host: &str, port: u16) -> Result<()> {
    let base_url = format!("http://{}:{}", host, port);
    let client = Client::new(&base_url)?;

    client.clear_logs().await.context("Failed to clear logs")?;

    println!("Logs cleared successfully.");

    Ok(())
}

/// Print a formatted log entry.
fn print_log_entry(log: &LogEntry) {
    let timestamp = &log.timestamp;
    let level = &log.level;
    let category = &log.category;
    let message = &log.message;

    // Color based on level
    let level_colored = match level.as_str() {
        "trace" => format!("\x1b[90m{}\x1b[0m", level.to_uppercase()),
        "debug" => format!("\x1b[36m{}\x1b[0m", level.to_uppercase()),
        "info" => format!("\x1b[32m{}\x1b[0m", level.to_uppercase()),
        "warn" => format!("\x1b[33m{}\x1b[0m", level.to_uppercase()),
        "error" => format!("\x1b[31m{}\x1b[0m", level.to_uppercase()),
        _ => level.to_uppercase(),
    };

    // Build context string
    let mut context_parts = Vec::new();
    if let Some(ref trace_id) = log.trace_id {
        let trace_str = trace_id.as_uuid().to_string();
        context_parts.push(format!("trace={}", &trace_str[..8.min(trace_str.len())]));
    }
    if let Some(node_id) = log.node_id {
        context_parts.push(format!("node={}", node_id.as_u32()));
    }
    if let Some(ref pipeline_id) = log.pipeline_id {
        context_parts.push(format!("pipeline={}", pipeline_id));
    }

    let context = if context_parts.is_empty() {
        String::new()
    } else {
        format!(" [{}]", context_parts.join(" "))
    };

    println!(
        "{} [{:>5}] [{}]{} {}",
        timestamp, level_colored, category, context, message
    );

    // Print fields if present
    if !log.fields.is_empty() {
        let field_strs: Vec<String> = log
            .fields
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        println!("         {{{}}}", field_strs.join(", "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xerv_client::NodeId;

    #[test]
    fn test_print_log_entry() {
        let log = LogEntry {
            id: 1,
            timestamp: "2024-01-15T10:30:00.000Z".to_string(),
            level: "info".to_string(),
            category: "node".to_string(),
            message: "Test message".to_string(),
            trace_id: Some(TraceId::from_uuid(uuid::Uuid::new_v4())),
            node_id: Some(NodeId::new(42)),
            pipeline_id: Some("test-pipeline".to_string()),
            fields: serde_json::Map::new(),
        };

        // Just verify it doesn't panic
        print_log_entry(&log);
    }
}
