//! Flux CD GitOps Toolkit integration.
//!
//! Discovers whichever served API version of the Flux CRDs is installed
//! (source.toolkit.fluxcd.io, kustomize.toolkit.fluxcd.io,
//! helm.toolkit.fluxcd.io) and reads GitRepositories, OCIRepositories,
//! HelmRepositories, Buckets, Kustomizations, and HelmReleases as dynamic
//! objects - the same discovery pattern used for Longhorn in `longhorn.rs`,
//! so this works across Flux versions without pinning an API version.

use super::served_crd::{ServedCrd, find_served};
use crate::protocol::{FluxOverview, FluxReadyState, FluxResourceSummary};
use anyhow::{Context as _, anyhow};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{ListParams, Patch, PatchParams};
use kube::core::DynamicObject;
use kube::{Client, api::Api};
use serde_json::{Value, json};

/// CRD names (plural.group) we look for, and which overview bucket they belong to.
const SOURCE_CRDS: &[&str] = &[
    "gitrepositories.source.toolkit.fluxcd.io",
    "ocirepositories.source.toolkit.fluxcd.io",
    "helmrepositories.source.toolkit.fluxcd.io",
    "buckets.source.toolkit.fluxcd.io",
];
const KUSTOMIZATION_CRD: &str = "kustomizations.kustomize.toolkit.fluxcd.io";
const HELMRELEASE_CRD: &str = "helmreleases.helm.toolkit.fluxcd.io";
/// The chart a HelmRelease is rendered from; only reached through `with_source` (049).
const HELMCHART_CRD: &str = "helmcharts.source.toolkit.fluxcd.io";
/// The annotation every Flux controller watches for an on-demand reconcile (049).
const RECONCILE_ANNOTATION: &str = "reconcile.fluxcd.io/requestedAt";
/// `status.inventory` is unbounded; the summary carries this many entries at most.
const MANAGED_CAP: usize = 500;

/// The CRD that serves a Flux `kind`, or `None` for a kind Fathom does not handle.
fn crd_name_for_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "GitRepository" => Some(SOURCE_CRDS[0]),
        "OCIRepository" => Some(SOURCE_CRDS[1]),
        "HelmRepository" => Some(SOURCE_CRDS[2]),
        "Bucket" => Some(SOURCE_CRDS[3]),
        "HelmChart" => Some(HELMCHART_CRD),
        "Kustomization" => Some(KUSTOMIZATION_CRD),
        "HelmRelease" => Some(HELMRELEASE_CRD),
        _ => None,
    }
}

/// Sources carry their revision in `status.artifact`; everything else in `lastAppliedRevision`.
pub(super) fn is_source_kind(kind: &str) -> bool {
    !matches!(kind, "Kustomization" | "HelmRelease")
}

/// Discover the served version of `kind`'s CRD, failing by name when Flux (or that part of it)
/// is not installed.
async fn served_for_kind(client: &Client, kind: &str) -> anyhow::Result<ServedCrd> {
    let crd_name =
        crd_name_for_kind(kind).ok_or_else(|| anyhow!("{kind} is not a Flux resource kind"))?;
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&ListParams::default()).await?.items;
    find_served(&crds, crd_name)
        .ok_or_else(|| anyhow!("the {kind} CRD ({crd_name}) is not installed in this cluster"))
}

async fn namespaced_api(
    client: &Client,
    kind: &str,
    namespace: &str,
) -> anyhow::Result<Api<DynamicObject>> {
    let crd = served_for_kind(client, kind).await?;
    Ok(Api::namespaced_with(
        client.clone(),
        namespace,
        &crd.api_resource(),
    ))
}

