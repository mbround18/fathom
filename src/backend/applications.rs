//! Synthesizes "Applications" (spec 004, User Stories 1-2) from already-collected Deployment/
//! StatefulSet/DaemonSet metadata plus Flux's own ownership labels, without any new external
//! integration or Helm release-storage parsing (research.md R1-R4).
//!
//! Attribution precedence (research.md R3): Flux ownership labels win over a bare Helm
//! annotation (a Flux `HelmRelease` also stamps the standard `meta.helm.sh/*` annotations onto
//! the resources it installs via Helm, so checking Flux's own labels first is required to avoid
//! misclassifying a Flux-managed release as plain Helm). Falls back to the
//! `app.kubernetes.io/name`/`part-of` label heuristic when neither is present.

use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use kube::{
    ResourceExt,
    api::{Api, ListParams},
};

use crate::protocol::{
    ApplicationDetails, ApplicationManagedBy, ApplicationSummary, BackendEvent, Correlation,
    ResourceListEvent, ResourceReadEvent, Route, section_keys,
};

use super::{events, flux, metrics, related, state::BackendState, workload_common};

const HELM_RELEASE_NAME_ANNOTATION: &str = "meta.helm.sh/release-name";
const HELM_RELEASE_NAMESPACE_ANNOTATION: &str = "meta.helm.sh/release-namespace";
const FLUX_KUSTOMIZATION_NAME_LABEL: &str = "kustomize.toolkit.fluxcd.io/name";
const FLUX_KUSTOMIZATION_NAMESPACE_LABEL: &str = "kustomize.toolkit.fluxcd.io/namespace";
const FLUX_HELMRELEASE_NAME_LABEL: &str = "helm.toolkit.fluxcd.io/name";
const FLUX_HELMRELEASE_NAMESPACE_LABEL: &str = "helm.toolkit.fluxcd.io/namespace";
const APP_NAME_LABEL: &str = "app.kubernetes.io/name";
const APP_PART_OF_LABEL: &str = "app.kubernetes.io/part-of";
const APP_VERSION_LABEL: &str = "app.kubernetes.io/version";

/// One owned workload, generic over Deployment/StatefulSet/DaemonSet, reduced to just what
/// attribution and readiness aggregation need.
pub(super) struct OwnedWorkload {
    kind: &'static str,
    namespace: String,
    name: String,
    labels: BTreeMap<String, String>,
    annotations: BTreeMap<String, String>,
    creation_timestamp: String,
    ready: bool,
    pod_selector: String,
}

impl OwnedWorkload {
    /// Exposed for `helm.rs`'s Helm-annotation-based attribution (Helm-installed resources
    /// typically carry no `ownerReferences` back to their release, so it re-derives ownership
    /// from the same `meta.helm.sh/*`/Flux labels this module already collects).
    pub(super) fn labels(&self) -> &BTreeMap<String, String> {
        &self.labels
    }

    pub(super) fn annotations(&self) -> &BTreeMap<String, String> {
        &self.annotations
    }
}

/// Which application a workload belongs to, and how Fathom attributed it.
pub(super) struct Attribution {
    pub(super) instance: String,
    pub(super) namespace: String,
    pub(super) managed_by: ApplicationManagedBy,
}

