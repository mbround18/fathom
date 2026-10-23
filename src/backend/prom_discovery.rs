//! Prometheus discovery: candidate ranking, service-proxy paths and failure classification
//! (feedback #31/#32 on spec 041).
//!
//! The 041 scan took the first Service whose *name* contained "prometheus". kube-prometheus-stack
//! creates a family of `<release>-kube-prometheus-stack-*` exporter-target Services in
//! `kube-system` (coredns, kubelet, etcd, scheduler...), which sort before `monitoring`, so the
//! scan "found" CoreDNS's metrics port and every query answered `404 page not found`. Discovery
//! now scores every Service on real Prometheus signals (labels, selector, operator objects, query
//! port, ready endpoints) and ranks them; this module is the pure part, unit-tested without a
//! cluster.

use std::collections::BTreeMap;
use std::fmt;

/// Namespaces Prometheus is commonly installed in. Listed one by one when a cluster-wide Service
/// list is denied, and a small ranking bonus otherwise.
pub const COMMON_NAMESPACES: &[&str] = &[
    "monitoring",
    "prometheus",
    "observability",
    "kube-prometheus-stack",
    "prometheus-system",
    "kube-monitoring",
    "monitoring-system",
    "cattle-monitoring-system",
    "openshift-monitoring",
    "telemetry",
    "metrics",
];

/// How many ranked candidates get an endpoint-readiness lookup (bounded cluster round trips).
pub const READINESS_LOOKUPS: usize = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortFacts {
    pub name: Option<String>,
    pub port: u16,
    pub app_protocol: Option<String>,
}

/// What discovery knows about one Service.
#[derive(Debug, Clone, Default)]
pub struct ServiceFacts {
    pub namespace: String,
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub selector: BTreeMap<String, String>,
    pub ports: Vec<PortFacts>,
    /// `clusterIP: None`.
    pub headless: bool,
    /// Whether the Service has at least one ready endpoint (`None`: not looked up / unknown).
    pub ready: Option<bool>,
}

/// A `monitoring.coreos.com/v1` `Prometheus` object (prometheus-operator).
#[derive(Debug, Clone, Default)]
pub struct OperatorPrometheus {
    pub namespace: String,
    pub name: String,
    /// `spec.routePrefix`: the HTTP API lives under it.
    pub route_prefix: Option<String>,
    /// `spec.portName` (default `web`).
    pub port_name: Option<String>,
}

/// A ranked Prometheus query target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub namespace: String,
    pub service: String,
    pub port: u16,
    pub https: bool,
    /// Normalized route prefix ("" or "/prefix").
    pub path_prefix: String,
    pub ready: Option<bool>,
    pub score: i32,
}

/// Identity-carrying label keys (their values name the workload).
const IDENTITY_KEYS: &[&str] = &[
    "app.kubernetes.io/name",
    "app",
    "k8s-app",
    "app.kubernetes.io/component",
    "component",
];

/// Tokens that mark a Service as some other part of a monitoring stack (never Prometheus).
const NOT_PROMETHEUS: &[&str] = &[
    "alertmanager",
    "grafana",
    "node-exporter",
    "kube-state-metrics",
    "operator",
    "adapter",
    "webhook",
    "pushgateway",
    "blackbox",
    "exporter",
    "kubelet",
    "coredns",
    "kube-dns",
    "kube-proxy",
    "etcd",
    "scheduler",
    "controller-manager",
    "sidecar",
    "reloader",
    "otel",
];

/// Port names that serve only a component's own `/metrics`, never the query API.
fn is_metrics_only_port(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("metrics") || n.contains("reloader") || n.starts_with("grpc") || n == "cadvisor"
}

fn identity_values(svc: &ServiceFacts) -> impl Iterator<Item = String> + '_ {
    [&svc.labels, &svc.selector].into_iter().flat_map(|m| {
        IDENTITY_KEYS
            .iter()
            .filter_map(|k| m.get(*k))
            .map(|v| v.to_lowercase())
    })
}

