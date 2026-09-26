//! OPA Gatekeeper policy compliance (020-gatekeeper-policy-pane).
//!
//! Discovers Gatekeeper through API discovery rather than a full CRD LIST: `/apis` tells whether
//! the `templates.gatekeeper.sh` / `constraints.gatekeeper.sh` groups are served, and one
//! `/apis/constraints.gatekeeper.sh/<version>` call lists every constraint kind (one CRD per
//! kind, dynamically registered per `ConstraintTemplate`). Each kind's instances are then LISTed
//! concurrently and read for `.status.totalViolations`/`.status.violations` — no separate watch
//! stream, no new dependency.
//!
//! Why not list CRDs (as `flux.rs` does): a CRD LIST returns every CRD's full OpenAPI schema,
//! which on a cluster with a few operators installed (Argo CD, Cilium, Prometheus) is megabytes
//! and was most of the first scan's latency, and it needs cluster-wide `list` on CRDs, which
//! plain discovery does not. The per-kind LISTs also ran one after another.
//!
//! Deliberately read-only and scoped to display: this module has no create/update/delete
//! path for constraints, constraint templates, or violating resources (FR-009), and never
//! will — policy authoring/editing UI is explicitly out of scope (FR-011, Kubescape-scope,
//! deferred; see `docs/FEATURE_BACKLOG.md`).

use crate::protocol::{
    GatekeeperConstraintSummary, GatekeeperInstallState, GatekeeperOverview, GatekeeperViolation,
    SectionKey, section_keys,
};
use futures::future::join_all;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{APIGroupList, APIResourceList};
use kube::Error as KubeError;
use kube::api::ListParams;
use kube::core::DynamicObject;
use kube::core::{ApiResource, GroupVersionKind};
use kube::{Client, api::Api};
use serde_json::Value;

const TEMPLATE_GROUP: &str = "templates.gatekeeper.sh";
const CONSTRAINT_GROUP: &str = "constraints.gatekeeper.sh";

struct DiscoveredCrd {
    group: String,
    version: String,
    kind: String,
    /// The resource name from discovery (e.g. `k8srequiredlabels`). kube's guessed plural is
    /// wrong for kinds ending in "s" (`k8srequiredlabelses`), which 404ed the whole scan.
    plural: String,
}

/// The served version of `group` in the cluster's API group list, preferring the server's
/// preferred version; `None` when the group is not served at all.
fn served_version(groups: &APIGroupList, group: &str) -> Option<String> {
    let g = groups.groups.iter().find(|g| g.name == group)?;
    g.preferred_version
        .as_ref()
        .map(|v| v.version.clone())
        .or_else(|| g.versions.first().map(|v| v.version.clone()))
}

/// Every constraint kind (e.g. `K8sRequiredLabels`) served under
/// `constraints.gatekeeper.sh/<version>`, skipping subresources such as `k8srequiredlabels/status`
/// (research.md R2).
fn constraint_kinds_from(resources: &APIResourceList, version: &str) -> Vec<DiscoveredCrd> {
    let mut kinds: Vec<DiscoveredCrd> = resources
        .resources
        .iter()
        .filter(|r| !r.name.contains('/'))
        .map(|r| DiscoveredCrd {
            group: CONSTRAINT_GROUP.to_owned(),
            version: version.to_owned(),
            kind: r.kind.clone(),
            plural: r.name.clone(),
        })
        .collect();
    kinds.sort_by(|a, b| a.kind.cmp(&b.kind));
    kinds.dedup_by(|a, b| a.kind == b.kind);
    kinds
}

/// List every instance of a discovered CRD as dynamic objects, across all namespaces
/// (mirrors `flux.rs::list_instances`). Returns `Err` only for a listing permission error,
/// which callers turn into `GatekeeperInstallState::PermissionDenied`; any other error also
/// propagates so the background poll's usual `notify_error` path handles it.
async fn list_instances(
    client: &Client,
    crd: &DiscoveredCrd,
) -> Result<Vec<DynamicObject>, KubeError> {
    let gvk = GroupVersionKind::gvk(&crd.group, &crd.version, &crd.kind);
    let resource = ApiResource::from_gvk_with_plural(&gvk, &crd.plural);
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &resource);
    api.list(&ListParams::default()).await.map(|l| l.items)
}

fn is_forbidden(err: &KubeError) -> bool {
    matches!(err, KubeError::Api(resp) if resp.code == 403)
}