/// Core attribution precedence (research.md R3: Flux labels, then Helm annotation, then the
/// `app.kubernetes.io/*` label heuristic), applied to any resource's own labels/annotations.
/// Shared by workload attribution (`attribute`), `ServiceAccount` attribution
/// (`list_owned_service_accounts`), and the reverse `Pod -> member-of -> Application` lookup
/// (`related.rs::pod_application_membership`) so the precedence rule lives in exactly one place.
pub(super) fn attribute_from_meta(
    labels: &BTreeMap<String, String>,
    annotations: &BTreeMap<String, String>,
    namespace: &str,
) -> Option<Attribution> {
    if let Some(name) = labels.get(FLUX_KUSTOMIZATION_NAME_LABEL) {
        let ns = labels
            .get(FLUX_KUSTOMIZATION_NAMESPACE_LABEL)
            .cloned()
            .unwrap_or_else(|| namespace.to_owned());
        return Some(Attribution {
            instance: name.clone(),
            namespace: ns,
            managed_by: ApplicationManagedBy::Flux,
        });
    }
    if let Some(name) = labels.get(FLUX_HELMRELEASE_NAME_LABEL) {
        let ns = labels
            .get(FLUX_HELMRELEASE_NAMESPACE_LABEL)
            .cloned()
            .unwrap_or_else(|| namespace.to_owned());
        return Some(Attribution {
            instance: name.clone(),
            namespace: ns,
            managed_by: ApplicationManagedBy::Flux,
        });
    }
    if let Some(name) = annotations.get(HELM_RELEASE_NAME_ANNOTATION) {
        let ns = annotations
            .get(HELM_RELEASE_NAMESPACE_ANNOTATION)
            .cloned()
            .unwrap_or_else(|| namespace.to_owned());
        return Some(Attribution {
            instance: name.clone(),
            namespace: ns,
            managed_by: ApplicationManagedBy::Helm,
        });
    }
    let heuristic_name = labels
        .get(APP_NAME_LABEL)
        .or_else(|| labels.get(APP_PART_OF_LABEL));
    if let Some(name) = heuristic_name {
        return Some(Attribution {
            instance: name.clone(),
            namespace: namespace.to_owned(),
            managed_by: ApplicationManagedBy::LabelHeuristic,
        });
    }
    None
}

fn attribute(workload: &OwnedWorkload) -> Option<Attribution> {
    attribute_from_meta(&workload.labels, &workload.annotations, &workload.namespace)
}

pub(super) async fn list_owned_workloads(
    state: &BackendState,
) -> anyhow::Result<Vec<OwnedWorkload>> {
    let client = state.cluster.client_or_err()?;
    let mut out = Vec::new();

    let deployments: Api<Deployment> = Api::all(client.clone());
    for d in deployments.list(&ListParams::default()).await?.items {
        let ready = d
            .status
            .as_ref()
            .and_then(|s| s.ready_replicas)
            .unwrap_or(0);
        let desired = d.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
        let pod_selector = d
            .spec
            .as_ref()
            .and_then(|s| s.selector.match_labels.as_ref())
            .map(|l| workload_common::selector_string_from_match_labels(Some(l)))
            .unwrap_or_default();
        out.push(OwnedWorkload {
            kind: "Deployment",
            namespace: d.namespace().unwrap_or_default(),
            name: d.name_any(),
            labels: workload_common::map_opt_btreemap(d.metadata.labels.clone()),
            annotations: workload_common::map_opt_btreemap(d.metadata.annotations.clone()),
            creation_timestamp: workload_common::creation_timestamp(&d.metadata),
            ready: desired > 0 && ready >= desired,
            pod_selector,
        });
    }

    let statefulsets: Api<StatefulSet> = Api::all(client.clone());
    for s in statefulsets.list(&ListParams::default()).await?.items {
        let ready = s
            .status
            .as_ref()
            .and_then(|st| st.ready_replicas)
            .unwrap_or(0);
        let desired = s.spec.as_ref().and_then(|sp| sp.replicas).unwrap_or(1);
        let pod_selector = s
            .spec
            .as_ref()
            .and_then(|sp| sp.selector.match_labels.as_ref())
            .map(|l| workload_common::selector_string_from_match_labels(Some(l)))
            .unwrap_or_default();
        out.push(OwnedWorkload {
            kind: "StatefulSet",
            namespace: s.namespace().unwrap_or_default(),
            name: s.name_any(),
            labels: workload_common::map_opt_btreemap(s.metadata.labels.clone()),
            annotations: workload_common::map_opt_btreemap(s.metadata.annotations.clone()),
            creation_timestamp: workload_common::creation_timestamp(&s.metadata),
            ready: desired > 0 && ready >= desired,
            pod_selector,
        });
    }

    let daemonsets: Api<DaemonSet> = Api::all(client.clone());
    for ds in daemonsets.list(&ListParams::default()).await?.items {
        let (ready, desired) = ds
            .status
            .as_ref()
            .map(|st| (st.number_ready, st.desired_number_scheduled))
            .unwrap_or((0, 0));
        let pod_selector = ds
            .spec
            .as_ref()
            .and_then(|sp| sp.selector.match_labels.as_ref())
            .map(|l| workload_common::selector_string_from_match_labels(Some(l)))
            .unwrap_or_default();
        out.push(OwnedWorkload {
            kind: "DaemonSet",
            namespace: ds.namespace().unwrap_or_default(),
            name: ds.name_any(),
            labels: workload_common::map_opt_btreemap(ds.metadata.labels.clone()),
            annotations: workload_common::map_opt_btreemap(ds.metadata.annotations.clone()),
            creation_timestamp: workload_common::creation_timestamp(&ds.metadata),
            ready: desired > 0 && ready >= desired,
            pod_selector,
        });
    }

    Ok(out)
}

