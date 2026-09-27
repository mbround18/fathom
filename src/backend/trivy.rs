//! Aqua Trivy Operator integration (021-trivy-vulnerability-pane).
//!
//! Discovers whichever served version of the Trivy Operator's CRDs
//! (`VulnerabilityReport`, `ExposedSecretReport`, `SbomReport`, `RbacAssessmentReport`,
//! API group `aquasecurity.github.io`) is installed and reads report instances as dynamic
//! objects - the same discovery pattern used for Flux in `flux.rs` and Longhorn in
//! `longhorn.rs`, so this works across Trivy Operator versions without pinning an API version.
//! Read-only: Fathom never triggers or configures a scan (spec Non-Goals).

use super::served_crd::{ServedCrd, find_served};
use crate::protocol::{
    PodVulnerabilityInspection, Severity, SeveritySummary, TrivyOverview, VulnInspectError,
    VulnerabilityFinding, WorkloadVulnSummary,
};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::ListParams;
use kube::core::DynamicObject;
use kube::{Client, api::Api};
use serde_json::Value;

/// CRD names (plural.group) the Trivy Operator installs. Any one of these being served is
/// enough to consider the operator "installed" (FR-001/FR-014).
const VULNERABILITY_REPORT_CRD: &str = "vulnerabilityreports.aquasecurity.github.io";
const EXPOSED_SECRET_REPORT_CRD: &str = "exposedsecretreports.aquasecurity.github.io";
const SBOM_REPORT_CRD: &str = "sbomreports.aquasecurity.github.io";
const RBAC_ASSESSMENT_REPORT_CRD: &str = "rbacassessmentreports.aquasecurity.github.io";

const ALL_CRDS: &[&str] = &[
    VULNERABILITY_REPORT_CRD,
    EXPOSED_SECRET_REPORT_CRD,
    SBOM_REPORT_CRD,
    RBAC_ASSESSMENT_REPORT_CRD,
];

/// The result of trying to list every instance of a discovered CRD: either the items, or a
/// signal that access was forbidden (FR-013), distinct from "no reports produced yet".
pub(super) enum ListOutcome {
    Items(Vec<DynamicObject>),
    Forbidden,
}

/// List every instance of a discovered CRD as dynamic objects, across all namespaces.
pub(super) async fn list_instances(client: &Client, crd: &ServedCrd) -> ListOutcome {
    let resource = crd.api_resource();
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &resource);
    match api.list(&ListParams::default()).await {
        Ok(list) => ListOutcome::Items(list.items),
        Err(kube::Error::Api(e)) if e.code == 403 => ListOutcome::Forbidden,
        Err(_) => ListOutcome::Items(Vec::new()),
    }
}

/// Discover which of the Trivy Operator's CRDs are served, so callers only need one
/// `CustomResourceDefinition` list per scan (mirrors `flux::scan_flux`'s discovery step).
pub(super) async fn discover(
    client: &Client,
) -> anyhow::Result<Vec<(&'static str, Option<ServedCrd>)>> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&ListParams::default()).await?.items;
    Ok(ALL_CRDS
        .iter()
        .map(|name| (*name, find_served(&crds, name)))
        .collect())
}

/// A single owner attribution: kind + name a `VulnerabilityReport` was labeled with by Trivy's
/// own built-in owner-resolution (research.md R1), falling back to the report's own
/// namespace/name when no owner labels are present.
struct ReportAttribution {
    namespace: String,
    kind: String,
    name: String,
    container: String,
    image: String,
}