/// Base score for how much `svc` looks like a Prometheus server, or `None` when it is not one.
pub fn prometheus_signal(svc: &ServiceFacts) -> Option<i32> {
    let name = svc.name.to_lowercase();
    let idents: Vec<String> = identity_values(svc).collect();
    let excluded = |s: &str| NOT_PROMETHEUS.iter().any(|t| s.contains(t));
    if excluded(&name) || idents.iter().any(|v| excluded(v)) {
        return None;
    }
    // kube-prometheus-stack's exporter-target Services (kube-system/*-coredns, -kube-etcd, ...).
    if svc.labels.contains_key("jobLabel") {
        return None;
    }
    let has = |m: &BTreeMap<String, String>, k: &str| m.contains_key(k);
    let mut score = 0;
    if idents
        .iter()
        .any(|v| v == "prometheus" || v == "prometheus-server")
    {
        score += 60;
    }
    if svc.labels.get("operated-prometheus").map(String::as_str) == Some("true")
        || name == "prometheus-operated"
    {
        score += 50;
    }
    if has(&svc.selector, "operator.prometheus.io/name")
        || has(&svc.labels, "operator.prometheus.io/name")
        || has(&svc.selector, "prometheus")
    {
        score += 40;
    }
    if name == "prometheus" || name.ends_with("-prometheus") || name.contains("prometheus-server") {
        score += 30;
    }
    if name.contains("prometheus") {
        score += 10;
    }
    // Thanos Query speaks the same HTTP API.
    if name.contains("thanos-quer") || idents.iter().any(|v| v.contains("thanos-quer")) {
        score += 20;
    }
    (score > 0).then_some(score)
}

/// The Service port that serves the query API: `(port, https, bonus)`.
pub fn query_port(
    svc: &ServiceFacts,
    operator_port_name: Option<&str>,
) -> Option<(u16, bool, i32)> {
    let https_of = |p: &PortFacts| {
        p.name
            .as_deref()
            .is_some_and(|n| n.to_lowercase().starts_with("https"))
            || p.app_protocol
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case("https"))
    };
    let named = |p: &PortFacts, want: &str| {
        p.name
            .as_deref()
            .is_some_and(|n| n.eq_ignore_ascii_case(want))
    };
    let pick = |p: &PortFacts, bonus: i32| Some((p.port, https_of(p), bonus));
    if let Some(want) = operator_port_name
        && let Some(p) = svc.ports.iter().find(|p| named(p, want))
    {
        return pick(p, 15);
    }
    if let Some(p) = svc
        .ports
        .iter()
        .find(|p| named(p, "web") || named(p, "http-web"))
    {
        return pick(p, if p.port == 9090 { 15 } else { 10 });
    }
    if let Some(p) = svc.ports.iter().find(|p| p.port == 9090) {
        return pick(p, 10);
    }
    let usable: Vec<&PortFacts> = svc
        .ports
        .iter()
        .filter(|p| !p.name.as_deref().is_some_and(is_metrics_only_port))
        .collect();
    if let Some(p) = usable
        .iter()
        .find(|p| named(p, "http") || named(p, "https") || named(p, "http-query"))
    {
        return pick(p, 0);
    }
    match usable.as_slice() {
        [only] => pick(only, -5),
        _ => None,
    }
}

/// Normalize a route prefix to "" or "/a/b" (no trailing slash).
pub fn normalize_prefix(prefix: &str) -> String {
    let t = prefix.trim().trim_matches('/');
    if t.is_empty() {
        String::new()
    } else {
        format!("/{t}")
    }
}

fn operator_for<'a>(
    svc: &ServiceFacts,
    operators: &'a [OperatorPrometheus],
) -> Option<&'a OperatorPrometheus> {
    let same_ns = operators.iter().filter(|o| o.namespace == svc.namespace);
    let named = |o: &OperatorPrometheus| {
        [&svc.selector, &svc.labels].into_iter().any(|m| {
            m.get("operator.prometheus.io/name") == Some(&o.name)
                || m.get("prometheus") == Some(&o.name)
        })
    };
    let mut same_ns: Vec<&OperatorPrometheus> = same_ns.collect();
    if let Some(o) = same_ns.iter().find(|o| named(o)) {
        return Some(o);
    }
    // `prometheus-operated` fronts every Prometheus the operator runs in its namespace.
    if svc.name == "prometheus-operated" && same_ns.len() == 1 {
        return same_ns.pop();
    }
    None
}