/// One Flux Kustomization/HelmRelease, keyed by (namespace, name) in [`flux_apps`].
#[derive(Debug, Clone)]
struct FluxApp {
    /// `status.lastAppliedRevision`, the `ApplicationSummary.version` of a Flux application.
    revision: Option<String>,
    creation_timestamp: String,
}

/// Every Kustomization/HelmRelease by (namespace, name): its `status.lastAppliedRevision` fills
/// `ApplicationSummary.version` for Flux-attributed applications (data-model.md), and its mere
/// existence keeps a Flux application listed after its last workload is gone (FR-016).
async fn flux_apps(client: &kube::Client) -> BTreeMap<(String, String), FluxApp> {
    let mut out = BTreeMap::new();
    let Ok((kustomizations, helmreleases)) =
        flux::list_kustomizations_and_helmreleases(client).await
    else {
        return out;
    };
    for obj in kustomizations.into_iter().chain(helmreleases) {
        let namespace = obj.metadata.namespace.clone().unwrap_or_default();
        let name = obj.metadata.name.clone().unwrap_or_default();
        let revision = obj
            .data
            .get("status")
            .and_then(|s| s.get("lastAppliedRevision"))
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        out.insert(
            (namespace, name),
            FluxApp {
                revision,
                creation_timestamp: workload_common::creation_timestamp(&obj.metadata),
            },
        );
    }
    out
}

/// FR-016: a Flux Kustomization/HelmRelease with no workload currently attributed to it is still
/// an application, listed with zero workloads (the UI shows "No workloads"). Bare Helm releases
/// can't get the same treatment: FR-019 forbids reading Helm's release storage, so once their
/// last labelled workload is gone nothing live identifies them.
fn add_flux_only_groups(
    groups: &mut Vec<AppGroup>,
    flux: &BTreeMap<(String, String), FluxApp>,
) {
    for ((namespace, name), app) in flux {
        if groups
            .iter()
            .any(|g| &g.namespace == namespace && &g.instance == name)
        {
            continue;
        }
        groups.push(AppGroup {
            instance: name.clone(),
            namespace: namespace.clone(),
            managed_by: ApplicationManagedBy::Flux,
            workloads: Vec::new(),
            creation_timestamp: app.creation_timestamp.clone(),
            labels: BTreeMap::new(),
        });
    }
}

pub(super) struct AppGroup {
    pub(super) instance: String,
    pub(super) namespace: String,
    pub(super) managed_by: ApplicationManagedBy,
    pub(super) workloads: Vec<usize>,
    creation_timestamp: String,
    labels: BTreeMap<String, String>,
}

pub(super) fn group_workloads(workloads: &[OwnedWorkload]) -> Vec<AppGroup> {
    let mut groups: BTreeMap<(String, String), AppGroup> = BTreeMap::new();
    for (idx, w) in workloads.iter().enumerate() {
        let Some(attribution) = attribute(w) else {
            continue;
        };
        let key = (attribution.namespace.clone(), attribution.instance.clone());
        let entry = groups.entry(key).or_insert_with(|| AppGroup {
            instance: attribution.instance.clone(),
            namespace: attribution.namespace.clone(),
            managed_by: attribution.managed_by,
            workloads: Vec::new(),
            creation_timestamp: w.creation_timestamp.clone(),
            labels: BTreeMap::new(),
        });
        // Flux precedence over a bare Helm annotation (research.md R3): once a group has been
        // seen as Flux-managed, a later Helm-only-attributed sibling workload must not downgrade it.
        if entry.managed_by != ApplicationManagedBy::Flux
            && attribution.managed_by == ApplicationManagedBy::Flux
        {
            entry.managed_by = ApplicationManagedBy::Flux;
        }
        if !w.creation_timestamp.is_empty()
            && (entry.creation_timestamp.is_empty()
                || w.creation_timestamp < entry.creation_timestamp)
        {
            entry.creation_timestamp = w.creation_timestamp.clone();
        }
        for (k, v) in &w.labels {
            entry.labels.entry(k.clone()).or_insert_with(|| v.clone());
        }
        entry.workloads.push(idx);
    }
    groups.into_values().collect()
}

