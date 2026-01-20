//! Dev command - run a flow in development mode with hot-reloading.

use anyhow::Result;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use xerv_core::types::TraceId;
use xerv_executor::loader::{FlowLoader, LoadedFlow, LoaderError};

/// Maximum number of recent traces to keep in memory.
const MAX_RECENT_TRACES: usize = 100;

/// Shared server state.
struct DevServerState {
    /// The currently loaded flow.
    flow: Option<LoadedFlow>,
    /// Path to the flow file.
    flow_path: String,
    /// Recent trace executions.
    recent_traces: VecDeque<TraceInfo>,
    /// Server start time.
    start_time: Instant,
    /// Number of total triggers.
    trigger_count: u64,
    /// Last reload time.
    last_reload: Option<Instant>,
    /// Last reload error.
    last_error: Option<String>,
}

/// Information about a triggered trace.
#[derive(Clone)]
struct TraceInfo {
    /// The trace ID.
    trace_id: TraceId,
    /// When the trace was triggered.
    triggered_at: Instant,
    /// Status of the trace.
    status: TraceStatus,
}

/// Status of a trace.
#[derive(Clone, Copy)]
enum TraceStatus {
    Pending,
    #[allow(dead_code)]
    Running,
    Completed,
    #[allow(dead_code)]
    Failed,
}

impl TraceStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// Run the dev command.
pub async fn run(file: &str, port: u16) -> Result<()> {
    let path = Path::new(file);

    if !path.exists() {
        anyhow::bail!("Flow file not found: {}", file);
    }

    tracing::info!(file = %file, port = %port, "Starting development server");

    // Load the flow initially
    let loaded = load_flow(path)?;

    println!("Starting XERV development server...");
    println!();
    print_flow_info(&loaded);

    // Create shared state
    let state = Arc::new(RwLock::new(DevServerState {
        flow: Some(loaded),
        flow_path: file.to_string(),
        recent_traces: VecDeque::with_capacity(MAX_RECENT_TRACES),
        start_time: Instant::now(),
        trigger_count: 0,
        last_reload: None,
        last_error: None,
    }));

    // Set up file watcher
    let (watch_tx, mut watch_rx) = mpsc::channel::<()>(1);
    let watch_state = Arc::clone(&state);
    let watch_path = path.to_path_buf();

    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res
                && event.kind.is_modify()
            {
                let _ = watch_tx.blocking_send(());
            }
        })?;

    watcher.watch(path, RecursiveMode::NonRecursive)?;

    // Spawn file watcher handler
    tokio::spawn(async move {
        while watch_rx.recv().await.is_some() {
            // Debounce: wait a bit for multiple rapid changes
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            // Drain any additional events
            while watch_rx.try_recv().is_ok() {}

            // Reload the flow
            let mut state = watch_state.write();
            println!();
            println!("File changed, reloading...");

            match load_flow(&watch_path) {
                Ok(loaded) => {
                    print_flow_info(&loaded);
                    state.flow = Some(loaded);
                    state.last_reload = Some(Instant::now());
                    state.last_error = None;
                    println!("Hot-reload successful.");
                }
                Err(e) => {
                    state.last_error = Some(e.to_string());
                    eprintln!("Reload failed: {}", e);
                    println!("Keeping previous configuration.");
                }
            }
            println!();
        }
    });

    // Start HTTP server
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;

    println!();
    println!("Port: {}", port);
    println!();
    println!("Endpoints:");
    println!(
        "  POST http://localhost:{}/trigger    - Trigger flow execution",
        port
    );
    println!(
        "  GET  http://localhost:{}/status     - Get server status",
        port
    );
    println!(
        "  GET  http://localhost:{}/traces     - List recent traces",
        port
    );
    println!("  GET  http://localhost:{}/health     - Health check", port);
    println!();
    println!("Watching: {}", file);
    println!("Press Ctrl+C to stop.");
    println!();

    // Accept connections
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let state = Arc::clone(&state);

        tokio::spawn(async move {
            let service = service_fn(move |req| {
                let state = Arc::clone(&state);
                async move { handle_request(req, state).await }
            });

            if let Err(e) = http1::Builder::new().serve_connection(io, service).await
                && !e.is_incomplete_message()
            {
                tracing::warn!("HTTP connection error: {}", e);
            }
        });
    }
}

/// Load a flow from a file path.
fn load_flow(path: &Path) -> Result<LoadedFlow> {
    FlowLoader::from_file(path).map_err(|e| match e {
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
    })
}

/// Print flow information to stdout.
fn print_flow_info(loaded: &LoadedFlow) {
    println!("Flow: {} v{}", loaded.name(), loaded.version());
    if let Some(ref desc) = loaded.definition.description {
        println!("Description: {}", desc);
    }
    println!();

    println!("Loaded Configuration:");
    println!("  Triggers: {}", loaded.definition.triggers.len());
    for trigger in &loaded.definition.triggers {
        if trigger.enabled {
            println!("    - {} ({})", trigger.id, trigger.trigger_type);
        }
    }
    println!("  Nodes: {}", loaded.definition.nodes.len());
    println!("  Edges: {}", loaded.definition.edges.len());
    println!();

    println!("Runtime Settings:");
    println!(
        "  Max concurrent executions: {}",
        loaded.settings.max_concurrent_executions
    );
    println!(
        "  Execution timeout: {}ms",
        loaded.settings.execution_timeout_ms
    );
    if loaded.settings.debug {
        println!("  Debug mode: enabled");
    }
}

