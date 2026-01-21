//! OpenTelemetry integration for distributed tracing.

use anyhow::{Context, Result};
use opentelemetry::{KeyValue, trace::TracerProvider};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    Resource, runtime,
    trace::{RandomIdGenerator, Sampler, TracerProvider as SdkTracerProvider},
};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::LookupSpan;

use super::TracingConfig;

/// Initialize an OpenTelemetry tracer.
///
/// Returns a layer that can be added to a tracing subscriber.
pub fn init_otel_tracer<S>(
    config: &TracingConfig,
) -> Result<OpenTelemetryLayer<S, opentelemetry_sdk::trace::Tracer>>
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
{
    let endpoint = config
        .otel_endpoint()
        .unwrap_or_else(|| "http://localhost:4317".to_string());

    // Create OTLP exporter
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&endpoint)
        .build()
        .context("Failed to create OTLP exporter")?;

    // Create resource with service information
    let resource = Resource::new(vec![
        KeyValue::new("service.name", config.service_name().to_string()),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
    ]);

    // Build tracer provider with batching for efficiency
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter, runtime::Tokio)
        .with_sampler(Sampler::AlwaysOn)
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource)
        .build();

    let tracer = provider.tracer(config.service_name().to_string());

    // Register the global tracer provider
    opentelemetry::global::set_tracer_provider(provider);

    Ok(tracing_opentelemetry::layer().with_tracer(tracer))
}

/// Shutdown OpenTelemetry, flushing any pending spans.
pub fn shutdown_otel() {
    opentelemetry::global::shutdown_tracer_provider();
}
