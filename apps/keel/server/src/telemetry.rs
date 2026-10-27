//! Logs, traces and metrics (spec 055 T029).
//!
//! - Logs are JSON lines on stderr, filtered by `KEEL_LOG` (an `EnvFilter`, default `info`).
//! - Spans and metrics are exported over OTLP/HTTP only when `--otel-endpoint`
//!   (`KEEL_OTEL_ENDPOINT` or `OTEL_EXPORTER_OTLP_ENDPOINT`) is set. Without one no exporter is
//!   built and nothing leaves the process.
//!
//! Call [`init`] before the Tokio runtime starts: the OTLP HTTP client is a blocking one. Drop the
//! runtime before the [`Guard`], so the final flush is not cut short.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

pub const SERVICE_NAME: &str = "keel";

/// Keeps the OTLP pipeline alive; dropping it flushes and shuts it down.
#[derive(Debug)]
pub struct Guard {
    traces: Option<SdkTracerProvider>,
    metrics: Option<SdkMeterProvider>,
}

impl Guard {
    /// Whether spans are being exported.
    #[must_use]
    pub fn exporting(&self) -> bool {
        self.traces.is_some()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        // Nothing useful can be done with a failed flush at exit.
        if let Some(p) = self.traces.take() {
            let _ = p.shutdown();
        }
        if let Some(m) = self.metrics.take() {
            let _ = m.shutdown();
        }
    }
}

/// The signal URL under an OTLP/HTTP base endpoint (`http://collector:4318` →
/// `http://collector:4318/v1/traces`).
#[must_use]
pub fn signal_url(base: &str, signal: &str) -> String {
    format!("{}/v1/{signal}", base.trim().trim_end_matches('/'))
}

/// Installs the global subscriber and, with an endpoint, the OTLP exporters.
///
/// # Errors
///
/// An exporter cannot be built, or a global subscriber is already set.
pub fn init(endpoint: Option<&str>) -> Result<Guard, String> {
    let filter = EnvFilter::try_from_env("KEEL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(std::io::stderr);
    let endpoint = endpoint.map(str::trim).filter(|e| !e.is_empty());
    let resource = || Resource::builder().with_service_name(SERVICE_NAME).build();

    let (traces, metrics) = match endpoint {
        Some(base) => {
            let spans = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(signal_url(base, "traces"))
                .build()
                .map_err(|e| format!("cannot build the OTLP span exporter: {e}"))?;
            let traces = SdkTracerProvider::builder()
                .with_resource(resource())
                .with_batch_exporter(spans)
                .build();
            let exporter = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_endpoint(signal_url(base, "metrics"))
                .build()
                .map_err(|e| format!("cannot build the OTLP metric exporter: {e}"))?;
            let metrics = SdkMeterProvider::builder()
                .with_resource(resource())
                .with_periodic_exporter(exporter)
                .build();
            opentelemetry::global::set_meter_provider(metrics.clone());
            (Some(traces), Some(metrics))
        }
        None => (None, None),
    };
    let otel = traces
        .as_ref()
        .map(|p| tracing_opentelemetry::layer().with_tracer(p.tracer(SERVICE_NAME)));
    Registry::default()
        .with(filter)
        .with(fmt)
        .with(otel)
        .try_init()
        .map_err(|e| format!("cannot install the log subscriber: {e}"))?;
    Ok(Guard { traces, metrics })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_urls() {
        assert_eq!(
            signal_url("http://c:4318", "traces"),
            "http://c:4318/v1/traces"
        );
        assert_eq!(
            signal_url(" http://c:4318/ ", "metrics"),
            "http://c:4318/v1/metrics"
        );
    }
}