/// One Flux resource by kind, namespace and name; `None` when it is gone (049, and the 005
/// origin click-through).
pub(super) async fn get_resource(
    client: &Client,
    kind: &str,
    namespace: &str,
    name: &str,
) -> anyhow::Result<Option<DynamicObject>> {
    let api = namespaced_api(client, kind, namespace).await?;
    match api.get(name).await {
        Ok(obj) => Ok(Some(obj)),
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 049: merge-patch `spec.suspend`. Idempotent; the API server decides whether it is allowed.
pub(super) async fn suspend_flux_resource(
    client: &Client,
    kind: &str,
    namespace: &str,
    name: &str,
    suspend: bool,
) -> anyhow::Result<()> {
    let api = namespaced_api(client, kind, namespace).await?;
    let patch = json!({ "spec": { "suspend": suspend } });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .with_context(|| format!("patch {kind} {namespace}/{name}"))?;
    Ok(())
}

/// 049: stamp `reconcile.fluxcd.io/requestedAt` with `now`. With `with_source`, the resource's
/// source is stamped first (a Kustomization's `spec.sourceRef`; a HelmRelease's `status.helmChart`
/// or `spec.chartRef`), the same order `flux reconcile --with-source` uses. A source kind ignores
/// the flag. Returns the lines of what was requested, source first, for the operator's toast.
pub(super) async fn reconcile_flux_resource(
    client: &Client,
    kind: &str,
    namespace: &str,
    name: &str,
    with_source: bool,
    now: &str,
) -> anyhow::Result<Vec<String>> {
    let api = namespaced_api(client, kind, namespace).await?;
    let mut requested = Vec::new();
    if with_source && !is_source_kind(kind) {
        let obj = api
            .get(name)
            .await
            .with_context(|| format!("read {kind} {namespace}/{name}"))?;
        let (src_kind, src_ns, src_name) = source_of(&obj, namespace)
            .ok_or_else(|| anyhow!("{kind} {namespace}/{name} names no source to reconcile"))?;
        let src_api = namespaced_api(client, &src_kind, &src_ns).await?;
        stamp(&src_api, &src_name, now)
            .await
            .with_context(|| format!("reconcile source {src_kind} {src_ns}/{src_name}"))?;
        requested.push(format!("{src_kind} {src_ns}/{src_name}"));
    }
    stamp(&api, name, now)
        .await
        .with_context(|| format!("reconcile {kind} {namespace}/{name}"))?;
    requested.push(format!("{kind} {namespace}/{name}"));
    Ok(requested)
}

async fn stamp(api: &Api<DynamicObject>, name: &str, now: &str) -> kube::Result<DynamicObject> {
    let patch = json!({ "metadata": { "annotations": { RECONCILE_ANNOTATION: now } } });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
}

/// The source a Kustomization or HelmRelease is reconciled from, as (kind, namespace, name).
/// A HelmRelease's chart is the HelmChart Flux generated (`status.helmChart`, `ns/name`), or
/// its `spec.chartRef` when it points at an OCIRepository directly.
fn source_of(obj: &DynamicObject, own_namespace: &str) -> Option<(String, String, String)> {
    let spec = obj.data.get("spec")?;
    if let Some(chart) = obj
        .data
        .get("status")
        .and_then(|s| s.get("helmChart"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        let (ns, name) = chart.split_once('/')?;
        return Some(("HelmChart".to_owned(), ns.to_owned(), name.to_owned()));
    }
    let r = spec.get("sourceRef").or_else(|| spec.get("chartRef"))?;
    let kind = r.get("kind").and_then(Value::as_str)?.to_owned();
    let name = r.get("name").and_then(Value::as_str)?.to_owned();
    let ns = r
        .get("namespace")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(own_namespace)
        .to_owned();
    Some((kind, ns, name))
}

/// A Kustomization inventory id (`namespace_name_group_kind`, namespace empty when the object
/// is cluster-scoped) as the `Kind namespace/name` line the confirm dialog shows.
fn inventory_entry_label(id: &str) -> Option<String> {
    let mut parts = id.splitn(4, '_');
    let ns = parts.next()?;
    let name = parts.next()?;
    let _group = parts.next()?;
    let kind = parts.next()?;
    Some(if ns.is_empty() {
        format!("{kind} {name}")
    } else {
        format!("{kind} {ns}/{name}")
    })
}

/// What a resource owns (049): a Kustomization's applied inventory, or the Helm release a
/// HelmRelease installs. Sources own nothing Flux enumerates.
fn managed(obj: &DynamicObject, kind: &str, namespace: &str, name: &str) -> Vec<String> {
    let status = obj.data.get("status");
    match kind {
        "Kustomization" => status
            .and_then(|s| s.get("inventory"))
            .and_then(|i| i.get("entries"))
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|e| e.get("id").and_then(Value::as_str))
                    .filter_map(inventory_entry_label)
                    .take(MANAGED_CAP)
                    .collect()
            })
            .unwrap_or_default(),
        "HelmRelease" => {
            let spec = obj.data.get("spec");
            let release_ns = spec
                .and_then(|s| s.get("storageNamespace"))
                .or_else(|| spec.and_then(|s| s.get("targetNamespace")))
                .and_then(Value::as_str)
                .unwrap_or(namespace);
            let release_name = spec
                .and_then(|s| s.get("releaseName"))
                .and_then(Value::as_str)
                .unwrap_or(name);
            vec![format!("Helm release {release_ns}/{release_name}")]
        }
        _ => Vec::new(),
    }
}