/// Score and rank every Service that looks like a Prometheus server, best first.
pub fn rank_candidates(
    services: &[ServiceFacts],
    operators: &[OperatorPrometheus],
) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = services
        .iter()
        .filter_map(|svc| {
            let mut score = prometheus_signal(svc)?;
            let op = operator_for(svc, operators);
            let (port, https, port_bonus) =
                query_port(svc, op.and_then(|o| o.port_name.as_deref()))?;
            score += port_bonus;
            if op.is_some() {
                score += 25;
            }
            if COMMON_NAMESPACES.contains(&svc.namespace.as_str()) {
                score += 15;
            }
            if svc.namespace == "kube-system" {
                score -= 10;
            }
            if !svc.headless {
                score += 5;
            }
            score += match svc.ready {
                Some(true) => 100,
                Some(false) => -100,
                None => 0,
            };
            Some(Candidate {
                namespace: svc.namespace.clone(),
                service: svc.name.clone(),
                port,
                https,
                path_prefix: op
                    .and_then(|o| o.route_prefix.as_deref())
                    .map(normalize_prefix)
                    .unwrap_or_default(),
                ready: svc.ready,
                score,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.namespace.cmp(&b.namespace))
            .then_with(|| a.service.cmp(&b.service))
    });
    out
}

/// The API server's service-proxy path for `path_and_query` on a Service port:
/// `/api/v1/namespaces/{ns}/services/{[https:]name:port}/proxy{prefix}{path}`.
pub fn service_proxy_path(
    namespace: &str,
    service: &str,
    https: bool,
    port: u16,
    path_prefix: &str,
    path_and_query: &str,
) -> String {
    let scheme = if https { "https:" } else { "" };
    let prefix = normalize_prefix(path_prefix);
    format!(
        "/api/v1/namespaces/{namespace}/services/{scheme}{service}:{port}/proxy{prefix}{path_and_query}"
    )
}

// --- Failure classification ---

/// Why a query to a known target failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The API server could not reach it (no ready endpoints, RBAC, service gone, timeout...).
    Unreachable,
    /// It answered, but not with a Prometheus API success (404 page, bad JSON, error status).
    QueryFailed,
}

/// A classified Prometheus query failure: a one-line `summary` plus the raw response for a
/// collapsed "details" view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromQueryError {
    pub kind: FailureKind,
    pub status: Option<u16>,
    pub summary: String,
    pub raw: String,
}

impl fmt::Display for PromQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(s) if !self.summary.starts_with(&s.to_string()) => {
                write!(f, "{s} {}", self.summary)
            }
            _ => f.write_str(&self.summary),
        }
    }
}

impl std::error::Error for PromQueryError {}

const RAW_LIMIT: usize = 4000;
const SUMMARY_LIMIT: usize = 120;

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

