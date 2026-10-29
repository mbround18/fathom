//! T042: logs and traces.
//!
//! - Logs are JSON lines on stderr (one object per event, for `docker logs` and log shippers),
//!   filtered by `DEPTHS_LOG` (an `EnvFilter`, default `info`).
//! - Spans are exported over OTLP/HTTP only when `OTEL_EXPORTER_OTLP_ENDPOINT` (or
//!   `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`) is set; the exporter reads the rest of the standard
//!   `OTEL_*` variables itself. Without one, no exporter is built and nothing leaves the process.
//!
//! - Metrics go to the same endpoint, exported every 60 s; without one the instruments are no-ops.
//!
//! Call [`init`] before the Tokio runtime starts: the OTLP HTTP client is a blocking one.

use std::sync::LazyLock;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

pub const SERVICE_NAME: &str = "depths";

/// Keeps the OTLP pipeline alive; dropping it flushes and shuts it down.
#[derive(Debug)]
pub struct Guard {
    provider: Option<SdkTracerProvider>,
    meter: Option<SdkMeterProvider>,
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
        if let Some(m) = self.meter.take() {
            let _ = m.shutdown();
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
    let resource = || {
        Resource::builder()
            .with_service_name(env("OTEL_SERVICE_NAME").unwrap_or_else(|| SERVICE_NAME.to_owned()))
            .build()
    };
    let provider = if otlp_enabled(env) {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .build()
            .map_err(|e| format!("cannot build the OTLP exporter (OTEL_EXPORTER_OTLP_*): {e}"))?;
        Some(
            SdkTracerProvider::builder()
                .with_resource(resource())
                .with_batch_exporter(exporter)
                .build(),
        )
    } else {
        None
    };
    let meter = if otlp_enabled(env) {
        let exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .build()
            .map_err(|e| format!("cannot build the OTLP metric exporter: {e}"))?;
        let m = SdkMeterProvider::builder()
            .with_resource(resource())
            .with_periodic_exporter(exporter)
            .build();
        opentelemetry::global::set_meter_provider(m.clone());
        Some(m)
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
    Ok(Guard { provider, meter })
}

/// `depths.files.helper_start_ms`: how long a volume helper took to answer `Hello` (spec 053).
static HELPER_START_MS: LazyLock<Histogram<u64>> = LazyLock::new(|| {
    opentelemetry::global::meter(SERVICE_NAME)
        .u64_histogram("depths.files.helper_start_ms")
        .with_unit("ms")
        .with_description("Time from starting a volume helper to its Hello")
        .build()
});

pub fn record_helper_start(took: Duration, kind: &'static str) {
    HELPER_START_MS.record(
        u64::try_from(took.as_millis()).unwrap_or(u64::MAX),
        &[KeyValue::new("mount_kind", kind)],
    );
}

/// The `files.op` span: what ran where, never the path (R15). Fill `outcome`, `bytes` and
/// `warm` with `Span::record` once known.
pub fn files_span(kind: &'static str, stack: &str, mount: &str) -> tracing::Span {
    tracing::info_span!(
        "files.op",
        kind,
        stack,
        mount,
        outcome = tracing::field::Empty,
        bytes = tracing::field::Empty,
        warm = tracing::field::Empty,
    )
}

// ---------------------------------------------------------------------------
// Image updates (spec 062 research R9). Labels are registry hosts, stacks, services and fixed
// words: never credentials, never paths, never `auths` keys.
// ---------------------------------------------------------------------------

/// A registry as a label: the host (and port) only. Anything that could carry a user, a
/// password, a path or a scheme is cut away.
pub fn registry_label(registry: &str) -> String {
    let s = registry.trim();
    let s = s.split_once("://").map_or(s, |(_, rest)| rest);
    let s = s.rsplit_once('@').map_or(s, |(_, host)| host);
    let s = s.split(['/', '?', '#']).next().unwrap_or_default();
    s.to_ascii_lowercase()
}

/// Which registry request a count is for (`oci-registry`'s `RequestCounts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryRequest {
    Head,
    Get,
    ManifestGet,
}

impl RegistryRequest {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Get => "get",
            Self::ManifestGet => "manifest_get",
        }
    }
}

static REGISTRY_REQUESTS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    opentelemetry::global::meter(SERVICE_NAME)
        .u64_counter("depths.updates.registry_requests")
        .with_description("Registry requests made by update checks, by kind")
        .build()
});

