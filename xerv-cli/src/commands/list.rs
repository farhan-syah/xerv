//! List command - list running pipelines.

use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

/// Run the list command.
pub async fn run(show_all: bool, host: &str, port: u16) -> Result<()> {
    tracing::info!(
        show_all = %show_all,
        host = %host,
        port = port,
        "Listing pipelines"
    );

    // Build the API URL
    let url = format!("http://{}:{}/api/v1/pipelines", host, port);

    // Create HTTP client
    let client = Client::builder(TokioExecutor::new()).build_http();

    // Build the request
    let req = Request::builder()
        .method("GET")
        .uri(&url)
        .header("accept", "application/json")
        .body(Empty::<Bytes>::new())
        .context("Failed to build HTTP request")?;

    // Send the request
    let resp = client
        .request(req)
        .await
        .context(format!("Failed to connect to XERV server at {}", url))?;

    // Check status
    if !resp.status().is_success() {
        anyhow::bail!(
            "Server returned error status: {} {}",
            resp.status().as_u16(),
            resp.status().canonical_reason().unwrap_or("Unknown")
        );
    }

    // Read the response body
    let body_bytes = resp
        .into_body()
        .collect()
        .await
        .context("Failed to read response body")?
        .to_bytes();

    // Parse JSON
    let data: serde_json::Value =
        serde_json::from_slice(&body_bytes).context("Failed to parse JSON response")?;

    // Extract pipelines array
    let pipelines = data["pipelines"]
        .as_array()
        .context("Response missing 'pipelines' array")?;

    // Display header
    println!();
    if pipelines.is_empty() {
        println!("No pipelines deployed");
        return Ok(());
    }

    println!("Running Pipelines");
    println!("=================");
    println!();
    println!(
        "ID                                   STATE        STARTED    COMPLETED  FAILED     ACTIVE"
    );
    println!(
        "--                                   -----        -------    ---------  ------     ------"
    );

    // Display each pipeline
    for pipeline in pipelines {
        let id = pipeline["id"].as_str().unwrap_or("unknown");
        let state = pipeline["state"].as_str().unwrap_or("unknown");
        let metrics = &pipeline["metrics"];

        let traces_started = metrics["traces_started"].as_u64().unwrap_or(0);
        let traces_completed = metrics["traces_completed"].as_u64().unwrap_or(0);
        let traces_failed = metrics["traces_failed"].as_u64().unwrap_or(0);
        let traces_active = metrics["traces_active"].as_u64().unwrap_or(0);

        // Filter by state if --all not specified
        if !show_all && (state == "stopped" || state == "paused") {
            continue;
        }

        println!(
            "{:<36} {:<12} {:<10} {:<10} {:<10} {}",
            id, state, traces_started, traces_completed, traces_failed, traces_active
        );
    }

    println!();
    if !show_all {
        println!("Use --all to show stopped pipelines");
    }
    println!("Total: {} pipeline(s)", pipelines.len());

    Ok(())
}