/// Map a violating resource's `kind` to the `SectionKey` its Inspector lives under (research.md
/// R4), mirroring `applications.rs::workload_owns_ref`'s match-based approach. Falls back to
/// `custom_resources()` for any kind not covered here (built-in kinds Gatekeeper doesn't
/// commonly target, and any CRD-instance violation).
fn section_for_kind(kind: &str) -> SectionKey {
    match kind {
        "Namespace" => section_keys::namespaces(),
        "Pod" => section_keys::workloads_pods(),
        "Deployment" => section_keys::workloads_deployments(),
        "StatefulSet" => section_keys::workloads_statefulsets(),
        "DaemonSet" => section_keys::workloads_daemonsets(),
        "Job" => section_keys::workloads_jobs(),
        "CronJob" => section_keys::workloads_cronjobs(),
        "ReplicaSet" => section_keys::workloads_replicasets(),
        "ConfigMap" => section_keys::config_configmaps(),
        "Secret" => section_keys::config_secrets(),
        "Service" => section_keys::network_services(),
        "Ingress" => section_keys::network_ingress(),
        "PersistentVolume" => section_keys::storage_pv(),
        "PersistentVolumeClaim" => section_keys::storage_pvc(),
        _ => section_keys::custom_resources(),
    }
}

/// Build a [`GatekeeperViolation`] from one entry of `.status.violations[]`.
fn summarize_violation(entry: &Value, constraint_enforcement_action: &str) -> GatekeeperViolation {
    let kind = entry
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let namespace = entry
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let name = entry
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let message = entry
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let enforcement_action = entry
        .get("enforcementAction")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(constraint_enforcement_action)
        .to_owned();

    GatekeeperViolation {
        section: section_for_kind(&kind),
        kind,
        namespace,
        name,
        enforcement_action,
        message,
    }
}

/// Build a [`GatekeeperConstraintSummary`] from one `Constraint` dynamic object.
fn summarize_constraint(
    obj: &DynamicObject,
    kind: &str,
    known_constraint_kinds: &[String],
) -> GatekeeperConstraintSummary {
    let name = obj.metadata.name.clone().unwrap_or_default();
    let spec = obj.data.get("spec");
    let status = obj.data.get("status");

    let enforcement_action = spec
        .and_then(|s| s.get("enforcementAction"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    let violations: Vec<GatekeeperViolation> = status
        .and_then(|s| s.get("violations"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|entry| summarize_violation(entry, &enforcement_action))
                .collect()
        })
        .unwrap_or_default();

    // `.status` absent entirely => "awaiting status" (None). `.status` present but
    // `totalViolations` absent => fall back to the violations array length. Never coerce
    // an absent status to `Some(0)` (FR-004).
    let total_violations: Option<u64> = status.map(|s| {
        s.get("totalViolations")
            .and_then(|v| v.as_u64())
            .unwrap_or(violations.len() as u64)
    });

    let violations_truncated = total_violations.is_some_and(|t| t > violations.len() as u64);

    // Orphaned-constraint edge case: this constraint's kind has no corresponding
    // `constraints.gatekeeper.sh`-group CRD among those discovered in the same scan (its
    // `ConstraintTemplate` was deleted after the CRD it generated was also removed, or the
    // CRD is mid-teardown). In the common case the constraint kind was itself discovered by
    // walking the same CRD list used to find `known_constraint_kinds`, so this only fires for
    // constraints resolved through some other path; kept as an explicit, testable flag rather
    // than silently assumed to always be `false`.
    let template_missing = !known_constraint_kinds.iter().any(|k| k == kind);

    GatekeeperConstraintSummary {
        kind: kind.to_owned(),
        name,
        enforcement_action,
        total_violations,
        violations,
        violations_truncated,
        template_missing,
    }
}

/// Upper bound on one scan, so a hung API server ends in `ScanFailed` instead of leaving the pane
/// on "Scanning".
const SCAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// [`scan_gatekeeper`], bounded by [`SCAN_TIMEOUT`], with any failure folded into a
/// `GatekeeperInstallState::ScanFailed` overview carrying the verbatim error. The `Err` side is
/// the same text, for callers that also report the failure elsewhere (a command's result).
pub(super) async fn scan_gatekeeper_overview(
    client: &Client,
) -> Result<GatekeeperOverview, (GatekeeperOverview, String)> {
    let result = match tokio::time::timeout(SCAN_TIMEOUT, scan_gatekeeper(client)).await {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!(
            "Gatekeeper scan timed out after {}s",
            SCAN_TIMEOUT.as_secs()
        )),
    };
    result.map_err(|e| {
        let message = format!("{e:#}");
        (GatekeeperOverview::scan_failed(message.clone()), message)
    })
}