fn attribution(obj: &DynamicObject) -> ReportAttribution {
    let labels = obj.metadata.labels.clone().unwrap_or_default();
    let namespace = obj.metadata.namespace.clone().unwrap_or_default();
    let kind = labels
        .get("trivy-operator.resource.kind")
        .cloned()
        .unwrap_or_else(|| "Pod".to_owned());
    let name = labels
        .get("trivy-operator.resource.name")
        .cloned()
        .unwrap_or_else(|| obj.metadata.name.clone().unwrap_or_default());
    let container = labels
        .get("trivy-operator.container.name")
        .cloned()
        .unwrap_or_default();
    let image = obj
        .data
        .get("report")
        .and_then(|r| r.get("artifact"))
        .and_then(|a| a.get("repository"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    ReportAttribution {
        namespace,
        kind,
        name,
        container,
        image,
    }
}

fn parse_severity(raw: &str) -> Severity {
    match raw.to_ascii_uppercase().as_str() {
        "CRITICAL" => Severity::Critical,
        "HIGH" => Severity::High,
        "MEDIUM" => Severity::Medium,
        "LOW" => Severity::Low,
        _ => Severity::Unknown,
    }
}

fn bump(summary: &mut SeveritySummary, severity: Severity) {
    match severity {
        Severity::Critical => summary.critical += 1,
        Severity::High => summary.high += 1,
        Severity::Medium => summary.medium += 1,
        Severity::Low => summary.low += 1,
        Severity::Unknown => summary.unknown += 1,
    }
}

/// Extract every CVE finding from one `VulnerabilityReport`'s `.report.vulnerabilities` array.
pub(super) fn findings_from_report(obj: &DynamicObject) -> Vec<VulnerabilityFinding> {
    let attrib = attribution(obj);
    let vulns = obj
        .data
        .get("report")
        .and_then(|r| r.get("vulnerabilities"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    vulns
        .iter()
        .map(|v| {
            let get_str = |key: &str| -> String {
                v.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
            };
            VulnerabilityFinding {
                cve_id: get_str("vulnerabilityID"),
                severity: parse_severity(&get_str("severity")),
                package: get_str("resource"),
                installed_version: get_str("installedVersion"),
                fixed_version: v
                    .get("fixedVersion")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                score: v.get("score").and_then(Value::as_f64),
                container: attrib.container.clone(),
                image: attrib.image.clone(),
            }
        })
        .collect()
}

/// Dedup `VulnerabilityReport` instances by (namespace, resource.kind, resource.name,
/// container.name), keeping only the latest by `creationTimestamp` (research.md R3) - Trivy
/// re-creates a report per scan, so stale reports for the same container must not double-count.
pub(super) fn dedup_latest(reports: Vec<DynamicObject>) -> Vec<DynamicObject> {
    use std::collections::HashMap;
    let mut latest: HashMap<(String, String, String, String), DynamicObject> = HashMap::new();
    for obj in reports {
        let attrib = attribution(&obj);
        let key = (attrib.namespace, attrib.kind, attrib.name, attrib.container);
        let ts = obj
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|t| t.0.to_rfc3339())
            .unwrap_or_default();
        match latest.get(&key) {
            Some(existing) => {
                let existing_ts = existing
                    .metadata
                    .creation_timestamp
                    .as_ref()
                    .map(|t| t.0.to_rfc3339())
                    .unwrap_or_default();
                if ts > existing_ts {
                    latest.insert(key, obj);
                }
            }
            None => {
                latest.insert(key, obj);
            }
        }
    }
    latest.into_values().collect()
}

/// Rank worst-first: critical desc, then high, medium, low, total; tie-break namespace/name asc
/// (research.md R2).
pub(super) fn rank_worst_first(workloads: &mut [WorkloadVulnSummary]) {
    workloads.sort_by(|a, b| {
        b.summary
            .critical
            .cmp(&a.summary.critical)
            .then(b.summary.high.cmp(&a.summary.high))
            .then(b.summary.medium.cmp(&a.summary.medium))
            .then(b.summary.low.cmp(&a.summary.low))
            .then(b.summary.total().cmp(&a.summary.total()))
            .then(a.namespace.cmp(&b.namespace))
            .then(a.name.cmp(&b.name))
    });
}

/// Aggregate every `VulnerabilityReport`'s findings into one row per (namespace, kind, name).
pub(super) fn summarize_workloads(reports: &[DynamicObject]) -> Vec<WorkloadVulnSummary> {
    use std::collections::HashMap;
    struct Acc {
        namespace: String,
        kind: String,
        name: String,
        summary: SeveritySummary,
        images: std::collections::HashSet<String>,
    }
    let mut acc: HashMap<(String, String, String), Acc> = HashMap::new();
    for obj in reports {
        let attrib = attribution(obj);
        let findings = findings_from_report(obj);
        let key = (
            attrib.namespace.clone(),
            attrib.kind.clone(),
            attrib.name.clone(),
        );
        let entry = acc.entry(key).or_insert_with(|| Acc {
            namespace: attrib.namespace.clone(),
            kind: attrib.kind.clone(),
            name: attrib.name.clone(),
            summary: SeveritySummary::default(),
            images: std::collections::HashSet::new(),
        });
        if !attrib.image.is_empty() {
            entry.images.insert(attrib.image.clone());
        }
        for f in &findings {
            bump(&mut entry.summary, f.severity);
        }
    }
    acc.into_values()
        .map(|a| WorkloadVulnSummary {
            namespace: a.namespace,
            kind: a.kind,
            name: a.name,
            summary: a.summary,
            image_count: a.images.len().max(1) as u32,
        })
        .collect()
}

/// Discover and list every `VulnerabilityReport` instance. `Ok(None)` means the CRD isn't
/// served (not installed); `Err(())` signals a 403 (forbidden) distinct from "not installed".
async fn list_vulnerability_reports(
    client: &Client,
) -> anyhow::Result<Result<Option<Vec<DynamicObject>>, ()>> {
    let discovered = discover(client).await?;
    let Some((_, Some(crd))) = discovered
        .into_iter()
        .find(|(name, _)| *name == VULNERABILITY_REPORT_CRD)
    else {
        return Ok(Ok(None));
    };
    match list_instances(client, &crd).await {
        ListOutcome::Items(items) => Ok(Ok(Some(items))),
        ListOutcome::Forbidden => Ok(Err(())),
    }
}

/// Any of the four Trivy CRDs being served means the operator is installed (FR-001/FR-014),
/// even if `VulnerabilityReport` access itself is forbidden.
async fn any_trivy_crd_installed(client: &Client) -> anyhow::Result<bool> {
    let discovered = discover(client).await?;
    Ok(discovered.iter().any(|(_, crd)| crd.is_some()))
}

/// Cluster-wide worst-offenders scan (US2). Mirrors `flux::scan_flux`'s
/// installed/not-installed shape, plus the `forbidden` distinction Flux doesn't need
/// (research.md R4).
pub(super) async fn scan_trivy(client: &Client) -> anyhow::Result<TrivyOverview> {
    let installed = any_trivy_crd_installed(client).await?;
    if !installed {
        return Ok(TrivyOverview {
            installed: false,
            forbidden: false,
            workloads: Vec::new(),
        });
    }

    match list_vulnerability_reports(client).await? {
        Err(()) => Ok(TrivyOverview {
            installed: true,
            forbidden: true,
            workloads: Vec::new(),
        }),
        Ok(None) => Ok(TrivyOverview {
            installed: true,
            forbidden: false,
            workloads: Vec::new(),
        }),
        Ok(Some(items)) => {
            let deduped = dedup_latest(items);
            let mut workloads = summarize_workloads(&deduped);
            rank_worst_first(&mut workloads);
            Ok(TrivyOverview {
                installed: true,
                forbidden: false,
                workloads,
            })
        }
    }
}

/// Whether a report belongs to this Pod. Trivy labels a report with whichever hop of the owner
/// chain it scanned: the ReplicaSet for a Deployment's Pod, the CronJob for a CronJob's Job,
/// the StatefulSet/DaemonSet/Job directly, or the bare Pod. So a report matches on the Pod
/// itself, its direct controller, or the top owner [`resolve_owner`] found.
fn report_matches_pod(
    attrib: &ReportAttribution,
    namespace: &str,
    pod_name: &str,
    direct_owner: Option<(&str, &str)>,
    top_owner: (&str, &str),
) -> bool {
    let is = |(kind, name): (&str, &str)| attrib.kind == kind && attrib.name == name;
    attrib.namespace == namespace
        && (is(("Pod", pod_name)) || direct_owner.is_some_and(is) || is(top_owner))
}

/// Owner-chain attribution (research.md R1): walk at most two hops of `ownerReferences`
/// (Pod -> ReplicaSet -> Deployment, or Pod -> Job -> CronJob), falling back to the Pod itself
/// when no recognized owner chain applies - the same bounded-hop approach the Applications
/// feature already uses for Helm/Flux ownership attribution.
async fn resolve_owner(client: &Client, namespace: &str, pod: &Pod) -> (String, String) {
    let pod_name = pod.metadata.name.clone().unwrap_or_default();
    let Some(owners) = pod.metadata.owner_references.as_ref() else {
        return ("Pod".to_owned(), pod_name);
    };
    let Some(rs_or_job) = owners.first() else {
        return ("Pod".to_owned(), pod_name);
    };

    match rs_or_job.kind.as_str() {
        "ReplicaSet" => {
            let api: Api<k8s_openapi::api::apps::v1::ReplicaSet> =
                Api::namespaced(client.clone(), namespace);
            if let Ok(rs) = api.get(&rs_or_job.name).await
                && let Some(owners) = rs.metadata.owner_references
                && let Some(owner) = owners.first()
            {
                return (owner.kind.clone(), owner.name.clone());
            }
            ("ReplicaSet".to_owned(), rs_or_job.name.clone())
        }
        "Job" => {
            let api: Api<k8s_openapi::api::batch::v1::Job> =
                Api::namespaced(client.clone(), namespace);
            if let Ok(job) = api.get(&rs_or_job.name).await
                && let Some(owners) = job.metadata.owner_references
                && let Some(owner) = owners.first()
            {
                return (owner.kind.clone(), owner.name.clone());
            }
            ("Job".to_owned(), rs_or_job.name.clone())
        }
        other => (other.to_owned(), rs_or_job.name.clone()),
    }
}

/// Inspect one Pod's vulnerabilities (US1). Never triggers a scan - only reads whatever
/// `VulnerabilityReport`s Trivy has already produced for the Pod's owner chain.
pub(super) async fn inspect_pod_vulnerabilities(
    client: &Client,
    namespace: &str,
    pod_name: &str,
) -> anyhow::Result<Result<PodVulnerabilityInspection, VulnInspectError>> {
    let installed = any_trivy_crd_installed(client).await?;
    if !installed {
        return Ok(Err(VulnInspectError::NotInstalled(
            "Trivy Operator CRDs were not found in this cluster.".to_owned(),
        )));
    }

    let pods_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pod = match pods_api.get(pod_name).await {
        Ok(p) => p,
        Err(e) => {
            return Ok(Err(VulnInspectError::Other(format!(
                "Failed to read pod: {e}"
            ))));
        }
    };
    let (owner_kind, owner_name) = resolve_owner(client, namespace, &pod).await;

    let reports = match list_vulnerability_reports(client).await? {
        Err(()) => {
            return Ok(Err(VulnInspectError::Forbidden(
                "Not permitted to read VulnerabilityReports in this cluster.".to_owned(),
            )));
        }
        Ok(None) => {
            return Ok(Err(VulnInspectError::NotInstalled(
                "Trivy Operator CRDs were not found in this cluster.".to_owned(),
            )));
        }
        Ok(Some(items)) => dedup_latest(items),
    };

    let direct_owner = pod
        .metadata
        .owner_references
        .as_ref()
        .and_then(|o| o.first())
        .map(|o| (o.kind.as_str(), o.name.as_str()));
    let matching: Vec<&DynamicObject> = reports
        .iter()
        .filter(|obj| {
            report_matches_pod(
                &attribution(obj),
                namespace,
                pod_name,
                direct_owner,
                (&owner_kind, &owner_name),
            )
        })
        .collect();

    if matching.is_empty() {
        return Ok(Err(VulnInspectError::NoReportYet(format!(
            "No VulnerabilityReport found yet for {namespace}/{pod_name}."
        ))));
    }

    let mut summary = SeveritySummary::default();
    let mut findings: Vec<VulnerabilityFinding> = Vec::new();
    let mut generated_at = String::new();
    for obj in &matching {
        for f in findings_from_report(obj) {
            bump(&mut summary, f.severity);
            findings.push(f);
        }
        let ts = obj
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|t| t.0.to_rfc3339())
            .unwrap_or_default();
        if ts > generated_at {
            generated_at = ts;
        }
    }

    let total_findings = findings.len() as u32;
    findings.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)))
    });
    findings.truncate(50);

    Ok(Ok(PodVulnerabilityInspection {
        namespace: namespace.to_owned(),
        pod: pod_name.to_owned(),
        owner_kind,
        owner_name,
        summary,
        top_findings: findings,
        total_findings,
        generated_at,
    }))
}

