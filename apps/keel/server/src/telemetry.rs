//! Logs, traces and metrics (spec 055 T029).
//!
//! - Logs are JSON lines on stderr, filtered by `KEEL_LOG` (an `EnvFilter`, default `info`).
//! - Spans and metrics are exported over OTLP/HTTP only when `--otel-endpoint`
//!   (`KEEL_OTEL_ENDPOINT` or `OTEL_EXPORTER_OTLP_ENDPOINT`) is set. Without one no exporter is
//!   built and nothing leaves the process.
//!
//! Call [`init`] before the Tokio runtime starts: the OTLP HTTP client is a blocking one. Drop the
//! runtime before the [`Guard`], so the final flush is not cut short.

use std::sync::LazyLock;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

pub const SERVICE_NAME: &str = "keel";

/// The resource's `service.version` (spec 061 R9): `KEEL_BUILD_VERSION`, else
/// `KEEL_BUILD_REVISION`, else the crate version. Empty values count as unset, since an untagged
/// image carries `KEEL_BUILD_VERSION=`.
#[must_use]
pub fn service_version(var: impl Fn(&str) -> Option<String>) -> String {
    let set = |name: &str| {
        var(name)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    set("KEEL_BUILD_VERSION")
        .or_else(|| set("KEEL_BUILD_REVISION"))
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned())
}

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
    let version = service_version(|name| std::env::var(name).ok());
    let resource = || {
        Resource::builder()
            .with_service_name(SERVICE_NAME)
            .with_attribute(opentelemetry::KeyValue::new(
                "service.version",
                version.clone(),
            ))
            .build()
    };

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

// Update checks (spec 061 research R9). The instruments bind to the global meter on first use, so
// they record through whatever `init` installed, or nowhere.

static UPDATE_CHECKS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter(SERVICE_NAME)
        .u64_counter("keel.update.checks")
        .with_description("Update checks, by outcome")
        .build()
});

static UPDATE_CHECK_DURATION: LazyLock<Histogram<u64>> = LazyLock::new(|| {
    global::meter(SERVICE_NAME)
        .u64_histogram("keel.update.check.duration")
        .with_unit("ms")
        .with_description("How long one update check took")
        .build()
});

static UPDATE_REQUESTS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter(SERVICE_NAME)
        .u64_counter("keel.update.requests")
        .with_description("Registry requests made by update checks, by method")
        .build()
});

static UPDATE_PULLS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter(SERVICE_NAME)
        .u64_counter("keel.update.pulls")
        .with_description("Update checks that counted as an image pull")
        .build()
});

/// The `keel.update_check` span. The outcome, request counts, status and error are recorded on it
/// when the check ends. Never carries credentials.
#[must_use]
pub fn update_check_span(trigger: &'static str, image: &str) -> tracing::Span {
    tracing::info_span!(
        "keel.update_check",
        keel.update.trigger = trigger,
        keel.update.image = image,
        keel.update.outcome = tracing::field::Empty,
        keel.update.requests.head = tracing::field::Empty,
        keel.update.requests.get = tracing::field::Empty,
        keel.update.pulls = tracing::field::Empty,
        http.response.status_code = tracing::field::Empty,
        error.type = tracing::field::Empty,
    )
}

/// Records one finished update check on the `keel.update.*` instruments.
pub fn record_update_check(outcome: &'static str, took: Duration, head: u32, get: u32, pulls: u32) {
    UPDATE_CHECKS.add(1, &[KeyValue::new("outcome", outcome)]);
    UPDATE_CHECK_DURATION.record(
        u64::try_from(took.as_millis()).unwrap_or(u64::MAX),
        &[KeyValue::new("outcome", outcome)],
    );
    if head > 0 {
        UPDATE_REQUESTS.add(u64::from(head), &[KeyValue::new("method", "HEAD")]);
    }
    if get > 0 {
        UPDATE_REQUESTS.add(u64::from(get), &[KeyValue::new("method", "GET")]);
    }
    if pulls > 0 {
        UPDATE_PULLS.add(u64::from(pulls), &[]);
    }
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

    fn version_from(pairs: &[(&str, &str)]) -> String {
        service_version(|name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    #[test]
    fn service_version_prefers_the_release_version() {
        assert_eq!(
            version_from(&[
                ("KEEL_BUILD_VERSION", "0.2.0"),
                ("KEEL_BUILD_REVISION", "cf7fdd6")
            ]),
            "0.2.0"
        );
    }

    #[test]
    fn service_version_falls_back_to_the_revision_then_the_crate() {
        assert_eq!(
            version_from(&[
                ("KEEL_BUILD_VERSION", ""),
                ("KEEL_BUILD_REVISION", "cf7fdd6-dirty")
            ]),
            "cf7fdd6-dirty"
        );
        assert_eq!(
            version_from(&[("KEEL_BUILD_VERSION", " ")]),
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(version_from(&[]), env!("CARGO_PKG_VERSION"));
    }
}
