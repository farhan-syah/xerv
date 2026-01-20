//! List command - list running pipelines.

use anyhow::Result;

/// Run the list command.
pub async fn run(show_all: bool) -> Result<()> {
    tracing::info!(show_all = %show_all, "Listing pipelines");

    println!("Running Pipelines");
    println!("=================");
    println!();

    // In a real implementation, this would connect to the pipeline controller
    // and retrieve the list of active pipelines. For now, show a placeholder.

    if show_all {
        println!("NAME                    VERSION    STATE        TRACES    ERROR RATE");
        println!("----                    -------    -----        ------    ----------");
        println!("fraud-detection         1.0.0      running      1,234     0.1%");
        println!("order-processing        2.1.0      running      5,678     0.0%");
        println!("notification-service    1.2.0      paused       0         0.0%");
        println!("legacy-import           0.9.0      stopped      0         2.5%");
    } else {
        println!("NAME                    VERSION    STATE        TRACES    ERROR RATE");
        println!("----                    -------    -----        ------    ----------");
        println!("fraud-detection         1.0.0      running      1,234     0.1%");
        println!("order-processing        2.1.0      running      5,678     0.0%");
        println!();
        println!("Use --all to show stopped pipelines");
    }

    println!();
    println!("(placeholder data - connect to XERV server for real data)");

    Ok(())
}
