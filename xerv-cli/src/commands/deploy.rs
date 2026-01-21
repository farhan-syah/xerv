//! Deploy command - deploy a flow from a YAML file.

use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::fs;
use std::path::Path;
use xerv_executor::loader::{FlowLoader, LoaderError};

/// Run the deploy command.
pub async fn run(file: &str, dry_run: bool, host: &str, port: u16) -> Result<()> {
    let path = Path::new(file);

    if !path.exists() {
        anyhow::bail!("Flow file not found: {}", file);
    }

    tracing::info!(
        file = %file,
        dry_run = %dry_run,
        host = %host,
        port = port,
        "Deploying flow"
    );

    // Read the YAML file
    let yaml_content =
        fs::read_to_string(path).context(format!("Failed to read flow file: {}", file))?;

    // Load and validate the flow using the typed FlowLoader
    let loaded = FlowLoader::from_yaml(&yaml_content).map_err(|e| match e {
        LoaderError::Io { path, source } => {
            anyhow::anyhow!("Failed to read flow file '{}': {}", path.display(), source)
        }
        LoaderError::Parse { path, source } => {
            let path_str = path.map(|p| p.display().to_string()).unwrap_or_default();
            anyhow::anyhow!("Failed to parse YAML '{}': {}", path_str, source)
        }
        LoaderError::Validation { errors } => {
            let mut msg = String::from("Flow validation failed:\n");
            for error in errors {
                msg.push_str(&format!("  - {}\n", error));
            }
            anyhow::anyhow!(msg)
        }
        LoaderError::Build { source } => {
            anyhow::anyhow!("Failed to build flow graph: {}", source)
        }
    })?;

    // Print flow info
    println!();
    println!("Flow: {} v{}", loaded.name(), loaded.version());
    if let Some(ref desc) = loaded.definition.description {
        println!("Description: {}", desc);
    }
    println!();

    // Print flow structure
    println!("Triggers: {}", loaded.definition.triggers.len());
    for trigger in &loaded.definition.triggers {
        let status = if trigger.enabled { "✓" } else { "○" };
        println!("  {} {} ({})", status, trigger.id, trigger.trigger_type);
    }

    println!("\nNodes: {}", loaded.definition.nodes.len());
    for (id, node) in &loaded.definition.nodes {
        let status = if node.enabled { "✓" } else { "○" };
        println!("  {} {} ({})", status, id, node.node_type);
    }

    println!("\nEdges: {}", loaded.definition.edges.len());
    for edge in &loaded.definition.edges {
        let arrow = if edge.loop_back { "↺" } else { "→" };
        println!("  {} {} {}", edge.from, arrow, edge.to);
    }

    // Print execution order
    println!("\nExecution Order:");
    match loaded.execution_order() {
        Ok(order) => {
            for (i, node_id) in order.iter().enumerate() {
                // Find the name for this node ID
                let name = loaded
                    .metadata
                    .node_ids
                    .iter()
                    .find(|(_, id)| **id == *node_id)
                    .map(|(name, _)| name.as_str())
                    .unwrap_or("unknown");
                println!("  {}. {}", i + 1, name);
            }
        }
        Err(e) => {
            println!("  Error computing execution order: {}", e);
        }
    }

    // Print settings
    println!("\nSettings:");
    println!(
        "  Max concurrent executions: {}",
        loaded.settings.max_concurrent_executions
    );
    println!(
        "  Execution timeout: {}ms",
        loaded.settings.execution_timeout_ms
    );
    println!();

    if dry_run {
        println!("✓ Dry run completed successfully. Flow is valid.");
        return Ok(());
    }

    // Deploy to server
    println!("Deploying to {}:{}...", host, port);

    let url = format!("http://{}:{}/api/v1/pipelines", host, port);

    // Create HTTP client
    let client = Client::builder(TokioExecutor::new()).build_http();

    // Build the request
    let req = Request::builder()
        .method("POST")
        .uri(&url)
        .header("content-type", "application/x-yaml")
        .header("accept", "application/json")
        .body(Full::new(Bytes::from(yaml_content)))
        .context("Failed to build HTTP request")?;

    // Send the request
    let resp = client
        .request(req)
        .await
        .context(format!("Failed to connect to XERV server at {}", url))?;

    // Read the response body
    let status = resp.status();
    let body_bytes = resp
        .into_body()
        .collect()
        .await
        .context("Failed to read response body")?
        .to_bytes();

    // Check status
    if !status.is_success() {
        // Try to parse error from JSON
        if let Ok(error_data) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
            let error_msg = error_data["error"].as_str().unwrap_or("Unknown error");
            anyhow::bail!("Deployment failed ({}): {}", status.as_u16(), error_msg);
        } else {
            anyhow::bail!(
                "Deployment failed ({}): {}",
                status.as_u16(),
                String::from_utf8_lossy(&body_bytes)
            );
        }
    }

    // Parse successful response
    let data: serde_json::Value =
        serde_json::from_slice(&body_bytes).context("Failed to parse JSON response")?;

    let pipeline_id = data["pipeline_id"].as_str().unwrap_or("unknown");
    let deployment_status = data["status"].as_str().unwrap_or("unknown");

    println!("✓ Flow deployed successfully");
    println!("  Pipeline ID: {}", pipeline_id);
    println!("  Status: {}", deployment_status);

    Ok(())
}