pub(super) async fn refresh_applications(
    state: &BackendState,
    sender: &crossbeam_channel::Sender<crate::protocol::BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let workloads = list_owned_workloads(state).await?;
    let flux = flux_apps(&client).await;
    let mut groups = group_workloads(&workloads);
    add_flux_only_groups(&mut groups, &flux);

    let mut summaries: Vec<ApplicationSummary> = groups
        .into_iter()
        .filter(|g| {
            state.cluster.namespace_filter.is_empty()
                || state
                    .cluster
                    .namespace_filter
                    .iter()
                    .any(|ns| ns == &g.namespace)
        })
        .map(|g| {
            let ready_count = g.workloads.iter().filter(|&&i| workloads[i].ready).count() as i32;
            let total_count = g.workloads.len() as i32;
            let version = match g.managed_by {
                ApplicationManagedBy::Flux => flux
                    .get(&(g.namespace.clone(), g.instance.clone()))
                    .and_then(|f| f.revision.clone()),
                ApplicationManagedBy::Helm => g.labels.get(APP_VERSION_LABEL).cloned(),
                ApplicationManagedBy::LabelHeuristic => None,
            };
            ApplicationSummary {
                instance: g.instance,
                name: g.labels.get(APP_NAME_LABEL).cloned().unwrap_or_else(|| {
                    g.labels.get(APP_PART_OF_LABEL).cloned().unwrap_or_default()
                }),
                namespace: g.namespace,
                managed_by: g.managed_by,
                version,
                creation_timestamp: g.creation_timestamp,
                ready_count,
                total_count,
            }
        })
        .collect();
    // Display name falls back to the instance id when no app.kubernetes.io/name-style label
    // was present (Helm/Flux attribution doesn't require one).
    for s in &mut summaries {
        if s.name.is_empty() {
            s.name = s.instance.clone();
        }
    }
    summaries.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::applications(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::ApplicationsUpdated(summaries)),
    );
    Ok(())
}

pub(super) async fn select_application(
    state: &BackendState,
    sender: &crossbeam_channel::Sender<crate::protocol::BackendEventEnvelope>,
    namespace: &str,
    instance: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let workloads = list_owned_workloads(state).await?;
    let flux = flux_apps(&client).await;
    let mut groups = group_workloads(&workloads);
    add_flux_only_groups(&mut groups, &flux);

    let Some(group) = groups
        .into_iter()
        .find(|g| g.namespace == namespace && g.instance == instance)
    else {
        // Application no longer resolvable (e.g. every owned workload was deleted out of
        // band): emit an empty-but-present details record rather than nothing (FR-016's
        // "no active workloads" state applies to the list; here it's a stale-selection no-op).
        return Ok(());
    };

    let version = match group.managed_by {
        ApplicationManagedBy::Flux => flux
            .get(&(group.namespace.clone(), group.instance.clone()))
            .and_then(|f| f.revision.clone()),
        ApplicationManagedBy::Helm => group.labels.get(APP_VERSION_LABEL).cloned(),
        ApplicationManagedBy::LabelHeuristic => None,
    };

    let owned: Vec<&OwnedWorkload> = group.workloads.iter().map(|&i| &workloads[i]).collect();
    let (related, all_pods) = owned_related_resources_and_pods(&client, &owned).await;

    let name = group
        .labels
        .get(APP_NAME_LABEL)
        .cloned()
        .unwrap_or_else(|| group.instance.clone());

    let details = ApplicationDetails {
        instance: group.instance.clone(),
        name,
        namespace: group.namespace.clone(),
        managed_by: group.managed_by,
        version,
        creation_timestamp: group.creation_timestamp,
        labels: group.labels,
        urls: application_urls(&client, &group.namespace, owned.as_slice()).await,
        related,
    };

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::applications(),
            state.cluster.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ApplicationDetailsUpdated(details)),
    );

    let aggregate = metrics::aggregate_workload_metrics_once(
        state.cluster.current_context.as_deref(),
        namespace,
        instance,
        "Application",
        &all_pods,
    )
    .await;
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::applications(),
            state.cluster.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ApplicationMetricsUpdated(aggregate)),
    );

    Ok(())
}