/// Command handler for `InspectPodVulnerabilities`: runs the inspection, caches success in
/// `pod_vuln_inspect_cache`, and emits the success/failure event on the Pods route.
pub(super) async fn handle_inspect(
    state: &mut super::state::BackendState,
    sender: &crossbeam_channel::Sender<crate::protocol::BackendEventEnvelope>,
    namespace: &str,
    pod: &str,
) -> anyhow::Result<()> {
    use crate::protocol::{BackendEvent, Correlation, ResourceReadEvent, Route, section_keys};
    let client = state.cluster.client_or_err()?;
    let route = Route::namespaced(
        section_keys::workloads_pods(),
        state.cluster.current_context.clone(),
        Some(namespace.to_owned()),
    );
    let event = match inspect_pod_vulnerabilities(&client, namespace, pod).await? {
        Ok(inspection) => {
            state
                .pod_vuln_inspect_cache
                .insert(format!("{namespace}/{pod}"), inspection.clone());
            ResourceReadEvent::PodVulnerabilitiesInspected {
                namespace: namespace.to_owned(),
                pod: pod.to_owned(),
                inspection,
            }
        }
        Err(error) => ResourceReadEvent::PodVulnerabilitiesInspectFailed {
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            error,
        },
    };
    super::events::emit(
        state.cluster.seq.as_ref(),
        sender,
        route,
        Correlation::None,
        BackendEvent::ResourceRead(event),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
    use kube::core::ObjectMeta;
    use std::collections::BTreeMap;

    fn make_report(
        namespace: &str,
        name: &str,
        owner_kind: &str,
        owner_name: &str,
        container: &str,
        created: &str,
        severities: &[&str],
    ) -> DynamicObject {
        let mut labels = BTreeMap::new();
        labels.insert(
            "trivy-operator.resource.kind".to_owned(),
            owner_kind.to_owned(),
        );
        labels.insert(
            "trivy-operator.resource.name".to_owned(),
            owner_name.to_owned(),
        );
        labels.insert(
            "trivy-operator.container.name".to_owned(),
            container.to_owned(),
        );
        let vulns: Vec<Value> = severities
            .iter()
            .enumerate()
            .map(|(i, sev)| {
                serde_json::json!({
                    "vulnerabilityID": format!("CVE-2026-{i}"),
                    "severity": sev,
                    "resource": "openssl",
                    "installedVersion": "1.0.0",
                    "fixedVersion": "1.0.1",
                    "score": 7.5,
                })
            })
            .collect();
        DynamicObject {
            types: None,
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                namespace: Some(namespace.to_owned()),
                labels: Some(labels),
                creation_timestamp: Some(Time(created.parse().expect("valid RFC3339 timestamp"))),
                ..Default::default()
            },
            data: serde_json::json!({
                "report": {
                    "artifact": { "repository": "nginx" },
                    "vulnerabilities": vulns,
                }
            }),
        }
    }

    #[test]
    fn deployment_pod_matches_the_replicaset_report_trivy_writes() {
        let rs_report = make_report(
            "fathom-dev",
            "replicaset-web-64dbd76659-nginx",
            "ReplicaSet",
            "web-64dbd76659",
            "nginx",
            "2026-01-01T00:00:00Z",
            &["HIGH"],
        );
        let attrib = attribution(&rs_report);
        let direct = Some(("ReplicaSet", "web-64dbd76659"));
        let top = ("Deployment", "web");
        assert!(report_matches_pod(
            &attrib,
            "fathom-dev",
            "web-64dbd76659-jtdwx",
            direct,
            top
        ));
        // A different rollout's ReplicaSet, or another namespace, does not match.
        assert!(!report_matches_pod(
            &attrib,
            "fathom-dev",
            "web-597467fbc6-abcde",
            Some(("ReplicaSet", "web-597467fbc6")),
            top
        ));
        assert!(!report_matches_pod(
            &attrib,
            "other",
            "web-64dbd76659-jtdwx",
            direct,
            top
        ));
    }

    #[test]
    fn cronjob_pod_matches_the_cronjob_report_and_bare_pod_its_own() {
        let cj = attribution(&make_report(
            "ns",
            "cronjob-nightly-report-main",
            "CronJob",
            "nightly-report",
            "main",
            "2026-01-01T00:00:00Z",
            &[],
        ));
        assert!(report_matches_pod(
            &cj,
            "ns",
            "nightly-report-29000000-x",
            Some(("Job", "nightly-report-29000000")),
            ("CronJob", "nightly-report")
        ));
        let bare = attribution(&make_report(
            "ns",
            "pod-lonely-main",
            "Pod",
            "lonely",
            "main",
            "2026-01-01T00:00:00Z",
            &[],
        ));
        assert!(report_matches_pod(
            &bare,
            "ns",
            "lonely",
            None,
            ("Pod", "lonely")
        ));
    }

    #[test]
    fn findings_from_report_parses_severities_and_attribution() {
        let obj = make_report(
            "default",
            "r1",
            "Deployment",
            "web",
            "app",
            "2026-01-01T00:00:00Z",
            &["CRITICAL", "LOW"],
        );
        let findings = findings_from_report(&obj);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, Severity::Critical);
        assert_eq!(findings[1].severity, Severity::Low);
        assert_eq!(findings[0].container, "app");
        assert_eq!(findings[0].image, "nginx");
    }

    #[test]
    fn dedup_latest_keeps_newest_per_owner_container() {
        let old = make_report(
            "default",
            "r-old",
            "Deployment",
            "web",
            "app",
            "2026-01-01T00:00:00Z",
            &["LOW"],
        );
        let new = make_report(
            "default",
            "r-new",
            "Deployment",
            "web",
            "app",
            "2026-01-02T00:00:00Z",
            &["CRITICAL"],
        );
        let deduped = dedup_latest(vec![old, new]);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].metadata.name.as_deref(), Some("r-new"));
    }

    #[test]
    fn rank_worst_first_orders_by_severity_then_name() {
        let mut workloads = vec![
            WorkloadVulnSummary {
                namespace: "default".into(),
                kind: "Deployment".into(),
                name: "low-sev".into(),
                summary: SeveritySummary {
                    critical: 0,
                    high: 1,
                    medium: 0,
                    low: 0,
                    unknown: 0,
                },
                image_count: 1,
            },
            WorkloadVulnSummary {
                namespace: "default".into(),
                kind: "Deployment".into(),
                name: "high-sev".into(),
                summary: SeveritySummary {
                    critical: 3,
                    high: 0,
                    medium: 0,
                    low: 0,
                    unknown: 0,
                },
                image_count: 1,
            },
        ];
        rank_worst_first(&mut workloads);
        assert_eq!(workloads[0].name, "high-sev");
        assert_eq!(workloads[1].name, "low-sev");
    }

    #[test]
    fn summarize_workloads_aggregates_by_owner() {
        let a = make_report(
            "default",
            "r1",
            "Deployment",
            "web",
            "app",
            "2026-01-01T00:00:00Z",
            &["CRITICAL"],
        );
        let b = make_report(
            "default",
            "r2",
            "Deployment",
            "web",
            "sidecar",
            "2026-01-01T00:00:00Z",
            &["HIGH"],
        );
        let summaries = summarize_workloads(&[a, b]);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].summary.critical, 1);
        assert_eq!(summaries[0].summary.high, 1);
    }
}