/// List every instance of a discovered CRD as dynamic objects, across all namespaces.
async fn list_instances(client: &Client, crd: &ServedCrd) -> Vec<DynamicObject> {
    let resource = crd.api_resource();
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &resource);
    api.list(&ListParams::default())
        .await
        .map(|l| l.items)
        .unwrap_or_default()
}

/// Find the `Ready` condition in `status.conditions` and return (status, reason, message, lastTransitionTime).
fn ready_condition(status: &Value) -> Option<(&str, &str, &str, &str)> {
    status
        .get("conditions")
        .and_then(|c| c.as_array())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|c| c.get("type").and_then(|t| t.as_str()) == Some("Ready"))
        })
        .map(|cond| {
            let status = cond
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let reason = cond.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            let message = cond.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let transition = cond
                .get("lastTransitionTime")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (status, reason, message, transition)
        })
}

fn classify_ready(
    suspended: bool,
    generation_stale: bool,
    ready_status: Option<&str>,
) -> FluxReadyState {
    if suspended {
        FluxReadyState::Suspended
    } else {
        match ready_status {
            Some("True") if !generation_stale => FluxReadyState::Ready,
            Some("False") => FluxReadyState::Failed,
            Some(_) => FluxReadyState::Reconciling,
            None => FluxReadyState::Unknown,
        }
    }
}

