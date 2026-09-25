//! Generic related-resources resolver (spec 004, User Story 3; research.md R5,
//! data-model.md's relationship vocabulary table). One dispatch point, keyed by kind, used by
//! every inspector that has a defined relationship not already inlined on its own `SelectX`
//! details (`RefreshRelatedResources`, contracts/protocol.md).

use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::core::v1::{Node, Pod, Service};
use kube::{
    Client, ResourceExt,
    api::{Api, ListParams},
};

use crate::protocol::{RelatedResourceRef, section_keys};

use super::state::BackendState;

/// `RefreshRelatedResources`: dispatch by kind, per data-model.md's relationship vocabulary.
pub(super) async fn related_resources(
    state: &BackendState,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
) -> anyhow::Result<Vec<RelatedResourceRef>> {
    let client = state.cluster.client_or_err()?;
    match kind {
        "Node" => related_for_node(&client, name).await,
        "Service" => related_for_service(&client, namespace, name).await,
        "PersistentVolumeClaim" => related_for_pvc(&client, namespace, name).await,
        "Pod" => related_for_pod(&client, namespace, name).await,
        "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" => {
            related_for_workload(&client, kind, namespace, name).await
        }
        "ScaledObject" | "ScaledJob" => related_for_scaler(&client, kind, namespace, name).await,
        "Application" => related_for_application(state, namespace, name).await,
        "HelmRelease" => {
            let Some(namespace) = namespace else {
                return Ok(Vec::new());
            };
            super::helm::related_for_helm_release(state, namespace, name).await
        }
        _ => Ok(Vec::new()),
    }
}

/// Application -> `owns` -> Deployment/StatefulSet/DaemonSet (via the same Helm/Flux/label
/// attribution `applications.rs::synthesize_applications` uses), transitively -> Pod (via each
/// owned workload's own `pod_selector`), per data-model.md's relationship vocabulary.
///
/// **Deviation from data-model.md**: the vocabulary table also lists Application `owns`
/// `ServiceAccount` and CRD instances. Neither is wired up: `ServiceAccount` is not yet a
/// browsable resource kind anywhere in Fathom (no `section_keys` entry, no nav, no inspector), so
/// a `RelatedResourceRef` pointing at one could never be opened; a CRD instance can't be
/// disambiguated either, since `RelatedResourceRef` carries no `group` field and a CRD `Kind`
/// name is not guaranteed unique across groups. Both require real new-feature work (a
/// ServiceAccount resource kind; a protocol change to `RelatedResourceRef`) beyond this resolver,
/// not just wiring - left for a future spec.
async fn related_for_application(
    state: &BackendState,
    namespace: Option<&str>,
    instance: &str,
) -> anyhow::Result<Vec<crate::protocol::RelatedResourceRef>> {
    let Some(namespace) = namespace else {
        return Ok(Vec::new());
    };
    let client = state.cluster.client_or_err()?;
    let workloads = super::applications::list_owned_workloads(state).await?;
    let groups = super::applications::group_workloads(&workloads);
    let Some(group) = groups
        .into_iter()
        .find(|g| g.namespace == namespace && g.instance == instance)
    else {
        return Ok(Vec::new());
    };
    let owned: Vec<&super::applications::OwnedWorkload> =
        group.workloads.iter().map(|&i| &workloads[i]).collect();
    let (related, _all_pods) =
        super::applications::owned_related_resources_and_pods(&client, &owned).await;
    Ok(related)
}

/// Deployment/StatefulSet/DaemonSet/ReplicaSet -> `owns` -> Pod, via the workload's own
/// selector (data-model.md). Frontend callers with an already-fetched `WorkloadDetails.pods`
/// list derive this locally instead (contracts/protocol.md); this arm exists for callers that
/// only have the identity and need a fresh lookup.
async fn related_for_workload(
    client: &Client,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
) -> anyhow::Result<Vec<RelatedResourceRef>> {
    let Some(namespace) = namespace else {
        return Ok(Vec::new());
    };
    let selector = match kind {
        "Deployment" => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
            api.get(name)
                .await?
                .spec
                .and_then(|s| s.selector.match_labels)
        }
        "StatefulSet" => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
            api.get(name)
                .await?
                .spec
                .and_then(|s| s.selector.match_labels)
        }
        "DaemonSet" => {
            let api: Api<DaemonSet> = Api::namespaced(client.clone(), namespace);
            api.get(name)
                .await?
                .spec
                .and_then(|s| s.selector.match_labels)
        }
        "ReplicaSet" => {
            let api: Api<ReplicaSet> = Api::namespaced(client.clone(), namespace);
            api.get(name)
                .await?
                .spec
                .and_then(|s| s.selector.match_labels)
        }
        _ => None,
    };
    let Some(match_labels) = selector else {
        return Ok(Vec::new());
    };
    let label_selector =
        super::workload_common::selector_string_from_match_labels(Some(&match_labels));
    let pods = list_pods_by_selector(client, namespace, &label_selector).await?;
    Ok(pods.into_iter().map(|p| pod_ref(&p, "owns")).collect())
}

