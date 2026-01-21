//! Trace inspection operations.

use crate::client::Client;
use crate::error::Result;
use crate::types::{TraceDetail, TraceId, TraceInfo, TraceStatus};
use chrono::{DateTime, Utc};
use serde::Deserialize;

/// Response from listing traces.
#[derive(Debug, Deserialize)]
struct ListTracesResponse {
    traces: Vec<TraceListItem>,
}

#[derive(Debug, Deserialize)]
struct TraceListItem {
    trace_id: TraceId,
    pipeline_id: String,
    status: String,
    started_at: String,
    completed_at: Option<String>,
}

/// Response from getting a single trace.
#[derive(Debug, Deserialize)]
struct GetTraceResponse {
    trace_id: TraceId,
    pipeline_id: String,
    status: String,
    started_at: String,
    completed_at: Option<String>,
    nodes_executed: usize,
    error: Option<String>,
}

impl Client {
    /// List all traces.
    ///
    /// # Returns
    ///
    /// Returns a vector of trace information.
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
    /// let traces = client.list_traces().await?;
    /// for trace in traces {
    ///     println!("{}: {:?}", trace.trace_id, trace.status);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_traces(&self) -> Result<Vec<TraceInfo>> {
        let response = self.get("traces").await?;
        let list_response: ListTracesResponse = self.handle_response(response).await?;

        Ok(list_response
            .traces
            .into_iter()
            .map(|t| TraceInfo {
                trace_id: t.trace_id,
                pipeline_id: t.pipeline_id,
                status: parse_trace_status(&t.status),
                started_at: parse_datetime(&t.started_at).unwrap_or_else(|_| Utc::now()),
                completed_at: t.completed_at.and_then(|ts| parse_datetime(&ts).ok()),
            })
            .collect())
    }

    /// List traces for a specific pipeline.
    ///
    /// # Arguments
    ///
    /// * `pipeline_id` - Pipeline identifier
    ///
    /// # Returns
    ///
    /// Returns a vector of trace information for the specified pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error if the pipeline is not found or the request fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use xerv_client::Client;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let client = Client::new("http://localhost:8080")?;
    /// let traces = client.list_traces_by_pipeline("my-pipeline").await?;
    /// println!("Found {} traces", traces.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_traces_by_pipeline(&self, pipeline_id: &str) -> Result<Vec<TraceInfo>> {
        let path = format!("pipelines/{}/traces", pipeline_id);
        let response = self.get(&path).await?;
        let list_response: ListTracesResponse = self.handle_response(response).await?;

        Ok(list_response
            .traces
            .into_iter()
            .map(|t| TraceInfo {
                trace_id: t.trace_id,
                pipeline_id: t.pipeline_id,
                status: parse_trace_status(&t.status),
                started_at: parse_datetime(&t.started_at).unwrap_or_else(|_| Utc::now()),
                completed_at: t.completed_at.and_then(|ts| parse_datetime(&ts).ok()),
            })
            .collect())
    }

    /// Get detailed information about a specific trace.
    ///
    /// # Arguments
    ///
    /// * `trace_id` - Trace identifier
    ///
    /// # Returns
    ///
    /// Returns detailed trace information.
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
    /// let trace = client.get_trace(trace_id).await?;
    /// println!("Trace status: {:?}", trace.status);
    /// println!("Nodes executed: {}", trace.nodes_executed);
    /// if let Some(error) = trace.error {
    ///     println!("Error: {}", error);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_trace(&self, trace_id: TraceId) -> Result<TraceDetail> {
        let path = format!("traces/{}", trace_id.as_uuid());
        let response = self.get(&path).await?;
        let get_response: GetTraceResponse = self.handle_response(response).await?;

        Ok(TraceDetail {
            trace_id: get_response.trace_id,
            pipeline_id: get_response.pipeline_id,
            status: parse_trace_status(&get_response.status),
            started_at: parse_datetime(&get_response.started_at).unwrap_or_else(|_| Utc::now()),
            completed_at: get_response
                .completed_at
                .and_then(|ts| parse_datetime(&ts).ok()),
            nodes_executed: get_response.nodes_executed,
            error: get_response.error,
        })
    }
}

/// Parse a string status into a TraceStatus enum.
fn parse_trace_status(status: &str) -> TraceStatus {
    match status.to_lowercase().as_str() {
        "queued" => TraceStatus::Queued,
        "running" => TraceStatus::Running,
        "completed" => TraceStatus::Completed,
        "failed" => TraceStatus::Failed,
        "suspended" => TraceStatus::Suspended,
        _ => TraceStatus::Queued, // Default fallback
    }
}

/// Parse an ISO 8601 datetime string to DateTime<Utc>.
fn parse_datetime(datetime_str: &str) -> std::result::Result<DateTime<Utc>, chrono::ParseError> {
    // Try parsing with RFC 3339 format (ISO 8601 with timezone)
    if let Ok(dt) = DateTime::parse_from_rfc3339(datetime_str) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Fallback for basic ISO 8601 format without timezone
    use chrono::NaiveDateTime;
    let naive = NaiveDateTime::parse_from_str(datetime_str, "%Y-%m-%dT%H:%M:%SZ")?;
    Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
}
