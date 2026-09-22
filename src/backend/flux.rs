//! Flux CD GitOps Toolkit integration.
//!
//! Discovers whichever served API version of the Flux CRDs is installed
//! (source.toolkit.fluxcd.io, kustomize.toolkit.fluxcd.io,
//! helm.toolkit.fluxcd.io) and reads GitRepositories, OCIRepositories,
//! HelmRepositories, Buckets, Kustomizations, and HelmReleases as dynamic
//! objects - the same discovery pattern used for Longhorn in `longhorn.rs`,
//! so this works across Flux versions without pinning an API version.

use crate::protocol::{FluxOverview, FluxReadyState, FluxResourceSummary};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::ListParams;
use kube::core::DynamicObject;
use kube::core::{ApiResource, GroupVersionKind};
use kube::{Client, api::Api};
use serde_json::Value;

/// CRD names (plural.group) we look for, and which overview bucket they belong to.
const SOURCE_CRDS: &[&str] = &[
    "gitrepositories.source.toolkit.fluxcd.io",
    "ocirepositories.source.toolkit.fluxcd.io",
    "helmrepositories.source.toolkit.fluxcd.io",
    "buckets.source.toolkit.fluxcd.io",
];
const KUSTOMIZATION_CRD: &str = "kustomizations.kustomize.toolkit.fluxcd.io";
const HELMRELEASE_CRD: &str = "helmreleases.helm.toolkit.fluxcd.io";

struct DiscoveredCrd {
    group: String,
    version: String,
    kind: String,
}

/// Find a served version for `crd_name` among already-listed CRDs.
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

/// List every instance of a discovered CRD as dynamic objects, across all namespaces.
async fn list_instances(client: &Client, crd: &DiscoveredCrd) -> Vec<DynamicObject> {
    let gvk = GroupVersionKind::gvk(&crd.group, &crd.version, &crd.kind);
    let resource = ApiResource::from_gvk(&gvk);
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

    let source_ref = spec.and_then(|s| s.get("sourceRef")).map(|sr| {
        let sr_kind = sr.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let sr_name = sr.get("name").and_then(|v| v.as_str()).unwrap_or("");
        format!("{sr_kind}/{sr_name}")
    });

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
