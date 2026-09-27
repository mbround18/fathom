//! Knative Serving integration (028-knative-pane).
//!
//! Discovers whichever served API version of the `serving.knative.dev` CRDs is installed
//! (Service, Revision, Route, Configuration) and reads them as dynamic objects — the same
//! discovery pattern `flux.rs` uses for the Flux GitOps Toolkit, so this works across Knative
//! versions without pinning an API version. Read-only: this module never mutates a Knative
//! object.

use super::served_crd::{ServedCrd, find_served};
use crate::protocol::{
    KnativeOverview, KnativeReadyState, KnativeRevisionSummary, KnativeRouteSummary,
    KnativeRouteTrafficTarget, KnativeScaleReason, KnativeServiceSummary,
};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::ListParams;
use kube::core::DynamicObject;
use kube::{Client, api::Api};
use serde_json::Value;
use std::collections::HashSet;

const SERVICE_CRD: &str = "services.serving.knative.dev";
const REVISION_CRD: &str = "revisions.serving.knative.dev";
const ROUTE_CRD: &str = "routes.serving.knative.dev";
const CONFIGURATION_CRD: &str = "configurations.serving.knative.dev";

/// Label Knative sets on every object it owns, naming the parent Service.
const SERVICE_LABEL: &str = "serving.knative.dev/service";

/// List every instance of a discovered CRD as dynamic objects, across all namespaces.
async fn list_instances(client: &Client, crd: &ServedCrd) -> Vec<DynamicObject> {
    let resource = crd.api_resource();
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &resource);
    api.list(&ListParams::default())
        .await
        .map(|l| l.items)
        .unwrap_or_default()
}

/// Find the `Ready` condition in `status.conditions` and return its `status` field
/// (`"True" | "False" | "Unknown"` or absent).
fn ready_state(status: Option<&Value>) -> KnativeReadyState {
    let ready = status
        .and_then(|s| s.get("conditions"))
        .and_then(|c| c.as_array())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|c| c.get("type").and_then(|t| t.as_str()) == Some("Ready"))
        })
        .and_then(|cond| cond.get("status"))
        .and_then(|v| v.as_str());
    match ready {
        Some("True") => KnativeReadyState::Ready,
        Some("False") => KnativeReadyState::NotReady,
        _ => KnativeReadyState::Unknown,
    }
}

fn owning_service_name(obj: &DynamicObject) -> String {
    obj.metadata
        .labels
        .as_ref()
        .and_then(|l| l.get(SERVICE_LABEL))
        .cloned()
        .or_else(|| {
            obj.metadata.owner_references.as_ref().and_then(|refs| {
                refs.iter()
                    .find(|r| r.kind == "Service" || r.kind == "Configuration")
                    .map(|r| r.name.clone())
            })
        })
        .unwrap_or_default()
}

/// Derive [`KnativeScaleReason`] from a Revision's `status`, per research.md R3:
/// 1. The `Active` condition's `status`/`reason` (scaled-to-zero / scaling-up), when present.
/// 2. Falling back to comparing `actualReplicas`/`desiredReplicas` when the condition is absent
///    or unrecognized.
///
/// `actual_replicas == desired_replicas` always wins as `AtDesiredScale` (FR-009) — a mismatch is
/// only explained when the counts actually differ.
fn resolve_scale_reason(
    status: &Value,
    actual_replicas: u32,
    desired_replicas: u32,
) -> KnativeScaleReason {
    if actual_replicas == desired_replicas {
        return KnativeScaleReason::AtDesiredScale;
    }

    let active = status
        .get("conditions")
        .and_then(|c| c.as_array())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|c| c.get("type").and_then(|t| t.as_str()) == Some("Active"))
        });

    if let Some(cond) = active {
        let cond_status = cond.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let reason = cond
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let message = cond
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let idle_since = cond
            .get("lastTransitionTime")
            .and_then(|v| v.as_str())
            .map(str::to_owned);

        let mentions_inactivity = reason.contains("notraffic")
            || reason.contains("inactive")
            || message.contains("no traffic");
        let mentions_activating = reason.contains("activating")
            || reason.contains("progressdeadline")
            || message.contains("pending");

        if cond_status == "False" && mentions_inactivity {
            return KnativeScaleReason::ScaledToZero { idle_since };
        }
        if cond_status == "Unknown" && (mentions_activating || cond_status == "Unknown") {
            let pending_requests = status
                .get("pendingRequests")
                .or_else(|| status.get("pendingRequestCount"))
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            return KnativeScaleReason::ScalingUp { pending_requests };
        }
    }

    // Replica counts differ but nothing recognizable classified it — be honest, don't guess.
    KnativeScaleReason::Unavailable
}

