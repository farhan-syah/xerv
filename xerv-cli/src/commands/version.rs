//! Version command - show version information.

use anyhow::Result;

/// Version information.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Run the version command.
pub fn run() -> Result<()> {
    println!("XERV - Zero-copy, Event-driven Orchestration Platform");
    println!();
    println!("Version:     {}", VERSION);
    println!(
        "Platform:    {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!();
    println!("Components:");
    println!("  xerv-core      Core types, arena, WAL, traits");
    println!("  xerv-executor  Scheduler, linker, pipeline controller");
    println!("  xerv-nodes     Standard library nodes");
    println!("  xerv-macros    Procedural macros");
    println!("  xerv-cli       Command-line interface");
    println!();
    println!("Repository: https://github.com/ml-rust/xerv");

    Ok(())
}
