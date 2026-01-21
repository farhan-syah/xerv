//! XERV CLI - Command-line interface for XERV orchestration platform.

mod commands;

use anyhow::Result;
use clap::{Parser, Subcommand};
use xerv_executor::observability::{LogFormat, TracingConfig, init_tracing};

/// XERV - Zero-copy, event-driven orchestration platform.
#[derive(Parser)]
#[command(name = "xerv")]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Increase verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Deploy a flow from a YAML file
    Deploy {
        /// Path to the flow YAML file
        #[arg(short, long)]
        file: String,

        /// Run in dry-run mode (validate only)
        #[arg(long)]
        dry_run: bool,

        /// Server host
        #[arg(short = 'H', long, default_value = "localhost")]
        host: String,

        /// Server port
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },

    /// Run a flow in development mode with hot-reloading
    Dev {
        /// Path to the flow YAML file
        #[arg(short, long)]
        file: String,

        /// Port for the dev server
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },

    /// Inspect a trace arena file
    Inspect {
        /// Trace ID or path to arena file
        trace: String,

        /// Show detailed node outputs
        #[arg(short, long)]
        detailed: bool,
    },

    /// Validate a flow YAML file
    Validate {
        /// Path to the flow YAML file
        file: String,
    },

    /// Show version information
    Version,

    /// List running pipelines
    List {
        /// Show all pipelines (including stopped)
        #[arg(short, long)]
        all: bool,

        /// Server host
        #[arg(short = 'H', long, default_value = "localhost")]
        host: String,

        /// Server port
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },

    /// Start the XERV API server
    Serve {
        /// Host to bind to
        #[arg(short = 'H', long, default_value = "0.0.0.0")]
        host: String,

        /// Port to listen on
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },

    /// Schema management commands
    Schema {
        #[command(subcommand)]
        action: SchemaAction,
    },

    /// Query and stream logs from a running server
    Logs {
        #[command(subcommand)]
        action: LogsAction,
    },
}

#[derive(Subcommand)]
enum LogsAction {
    /// Query logs with optional filters
    Query {
        /// Server host
        #[arg(short = 'H', long, default_value = "localhost")]
        host: String,

        /// Server port
        #[arg(short, long, default_value = "8080")]
        port: u16,

        /// Minimum log level (trace, debug, info, warn, error)
        #[arg(short, long)]
        level: Option<String>,

        /// Filter by trace ID
        #[arg(short, long)]
        trace_id: Option<String>,

        /// Filter by pipeline ID
        #[arg(long)]
        pipeline_id: Option<String>,

        /// Filter by node ID
        #[arg(short, long)]
        node_id: Option<u32>,

        /// Filter by message content (case-insensitive)
        #[arg(short, long)]
        contains: Option<String>,

        /// Maximum number of logs to show
        #[arg(long, default_value = "100")]
        limit: usize,

        /// Follow logs in real-time
        #[arg(short, long)]
        follow: bool,
    },

    /// Get logs for a specific trace
    Trace {
        /// Trace ID to query
        trace_id: String,

        /// Server host
        #[arg(short = 'H', long, default_value = "localhost")]
        host: String,

        /// Server port
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },

    /// Show log statistics
    Stats {
        /// Server host
        #[arg(short = 'H', long, default_value = "localhost")]
        host: String,

        /// Server port
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },

    /// Clear all logs
    Clear {
        /// Server host
        #[arg(short = 'H', long, default_value = "localhost")]
        host: String,

        /// Server port
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },
}

#[derive(Subcommand)]
enum SchemaAction {
    /// List all registered schemas
    List {
        /// Path to schema registry file
        #[arg(short, long)]
        registry: Option<String>,
    },

    /// Show details of a schema
    Show {
        /// Schema name (e.g., "Order" or "Order@v1")
        name: String,

        /// Path to schema registry file
        #[arg(short, long)]
        registry: Option<String>,
    },

    /// Check compatibility between two schemas
    Check {
        /// Source schema (e.g., "Order@v1")
        from: String,

        /// Target schema (e.g., "Order@v2")
        to: String,

        /// Path to schema registry file
        #[arg(short, long)]
        registry: Option<String>,
    },

