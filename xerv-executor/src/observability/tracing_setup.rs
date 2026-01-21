//! Tracing subscriber setup with format selection and OpenTelemetry integration.

use anyhow::{Context, Result};
use tracing_subscriber::{
    EnvFilter,
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

use super::{TracingConfig, config::LogFormat};

#[cfg(feature = "otel")]
use super::otel;

/// Guard that handles cleanup when dropped.
///
/// Keeps the tracing infrastructure alive and handles OpenTelemetry shutdown.
pub struct TracingGuard {
    #[cfg(feature = "otel")]
    otel_enabled: bool,
}

impl Drop for TracingGuard {
    fn drop(&mut self) {
        #[cfg(feature = "otel")]
        if self.otel_enabled {
            otel::shutdown_otel();
        }
    }
}

/// Initialize tracing with the given configuration.
///
/// Returns a guard that must be kept alive for the duration of the program.
/// When dropped, it will flush and shutdown OpenTelemetry if enabled.
///
/// # Example
///
/// ```ignore
/// let _guard = init_tracing(TracingConfig::default())?;
/// // ... application code ...
/// // Guard is dropped here, flushing traces
/// ```
///
/// # OpenTelemetry
///
/// OpenTelemetry support requires the `otel` feature flag. Without it,
/// the `otel_enabled` config option is ignored and only logging is set up.
pub fn init_tracing(config: TracingConfig) -> Result<TracingGuard> {
    // Create env filter from config
    let filter = EnvFilter::try_new(config.log_filter()).unwrap_or_else(|_| EnvFilter::new("info"));

    // Log a warning if otel is requested but feature is not enabled
    #[cfg(not(feature = "otel"))]
    if config.otel_enabled() {
        eprintln!(
            "Warning: OpenTelemetry requested but 'otel' feature is not enabled. \
             Recompile with `--features otel` to enable distributed tracing."
        );
    }

    #[cfg(feature = "otel")]
    {
        if config.otel_enabled() {
            init_with_otel(filter, &config)?;
            return Ok(TracingGuard { otel_enabled: true });
        }
    }

    // Initialize without OpenTelemetry
    init_without_otel(filter, &config)?;

    Ok(TracingGuard {
        #[cfg(feature = "otel")]
        otel_enabled: false,
    })
}

/// Initialize tracing without OpenTelemetry.
fn init_without_otel(filter: EnvFilter, config: &TracingConfig) -> Result<()> {
    match config.log_format() {
        LogFormat::Json => {
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .json()
                        .with_file(config.include_location())
                        .with_line_number(config.include_location())
                        .with_target(config.include_target())
                        .with_thread_names(config.include_thread_names())
                        .with_thread_ids(config.include_thread_ids())
                        .with_span_events(FmtSpan::CLOSE)
                        .flatten_event(true),
                )
                .try_init()
                .context("Failed to initialize tracing subscriber")?;
        }
        LogFormat::Pretty => {
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .pretty()
                        .with_file(config.include_location())
                        .with_line_number(config.include_location())
                        .with_target(config.include_target())
                        .with_thread_names(config.include_thread_names())
                        .with_thread_ids(config.include_thread_ids()),
                )
                .try_init()
                .context("Failed to initialize tracing subscriber")?;
        }
        LogFormat::Compact => {
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .compact()
                        .with_file(config.include_location())
                        .with_line_number(config.include_location())
                        .with_target(config.include_target())
                        .with_thread_names(config.include_thread_names())
                        .with_thread_ids(config.include_thread_ids()),
                )
                .try_init()
                .context("Failed to initialize tracing subscriber")?;
        }
    }
    Ok(())
}

/// Initialize tracing with OpenTelemetry.
#[cfg(feature = "otel")]
fn init_with_otel(filter: EnvFilter, config: &TracingConfig) -> Result<()> {
    // We must create the otel layer inside each match arm because
    // the OpenTelemetryLayer<S> type parameter depends on the subscriber type,
    // which differs for each log format.
    match config.log_format() {
        LogFormat::Json => {
            let otel_layer =
                otel::init_otel_tracer(config).context("Failed to initialize OpenTelemetry")?;
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .json()
                        .with_file(config.include_location())
                        .with_line_number(config.include_location())
                        .with_target(config.include_target())
                        .with_thread_names(config.include_thread_names())
                        .with_thread_ids(config.include_thread_ids())
                        .with_span_events(FmtSpan::CLOSE)
                        .flatten_event(true),
                )
                .with(otel_layer)
                .try_init()
                .context("Failed to initialize tracing subscriber")?;
        }
        LogFormat::Pretty => {
            let otel_layer =
                otel::init_otel_tracer(config).context("Failed to initialize OpenTelemetry")?;
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .pretty()
                        .with_file(config.include_location())
                        .with_line_number(config.include_location())
                        .with_target(config.include_target())
                        .with_thread_names(config.include_thread_names())
                        .with_thread_ids(config.include_thread_ids()),
                )
                .with(otel_layer)
                .try_init()
                .context("Failed to initialize tracing subscriber")?;
        }
        LogFormat::Compact => {
            let otel_layer =
                otel::init_otel_tracer(config).context("Failed to initialize OpenTelemetry")?;
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .compact()
                        .with_file(config.include_location())
                        .with_line_number(config.include_location())
                        .with_target(config.include_target())
                        .with_thread_names(config.include_thread_names())
                        .with_thread_ids(config.include_thread_ids()),
                )
                .with(otel_layer)
                .try_init()
                .context("Failed to initialize tracing subscriber")?;
        }
    }
    Ok(())
}