async fn related_for_node(
    client: &Client,
    node_name: &str,
) -> anyhow::Result<Vec<RelatedResourceRef>> {
    let api: Api<Pod> = Api::all(client.clone());
    let field_selector = format!("spec.nodeName={node_name}");
    let pods = api
        .list(&ListParams::default().fields(&field_selector))
        .await?
        .items;
    Ok(pods.into_iter().map(|p| pod_ref(&p, "runs")).collect())
}

async fn related_for_service(
    client: &Client,
    namespace: Option<&str>,
    name: &str,
) -> anyhow::Result<Vec<RelatedResourceRef>> {
    let Some(namespace) = namespace else {
        return Ok(Vec::new());
    };
    let svc_api: Api<Service> = Api::namespaced(client.clone(), namespace);
    let svc = svc_api.get(name).await?;
    let Some(selector) = svc.spec.as_ref().and_then(|s| s.selector.as_ref()) else {
        return Ok(Vec::new());
    };
    if selector.is_empty() {
        return Ok(Vec::new());
    }
    let label_selector = super::workload_common::selector_string_from_match_labels(Some(selector));
    let pods = list_pods_by_selector(client, namespace, &label_selector).await?;
    Ok(pods.into_iter().map(|p| pod_ref(&p, "selects")).collect())
}