static CHECK_MS: LazyLock<Histogram<u64>> = LazyLock::new(|| {
    opentelemetry::global::meter(SERVICE_NAME)
        .u64_histogram("depths.updates.check_ms")
        .with_unit("ms")
        .with_description("Time one update check pass took")
        .build()
});

static OUTCOMES: LazyLock<Counter<u64>> = LazyLock::new(|| {
    opentelemetry::global::meter(SERVICE_NAME)
        .u64_counter("depths.updates.outcomes")
        .with_description("Stack update outcomes, by trigger")
        .build()
});

static HEALTH_WAIT_MS: LazyLock<Histogram<u64>> = LazyLock::new(|| {
    opentelemetry::global::meter(SERVICE_NAME)
        .u64_histogram("depths.updates.health_wait_ms")
        .with_unit("ms")
        .with_description("Time spent waiting for updated containers to be healthy")
        .build()
});

fn millis(took: Duration) -> u64 {
    u64::try_from(took.as_millis()).unwrap_or(u64::MAX)
}

/// Adds `n` requests of `kind` to `registry` (a host; see [`registry_label`]).
pub fn record_registry_requests(registry: &str, kind: RegistryRequest, n: u64) {
    if n > 0 {
        REGISTRY_REQUESTS.add(
            n,
            &[
                KeyValue::new("registry", registry_label(registry)),
                KeyValue::new("kind", kind.as_str()),
            ],
        );
    }
}

pub fn record_check(took: Duration) {
    CHECK_MS.record(millis(took), &[]);
}

/// `outcome` and `trigger` (`manual` | `scheduled`) are fixed words.
pub fn record_outcome(outcome: &'static str, trigger: &'static str) {
    OUTCOMES.add(
        1,
        &[
            KeyValue::new("outcome", outcome),
            KeyValue::new("trigger", trigger),
        ],
    );
}

pub fn record_health_wait(took: Duration) {
    HEALTH_WAIT_MS.record(millis(took), &[]);
}

/// `updates.check`. Fill `outcome` once known.
pub fn updates_check_span(stack: &str, registry: &str) -> tracing::Span {
    tracing::info_span!(
        "updates.check",
        stack,
        registry = registry_label(registry),
        outcome = tracing::field::Empty,
    )
}

/// `updates.resolve`. Fill `outcome` once known.
pub fn updates_resolve_span(registry: &str) -> tracing::Span {
    tracing::info_span!(
        "updates.resolve",
        registry = registry_label(registry),
        outcome = tracing::field::Empty,
    )
}

/// `updates.run`: `trigger` is `manual` or `scheduled`.
pub fn updates_run_span(run_id: &str, trigger: &'static str, stacks: usize) -> tracing::Span {
    tracing::info_span!("updates.run", run_id, trigger, stacks)
}

/// `updates.stack`. Fill `outcome` once known.
pub fn updates_stack_span(stack: &str) -> tracing::Span {
    tracing::info_span!("updates.stack", stack, outcome = tracing::field::Empty)
}

/// `updates.health_wait`. Fill `outcome` and `waited_ms` once known.
pub fn updates_health_wait_span(stack: &str, service: &str) -> tracing::Span {
    tracing::info_span!(
        "updates.health_wait",
        stack,
        service,
        outcome = tracing::field::Empty,
        waited_ms = tracing::field::Empty,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registry_label_is_the_host_only() {
        assert_eq!(registry_label("ghcr.io"), "ghcr.io");
        assert_eq!(
            registry_label("Registry.Example:5000"),
            "registry.example:5000"
        );
        assert_eq!(
            registry_label("https://user:s3cret@registry.example/v2/app?x=1"),
            "registry.example"
        );
        assert_eq!(
            registry_label("https://index.docker.io/v1/"),
            "index.docker.io"
        );
        assert_eq!(registry_label("token@host"), "host");
    }

    #[test]
    fn the_update_instruments_record_without_an_exporter() {
        // No meter provider is installed: every instrument is a no-op and must not panic.
        record_registry_requests("ghcr.io", RegistryRequest::Head, 2);
        record_registry_requests("ghcr.io", RegistryRequest::Get, 0);
        record_check(Duration::from_millis(12));
        record_outcome("updated", "manual");
        record_health_wait(Duration::from_secs(3));
        let span = updates_health_wait_span("media", "app");
        span.record("outcome", "healthy");
        span.record("waited_ms", 3000_u64);
        drop(updates_check_span("media", "user:pw@ghcr.io"));
        drop(updates_resolve_span("ghcr.io"));
        drop(updates_run_span("6f1c", "scheduled", 2));
        drop(updates_stack_span("media"));
    }

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
