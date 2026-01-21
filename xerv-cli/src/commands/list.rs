//! List command - list running pipelines.

use anyhow::Context;
use xerv_client::Client;

/// Run the list command.
pub async fn run(show_all: bool, host: &str, port: u16) -> anyhow::Result<()> {
    tracing::info!(
        show_all = %show_all,
        host = %host,
        port = port,
        "Listing pipelines"
    );

    // Initialize the XERV client
    let base_url = format!("http://{}:{}", host, port);
    let client = Client::new(&base_url).context("Failed to create XERV client")?;

    // Fetch pipelines using the SDK
    let pipelines = client
        .list_pipelines()
        .await
        .context("Failed to list pipelines")?;

    // Display header
    println!();
    if pipelines.is_empty() {
        println!("No pipelines deployed");
        return Ok(());
    }

    let total_pipelines = pipelines.len();

    println!("Running Pipelines");
    println!("=================");
    println!();
    println!(
        "ID                                   STATE        TRIGGER_CT STARTED    COMPLETED  NODES"
    );
    println!(
        "--                                   -----        ---------- -------    ---------  -----"
    );

    // Display each pipeline
    for pipeline in pipelines {
        let state = format!("{:?}", pipeline.status).to_lowercase();

        // Filter by state if --all not specified
        if !show_all && (state == "stopped" || state == "paused") {
            continue;
        }

        println!(
            "{:<36} {:<12} {:<10} {:<10} {:<10} {}",
            pipeline.pipeline_id,
            state,
            pipeline.trigger_count,
            0, // TODO: started_at from timestamp
            0, // TODO: completed_at calculation
            pipeline.node_count
        );
    }

    println!();
    if !show_all {
        println!("Use --all to show stopped pipelines");
    }
    println!("Total: {} pipeline(s)", total_pipelines);

    Ok(())
}