/// Application `owns` Deployment/StatefulSet/DaemonSet (data-model.md's relationship vocabulary),
/// as a pure mapping so it's unit-testable without a live cluster (T026).
fn workload_owns_ref(w: &OwnedWorkload) -> crate::protocol::RelatedResourceRef {
    crate::protocol::RelatedResourceRef {
        section: match w.kind {
            "Deployment" => section_keys::workloads_deployments(),
            "StatefulSet" => section_keys::workloads_statefulsets(),
            _ => section_keys::workloads_daemonsets(),
        },
        kind: w.kind.to_owned(),
        namespace: Some(w.namespace.clone()),
        name: w.name.clone(),
        relation: "owns".to_owned(),
        status_hint: Some(if w.ready {
            "Ready".to_owned()
        } else {
            "NotReady".to_owned()
        }),
    }
}

/// Owned-workload `owns` links plus their pods' `owns` links, and the flattened pod list
/// (reused for the application's aggregated metrics query) — split out of `select_application`
/// to keep that function under the line-count lint.
pub(super) async fn owned_related_resources_and_pods(
    client: &kube::Client,
    owned: &[&OwnedWorkload],
) -> (
    Vec<crate::protocol::RelatedResourceRef>,
    Vec<k8s_openapi::api::core::v1::Pod>,
) {
    let mut related = Vec::new();
    let mut all_pods: Vec<k8s_openapi::api::core::v1::Pod> = Vec::new();
    for w in owned {
        related.push(workload_owns_ref(w));
        if w.pod_selector.is_empty() {
            continue;
        }
        let Ok(pods) = related::list_pods_by_selector(client, &w.namespace, &w.pod_selector).await
        else {
            continue;
        };
        for p in &pods {
            related.push(crate::protocol::RelatedResourceRef {
                section: section_keys::workloads_pods(),
                kind: "Pod".to_owned(),
                namespace: p.namespace(),
                name: p.name_any(),
                relation: "owns".to_owned(),
                status_hint: p.status.as_ref().and_then(|s| s.phase.clone()),
            });
        }
        all_pods.extend(pods);
    }
    (related, all_pods)
}