/// A short single line from a response body: HTML tags dropped, whitespace collapsed, clipped.
pub fn summarize_body(body: &str) -> String {
    let mut text = String::with_capacity(body.len().min(512));
    let mut in_tag = false;
    for c in body.chars().take(8192) {
        match c {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        "empty response".to_owned()
    } else {
        clip(&line, SUMMARY_LIMIT)
    }
}

/// Classify a non-success HTTP status. `upstream`: the body came from the target itself (not an
/// API server `Status`).
pub fn classify_status(status: u16, body: &str, upstream: bool) -> PromQueryError {
    let unreachable = !upstream || matches!(status, 502..=504);
    let lower = body.to_lowercase();
    let summary = if lower.contains("no endpoints available") {
        "no ready endpoints".to_owned()
    } else if status == 403 && !upstream {
        format!("forbidden ({})", summarize_body(body))
    } else {
        summarize_body(body)
    };
    PromQueryError {
        kind: if unreachable {
            FailureKind::Unreachable
        } else {
            FailureKind::QueryFailed
        },
        status: Some(status),
        summary,
        raw: clip(body, RAW_LIMIT),
    }
}

/// Classify an error from a service-proxy request.
pub fn classify_kube_error(err: &kube::Error) -> PromQueryError {
    match err {
        kube::Error::Api(resp) => {
            // kube-client wraps a non-JSON error body (the upstream's own reply) as
            // `Status::failure(text, "Failed to parse error data")`, the text verbatim.
            if resp.reason == "Failed to parse error data" {
                classify_status(resp.code, &resp.message, true)
            } else {
                classify_status(resp.code, &resp.message, false)
            }
        }
        other => {
            let text = other.to_string();
            PromQueryError {
                kind: FailureKind::Unreachable,
                status: None,
                summary: summarize_body(&text),
                raw: clip(&text, RAW_LIMIT),
            }
        }
    }
}

/// A transport-level failure (connection refused, timeout).
pub fn unreachable(summary: &str) -> PromQueryError {
    PromQueryError {
        kind: FailureKind::Unreachable,
        status: None,
        summary: summarize_body(summary),
        raw: clip(summary, RAW_LIMIT),
    }
}

/// The target answered 2xx but not with a Prometheus API success.
pub fn bad_answer(summary: &str, body: &str) -> PromQueryError {
    PromQueryError {
        kind: FailureKind::QueryFailed,
        status: None,
        summary: summary.to_owned(),
        raw: clip(body, RAW_LIMIT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn port(name: &str, port: u16) -> PortFacts {
        PortFacts {
            name: Some(name.to_owned()),
            port,
            app_protocol: None,
        }
    }

    fn svc(
        ns: &str,
        name: &str,
        labels: &[(&str, &str)],
        selector: &[(&str, &str)],
        ports: Vec<PortFacts>,
    ) -> ServiceFacts {
        ServiceFacts {
            namespace: ns.to_owned(),
            name: name.to_owned(),
            labels: map(labels),
            selector: map(selector),
            ports,
            headless: false,
            ready: None,
        }
    }

    /// The chart's exporter-target Services in `kube-system`: they sort first and all contain
    /// "prometheus", but none of them is Prometheus.
    fn exporter_targets(rel: &str) -> Vec<ServiceFacts> {
        let job = [("jobLabel", "coredns"), ("release", rel)];
        vec![
            svc(
                "kube-system",
                "kube-prometheus-stack-coredns",
                &job,
                &[("k8s-app", "kube-dns")],
                vec![port("http-metrics", 9153)],
            ),
            svc(
                "kube-system",
                "kube-prometheus-stack-kube-controller-manager",
                &[("jobLabel", "kube-controller-manager")],
                &[("component", "kube-controller-manager")],
                vec![port("http-metrics", 10257)],
            ),
            svc(
                "kube-system",
                "kube-prometheus-stack-kubelet",
                &[
                    ("app.kubernetes.io/name", "kubelet"),
                    ("k8s-app", "kubelet"),
                ],
                &[],
                vec![
                    port("https-metrics", 10250),
                    port("cadvisor", 4194),
                    port("http-metrics", 10255),
                ],
            ),
        ]
    }

    /// Anonymized from a production kube-prometheus-stack install: the real Prometheus in
    /// `monitoring`, plus the exporter targets above, plus the rest of the stack.
    fn kube_prometheus_stack() -> Vec<ServiceFacts> {
        let rel = "kube-prometheus-stack";
        let mut v = exporter_targets(rel);
        v.extend([
            svc(
                "monitoring",
                "alertmanager-operated",
                &[],
                &[("app.kubernetes.io/name", "alertmanager")],
                vec![port("http-web", 9093), port("tcp-mesh", 9094)],
            ),
            svc(
                "monitoring",
                "kube-prometheus-stack-alertmanager",
                &[
                    ("app", "kube-prometheus-stack-alertmanager"),
                    ("release", rel),
                ],
                &[("app.kubernetes.io/name", "alertmanager")],
                vec![port("http-web", 9093), port("reloader-web", 8080)],
            ),
            svc(
                "monitoring",
                "kube-prometheus-stack-grafana",
                &[("app.kubernetes.io/name", "grafana")],
                &[("app.kubernetes.io/name", "grafana")],
                vec![port("http-web", 80)],
            ),
            svc(
                "monitoring",
                "kube-prometheus-stack-kube-state-metrics",
                &[("app.kubernetes.io/name", "kube-state-metrics")],
                &[("app.kubernetes.io/name", "kube-state-metrics")],
                vec![port("http", 8080)],
            ),
            svc(
                "monitoring",
                "kube-prometheus-stack-operator",
                &[("app", "kube-prometheus-stack-operator")],
                &[("app", "kube-prometheus-stack-operator")],
                vec![port("https", 443)],
            ),
            svc(
                "monitoring",
                "kube-prometheus-stack-prometheus",
                &[
                    ("app", "kube-prometheus-stack-prometheus"),
                    ("release", rel),
                    ("self-monitor", "true"),
                ],
                &[
                    ("app.kubernetes.io/name", "prometheus"),
                    (
                        "operator.prometheus.io/name",
                        "kube-prometheus-stack-prometheus",
                    ),
                ],
                vec![port("http-web", 9090), port("reloader-web", 8080)],
            ),
            svc(
                "monitoring",
                "kube-prometheus-stack-prometheus-node-exporter",
                &[("app.kubernetes.io/name", "prometheus-node-exporter")],
                &[("app.kubernetes.io/name", "prometheus-node-exporter")],
                vec![port("http-metrics", 9100)],
            ),
        ]);
        let mut operated = svc(
            "monitoring",
            "prometheus-operated",
            &[("operated-prometheus", "true")],
            &[("app.kubernetes.io/name", "prometheus")],
            vec![port("http-web", 9090)],
        );
        operated.headless = true;
        v.push(operated);
        v
    }

    #[test]
    fn prod_layout_ranks_the_real_prometheus_first() {
        let ranked = rank_candidates(&kube_prometheus_stack(), &[]);
        let names: Vec<&str> = ranked.iter().map(|c| c.service.as_str()).collect();
        assert_eq!(
            names,
            ["kube-prometheus-stack-prometheus", "prometheus-operated"],
            "only real Prometheus servers are candidates"
        );
        assert_eq!(ranked[0].namespace, "monitoring");
        assert_eq!(ranked[0].port, 9090);
        assert!(!ranked[0].https);
        assert_eq!(ranked[0].path_prefix, "");
    }

    #[test]
    fn exporter_target_services_are_never_prometheus() {
        for s in kube_prometheus_stack()
            .iter()
            .filter(|s| s.namespace == "kube-system")
        {
            assert_eq!(prometheus_signal(s), None, "{}", s.name);
        }
    }

    #[test]
    fn ready_endpoints_outrank_a_better_named_service_without_them() {
        let mut services = kube_prometheus_stack();
        for s in &mut services {
            s.ready = match s.name.as_str() {
                "kube-prometheus-stack-prometheus" => Some(false),
                "prometheus-operated" => Some(true),
                _ => None,
            };
        }
        let ranked = rank_candidates(&services, &[]);
        assert_eq!(ranked[0].service, "prometheus-operated");
        assert_eq!(ranked[0].ready, Some(true));
        assert_eq!(ranked[1].ready, Some(false));
    }

    #[test]
    fn operator_object_supplies_route_prefix_and_port_name() {
        let mut services = kube_prometheus_stack();
        services
            .iter_mut()
            .find(|s| s.name == "kube-prometheus-stack-prometheus")
            .unwrap()
            .ports = vec![port("reloader-web", 8080), port("custom", 9091)];
        let op = OperatorPrometheus {
            namespace: "monitoring".into(),
            name: "kube-prometheus-stack-prometheus".into(),
            route_prefix: Some("/prometheus/".into()),
            port_name: Some("custom".into()),
        };
        let ranked = rank_candidates(&services, &[op]);
        assert_eq!(ranked[0].service, "kube-prometheus-stack-prometheus");
        assert_eq!(ranked[0].port, 9091);
        assert_eq!(ranked[0].path_prefix, "/prometheus");
    }

    #[test]
    fn plain_helm_chart_and_namespaces_rank() {
        let services = vec![
            svc(
                "default",
                "my-prometheus-thing",
                &[],
                &[],
                vec![port("a", 1), port("b", 2)],
            ),
            svc(
                "observability",
                "prometheus-server",
                &[("app.kubernetes.io/name", "prometheus")],
                &[("app.kubernetes.io/name", "prometheus")],
                vec![port("http", 80)],
            ),
            svc(
                "apps",
                "prometheus",
                &[],
                &[("app", "prometheus")],
                vec![port("web", 9090)],
            ),
        ];
        let ranked = rank_candidates(&services, &[]);
        let names: Vec<(&str, u16)> = ranked
            .iter()
            .map(|c| (c.namespace.as_str(), c.port))
            .collect();
        // Ambiguous ports on a name-only match are skipped rather than guessed.
        assert_eq!(names, [("apps", 9090), ("observability", 80)]);
    }

    #[test]
    fn https_port_is_proxied_with_scheme() {
        let s = svc(
            "monitoring",
            "prometheus",
            &[("app.kubernetes.io/name", "prometheus")],
            &[],
            vec![port("https", 9091)],
        );
        let c = &rank_candidates(&[s], &[])[0];
        assert!(c.https);
        assert_eq!(
            service_proxy_path(
                &c.namespace,
                &c.service,
                c.https,
                c.port,
                "",
                "/api/v1/query"
            ),
            "/api/v1/namespaces/monitoring/services/https:prometheus:9091/proxy/api/v1/query"
        );
    }

    #[test]
    fn proxy_path_handles_prefixes() {
        assert_eq!(
            service_proxy_path(
                "mon",
                "prometheus",
                false,
                9090,
                "",
                "/api/v1/query?query=up"
            ),
            "/api/v1/namespaces/mon/services/prometheus:9090/proxy/api/v1/query?query=up"
        );
        for p in ["/prometheus", "prometheus/", "/prometheus/"] {
            assert_eq!(
                service_proxy_path("mon", "p", false, 9090, p, "/api/v1/query"),
                "/api/v1/namespaces/mon/services/p:9090/proxy/prometheus/api/v1/query"
            );
        }
        assert_eq!(
            service_proxy_path("mon", "p", false, 9090, "/", "/api/v1/query"),
            "/api/v1/namespaces/mon/services/p:9090/proxy/api/v1/query"
        );
    }

    #[test]
    fn upstream_404_page_is_a_query_failure_with_short_summary() {
        let e = classify_status(404, "404 page not found\n", true);
        assert_eq!(e.kind, FailureKind::QueryFailed);
        assert_eq!(e.to_string(), "404 page not found");
        assert_eq!(e.raw, "404 page not found\n");
    }

    #[test]
    fn no_endpoints_is_unreachable() {
        let e = classify_status(
            503,
            "no endpoints available for service \"kube-prometheus-stack-prometheus:9090\"",
            false,
        );
        assert_eq!(e.kind, FailureKind::Unreachable);
        assert_eq!(e.to_string(), "503 no ready endpoints");
    }

    #[test]
    fn html_bodies_are_stripped_and_clipped() {
        let body = format!(
            "<html><head><title>502 Bad Gateway</title></head><body>{}</body></html>",
            "x".repeat(5000)
        );
        let e = classify_status(502, &body, true);
        assert_eq!(e.kind, FailureKind::Unreachable);
        assert!(!e.summary.contains('<'));
        assert!(e.summary.starts_with("502 Bad Gateway"));
        assert!(e.summary.chars().count() <= SUMMARY_LIMIT + 1);
        assert!(e.raw.chars().count() <= RAW_LIMIT + 1);
    }

    #[test]
    fn kube_unparsed_error_body_is_upstream_text() {
        let err = kube::Error::Api(
            kube::core::Status::failure("404 page not found\n", "Failed to parse error data")
                .with_code(404)
                .boxed(),
        );
        let e = classify_kube_error(&err);
        assert_eq!(e.kind, FailureKind::QueryFailed);
        assert_eq!(e.to_string(), "404 page not found");
        // The body is taken verbatim: quotes and backslashes in it survive.
        let body = r#"parse error: unexpected "\n" in query"#;
        let err = kube::Error::Api(
            kube::core::Status::failure(body, "Failed to parse error data")
                .with_code(400)
                .boxed(),
        );
        assert_eq!(classify_kube_error(&err).raw, body);
        let status = kube::Error::Api(
            kube::core::Status::failure("services \"prometheus\" not found", "NotFound")
                .with_code(404)
                .boxed(),
        );
        assert_eq!(classify_kube_error(&status).kind, FailureKind::Unreachable);
    }
}
