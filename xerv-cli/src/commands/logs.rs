//! Log streaming and query CLI commands.
//!
//! Provides commands for querying and streaming logs from a running XERV server.

use anyhow::{Context, Result};
use bytes::Buf;
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper::{Method, Request};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

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
    let base_url = format!("{}:{}", options.host, options.port);

    // Build query parameters
    let mut params = Vec::new();
    if let Some(l) = options.level {
        params.push(format!("level={}", l));
    }
    if let Some(tid) = options.trace_id {
        params.push(format!("trace_id={}", tid));
    }
    if let Some(pid) = options.pipeline_id {
        params.push(format!("pipeline_id={}", urlencoding::encode(pid)));
    }
    if let Some(nid) = options.node_id {
        params.push(format!("node_id={}", nid));
    }
    if let Some(c) = options.contains {
        params.push(format!("contains={}", urlencoding::encode(c)));
    }
    if let Some(lim) = options.limit {
        params.push(format!("limit={}", lim));
    }

    let query_string = if params.is_empty() {
        String::new()
    } else {
        format!("?{}", params.join("&"))
    };

    if options.follow {
        // Poll for logs continuously
        follow_logs(&base_url, &query_string).await
    } else {
        // Single query
        query_logs_once(&base_url, &query_string).await
    }
}

/// Query logs once and display.
async fn query_logs_once(base_url: &str, query_string: &str) -> Result<()> {
    let uri = format!("http://{}/api/v1/logs{}", base_url, query_string);
    let response = fetch_json(&uri).await?;

    let logs = response["logs"]
        .as_array()
        .context("Expected 'logs' array in response")?;

    if logs.is_empty() {
        println!("No logs found matching the filter.");
        return Ok(());
    }

    for log in logs {
        print_log_entry(log);
    }

    let count = response["count"].as_u64().unwrap_or(0);
    let filter = response["filter"].as_str().unwrap_or("all events");
    println!();
    println!("Showing {} logs (filter: {})", count, filter);

    Ok(())
}