/// Best-effort URLs derived from the application's owned workloads' matching Services
/// (data-model.md `ApplicationDetails.urls`) — Ingress host rules are not consulted (a scoped
/// deviation from data-model.md's "and Ingress host rules" wording, noted for follow-up; the
/// Service-only source already covers the common ClusterIP/LoadBalancer case).
async fn application_urls(
    client: &kube::Client,
    namespace: &str,
    owned: &[&OwnedWorkload],
) -> Vec<String> {
    use k8s_openapi::api::core::v1::Service;
    let api: Api<Service> = Api::namespaced(client.clone(), namespace);
    let Ok(services) = api.list(&ListParams::default()).await else {
        return Vec::new();
    };
    let owned_pod_labels: Vec<&BTreeMap<String, String>> =
        owned.iter().map(|w| &w.labels).collect();
    let mut urls = Vec::new();
    for svc in services.items {
        let Some(selector) = svc.spec.as_ref().and_then(|s| s.selector.as_ref()) else {
            continue;
        };
        let matches = owned_pod_labels
            .iter()
            .any(|labels| selector.iter().all(|(k, v)| labels.get(k) == Some(v)));
        if !matches {
            continue;
        }
        let name = svc.name_any();
        if let Some(ports) = svc.spec.as_ref().and_then(|s| s.ports.as_ref()) {
            for p in ports {
                urls.push(format!("http://{name}.{namespace}.svc:{}", p.port));
            }
        }
    }
    urls
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workload(
        kind: &'static str,
        labels: &[(&str, &str)],
        annotations: &[(&str, &str)],
    ) -> OwnedWorkload {
        OwnedWorkload {
            kind,
            namespace: "default".to_owned(),
            name: "w".to_owned(),
            labels: labels
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            annotations: annotations
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            creation_timestamp: String::new(),
            ready: true,
            pod_selector: String::new(),
        }
    }

    #[test]
    fn attributes_helm_from_release_annotations() {
        let w = workload(
            "Deployment",
            &[],
            &[
                (HELM_RELEASE_NAME_ANNOTATION, "redis"),
                (HELM_RELEASE_NAMESPACE_ANNOTATION, "cache"),
            ],
        );
        let a = attribute(&w).expect("attribution");
        assert_eq!(a.instance, "redis");
        assert_eq!(a.namespace, "cache");
        assert_eq!(a.managed_by, ApplicationManagedBy::Helm);
    }

    #[test]
    fn flux_kustomization_label_wins_over_helm_annotation() {
        let w = workload(
            "Deployment",
            &[(FLUX_KUSTOMIZATION_NAME_LABEL, "podinfo")],
            &[(HELM_RELEASE_NAME_ANNOTATION, "podinfo-helm")],
        );
        let a = attribute(&w).expect("attribution");
        assert_eq!(a.instance, "podinfo");
        assert_eq!(a.managed_by, ApplicationManagedBy::Flux);
    }

    #[test]
    fn falls_back_to_label_heuristic() {
        let w = workload("Deployment", &[(APP_NAME_LABEL, "myapp")], &[]);
        let a = attribute(&w).expect("attribution");
        assert_eq!(a.instance, "myapp");
        assert_eq!(a.managed_by, ApplicationManagedBy::LabelHeuristic);
    }

    #[test]
    fn no_attribution_when_no_markers_present() {
        let w = workload("Deployment", &[], &[]);
        assert!(attribute(&w).is_none());
    }

    /// T026: Application `owns` Deployment/StatefulSet/DaemonSet, per data-model.md's
    /// relationship vocabulary table (`related.rs::related_for_application` builds these through
    /// `workload_owns_ref`/`owned_related_resources_and_pods`).
    #[test]
    fn workload_owns_ref_covers_deployment_statefulset_daemonset() {
        for (kind, expected_section) in [
            ("Deployment", section_keys::workloads_deployments()),
            ("StatefulSet", section_keys::workloads_statefulsets()),
            ("DaemonSet", section_keys::workloads_daemonsets()),
        ] {
            let mut w = workload(kind, &[], &[]);
            w.namespace = "cache".to_owned();
            w.name = "redis".to_owned();
            w.ready = true;
            let r = workload_owns_ref(&w);
            assert_eq!(r.section, expected_section);
            assert_eq!(r.kind, kind);
            assert_eq!(r.namespace.as_deref(), Some("cache"));
            assert_eq!(r.name, "redis");
            assert_eq!(r.relation, "owns");
            assert_eq!(r.status_hint.as_deref(), Some("Ready"));
        }
    }

    #[test]
    fn workload_owns_ref_reports_not_ready() {
        let mut w = workload("Deployment", &[], &[]);
        w.ready = false;
        assert_eq!(
            workload_owns_ref(&w).status_hint.as_deref(),
            Some("NotReady")
        );
    }

    #[test]
    fn flux_object_without_workloads_is_still_an_application() {
        let w = workload(
            "Deployment",
            &[
                (FLUX_HELMRELEASE_NAME_LABEL, "podinfo"),
                (FLUX_HELMRELEASE_NAMESPACE_LABEL, "default"),
            ],
            &[],
        );
        let workloads = vec![w];
        let mut groups = group_workloads(&workloads);
        let flux: BTreeMap<(String, String), FluxApp> = [
            ("default", "podinfo"),
            ("default", "emptied"),
        ]
        .into_iter()
        .map(|(ns, name)| {
            (
                (ns.to_owned(), name.to_owned()),
                FluxApp {
                    revision: Some("1.0.0".to_owned()),
                    creation_timestamp: "2026-01-01T00:00:00Z".to_owned(),
                },
            )
        })
        .collect();
        add_flux_only_groups(&mut groups, &flux);

        assert_eq!(groups.len(), 2, "no duplicate for the app that has a workload");
        let podinfo = groups.iter().find(|g| g.instance == "podinfo").unwrap();
        assert_eq!(podinfo.workloads, vec![0]);
        let emptied = groups.iter().find(|g| g.instance == "emptied").unwrap();
        assert!(emptied.workloads.is_empty());
        assert_eq!(emptied.managed_by, ApplicationManagedBy::Flux);
        assert_eq!(emptied.namespace, "default");
        assert_eq!(emptied.creation_timestamp, "2026-01-01T00:00:00Z");
    }
}