    /// Validate migration path between schemas
    ValidateMigration {
        /// Source schema (e.g., "Order@v1")
        from: String,

        /// Target schema (e.g., "Order@v2")
        to: String,

        /// Path to schema registry file
        #[arg(short, long)]
        registry: Option<String>,
    },

    /// Export schema registry to file
    Export {
        /// Output file path
        path: String,

        /// Path to source schema registry file
        #[arg(short, long)]
        registry: Option<String>,
    },

    /// Import schemas from file
    Import {
        /// Input file path
        path: String,
    },

    /// Parse and display version information
    Version {
        /// Version string to parse (e.g., "v1.2")
        version: String,
    },
}

fn setup_logging(verbosity: u8) -> Result<xerv_executor::observability::TracingGuard> {
    let filter = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    // Check for explicit log format override, otherwise auto-detect
    let log_format = std::env::var("XERV_LOG_FORMAT")
        .ok()
        .and_then(|s| s.parse::<LogFormat>().ok())
        .unwrap_or_else(|| {
            if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                LogFormat::Pretty
            } else {
                LogFormat::Compact
            }
        });

    // Build config, respecting RUST_LOG if set
    let log_filter = std::env::var("RUST_LOG").unwrap_or_else(|_| filter.to_string());

    let config = TracingConfig::builder()
        .log_format(log_format)
        .log_filter(log_filter)
        .build();

    init_tracing(config)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let _tracing_guard = setup_logging(cli.verbose)?;

    match cli.command {
        Commands::Deploy {
            file,
            dry_run,
            host,
            port,
        } => commands::deploy::run(&file, dry_run, &host, port).await,
        Commands::Dev { file, port } => commands::dev::run(&file, port).await,
        Commands::Inspect { trace, detailed } => commands::inspect::run(&trace, detailed).await,
        Commands::Validate { file } => commands::validate::run(&file).await,
        Commands::Version => commands::version::run(),
        Commands::List { all, host, port } => commands::list::run(all, &host, port).await,
        Commands::Serve { host, port } => commands::serve::run(&host, port).await,
        Commands::Schema { action } => match action {
            SchemaAction::List { registry } => commands::schema::list(registry.as_deref()).await,
            SchemaAction::Show { name, registry } => {
                commands::schema::show(&name, registry.as_deref()).await
            }
            SchemaAction::Check { from, to, registry } => {
                commands::schema::check(&from, &to, registry.as_deref()).await
            }
            SchemaAction::ValidateMigration { from, to, registry } => {
                commands::schema::validate_migration(&from, &to, registry.as_deref()).await
            }
            SchemaAction::Export { path, registry } => {
                commands::schema::export(&path, registry.as_deref()).await
            }
            SchemaAction::Import { path } => commands::schema::import(&path).await,
            SchemaAction::Version { version } => commands::schema::version_info(&version).await,
        },
        Commands::Logs { action } => match action {
            LogsAction::Query {
                host,
                port,
                level,
                trace_id,
                pipeline_id,
                node_id,
                contains,
                limit,
                follow,
            } => {
                let mut options = commands::logs::LogQueryOptions::new(&host, port);
                if let Some(ref l) = level {
                    options.level = Some(l);
                }
                if let Some(ref tid) = trace_id {
                    options.trace_id = Some(tid);
                }
                if let Some(ref pid) = pipeline_id {
                    options.pipeline_id = Some(pid);
                }
                if let Some(nid) = node_id {
                    options.node_id = Some(nid);
                }
                if let Some(ref c) = contains {
                    options.contains = Some(c);
                }
                options.limit = Some(limit);
                options.follow = follow;
                commands::logs::query(options).await
            }
            LogsAction::Trace {
                trace_id,
                host,
                port,
            } => commands::logs::by_trace(&host, port, &trace_id).await,
            LogsAction::Stats { host, port } => commands::logs::stats(&host, port).await,
            LogsAction::Clear { host, port } => commands::logs::clear(&host, port).await,
        },
    }
}