/// Follow logs continuously.
async fn follow_logs(base_url: &str, query_string: &str) -> Result<()> {
    use std::collections::HashSet;
    use tokio::time::{Duration, sleep};

    let mut seen_ids: HashSet<u64> = HashSet::new();
    let recent_uri = format!("http://{}/api/v1/logs/recent?limit=50", base_url);

    println!("Following logs (Ctrl+C to stop)...");
    println!();

    // First, show recent logs
    if let Ok(response) = fetch_json(&recent_uri).await
        && let Some(logs) = response["logs"].as_array()
    {
        for log in logs {
            if let Some(id) = log["id"].as_u64() {
                seen_ids.insert(id);
            }
        }
    }

    loop {
        let uri = format!("http://{}/api/v1/logs{}", base_url, query_string);
        match fetch_json(&uri).await {
            Ok(response) => {
                if let Some(logs) = response["logs"].as_array() {
                    for log in logs {
                        if let Some(id) = log["id"].as_u64()
                            && !seen_ids.contains(&id)
                        {
                            seen_ids.insert(id);
                            print_log_entry(log);
                        }
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
    let uri = format!("http://{}:{}/api/v1/logs/by-trace/{}", host, port, trace_id);

    let response = fetch_json(&uri).await?;

    let logs = response["logs"]
        .as_array()
        .context("Expected 'logs' array in response")?;

    if logs.is_empty() {
        println!("No logs found for trace: {}", trace_id);
        return Ok(());
    }

    println!("Logs for trace: {}", trace_id);
    println!("{:-<80}", "");
    println!();

    for log in logs {
        print_log_entry(log);
    }

    let count = response["count"].as_u64().unwrap_or(0);
    println!();
    println!("Total: {} logs", count);

    Ok(())
}

/// Get log statistics.
pub async fn stats(host: &str, port: u16) -> Result<()> {
    let uri = format!("http://{}:{}/api/v1/logs/stats", host, port);
    let response = fetch_json(&uri).await?;

    println!("Log Statistics");
    println!("{:-<40}", "");

    let total = response["total"].as_u64().unwrap_or(0);
    let capacity = response["capacity"].as_u64().unwrap_or(0);

    println!("  Total Events: {}", total);
    println!("  Buffer Capacity: {}", capacity);
    if capacity > 0 {
        println!(
            "  Buffer Usage: {:.1}%",
            (total as f64 / capacity as f64) * 100.0
        );
    }
    println!();

    if let Some(by_level) = response["by_level"].as_object() {
        println!("  By Level:");
        let levels = ["trace", "debug", "info", "warn", "error"];
        for level in levels {
            let count = by_level.get(level).and_then(|v| v.as_u64()).unwrap_or(0);
            if count > 0 || total == 0 {
                let bar_len = if total > 0 {
                    ((count as f64 / total as f64) * 20.0) as usize
                } else {
                    0
                };
                let bar = "█".repeat(bar_len);
                println!("    {:>5}: {:>5} {}", level, count, bar);
            }
        }
    }

    Ok(())
}

/// Clear all logs on the server.
pub async fn clear(host: &str, port: u16) -> Result<()> {
    let uri: hyper::Uri = format!("http://{}:{}/api/v1/logs", host, port)
        .parse()
        .context("Invalid URI")?;

    let client: Client<_, Empty<Bytes>> = Client::builder(TokioExecutor::new()).build_http();

    let req = Request::builder()
        .method(Method::DELETE)
        .uri(uri)
        .body(Empty::new())
        .context("Failed to build request")?;

    let response = client
        .request(req)
        .await
        .context("Failed to connect to server")?;

    if response.status().is_success() {
        println!("Logs cleared successfully.");
    } else {
        let status = response.status();
        let body = response.collect().await?.to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        anyhow::bail!("Failed to clear logs: {} - {}", status, body_str);
    }

    Ok(())
}

/// Fetch JSON from a URL.
async fn fetch_json(uri: &str) -> Result<serde_json::Value> {
    let uri: hyper::Uri = uri.parse().context("Invalid URI")?;

    let client: Client<_, Empty<Bytes>> = Client::builder(TokioExecutor::new()).build_http();

    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Empty::new())
        .context("Failed to build request")?;

    let response = client
        .request(req)
        .await
        .context("Failed to connect to server")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.collect().await?.to_bytes();
        let body_str = String::from_utf8_lossy(&body);
        anyhow::bail!("Server returned error: {} - {}", status, body_str);
    }

    let body = response.collect().await?.aggregate();
    let json: serde_json::Value =
        serde_json::from_reader(body.reader()).context("Failed to parse JSON response")?;

    Ok(json)
}

/// Print a formatted log entry.
fn print_log_entry(log: &serde_json::Value) {
    let timestamp = log["timestamp"].as_str().unwrap_or("-");
    let level = log["level"].as_str().unwrap_or("?");
    let category = log["category"].as_str().unwrap_or("?");
    let message = log["message"].as_str().unwrap_or("");

    // Color based on level
    let level_colored = match level {
        "trace" => format!("\x1b[90m{}\x1b[0m", level.to_uppercase()),
        "debug" => format!("\x1b[36m{}\x1b[0m", level.to_uppercase()),
        "info" => format!("\x1b[32m{}\x1b[0m", level.to_uppercase()),
        "warn" => format!("\x1b[33m{}\x1b[0m", level.to_uppercase()),
        "error" => format!("\x1b[31m{}\x1b[0m", level.to_uppercase()),
        _ => level.to_uppercase(),
    };

    // Build context string
    let mut context_parts = Vec::new();
    if let Some(trace_id) = log["trace_id"].as_str() {
        context_parts.push(format!("trace={}", &trace_id[..8.min(trace_id.len())]));
    }
    if let Some(node_id) = log["node_id"].as_u64() {
        context_parts.push(format!("node={}", node_id));
    }
    if let Some(pipeline_id) = log["pipeline_id"].as_str() {
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
    if let Some(fields) = log["fields"].as_object()
        && !fields.is_empty()
    {
        let field_strs: Vec<String> = fields.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
        println!("         {{{}}}", field_strs.join(", "));
    }
}

/// URL encode a string.
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut result = String::with_capacity(s.len() * 3);
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                _ => {
                    result.push('%');
                    result.push_str(&format!("{:02X}", byte));
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_log_entry() {
        let log = serde_json::json!({
            "timestamp": "2024-01-15T10:30:00.000Z",
            "level": "info",
            "category": "node",
            "message": "Test message",
            "trace_id": "550e8400-e29b-41d4-a716-446655440000",
            "node_id": 42
        });

        // Just verify it doesn't panic
        print_log_entry(&log);
    }

    #[test]
    fn test_url_encoding() {
        assert_eq!(urlencoding::encode("hello"), "hello");
        assert_eq!(urlencoding::encode("hello world"), "hello%20world");
        assert_eq!(urlencoding::encode("a=b&c=d"), "a%3Db%26c%3Dd");
    }
}
