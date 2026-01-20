//! Serve command - run the XERV API server.

use anyhow::Result;
use std::sync::Arc;
use xerv_executor::api::{ApiServer, ServerConfig};
use xerv_executor::listener::ListenerPool;
use xerv_executor::pipeline::PipelineController;

/// Run the serve command.
pub async fn run(host: &str, port: u16) -> Result<()> {
    tracing::info!(host = %host, port = %port, "Starting XERV API server");

    // Create the pipeline controller and listener pool
    let controller = Arc::new(PipelineController::new());
    let listener_pool = Arc::new(ListenerPool::new());

    // Create server configuration
    let config = ServerConfig::new(host, port);

    // Create and run the server
    let mut server = ApiServer::new(config, controller, listener_pool);

    println!("Starting XERV API server...");
    println!();
    println!("Server: http://{}:{}", host, port);
    println!();
    println!("Endpoints:");
    println!(
        "  GET  http://{}:{}/api/v1/health     - Health check",
        host, port
    );
    println!(
        "  GET  http://{}:{}/api/v1/status     - System status",
        host, port
    );
    println!(
        "  GET  http://{}:{}/api/v1/pipelines  - List pipelines",
        host, port
    );
    println!(
        "  POST http://{}:{}/api/v1/pipelines  - Deploy pipeline (YAML body)",
        host, port
    );
    println!(
        "  GET  http://{}:{}/api/v1/traces     - List traces",
        host, port
    );
    println!();
    println!("Press Ctrl+C to stop.");
    println!();

    // Handle graceful shutdown on Ctrl+C
    let server_handle = tokio::spawn(async move { server.run().await });

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    tracing::info!("Shutdown signal received");
    println!();
    println!("Shutting down...");

    // The server will exit when the task is dropped
    server_handle.abort();

    Ok(())
}