/// Handle an HTTP request.
async fn handle_request(
    req: Request<Incoming>,
    state: Arc<RwLock<DevServerState>>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let path = req.uri().path();
    let method = req.method();

    let response = match (method, path) {
        (&Method::GET, "/health") => handle_health(),
        (&Method::GET, "/status") => handle_status(&state),
        (&Method::GET, "/traces") => handle_traces(&state),
        (&Method::POST, "/trigger") => handle_trigger(&state),
        _ => {
            let body = serde_json::json!({
                "error": "Not Found",
                "message": format!("No route for {} {}", method, path),
                "endpoints": [
                    "GET /health - Health check",
                    "GET /status - Server status",
                    "GET /traces - Recent traces",
                    "POST /trigger - Trigger flow execution"
                ]
            });
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(body.to_string())))
                .unwrap()
        }
    };

    Ok(response)
}

/// Handle GET /health.
fn handle_health() -> Response<Full<Bytes>> {
    let body = serde_json::json!({
        "status": "healthy",
        "service": "xerv-dev"
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}

/// Handle GET /status.
fn handle_status(state: &Arc<RwLock<DevServerState>>) -> Response<Full<Bytes>> {
    let state = state.read();

    let flow_info = state.flow.as_ref().map(|f| {
        serde_json::json!({
            "name": f.name(),
            "version": f.version(),
            "triggers": f.definition.triggers.len(),
            "nodes": f.definition.nodes.len(),
            "edges": f.definition.edges.len()
        })
    });

    let uptime_secs = state.start_time.elapsed().as_secs();
    let last_reload_secs = state.last_reload.map(|t| t.elapsed().as_secs());

    let body = serde_json::json!({
        "status": if state.flow.is_some() { "ready" } else { "error" },
        "flow_path": state.flow_path,
        "flow": flow_info,
        "uptime_seconds": uptime_secs,
        "trigger_count": state.trigger_count,
        "recent_traces": state.recent_traces.len(),
        "last_reload_seconds_ago": last_reload_secs,
        "last_error": state.last_error
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}

/// Handle GET /traces.
fn handle_traces(state: &Arc<RwLock<DevServerState>>) -> Response<Full<Bytes>> {
    let state = state.read();

    let traces: Vec<_> = state
        .recent_traces
        .iter()
        .map(|t| {
            serde_json::json!({
                "trace_id": t.trace_id.as_uuid().to_string(),
                "elapsed_ms": t.triggered_at.elapsed().as_millis(),
                "status": t.status.as_str()
            })
        })
        .collect();

    let body = serde_json::json!({
        "count": traces.len(),
        "traces": traces
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}

/// Handle POST /trigger.
fn handle_trigger(state: &Arc<RwLock<DevServerState>>) -> Response<Full<Bytes>> {
    let mut state = state.write();

    if state.flow.is_none() {
        let body = serde_json::json!({
            "error": "No flow loaded",
            "message": "The flow failed to load. Check the logs for details."
        });

        return Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap();
    }

    // Extract flow info before any mutable operations
    let flow = state.flow.as_ref().unwrap();
    let flow_name = flow.name().to_string();
    let flow_version = flow.version().to_string();

    let trace_id = TraceId::new();
    state.trigger_count += 1;

    // Add to recent traces
    if state.recent_traces.len() >= MAX_RECENT_TRACES {
        state.recent_traces.pop_front();
    }
    state.recent_traces.push_back(TraceInfo {
        trace_id,
        triggered_at: Instant::now(),
        status: TraceStatus::Pending,
    });

    println!(
        "[{}] Triggered flow '{}' - trace {}",
        chrono_timestamp(),
        flow_name,
        trace_id.as_uuid()
    );

    // In a full implementation, this would:
    // 1. Create an arena for the trace
    // 2. Initialize the trace execution context
    // 3. Execute nodes according to the flow graph
    // 4. Update trace status in recent_traces
    //
    // For now, we simulate by marking it as completed
    if let Some(trace) = state
        .recent_traces
        .iter_mut()
        .find(|t| t.trace_id == trace_id)
    {
        trace.status = TraceStatus::Completed;
    }

    let body = serde_json::json!({
        "trace_id": trace_id.as_uuid().to_string(),
        "flow": flow_name,
        "version": flow_version,
        "status": "triggered",
        "message": "Flow execution triggered successfully"
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}

/// Get current timestamp for logging.
fn chrono_timestamp() -> String {
    use std::time::SystemTime;

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs();
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    let seconds = secs % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
