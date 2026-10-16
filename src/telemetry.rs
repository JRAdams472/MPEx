//! Optional OTLP exporters on top of the always-on stdout JSON log:
//!
//! - `OTEL_EXPORTER_OTLP_ENDPOINT` — trace spans shipped over OTLP/gRPC
//!   (compose points this at Jaeger, `http://jaeger:4317`).
//! - `SEQ_URL` — `tracing` events shipped as OTLP logs (Seq ingest,
//!   `http://seq:5341`); `SEQ_API_KEY` adds an `x-seq-apikey` header.
//!
//! Both off by default — with no env vars the subscriber is identical to the
//! previous `fmt().json()` setup. `OTEL_SERVICE_NAME` overrides the resource
//! `service.name` (default `mpex-api`).

use std::collections::HashMap;

use anyhow::Context;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::Config;

/// Keeps the OTLP providers alive for the process lifetime so `shutdown` can
/// flush pending batches when the server exits.
pub struct TelemetryGuard {
    tracer_provider: Option<SdkTracerProvider>,
    logger_provider: Option<SdkLoggerProvider>,
}

impl TelemetryGuard {
    pub fn shutdown(self) {
        if let Some(p) = self.tracer_provider {
            let _ = p.shutdown();
        }
        if let Some(p) = self.logger_provider {
            let _ = p.shutdown();
        }
    }
}

pub fn init(cfg: &Config) -> anyhow::Result<TelemetryGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,mpex=debug".into());
    let fmt = tracing_subscriber::fmt::layer().json();

    let mut guard = TelemetryGuard {
        tracer_provider: None,
        logger_provider: None,
    };

    let trace_layer = match cfg.otel_endpoint.as_deref() {
        Some(endpoint) => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()
                .context("otlp span exporter")?;
            let provider = SdkTracerProvider::builder()
                .with_resource(resource())
                .with_batch_exporter(exporter)
                .build();
            let tracer = provider.tracer("mpex");
            guard.tracer_provider = Some(provider);
            Some(tracing_opentelemetry::layer().with_tracer(tracer))
        }
        None => None,
    };

    let log_layer = match cfg.seq_url.as_deref() {
        Some(endpoint) => {
            // Seq's ingest listener is HTTP/1.1 — OTLP/HTTP protobuf.
            // `with_endpoint` is verbatim, so SEQ_URL must include the full
            // `/ingest/otlp/v1/logs` path. gRPC would need a dedicated h2c
            // port Seq doesn't expose on the container's :80.
            let mut builder = opentelemetry_otlp::LogExporter::builder()
                .with_http()
                .with_endpoint(endpoint);
            if let Some(key) = &cfg.seq_api_key {
                let mut headers = HashMap::new();
                headers.insert("x-seq-apikey".into(), key.clone());
                builder = builder.with_headers(headers);
            }
            let exporter = builder.build().context("otlp log exporter")?;
            // The dedicated-thread batch processor can't drive an async HTTP
            // client — run the processor on the ambient tokio runtime instead.
            let processor =
                opentelemetry_sdk::logs::log_processor_with_async_runtime::BatchLogProcessor::builder(
                    exporter,
                    opentelemetry_sdk::runtime::Tokio,
                )
                .build();
            let provider = SdkLoggerProvider::builder()
                .with_resource(resource())
                .with_log_processor(processor)
                .build();
            let bridge =
                opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(&provider);
            guard.logger_provider = Some(provider);
            Some(bridge)
        }
        None => None,
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt)
        .with(trace_layer)
        .with(log_layer)
        .init();
    Ok(guard)
}

fn resource() -> Resource {
    let name = std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "mpex-api".into());
    Resource::builder().with_service_name(name).build()
}