async fn related_for_pvc(
    client: &Client,
    namespace: Option<&str>,
    name: &str,
) -> anyhow::Result<Vec<RelatedResourceRef>> {
    let Some(namespace) = namespace else {
        return Ok(Vec::new());
    };
    // No field/label selector exists for "mounts this PVC" - list the namespace's pods and
    // filter client-side, bounded to one namespace per data-model.md's own scoping note.
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pods = pod_api.list(&ListParams::default()).await?.items;
    let matching: Vec<Pod> = pods
        .into_iter()
        .filter(|p| {
            p.spec
                .as_ref()
                .map(|s| {
                    s.volumes.iter().flatten().any(|v| {
                        v.persistent_volume_claim
                            .as_ref()
                            .map(|c| c.claim_name == name)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        })
        .collect();
    Ok(matching.iter().map(|p| pod_ref(p, "mounted-by")).collect())
}

async fn related_for_pod(
    client: &Client,
    namespace: Option<&str>,
    name: &str,
) -> anyhow::Result<Vec<RelatedResourceRef>> {
    let Some(namespace) = namespace else {
        return Ok(Vec::new());
    };
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pod = pod_api.get(name).await?;
    let mut out = Vec::new();

    for owner in pod.owner_references() {
        let section = match owner.kind.as_str() {
            "ReplicaSet" => section_keys::workloads_replicasets(),
            "StatefulSet" => section_keys::workloads_statefulsets(),
            "DaemonSet" => section_keys::workloads_daemonsets(),
            "Job" => section_keys::workloads_jobs(),
            _ => continue,
        };
        out.push(RelatedResourceRef {
            section,
            kind: owner.kind.clone(),
            namespace: Some(namespace.to_owned()),
            name: owner.name.clone(),
            relation: "owned-by".to_owned(),
            status_hint: None,
        });
    }

    if let Some(node_name) = pod.spec.as_ref().and_then(|s| s.node_name.clone()) {
        let node_api: Api<Node> = Api::all(client.clone());
        let status_hint = node_api
            .get(&node_name)
            .await
            .ok()
            .map(|_| "Ready".to_owned());
        out.push(RelatedResourceRef {
            section: section_keys::nodes(),
            kind: "Node".to_owned(),
            namespace: None,
            name: node_name,
            relation: "scheduled-on".to_owned(),
            status_hint,
        });
    }

    let labels: BTreeMap<String, String> =
        super::workload_common::map_opt_btreemap(pod.metadata.labels.clone());
    let annotations: BTreeMap<String, String> =
        super::workload_common::map_opt_btreemap(pod.metadata.annotations.clone());
    if let Some((instance, app_namespace)) =
        pod_application_membership(&labels, &annotations, namespace)
    {
        out.push(RelatedResourceRef {
            section: section_keys::applications(),
            kind: "Application".to_owned(),
            namespace: Some(app_namespace),
            name: instance,
            relation: "member-of".to_owned(),
            status_hint: None,
        });
    }

    // Pod -> scaledBy -> ScaledObject, resolved via the pod's owning workload.
    if let Some(workload) = pod_owning_workload(&client_owner_chain(client, namespace, &pod).await)
    {
        for (scaler_kind, scaler_name, target_kind, target_name) in
            super::keda::list_scale_targets(client, namespace).await
        {
            if target_kind == workload.0 && target_name == workload.1 {
                out.push(RelatedResourceRef {
                    section: section_keys::autoscalers(),
                    kind: scaler_kind,
                    namespace: Some(namespace.to_owned()),
                    name: scaler_name,
                    relation: "scaledBy".to_owned(),
                    status_hint: None,
                });
            }
        }
    }

    Ok(out)
}

/// Owner chain of a pod as `(kind, name)`, with ReplicaSets resolved up to their Deployment.
async fn client_owner_chain(client: &Client, namespace: &str, pod: &Pod) -> Vec<(String, String)> {
    let mut chain = Vec::new();
    for owner in pod.owner_references() {
        if owner.kind == "ReplicaSet" {
            let api: Api<ReplicaSet> = Api::namespaced(client.clone(), namespace);
            if let Ok(Some(rs)) = api.get_opt(&owner.name).await
                && let Some(dep) = rs
                    .owner_references()
                    .iter()
                    .find(|o| o.kind == "Deployment")
            {
                chain.push(("Deployment".to_owned(), dep.name.clone()));
                continue;
            }
        }
        chain.push((owner.kind.clone(), owner.name.clone()));
    }
    chain
}

/// First workload-like owner from a resolved owner chain.
fn pod_owning_workload(chain: &[(String, String)]) -> Option<(String, String)> {
    chain
        .iter()
        .find(|(k, _)| matches!(k.as_str(), "Deployment" | "StatefulSet" | "Job"))
        .cloned()
}

/// ScaledObject/ScaledJob -> `scales` -> target workload, and `owns` -> the target's pods.
async fn related_for_scaler(
    client: &Client,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
) -> anyhow::Result<Vec<RelatedResourceRef>> {
    let Some(namespace) = namespace else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    if kind == "ScaledJob" {
        return Ok(out);
    }
    let Some((_, _, target_kind, target_name)) = super::keda::list_scale_targets(client, namespace)
        .await
        .into_iter()
        .find(|(k, n, _, _)| k == kind && n == name)
    else {
        return Ok(out);
    };
    let section = match target_kind.as_str() {
        "Deployment" => section_keys::workloads_deployments(),
        "StatefulSet" => section_keys::workloads_statefulsets(),
        _ => return Ok(out),
    };
    out.push(RelatedResourceRef {
        section,
        kind: target_kind.clone(),
        namespace: Some(namespace.to_owned()),
        name: target_name.clone(),
        relation: "scales".to_owned(),
        status_hint: None,
    });
    out.extend(related_for_workload(client, &target_kind, Some(namespace), &target_name).await?);
    Ok(out)
}

/// Reverse `Pod -> member-of -> Application` relation (data-model.md), via the same attribution
/// precedence `applications.rs::attribute_from_meta` uses for the forward `Application -> owns ->
/// workload` direction, applied directly to a pod's own metadata.
fn pod_application_membership(
    labels: &BTreeMap<String, String>,
    annotations: &BTreeMap<String, String>,
    pod_namespace: &str,
) -> Option<(String, String)> {
    super::applications::attribute_from_meta(labels, annotations, pod_namespace)
        .map(|a| (a.instance, a.namespace))
}

fn pod_ref(pod: &Pod, relation: &str) -> RelatedResourceRef {
    RelatedResourceRef {
        section: section_keys::workloads_pods(),
        kind: "Pod".to_owned(),
        namespace: pod.namespace(),
        name: pod.name_any(),
        relation: relation.to_owned(),
        status_hint: pod.status.as_ref().and_then(|s| s.phase.clone()),
    }
}

/// Shared by `applications.rs`'s owned-pods lookup and this module's `Service`/general
/// label-selector cases, so both go through one `Api::<Pod>::list` call shape.
pub(super) async fn list_pods_by_selector(
    client: &Client,
    namespace: &str,
    selector: &str,
) -> anyhow::Result<Vec<Pod>> {
    if selector.is_empty() {
        return Ok(Vec::new());
    }
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    Ok(api
        .list(&ListParams::default().labels(selector))
        .await?
        .items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_owning_workload_picks_workload_owner() {
        let chain = vec![
            ("Node".to_owned(), "n1".to_owned()),
            ("Deployment".to_owned(), "queue-worker".to_owned()),
        ];
        assert_eq!(
            pod_owning_workload(&chain),
            Some(("Deployment".to_owned(), "queue-worker".to_owned()))
        );
        assert_eq!(pod_owning_workload(&[]), None);
    }

    #[test]
    fn pod_membership_prefers_flux_over_helm() {
        let mut labels = BTreeMap::new();
        labels.insert(
            "kustomize.toolkit.fluxcd.io/name".to_owned(),
            "podinfo".to_owned(),
        );
        let mut annotations = BTreeMap::new();
        annotations.insert(
            "meta.helm.sh/release-name".to_owned(),
            "podinfo-helm".to_owned(),
        );

        let (instance, _) = pod_application_membership(&labels, &annotations, "default").unwrap();
        assert_eq!(instance, "podinfo");
    }

    #[test]
    fn pod_membership_falls_back_to_label_heuristic() {
        let mut labels = BTreeMap::new();
        labels.insert("app.kubernetes.io/name".to_owned(), "myapp".to_owned());
        let annotations = BTreeMap::new();

        let (instance, ns) = pod_application_membership(&labels, &annotations, "default").unwrap();
        assert_eq!(instance, "myapp");
        assert_eq!(ns, "default");
    }

    #[test]
    fn pod_membership_none_when_no_markers() {
        assert!(
            pod_application_membership(&BTreeMap::new(), &BTreeMap::new(), "default").is_none()
        );
    }
}