/// Build a [`FluxResourceSummary`] from a dynamic object's spec/status/metadata.
pub(super) fn summarize(obj: &DynamicObject, kind: &str, is_source: bool) -> FluxResourceSummary {
    let name = obj.metadata.name.clone().unwrap_or_default();
    let namespace = obj.metadata.namespace.clone().unwrap_or_default();
    let generation = obj.metadata.generation.unwrap_or(0);

    let spec = obj.data.get("spec");
    let status = obj.data.get("status");

    let suspended = spec
        .and_then(|s| s.get("suspend"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let interval = spec
        .and_then(|s| s.get("interval"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    let observed_generation = status
        .and_then(|s| s.get("observedGeneration"))
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);
    let generation_stale = observed_generation != generation;

    let (ready_status, reason, message, last_transition) = status
        .and_then(ready_condition)
        .map(|(s, r, m, t)| (Some(s), r.to_owned(), m.to_owned(), t.to_owned()))
        .unwrap_or((None, String::new(), String::new(), String::new()));

    let message = if message.is_empty() { reason } else { message };

    let revision = if is_source {
        status
            .and_then(|s| s.get("artifact"))
            .and_then(|a| a.get("revision"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned()
    } else {
        status
            .and_then(|s| s.get("lastAppliedRevision"))
            .or_else(|| status.and_then(|s| s.get("lastAttemptedRevision")))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned()
    };

    // A Kustomization names its source directly; a HelmRelease through `spec.chart.spec` (or
    // `spec.chartRef` for an OCI chart). Both read as `Kind/name` so a source's dependents can
    // be found from the overview (049).
    let source_ref = spec
        .and_then(|s| s.get("sourceRef"))
        .or_else(|| {
            spec.and_then(|s| s.get("chart"))
                .and_then(|c| c.get("spec"))
                .and_then(|c| c.get("sourceRef"))
        })
        .or_else(|| spec.and_then(|s| s.get("chartRef")))
        .map(|sr| {
            let sr_kind = sr.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let sr_name = sr.get("name").and_then(|v| v.as_str()).unwrap_or("");
            format!("{sr_kind}/{sr_name}")
        });
    let managed = managed(obj, kind, &namespace, &name);

    FluxResourceSummary {
        kind: kind.to_owned(),
        name,
        namespace,
        ready: classify_ready(suspended, generation_stale, ready_status),
        message,
        revision,
        source_ref,
        suspended,
        interval,
        last_transition,
        managed,
    }
}

#[cfg(test)]
mod mutate_tests {
    use super::*;

    fn obj(data: Value) -> DynamicObject {
        let mut o = DynamicObject::new(
            "apps",
            &kube::core::ApiResource::erase::<k8s_openapi::api::core::v1::ConfigMap>(&()),
        );
        o.metadata.namespace = Some("flux-system".to_owned());
        o.data = data;
        o
    }

    #[test]
    fn inventory_ids_read_as_kind_and_name() {
        assert_eq!(
            inventory_entry_label("default_podinfo_apps_Deployment").as_deref(),
            Some("Deployment default/podinfo")
        );
        assert_eq!(
            inventory_entry_label("_cluster-admin_rbac.authorization.k8s.io_ClusterRoleBinding")
                .as_deref(),
            Some("ClusterRoleBinding cluster-admin")
        );
        assert_eq!(inventory_entry_label("garbage"), None);
    }

    #[test]
    fn kustomization_managed_comes_from_the_inventory_and_is_capped() {
        let entries: Vec<Value> = (0..(MANAGED_CAP + 5))
            .map(|i| json!({ "id": format!("default_cm-{i}__ConfigMap"), "v": "v1" }))
            .collect();
        let o = obj(json!({ "status": { "inventory": { "entries": entries } } }));
        let m = managed(&o, "Kustomization", "flux-system", "apps");
        assert_eq!(m.len(), MANAGED_CAP);
        assert_eq!(m[0], "ConfigMap default/cm-0");
    }

    #[test]
    fn helmrelease_managed_names_its_release_in_the_target_namespace() {
        let o = obj(json!({ "spec": { "targetNamespace": "web", "releaseName": "site" } }));
        assert_eq!(
            managed(&o, "HelmRelease", "flux-system", "podinfo"),
            vec!["Helm release web/site".to_owned()]
        );
        let bare = obj(json!({ "spec": {} }));
        assert_eq!(
            managed(&bare, "HelmRelease", "fathom-dev", "podinfo"),
            vec!["Helm release fathom-dev/podinfo".to_owned()]
        );
    }

    #[test]
    fn source_of_prefers_the_generated_helmchart_then_falls_back_to_the_ref() {
        let hr = obj(json!({
            "spec": { "chart": { "spec": { "sourceRef": { "kind": "HelmRepository", "name": "podinfo" } } } },
            "status": { "helmChart": "flux-system/fathom-dev-podinfo" }
        }));
        assert_eq!(
            source_of(&hr, "fathom-dev"),
            Some((
                "HelmChart".into(),
                "flux-system".into(),
                "fathom-dev-podinfo".into()
            ))
        );
        let ks = obj(
            json!({ "spec": { "sourceRef": { "kind": "GitRepository", "name": "fleet", "namespace": "flux-system" } } }),
        );
        assert_eq!(
            source_of(&ks, "apps"),
            Some(("GitRepository".into(), "flux-system".into(), "fleet".into()))
        );
        let same_ns =
            obj(json!({ "spec": { "chartRef": { "kind": "OCIRepository", "name": "chart" } } }));
        assert_eq!(
            source_of(&same_ns, "apps"),
            Some(("OCIRepository".into(), "apps".into(), "chart".into()))
        );
        assert_eq!(source_of(&obj(json!({ "spec": {} })), "apps"), None);
    }

    #[test]
    fn summarize_reads_a_helmrelease_chart_source() {
        let o = obj(json!({
            "spec": { "chart": { "spec": { "sourceRef": { "kind": "HelmRepository", "name": "podinfo" } } } }
        }));
        assert_eq!(
            summarize(&o, "HelmRelease", false).source_ref.as_deref(),
            Some("HelmRepository/podinfo")
        );
    }
}

/// Discover the Flux GitOps Toolkit CRDs installed in this cluster and build
/// a full overview. Returns `installed: false` (with empty lists) when no
/// Flux CRDs are found, so the UI can show an appropriate empty state.
pub(super) async fn scan_flux(client: &Client) -> anyhow::Result<FluxOverview> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&ListParams::default()).await?.items;

    let mut sources = Vec::new();
    for crd_name in SOURCE_CRDS {
        if let Some(discovered) = find_served(&crds, crd_name) {
            let kind = discovered.kind.clone();
            for obj in list_instances(client, &discovered).await {
                sources.push(summarize(&obj, &kind, true));
            }
        }
    }

    let kustomization_crd = find_served(&crds, KUSTOMIZATION_CRD);
    let helmrelease_crd = find_served(&crds, HELMRELEASE_CRD);

    let installed = !sources.is_empty() || kustomization_crd.is_some() || helmrelease_crd.is_some();

    let mut kustomizations = Vec::new();
    if let Some(discovered) = &kustomization_crd {
        let kind = discovered.kind.clone();
        for obj in list_instances(client, discovered).await {
            kustomizations.push(summarize(&obj, &kind, false));
        }
    }

    let mut helmreleases = Vec::new();
    if let Some(discovered) = &helmrelease_crd {
        let kind = discovered.kind.clone();
        for obj in list_instances(client, discovered).await {
            helmreleases.push(summarize(&obj, &kind, false));
        }
    }

    sources.sort_by(|a, b| {
        (a.kind.as_str(), a.namespace.as_str(), a.name.as_str()).cmp(&(
            b.kind.as_str(),
            b.namespace.as_str(),
            b.name.as_str(),
        ))
    });
    kustomizations.sort_by(|a, b| {
        (a.namespace.as_str(), a.name.as_str()).cmp(&(b.namespace.as_str(), b.name.as_str()))
    });
    helmreleases.sort_by(|a, b| {
        (a.namespace.as_str(), a.name.as_str()).cmp(&(b.namespace.as_str(), b.name.as_str()))
    });

    Ok(FluxOverview {
        installed,
        sources,
        kustomizations,
        helmreleases,
    })
}

/// Raw Kustomization and HelmRelease dynamic objects (kind, spec, status.inventory included),
/// for the Applications feature's Flux attribution (research.md R2/R3) — reuses this module's
/// existing CRD-discovery/listing rather than a second discovery pass.
pub(super) async fn list_kustomizations_and_helmreleases(
    client: &Client,
) -> anyhow::Result<(Vec<DynamicObject>, Vec<DynamicObject>)> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&ListParams::default()).await?.items;

    let kustomizations = match find_served(&crds, KUSTOMIZATION_CRD) {
        Some(discovered) => list_instances(client, &discovered).await,
        None => Vec::new(),
    };
    let helmreleases = match find_served(&crds, HELMRELEASE_CRD) {
        Some(discovered) => list_instances(client, &discovered).await,
        None => Vec::new(),
    };
    Ok((kustomizations, helmreleases))
}

#[cfg(test)]
mod live_cluster_tests {
    use super::*;

    /// Not run in CI - requires a real cluster with Flux installed and a
    /// working kubeconfig. `cargo test -p fathom scan_flux_against_live_cluster -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn scan_flux_against_live_cluster() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::try_default().await.expect("kube client");
        let overview = scan_flux(&client).await.expect("scan_flux");

        println!("installed: {}", overview.installed);
        for (label, items) in [
            ("sources", &overview.sources),
            ("kustomizations", &overview.kustomizations),
            ("helmreleases", &overview.helmreleases),
        ] {
            println!("-- {label} ({}) --", items.len());
            for item in items {
                println!(
                    "  [{:?}] {}/{} kind={} rev={} suspended={} msg={}",
                    item.ready,
                    item.namespace,
                    item.name,
                    item.kind,
                    item.revision,
                    item.suspended,
                    item.message
                );
            }
        }

        assert!(overview.installed);
        assert!(!overview.kustomizations.is_empty());
        assert!(!overview.helmreleases.is_empty());
    }
}
