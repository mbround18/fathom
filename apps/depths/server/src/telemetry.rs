//! T042: logs and traces.
//!
//! - Logs are JSON lines on stderr (one object per event, for `docker logs` and log shippers),
//!   filtered by `DEPTHS_LOG` (an `EnvFilter`, default `info`).
//! - Spans are exported over OTLP/HTTP only when `OTEL_EXPORTER_OTLP_ENDPOINT` (or
//!   `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`) is set; the exporter reads the rest of the standard
//!   `OTEL_*` variables itself. Without one, no exporter is built and nothing leaves the process.
//!
//! Call [`init`] before the Tokio runtime starts: the OTLP HTTP client is a blocking one.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

pub const SERVICE_NAME: &str = "depths";

/// Keeps the OTLP pipeline alive; dropping it flushes and shuts it down.
#[derive(Debug)]
pub struct Guard {
    provider: Option<SdkTracerProvider>,
}

impl Guard {
    /// Whether spans are being exported.
    pub fn exporting(&self) -> bool {
        self.provider.is_some()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(p) = self.provider.take() {
            // Nothing useful can be done with a failed flush at exit.
            let _ = p.shutdown();
        }
    }
}

/// Whether an OTLP endpoint is configured: either variable set and not blank.
pub fn otlp_enabled(env: impl Fn(&str) -> Option<String>) -> bool {
    [
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    ]
    .iter()
    .any(|k| env(k).is_some_and(|v| !v.trim().is_empty()))
}

/// Installs the global subscriber (and, with an endpoint, the OTLP span exporter).
///
/// # Errors
///
/// The OTLP exporter cannot be built, or a global subscriber is already set.
pub fn init() -> Result<Guard, String> {
    let env = |k: &str| std::env::var(k).ok();
    let filter = EnvFilter::try_from_env("DEPTHS_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(std::io::stderr);
    let provider = if otlp_enabled(env) {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .build()
            .map_err(|e| format!("cannot build the OTLP exporter (OTEL_EXPORTER_OTLP_*): {e}"))?;
        Some(
            SdkTracerProvider::builder()
                .with_resource(
                    Resource::builder()
                        .with_service_name(
                            env("OTEL_SERVICE_NAME").unwrap_or_else(|| SERVICE_NAME.to_owned()),
                        )
                        .build(),
                )
                .with_batch_exporter(exporter)
                .build(),
        )
    } else {
        None
    };
    let otel = provider
        .as_ref()
        .map(|p| tracing_opentelemetry::layer().with_tracer(p.tracer(SERVICE_NAME)));
    Registry::default()
        .with(filter)
        .with(fmt)
        .with(otel)
        .try_init()
        .map_err(|e| format!("cannot install the log subscriber: {e}"))?;
    Ok(Guard { provider })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otlp_only_with_an_endpoint() {
        let none = |_: &str| None;
        assert!(!otlp_enabled(none));
        let blank = |_: &str| Some("  ".to_owned());
        assert!(!otlp_enabled(blank));
        let set = |k: &str| (k == "OTEL_EXPORTER_OTLP_ENDPOINT").then(|| "http://otel:4318".into());
        assert!(otlp_enabled(set));
        let traces = |k: &str| {
            (k == "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").then(|| "http://otel:4318".into())
        };
        assert!(otlp_enabled(traces));
    }
}
