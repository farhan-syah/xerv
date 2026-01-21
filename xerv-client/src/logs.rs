//! Log query operations.

use crate::client::Client;
use crate::error::Result;
use crate::types::{LogEntry, TraceId};
use serde::Deserialize;

/// Response from querying logs.
///
/// Note: The server also sends a `count` field, but we ignore it since
/// it's redundant with `logs.len()`. Serde automatically skips unknown fields.
#[derive(Debug, Deserialize)]
struct LogsResponse {
    logs: Vec<LogEntry>,
}

/// Response from log statistics.
#[derive(Debug, Deserialize)]
pub struct LogStats {
    /// Total number of log entries.
    pub total: usize,
    /// Buffer capacity.
    pub capacity: usize,
    /// Breakdown by log level.
    pub by_level: std::collections::HashMap<String, usize>,
}

impl Client {
    /// Query logs with optional filters.
    ///
    /// # Arguments
    ///
    /// * `level` - Optional log level filter (trace, debug, info, warn, error)
    /// * `trace_id` - Optional trace ID filter
    /// * `pipeline_id` - Optional pipeline ID filter
    /// * `contains` - Optional message content filter
    /// * `limit` - Maximum number of logs to return
    ///
    /// # Returns
    ///
    /// Returns a vector of log entries matching the filters.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use xerv_client::Client;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("http://localhost:8080")?;
    /// // Get all error logs
    /// let logs = client.query_logs(Some("error"), None, None, None, Some(100)).await?;
    /// for log in logs {
    ///     println!("{}: {}", log.timestamp, log.message);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn query_logs(
        &self,
        level: Option<&str>,
        trace_id: Option<TraceId>,
        pipeline_id: Option<&str>,
        contains: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<LogEntry>> {
        let mut query_params = Vec::new();

        if let Some(l) = level {
            query_params.push(format!("level={}", l));
        }
        if let Some(tid) = trace_id {
            query_params.push(format!("trace_id={}", tid.as_uuid()));
        }
        if let Some(pid) = pipeline_id {
            query_params.push(format!("pipeline_id={}", urlencoding::encode(pid)));
        }
        if let Some(c) = contains {
            query_params.push(format!("contains={}", urlencoding::encode(c)));
        }
        if let Some(lim) = limit {
            query_params.push(format!("limit={}", lim));
        }

        let path = if query_params.is_empty() {
            "logs".to_string()
        } else {
            format!("logs?{}", query_params.join("&"))
        };

        let response = self.get(&path).await?;
        let logs_response: LogsResponse = self.handle_response(response).await?;

        Ok(logs_response.logs)
    }

    /// Get recent logs.
    ///
    /// # Arguments
    ///
    /// * `limit` - Maximum number of logs to return (default: 50)
    ///
    /// # Returns
    ///
    /// Returns the most recent log entries.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use xerv_client::Client;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("http://localhost:8080")?;
    /// let recent_logs = client.get_recent_logs(20).await?;
    /// for log in recent_logs {
    ///     println!("{}: {}", log.level, log.message);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_recent_logs(&self, limit: usize) -> Result<Vec<LogEntry>> {
        let path = format!("logs/recent?limit={}", limit);
        let response = self.get(&path).await?;
        let logs_response: LogsResponse = self.handle_response(response).await?;

        Ok(logs_response.logs)
    }

    /// Get logs for a specific trace.
    ///
    /// # Arguments
    ///
    /// * `trace_id` - Trace identifier
    ///
    /// # Returns
    ///
    /// Returns all log entries for the specified trace.
    ///
    /// # Errors
    ///
    /// Returns an error if the trace is not found or the request fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use xerv_client::{Client, TraceId};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("http://localhost:8080")?;
    /// # let trace_id = TraceId::from_uuid(uuid::Uuid::new_v4());
    /// let logs = client.get_trace_logs(trace_id).await?;
    /// println!("Found {} log entries for trace", logs.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_trace_logs(&self, trace_id: TraceId) -> Result<Vec<LogEntry>> {
        let path = format!("logs/by-trace/{}", trace_id.as_uuid());
        let response = self.get(&path).await?;
        let logs_response: LogsResponse = self.handle_response(response).await?;

        Ok(logs_response.logs)
    }

    /// Get logs filtered by level.
    ///
    /// # Arguments
    ///
    /// * `level` - Log level (trace, debug, info, warn, error)
    /// * `limit` - Optional maximum number of logs to return
    ///
    /// # Returns
    ///
    /// Returns log entries at the specified level.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn get_logs_by_level(
        &self,
        level: &str,
        limit: Option<usize>,
    ) -> Result<Vec<LogEntry>> {
        let path = if let Some(lim) = limit {
            format!("logs/by-level/{}?limit={}", level, lim)
        } else {
            format!("logs/by-level/{}", level)
        };

        let response = self.get(&path).await?;
        let logs_response: LogsResponse = self.handle_response(response).await?;

        Ok(logs_response.logs)
    }

    /// Get log statistics.
    ///
    /// # Returns
    ///
    /// Returns statistics about logs in the buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use xerv_client::Client;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("http://localhost:8080")?;
    /// let stats = client.get_log_stats().await?;
    /// println!("Total logs: {} / {}", stats.total, stats.capacity);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_log_stats(&self) -> Result<LogStats> {
        let response = self.get("logs/stats").await?;
        self.handle_response(response).await
    }

    /// Clear all logs.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or authentication is insufficient.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use xerv_client::Client;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("http://localhost:8080")?;
    /// client.clear_logs().await?;
    /// println!("All logs cleared");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn clear_logs(&self) -> Result<()> {
        let response = self.delete("logs").await?;
        self.handle_empty_response(response).await
    }
}