/// Resolve a Route's `status.traffic[]` into [`KnativeRouteTrafficTarget`]s. Percentages are
/// displayed exactly as read — never renormalized to sum to 100 (spec.md edge case).
fn resolve_route_traffic(
    status: Option<&Value>,
    known_revision_names: &HashSet<String>,
) -> Vec<KnativeRouteTrafficTarget> {
    status
        .and_then(|s| s.get("traffic"))
        .and_then(|t| t.as_array())
        .map(|targets| {
            targets
                .iter()
                .filter_map(|t| {
                    let revision_name = t.get("revisionName").and_then(|v| v.as_str())?.to_owned();
                    let percent = t.get("percent").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    let tag = t.get("tag").and_then(|v| v.as_str()).map(str::to_owned);
                    let revision_missing = !known_revision_names.contains(&revision_name);
                    Some(KnativeRouteTrafficTarget {
                        revision_name,
                        percent,
                        tag,
                        revision_missing,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn summarize_service(obj: &DynamicObject) -> KnativeServiceSummary {
    let name = obj.metadata.name.clone().unwrap_or_default();
    let namespace = obj.metadata.namespace.clone().unwrap_or_default();
    let status = obj.data.get("status");
    let ready = ready_state(status);
    let latest_ready_revision = status
        .and_then(|s| s.get("latestReadyRevisionName"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let latest_created_revision = status
        .and_then(|s| s.get("latestCreatedRevisionName"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    KnativeServiceSummary {
        name,
        namespace,
        ready,
        latest_ready_revision,
        latest_created_revision,
    }
}

fn summarize_revision(obj: &DynamicObject) -> KnativeRevisionSummary {
    let name = obj.metadata.name.clone().unwrap_or_default();
    let namespace = obj.metadata.namespace.clone().unwrap_or_default();
    let service_name = owning_service_name(obj);
    let status = obj.data.get("status");
    let ready = ready_state(status);

    let actual_replicas = status
        .and_then(|s| s.get("actualReplicas"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let desired_replicas = status
        .and_then(|s| s.get("desiredReplicas"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let scale_reason = status
        .map(|s| resolve_scale_reason(s, actual_replicas, desired_replicas))
        .unwrap_or(KnativeScaleReason::Unavailable);

    KnativeRevisionSummary {
        name,
        namespace,
        service_name,
        ready,
        actual_replicas,
        desired_replicas,
        scale_reason,
        traffic_percent: 0, // filled in by `scan_knative` once all Routes are known
    }
}

fn summarize_route(
    obj: &DynamicObject,
    known_revision_names: &HashSet<String>,
) -> KnativeRouteSummary {
    let name = obj.metadata.name.clone().unwrap_or_default();
    let namespace = obj.metadata.namespace.clone().unwrap_or_default();
    let status = obj.data.get("status");
    let ready = ready_state(status);
    let url = status
        .and_then(|s| s.get("url"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let service_name = owning_service_name(obj);
    let traffic = resolve_route_traffic(status, known_revision_names);

    KnativeRouteSummary {
        name,
        namespace,
        service_name,
        ready,
        url,
        traffic,
    }
}

/// Discover the Knative Serving CRDs installed in this cluster and build a full overview.
/// Returns `installed: false` (with empty lists) only when none of the four CRDs are found
/// served, so the UI can show an honest "not installed" empty state (FR-001/FR-002).
pub(super) async fn scan_knative(client: &Client) -> anyhow::Result<KnativeOverview> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&ListParams::default()).await?.items;

    let service_crd = find_served(&crds, SERVICE_CRD);
    let revision_crd = find_served(&crds, REVISION_CRD);
    let route_crd = find_served(&crds, ROUTE_CRD);
    let configuration_crd = find_served(&crds, CONFIGURATION_CRD);

    let installed = service_crd.is_some()
        || revision_crd.is_some()
        || route_crd.is_some()
        || configuration_crd.is_some();

    let mut services: Vec<KnativeServiceSummary> = Vec::new();
    if let Some(discovered) = &service_crd {
        for obj in list_instances(client, discovered).await {
            services.push(summarize_service(&obj));
        }
    }

    let mut revisions: Vec<KnativeRevisionSummary> = Vec::new();
    if let Some(discovered) = &revision_crd {
        for obj in list_instances(client, discovered).await {
            revisions.push(summarize_revision(&obj));
        }
    }

    let known_revision_names: HashSet<String> = revisions.iter().map(|r| r.name.clone()).collect();

    let mut routes: Vec<KnativeRouteSummary> = Vec::new();
    if let Some(discovered) = &route_crd {
        for obj in list_instances(client, discovered).await {
            routes.push(summarize_route(&obj, &known_revision_names));
        }
    }

    // Denormalize each Revision's total traffic share across every Route target that resolves
    // to it (data-model.md `KnativeRevisionSummary.traffic_percent`).
    for revision in &mut revisions {
        revision.traffic_percent = routes
            .iter()
            .flat_map(|r| &r.traffic)
            .filter(|t| t.revision_name == revision.name)
            .map(|t| t.percent)
            .sum();
    }

    services.sort_by(|a, b| {
        (a.namespace.as_str(), a.name.as_str()).cmp(&(b.namespace.as_str(), b.name.as_str()))
    });
    revisions.sort_by(|a, b| {
        (a.namespace.as_str(), a.name.as_str()).cmp(&(b.namespace.as_str(), b.name.as_str()))
    });
    routes.sort_by(|a, b| {
        (a.namespace.as_str(), a.name.as_str()).cmp(&(b.namespace.as_str(), b.name.as_str()))
    });

    Ok(KnativeOverview {
        installed,
        services,
        revisions,
        routes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scale_reason_scaled_to_zero() {
        let status = json!({
            "actualReplicas": 0,
            "desiredReplicas": 1,
            "conditions": [
                {"type": "Active", "status": "False", "reason": "NoTraffic", "message": "The target is not receiving traffic.", "lastTransitionTime": "2026-09-23T00:00:00Z"}
            ]
        });
        let reason = resolve_scale_reason(&status, 0, 1);
        assert_eq!(
            reason,
            KnativeScaleReason::ScaledToZero {
                idle_since: Some("2026-09-23T00:00:00Z".to_owned())
            }
        );
    }

    #[test]
    fn scale_reason_scaling_up() {
        let status = json!({
            "actualReplicas": 0,
            "desiredReplicas": 2,
            "conditions": [
                {"type": "Active", "status": "Unknown", "reason": "Activating", "message": "activating"}
            ],
            "pendingRequests": 5
        });
        let reason = resolve_scale_reason(&status, 0, 2);
        assert_eq!(
            reason,
            KnativeScaleReason::ScalingUp {
                pending_requests: Some(5)
            }
        );
    }

    #[test]
    fn scale_reason_at_desired_scale() {
        let status = json!({ "actualReplicas": 2, "desiredReplicas": 2 });
        assert_eq!(
            resolve_scale_reason(&status, 2, 2),
            KnativeScaleReason::AtDesiredScale
        );
    }

    #[test]
    fn scale_reason_unavailable_when_unrecognized() {
        let status = json!({ "actualReplicas": 1, "desiredReplicas": 3 });
        assert_eq!(
            resolve_scale_reason(&status, 1, 3),
            KnativeScaleReason::Unavailable
        );
    }

    #[test]
    fn route_traffic_two_way_split() {
        let status = json!({
            "traffic": [
                {"revisionName": "hello-00001", "percent": 80},
                {"revisionName": "hello-00002", "percent": 20}
            ]
        });
        let known: HashSet<String> = ["hello-00001".to_owned(), "hello-00002".to_owned()]
            .into_iter()
            .collect();
        let targets = resolve_route_traffic(Some(&status), &known);
        assert_eq!(targets.len(), 2);
        let total: u32 = targets.iter().map(|t| t.percent).sum();
        assert_eq!(total, 100);
        assert!(targets.iter().all(|t| !t.revision_missing));
    }

    #[test]
    fn route_traffic_single_target() {
        let status = json!({ "traffic": [{"revisionName": "hello-00001", "percent": 100}] });
        let known: HashSet<String> = ["hello-00001".to_owned()].into_iter().collect();
        let targets = resolve_route_traffic(Some(&status), &known);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].percent, 100);
    }

    #[test]
    fn route_traffic_stale_target_flagged_not_dropped() {
        let status = json!({ "traffic": [{"revisionName": "ghost-00099", "percent": 100}] });
        let known: HashSet<String> = ["hello-00001".to_owned()].into_iter().collect();
        let targets = resolve_route_traffic(Some(&status), &known);
        assert_eq!(targets.len(), 1);
        assert!(targets[0].revision_missing);
    }

    /// Not run in CI - requires a real cluster with Knative Serving installed and a working
    /// kubeconfig. `cargo test -p fathom scan_knative_against_live_cluster -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn scan_knative_against_live_cluster() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::try_default().await.expect("kube client");
        let overview = scan_knative(&client).await.expect("scan_knative");

        println!("installed: {}", overview.installed);
        println!("services: {}", overview.services.len());
        println!("revisions: {}", overview.revisions.len());
        println!("routes: {}", overview.routes.len());

        assert!(overview.installed);
    }
}