/// Discover OPA Gatekeeper and build a full [`GatekeeperOverview`]. Returns
/// `GatekeeperInstallState::NotInstalled` (empty `constraints`) when neither the
/// `templates.gatekeeper.sh` nor the `constraints.gatekeeper.sh` API group is served, and
/// `GatekeeperInstallState::PermissionDenied` (with the verbatim error) when Gatekeeper is present
/// but listing `Constraint` instances is forbidden by RBAC. Any other failure is an `Err`, which
/// callers report as `GatekeeperInstallState::ScanFailed`.
pub(super) async fn scan_gatekeeper(client: &Client) -> anyhow::Result<GatekeeperOverview> {
    let groups = client.list_api_groups().await?;
    let template_served = served_version(&groups, TEMPLATE_GROUP).is_some();
    let constraint_kinds = match served_version(&groups, CONSTRAINT_GROUP) {
        Some(version) => {
            let resources = client
                .list_api_group_resources(&format!("{CONSTRAINT_GROUP}/{version}"))
                .await?;
            constraint_kinds_from(&resources, &version)
        }
        None => Vec::new(),
    };

    if !template_served && constraint_kinds.is_empty() {
        return Ok(GatekeeperOverview::default());
    }

    let known_kinds: Vec<String> = constraint_kinds.iter().map(|c| c.kind.clone()).collect();

    // One LIST per constraint kind, all in flight at once (they are independent).
    let listed = join_all(
        constraint_kinds
            .iter()
            .map(|crd| async move { (crd, list_instances(client, crd).await) }),
    )
    .await;

    let mut constraints = Vec::new();
    for (crd, result) in listed {
        match result {
            Ok(instances) => {
                for obj in &instances {
                    constraints.push(summarize_constraint(obj, &crd.kind, &known_kinds));
                }
            }
            Err(err) if is_forbidden(&err) => {
                return Ok(GatekeeperOverview {
                    state: GatekeeperInstallState::PermissionDenied,
                    error: err.to_string(),
                    constraints: Vec::new(),
                });
            }
            Err(err) => return Err(err.into()),
        }
    }

    constraints.sort_by(|a, b| {
        (a.kind.as_str(), a.name.as_str()).cmp(&(b.kind.as_str(), b.name.as_str()))
    });

    let state = if constraints.is_empty() {
        GatekeeperInstallState::InstalledNoConstraints
    } else {
        GatekeeperInstallState::Active
    };

    Ok(GatekeeperOverview {
        state,
        error: String::new(),
        constraints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dynamic_object(name: &str, data: Value) -> DynamicObject {
        let mut obj: DynamicObject = serde_json::from_value(json!({
            "apiVersion": "constraints.gatekeeper.sh/v1beta1",
            "kind": "K8sRequiredLabels",
            "metadata": { "name": name },
        }))
        .expect("base dynamic object");
        if let Value::Object(map) = data {
            for (k, v) in map {
                obj.data.as_object_mut().expect("object data").insert(k, v);
            }
        }
        obj
    }

    #[test]
    fn awaiting_status_when_status_absent() {
        let obj = dynamic_object(
            "require-team-label",
            json!({ "spec": { "enforcementAction": "deny" } }),
        );
        let summary =
            summarize_constraint(&obj, "K8sRequiredLabels", &["K8sRequiredLabels".to_owned()]);
        assert_eq!(summary.total_violations, None);
        assert!(summary.violations.is_empty());
        assert!(!summary.violations_truncated);
        assert!(!summary.template_missing);
    }

    #[test]
    fn zero_violations_when_status_present_but_empty() {
        let obj = dynamic_object(
            "require-team-label",
            json!({ "spec": { "enforcementAction": "deny" }, "status": { "totalViolations": 0, "violations": [] } }),
        );
        let summary =
            summarize_constraint(&obj, "K8sRequiredLabels", &["K8sRequiredLabels".to_owned()]);
        assert_eq!(summary.total_violations, Some(0));
        assert!(summary.violations.is_empty());
        assert!(!summary.violations_truncated);
    }

    #[test]
    fn truncation_detected_when_total_exceeds_reported_violations() {
        let obj = dynamic_object(
            "require-team-label",
            json!({
                "spec": { "enforcementAction": "deny" },
                "status": {
                    "totalViolations": 143,
                    "violations": [
                        { "kind": "Namespace", "name": "team-a", "message": "missing label" }
                    ]
                }
            }),
        );
        let summary =
            summarize_constraint(&obj, "K8sRequiredLabels", &["K8sRequiredLabels".to_owned()]);
        assert_eq!(summary.total_violations, Some(143));
        assert_eq!(summary.violations.len(), 1);
        assert!(summary.violations_truncated);
    }

    #[test]
    fn violation_enforcement_action_falls_back_to_constraint_level() {
        let obj = dynamic_object(
            "require-team-label",
            json!({
                "spec": { "enforcementAction": "deny" },
                "status": {
                    "totalViolations": 1,
                    "violations": [
                        { "kind": "Namespace", "name": "team-a", "message": "missing label" }
                    ]
                }
            }),
        );
        let summary =
            summarize_constraint(&obj, "K8sRequiredLabels", &["K8sRequiredLabels".to_owned()]);
        assert_eq!(summary.violations[0].enforcement_action, "deny");
    }

    #[test]
    fn violation_kind_maps_to_known_section() {
        let v = summarize_violation(
            &json!({ "kind": "Pod", "namespace": "default", "name": "nginx" }),
            "deny",
        );
        assert_eq!(v.section, section_keys::workloads_pods());
    }

    #[test]
    fn violation_kind_falls_back_to_custom_resources() {
        let v = summarize_violation(
            &json!({ "kind": "SomeCustomThing", "namespace": "default", "name": "x" }),
            "deny",
        );
        assert_eq!(v.section, section_keys::custom_resources());
    }

    #[test]
    fn template_missing_flagged_when_kind_not_in_known_list() {
        let obj = dynamic_object("orphaned", json!({}));
        let summary =
            summarize_constraint(&obj, "K8sOrphanedKind", &["K8sRequiredLabels".to_owned()]);
        assert!(summary.template_missing);
    }

    #[test]
    fn served_version_prefers_the_preferred_version() {
        let groups: APIGroupList = serde_json::from_value(json!({
            "groups": [
                {
                    "name": "constraints.gatekeeper.sh",
                    "versions": [
                        { "groupVersion": "constraints.gatekeeper.sh/v1alpha1", "version": "v1alpha1" },
                        { "groupVersion": "constraints.gatekeeper.sh/v1beta1", "version": "v1beta1" }
                    ],
                    "preferredVersion": { "groupVersion": "constraints.gatekeeper.sh/v1beta1", "version": "v1beta1" }
                },
                {
                    "name": "templates.gatekeeper.sh",
                    "versions": [ { "groupVersion": "templates.gatekeeper.sh/v1", "version": "v1" } ]
                }
            ]
        }))
        .expect("group list");
        assert_eq!(
            served_version(&groups, CONSTRAINT_GROUP).as_deref(),
            Some("v1beta1")
        );
        assert_eq!(
            served_version(&groups, TEMPLATE_GROUP).as_deref(),
            Some("v1")
        );
        assert_eq!(served_version(&groups, "fluxcd.io"), None);
    }

    #[test]
    fn constraint_kinds_skip_subresources() {
        let resources: APIResourceList = serde_json::from_value(json!({
            "groupVersion": "constraints.gatekeeper.sh/v1beta1",
            "resources": [
                { "name": "k8srequiredlabels", "singularName": "", "namespaced": false, "kind": "K8sRequiredLabels", "verbs": ["list"] },
                { "name": "k8srequiredlabels/status", "singularName": "", "namespaced": false, "kind": "K8sRequiredLabels", "verbs": ["get"] },
                { "name": "k8sallowedrepos", "singularName": "", "namespaced": false, "kind": "K8sAllowedRepos", "verbs": ["list"] }
            ]
        }))
        .expect("resource list");
        let kinds: Vec<String> = constraint_kinds_from(&resources, "v1beta1")
            .into_iter()
            .map(|k| format!("{}/{}/{}", k.group, k.version, k.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                "constraints.gatekeeper.sh/v1beta1/K8sAllowedRepos",
                "constraints.gatekeeper.sh/v1beta1/K8sRequiredLabels",
            ]
        );
    }

    /// A fake API server answering from `route(path)` (status, JSON body).
    fn fake_client(route: fn(&str) -> (u16, Value)) -> Client {
        let service = tower::service_fn(move |req: http::Request<kube::client::Body>| {
            let (status, body) = route(req.uri().path());
            async move {
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(status)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(body.to_string().into_bytes()))
                        .expect("response"),
                )
            }
        });
        Client::new(service, "default")
    }

    fn gatekeeper_api(path: &str) -> (u16, Value) {
        match path {
            "/apis" => (
                200,
                json!({ "groups": [
                    { "name": "templates.gatekeeper.sh",
                      "versions": [ { "groupVersion": "templates.gatekeeper.sh/v1", "version": "v1" } ] },
                    { "name": "constraints.gatekeeper.sh",
                      "versions": [ { "groupVersion": "constraints.gatekeeper.sh/v1beta1", "version": "v1beta1" } ] }
                ] }),
            ),
            "/apis/constraints.gatekeeper.sh/v1beta1" => (
                200,
                json!({ "groupVersion": "constraints.gatekeeper.sh/v1beta1", "resources": [
                    { "name": "k8srequiredlabels", "singularName": "", "namespaced": false, "kind": "K8sRequiredLabels", "verbs": ["list"] },
                    { "name": "k8sallowedrepos", "singularName": "", "namespaced": false, "kind": "K8sAllowedRepos", "verbs": ["list"] }
                ] }),
            ),
            "/apis/constraints.gatekeeper.sh/v1beta1/k8srequiredlabels" => (
                200,
                json!({ "apiVersion": "constraints.gatekeeper.sh/v1beta1", "kind": "K8sRequiredLabelsList",
                        "metadata": {}, "items": [
                    { "apiVersion": "constraints.gatekeeper.sh/v1beta1", "kind": "K8sRequiredLabels",
                      "metadata": { "name": "require-team" },
                      "spec": { "enforcementAction": "deny" },
                      "status": { "totalViolations": 0 } }
                ] }),
            ),
            "/apis/constraints.gatekeeper.sh/v1beta1/k8sallowedrepos" => (
                500,
                json!({ "kind": "Status", "apiVersion": "v1", "status": "Failure",
                        "message": "etcdserver: request timed out", "reason": "InternalError", "code": 500 }),
            ),
            other => (
                404,
                json!({ "kind": "Status", "code": 404, "status": "Failure", "message": format!("not found: {other}") }),
            ),
        }
    }

    #[tokio::test]
    async fn scan_discovers_kinds_without_listing_crds() {
        fn route(path: &str) -> (u16, Value) {
            // Every constraint kind lists fine here.
            if path.ends_with("/k8sallowedrepos") {
                return (
                    200,
                    json!({ "apiVersion": "constraints.gatekeeper.sh/v1beta1", "kind": "List",
                            "metadata": {}, "items": [] }),
                );
            }
            assert!(
                !path.contains("customresourcedefinitions"),
                "the scan must not LIST CRDs"
            );
            gatekeeper_api(path)
        }
        let overview = scan_gatekeeper(&fake_client(route)).await.expect("scan");
        assert_eq!(overview.state, GatekeeperInstallState::Active);
        assert_eq!(overview.constraints.len(), 1);
        assert_eq!(overview.constraints[0].name, "require-team");
        assert!(!overview.constraints[0].template_missing);
    }

    #[tokio::test]
    async fn scan_reports_not_installed_when_groups_are_absent() {
        fn route(path: &str) -> (u16, Value) {
            match path {
                "/apis" => (200, json!({ "groups": [] })),
                other => panic!("unexpected request {other}"),
            }
        }
        let overview = scan_gatekeeper(&fake_client(route)).await.expect("scan");
        assert_eq!(overview.state, GatekeeperInstallState::NotInstalled);
    }

    #[tokio::test]
    async fn failed_scan_becomes_a_scan_failed_overview() {
        let (overview, message) = scan_gatekeeper_overview(&fake_client(gatekeeper_api))
            .await
            .expect_err("a 500 on one constraint kind fails the scan");
        assert_eq!(overview.state, GatekeeperInstallState::ScanFailed);
        assert!(overview.error.contains("etcdserver: request timed out"));
        assert_eq!(overview.error, message);
        assert!(overview.constraints.is_empty());
    }
}

#[cfg(test)]
mod live_cluster_tests {
    use super::*;

    /// Not run in CI - requires a real cluster with Gatekeeper installed and a working
    /// kubeconfig. `cargo test -p fathom scan_gatekeeper_against_live_cluster -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn scan_gatekeeper_against_live_cluster() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::try_default().await.expect("kube client");
        let overview = scan_gatekeeper(&client).await.expect("scan_gatekeeper");

        println!("state: {:?}", overview.state);
        for c in &overview.constraints {
            println!(
                "  [{}] {} enforcement={} total_violations={:?} truncated={} template_missing={}",
                c.kind,
                c.name,
                c.enforcement_action,
                c.total_violations,
                c.violations_truncated,
                c.template_missing
            );
        }

        assert!(matches!(overview.state, GatekeeperInstallState::Active));
        assert!(!overview.constraints.is_empty());
    }
}
