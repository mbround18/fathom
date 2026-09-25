//! OPA Gatekeeper policy compliance (020-gatekeeper-policy-pane).
//!
//! Discovers Gatekeeper's CRDs the same way `flux.rs` discovers Flux's: list every
//! `CustomResourceDefinition` in the cluster, look for `constrainttemplates.templates.gatekeeper.sh`
//! and any served CRD in the `constraints.gatekeeper.sh` API group (one CRD per constraint kind,
//! dynamically registered per `ConstraintTemplate`), then read each `Constraint` instance's
//! `.status.totalViolations`/`.status.violations` — no separate watch stream, no new dependency.
//!
//! Deliberately read-only and scoped to display: this module has no create/update/delete
//! path for constraints, constraint templates, or violating resources (FR-009), and never
//! will — policy authoring/editing UI is explicitly out of scope (FR-011, Kubescape-scope,
//! deferred; see `docs/FEATURE_BACKLOG.md`).

use crate::protocol::{
    GatekeeperConstraintSummary, GatekeeperInstallState, GatekeeperOverview, GatekeeperViolation,
    SectionKey, section_keys,
};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::Error as KubeError;
use kube::api::ListParams;
use kube::core::DynamicObject;
use kube::core::{ApiResource, GroupVersionKind};
use kube::{Client, api::Api};
use serde_json::Value;

const CONSTRAINT_TEMPLATE_CRD: &str = "constrainttemplates.templates.gatekeeper.sh";
const CONSTRAINT_GROUP: &str = "constraints.gatekeeper.sh";

struct DiscoveredCrd {
    group: String,
    version: String,
    kind: String,
}

/// Find a served version for `crd_name` among already-listed CRDs (mirrors `flux.rs::find_served`).
fn find_served(crds: &[CustomResourceDefinition], crd_name: &str) -> Option<DiscoveredCrd> {
    let crd = crds
        .iter()
        .find(|c| c.metadata.name.as_deref() == Some(crd_name))?;
    let group = crd.spec.group.clone();
    let version = crd
        .spec
        .versions
        .iter()
        .find(|v| v.served)
        .map(|v| v.name.clone())?;
    let kind = crd.spec.names.kind.clone();
    Some(DiscoveredCrd {
        group,
        version,
        kind,
    })
}

/// Every served CRD whose API group is `constraints.gatekeeper.sh` — each one is a distinct
/// constraint kind (e.g. `K8sRequiredLabels`), dynamically registered by Gatekeeper per
/// `ConstraintTemplate` (research.md R2).
fn find_constraint_kinds(crds: &[CustomResourceDefinition]) -> Vec<DiscoveredCrd> {
    crds.iter()
        .filter(|c| c.spec.group == CONSTRAINT_GROUP)
        .filter_map(|c| {
            let version = c.spec.versions.iter().find(|v| v.served)?.name.clone();
            Some(DiscoveredCrd {
                group: c.spec.group.clone(),
                version,
                kind: c.spec.names.kind.clone(),
            })
        })
        .collect()
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
    let resource = ApiResource::from_gvk(&gvk);
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

/// Discover OPA Gatekeeper's CRDs and build a full [`GatekeeperOverview`]. Returns
/// `GatekeeperInstallState::NotInstalled` (empty `constraints`) when neither the
/// `ConstraintTemplate` CRD nor any `constraints.gatekeeper.sh`-group CRD is found, and
/// `GatekeeperInstallState::PermissionDenied` (with the verbatim error) when CRDs are present
/// but listing `Constraint` instances is forbidden by RBAC.
pub(super) async fn scan_gatekeeper(client: &Client) -> anyhow::Result<GatekeeperOverview> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&ListParams::default()).await?.items;

    let template_crd = find_served(&crds, CONSTRAINT_TEMPLATE_CRD);
    let constraint_kinds = find_constraint_kinds(&crds);

    if template_crd.is_none() && constraint_kinds.is_empty() {
        return Ok(GatekeeperOverview {
            state: GatekeeperInstallState::NotInstalled,
            error: String::new(),
            constraints: Vec::new(),
        });
    }

    let known_kinds: Vec<String> = constraint_kinds.iter().map(|c| c.kind.clone()).collect();

    let mut constraints = Vec::new();
    for crd in &constraint_kinds {
        match list_instances(client, crd).await {
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
