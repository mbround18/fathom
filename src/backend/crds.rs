//! Generic, read-only Custom Resource Definition browsing (spec 003, User Story 3).
//!
//! Supersedes the hardcoded per-CRD pattern in `flux.rs`/`longhorn.rs` for *browsing*: instead of
//! one dedicated integration module per CRD kind, this discovers every installed CRD and reads
//! its instances generically through `kube_core::dynamic::DynamicObject`.
//!
//! Discovery reads the `apiextensions.k8s.io/v1` `CustomResourceDefinition` object directly
//! (`k8s_openapi`'s typed binding) rather than a two-hop `kube::discovery` API-group scan —
//! equivalent information, one list call, still against the `apiextensions.k8s.io` API that
//! research.md points at, and it avoids scanning every other installed API group just to find
//! CRDs. Instances are listed/fetched through `Api::<DynamicObject>::all_with`/`namespaced_with`,
//! scoped per CRD's discovered `Scope` (research.md R-CRD).
//!
//! Every command here runs while holding the backend state lock, so each one is bounded: the
//! per-CRD permission probes run concurrently with a per-probe timeout (a cluster with ~150 CRDs
//! took ~18s probed one after another, which stalled every other command, including the
//! instance list the operator had just asked for), and instance lists are bounded too.

use std::time::Duration;

use crossbeam_channel::Sender;
use futures::{StreamExt, stream};
use kube::{
    Client, Error as KubeError, ResourceExt,
    api::{Api, ApiResource, DynamicObject, GroupVersionKind, ListParams},
    core::Request,
    discovery::Scope,
};
use serde::Deserialize;

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, CrdCatalogEntry, CrdInstanceDetails,
    CrdListFailure, CrdScope, ResourceReadEvent, Route, SystemEvent, section_keys,
};

use super::{events, state::BackendState};

/// How many permission probes run at once during discovery.
const PROBE_CONCURRENCY: usize = 16;
/// A probe that has not answered by then counts as "not denied"; the instance list reports
/// the real failure if there is one.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Upper bound for one instance list (the backend is busy for as long as it runs).
const LIST_TIMEOUT: Duration = Duration::from_secs(20);

/// Route for this module's events: the Custom Resources section of the current context, so
/// results that arrive after a context switch are dropped by the UI instead of being shown
/// against the wrong cluster.
fn route(state: &BackendState) -> Route {
    Route::cluster(
        section_keys::custom_resources(),
        state.cluster.current_context.clone(),
    )
}

/// `RefreshCrds`: discover every installed CRD and emit the full catalog.
pub(super) async fn refresh_crds(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let entries = discover_crds(&client).await?;
    state.cluster.crd_catalog = entries.clone();

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        route(state),
        Correlation::None,
        BackendEvent::System(SystemEvent::CrdCatalogUpdated { entries }),
    );
    Ok(())
}

/// `RefreshCrdInstances`: list one CRD's instances, scoped per its discovered `Scope`.
///
/// Always answers with `CrdInstancesUpdated`: either the (possibly empty) list, or a typed
/// `failure` (forbidden, not found, other) so the UI can say why instead of showing an empty
/// list or waiting forever. The kind is looked up in the last-discovered catalog and, when the
/// catalog does not have it (not discovered yet on this connection, or a newer CRD), resolved
/// straight from the API server rather than answering with a silent empty list.
pub(super) async fn refresh_crd_instances(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    group: &str,
    version: &str,
    kind: &str,
    namespace: Option<&str>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let shape = match find_catalog_entry(state, group, kind) {
        Some(entry) => Ok((entry.plural.clone(), entry.scope)),
        None => resolve_kind(&client, group, version, kind).await,
    };
    let result = match shape {
        Ok((plural, scope)) => {
            let target = CrdInstanceRef {
                group,
                version,
                kind,
                plural: &plural,
                scope,
            };
            list_crd_instances(&client, target, namespace).await
        }
        Err(failure) => Err(failure),
    };
    let (instances, failure) = match result {
        Ok(instances) => (instances, None),
        Err(failure) => (Vec::new(), Some(failure)),
    };

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        route(state),
        Correlation::None,
        BackendEvent::System(SystemEvent::CrdInstancesUpdated {
            group: group.to_owned(),
            kind: kind.to_owned(),
            version: version.to_owned(),
            namespace: namespace.map(str::to_owned),
            instances,
            failure,
        }),
    );
    Ok(())
}

/// `SelectCrdInstance`: fetch one custom resource instance's details, emitted through the same
/// generic `ResourceRead` shape every built-in kind's select command uses (`raw_json`), so the
/// existing Inspector's YAML/Details tabs render it unchanged.
pub(super) async fn select_crd_instance(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    group: &str,
    version: &str,
    kind: &str,
    namespace: Option<&str>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let (plural, scope) = match find_catalog_entry(state, group, kind) {
        Some(entry) => (entry.plural.clone(), entry.scope),
        None => resolve_kind(&client, group, version, kind)
            .await
            .map_err(|f| anyhow::anyhow!(failure_message(&f).to_owned()))?,
    };
    let details = get_crd_instance(
        &client,
        CrdInstanceRef {
            group,
            version,
            kind,
            plural: &plural,
            scope,
        },
        namespace,
        name,
    )
    .await?;

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        route(state),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::CrdInstanceDetailsUpdated(details)),
    );
    Ok(())
}

fn find_catalog_entry<'a>(
    state: &'a BackendState,
    group: &str,
    kind: &str,
) -> Option<&'a CrdCatalogEntry> {
    state
        .cluster
        .crd_catalog
        .iter()
        .find(|e| e.group == group && e.kind == kind)
}

/// Resolve a kind's plural and scope from the API server (one discovery call for its
/// group/version) when the catalog does not have it.
async fn resolve_kind(
    client: &Client,
    group: &str,
    version: &str,
    kind: &str,
) -> Result<(String, CrdScope), CrdListFailure> {
    let gvk = GroupVersionKind::gvk(group, version, kind);
    match kube::discovery::pinned_kind(client, &gvk).await {
        Ok((ar, caps)) => Ok((ar.plural, scope_of(&caps.scope))),
        Err(KubeError::Discovery(_)) => Err(CrdListFailure::NotFound {
            message: format!("{kind}.{group}/{version} is not served by this cluster"),
        }),
        Err(err) => Err(failure_for(&err)),
    }
}

fn scope_of(scope: &Scope) -> CrdScope {
    match scope {
        Scope::Cluster => CrdScope::Cluster,
        Scope::Namespaced => CrdScope::Namespaced,
    }
}

/// Classify an API error for the instance list: 403 and 404 are named, everything else is
/// passed through as text.
fn failure_for(err: &KubeError) -> CrdListFailure {
    match err {
        KubeError::Api(resp) if resp.code == 403 => CrdListFailure::Forbidden {
            message: resp.message.clone(),
        },
        KubeError::Api(resp) if resp.code == 404 => CrdListFailure::NotFound {
            message: resp.message.clone(),
        },
        other => CrdListFailure::Failed {
            message: other.to_string(),
        },
    }
}

fn failure_message(failure: &CrdListFailure) -> &str {
    match failure {
        CrdListFailure::Forbidden { message }
        | CrdListFailure::NotFound { message }
        | CrdListFailure::Failed { message } => message,
    }
}

/// The versions a CRD serves, storage version first. The UI lists instances through the first
/// one, and reading at the storage version needs no conversion (a broken conversion webhook
/// fails every read at any other version). Falls back to every declared version when none is
/// marked served.
fn ordered_versions(versions: &[CrdVersion]) -> Vec<String> {
    let mut served: Vec<&CrdVersion> = versions.iter().filter(|v| v.served).collect();
    if served.is_empty() {
        served = versions.iter().collect();
    }
    // Stable sort: storage first, the rest keep the CRD's declared order.
    served.sort_by_key(|v| !v.storage);
    served.into_iter().map(|v| v.name.clone()).collect()
}

/// The few `CustomResourceDefinition` fields discovery needs. CRD objects carry their full
/// OpenAPI schemas (~10 MB for ~150 CRDs on a real cluster); decoding only these fields, and
/// skipping the rest, keeps discovery cheap.
#[derive(Debug, Deserialize)]
struct CrdList {
    items: Vec<Crd>,
}

#[derive(Debug, Deserialize)]
struct Crd {
    spec: CrdSpec,
}

#[derive(Debug, Deserialize)]
struct CrdSpec {
    group: String,
    names: CrdNames,
    scope: String,
    #[serde(default)]
    versions: Vec<CrdVersion>,
}

#[derive(Debug, Deserialize)]
struct CrdNames {
    kind: String,
    plural: String,
}

#[derive(Debug, Deserialize)]
struct CrdVersion {
    name: String,
    #[serde(default)]
    served: bool,
    #[serde(default)]
    storage: bool,
}

/// One CRD as declared, before its permission probe.
struct Candidate {
    group: String,
    kind: String,
    plural: String,
    versions: Vec<String>,
    scope: CrdScope,
}

fn candidate(crd: Crd) -> Option<Candidate> {
    let spec = crd.spec;
    let versions = ordered_versions(&spec.versions);
    if versions.is_empty() {
        return None; // No usable version; nothing to browse for this CRD.
    }
    let scope = if spec.scope == "Cluster" {
        CrdScope::Cluster
    } else {
        CrdScope::Namespaced
    };
    Some(Candidate {
        group: spec.group,
        kind: spec.names.kind,
        plural: spec.names.plural,
        versions,
        scope,
    })
}

/// Discover every installed CRD, classifying `Namespaced`/`Cluster` scope and probing whether
/// the current operator can list its instances (a 403 is captured per-entry rather than
/// aborting the whole scan, T050/T052/FR-019). Probes run concurrently and are bounded.
async fn discover_crds(client: &Client) -> anyhow::Result<Vec<CrdCatalogEntry>> {
    let started = std::time::Instant::now();
    let request = Request::new("/apis/apiextensions.k8s.io/v1/customresourcedefinitions")
        .list(&ListParams::default())
        .map_err(KubeError::BuildRequest)?;
    let crds: CrdList = client.request(request).await?;
    let listed = started.elapsed();

    let mut entries: Vec<CrdCatalogEntry> =
        stream::iter(crds.items.into_iter().filter_map(candidate))
            .map(|c| async move {
                let permission_denied = match c.versions.first() {
                    Some(version) => {
                        let ar = api_resource(&c.group, version, &c.kind, &c.plural);
                        probe_permission_denied(client, &ar).await
                    }
                    None => false,
                };
                CrdCatalogEntry {
                    group: c.group,
                    kind: c.kind,
                    plural: c.plural,
                    versions: c.versions,
                    scope: c.scope,
                    permission_denied,
                }
            })
            .buffer_unordered(PROBE_CONCURRENCY)
            .collect()
            .await;

    entries.sort_by(|a, b| a.group.cmp(&b.group).then(a.kind.cmp(&b.kind)));
    tracing::info!(
        crds = entries.len(),
        list_ms = listed.as_millis() as u64,
        total_ms = started.elapsed().as_millis() as u64,
        "CRD catalog discovered"
    );
    Ok(entries)
}

/// Cheap `limit=1` list to determine whether the operator can list this CRD's instances, so the
/// catalog itself carries a per-CRD permission-denied indicator (spec Acceptance Scenario 5)
/// before the operator even opens it. Probed cluster-wide even for namespaced CRDs (a
/// namespace-scoped `list` needs the same verb). Any error other than a 403, and a probe that
/// times out, is treated as "not denied" here — a real failure still surfaces, typed, when the
/// operator opens the instance list.
async fn probe_permission_denied(client: &Client, ar: &ApiResource) -> bool {
    let lp = ListParams::default().limit(1);
    let api = Api::<DynamicObject>::all_with(client.clone(), ar);
    match tokio::time::timeout(PROBE_TIMEOUT, api.list(&lp)).await {
        Ok(result) => is_forbidden(&result),
        Err(_) => false,
    }
}

fn is_forbidden<T>(result: &Result<T, KubeError>) -> bool {
    matches!(result, Err(KubeError::Api(resp)) if resp.code == 403)
}

fn api_resource(group: &str, version: &str, kind: &str, plural: &str) -> ApiResource {
    ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk(group, version, kind), plural)
}

fn dynamic_api(
    client: &Client,
    ar: &ApiResource,
    scope: CrdScope,
    namespace: Option<&str>,
) -> Api<DynamicObject> {
    match (scope, namespace) {
        (CrdScope::Namespaced, Some(ns)) => Api::namespaced_with(client.clone(), ns, ar),
        _ => Api::all_with(client.clone(), ar),
    }
}

async fn list_crd_instances(
    client: &Client,
    crd_ref: CrdInstanceRef<'_>,
    namespace: Option<&str>,
) -> Result<Vec<String>, CrdListFailure> {
    let CrdInstanceRef {
        group,
        version,
        kind,
        plural,
        scope,
    } = crd_ref;
    let ar = api_resource(group, version, kind, plural);
    let api = dynamic_api(client, &ar, scope, namespace);
    match tokio::time::timeout(LIST_TIMEOUT, api.list(&ListParams::default())).await {
        Ok(Ok(list)) => Ok(list
            .items
            .iter()
            .map(|obj| serde_json::to_string(obj).unwrap_or_default())
            .collect()),
        Ok(Err(err)) => Err(failure_for(&err)),
        Err(_) => Err(CrdListFailure::Failed {
            message: format!(
                "Listing {kind}.{group} timed out after {}s",
                LIST_TIMEOUT.as_secs()
            ),
        }),
    }
}

/// Bundles the group/version/kind/plural/scope quintet that identifies a CRD's REST shape, so
/// the list/get helpers take one struct instead of tripping clippy's `too_many_arguments` lint.
struct CrdInstanceRef<'a> {
    group: &'a str,
    version: &'a str,
    kind: &'a str,
    plural: &'a str,
    scope: CrdScope,
}

async fn get_crd_instance(
    client: &Client,
    crd_ref: CrdInstanceRef<'_>,
    namespace: Option<&str>,
    name: &str,
) -> anyhow::Result<CrdInstanceDetails> {
    let CrdInstanceRef {
        group,
        version,
        kind,
        plural,
        scope,
    } = crd_ref;
    let ar = api_resource(group, version, kind, plural);
    let api = dynamic_api(client, &ar, scope, namespace);
    let obj = api.get(name).await?;
    let resource_version = obj.resource_version().unwrap_or_default();
    let raw_json = serde_json::to_string_pretty(&obj).unwrap_or_default();
    Ok(CrdInstanceDetails {
        group: group.to_owned(),
        kind: kind.to_owned(),
        version: version.to_owned(),
        namespace: obj.namespace(),
        name: obj.name_any(),
        resource_version,
        raw_json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crd(group: &str, kind: &str, plural: &str, scope: &str) -> Crd {
        Crd {
            spec: CrdSpec {
                group: group.to_owned(),
                names: CrdNames {
                    kind: kind.to_owned(),
                    plural: plural.to_owned(),
                },
                scope: scope.to_owned(),
                versions: vec![version("v1", true, true)],
            },
        }
    }

    fn version(name: &str, served: bool, storage: bool) -> CrdVersion {
        CrdVersion {
            name: name.to_owned(),
            served,
            storage,
        }
    }

    /// The slim decode reads a real CRD object and ignores its schema.
    #[test]
    fn slim_crd_list_decodes_a_real_shape() {
        let json = serde_json::json!({
            "kind": "CustomResourceDefinitionList",
            "items": [{
                "metadata": {"name": "orders.acme.cert-manager.io"},
                "spec": {
                    "group": "acme.cert-manager.io",
                    "names": {"kind": "Order", "plural": "orders", "listKind": "OrderList"},
                    "scope": "Namespaced",
                    "versions": [{
                        "name": "v1", "served": true, "storage": true,
                        "schema": {"openAPIV3Schema": {"type": "object"}}
                    }]
                },
                "status": {"acceptedNames": {"kind": "Order", "plural": "orders"}}
            }]
        });
        let list: CrdList = serde_json::from_value(json).expect("decodes");
        let c = candidate(list.items.into_iter().next().expect("one")).expect("usable");
        assert_eq!(
            (c.group.as_str(), c.kind.as_str()),
            ("acme.cert-manager.io", "Order")
        );
        assert_eq!(c.versions, ["v1"]);
        assert_eq!(c.scope, CrdScope::Namespaced);
    }

    fn api_error(code: u16, message: &str) -> KubeError {
        KubeError::Api(kube::core::ErrorResponse {
            status: "Failure".to_owned(),
            message: message.to_owned(),
            reason: String::new(),
            code,
        })
    }

    /// T050: scope classification is a pure mapping from the CRD spec's `scope` string.
    #[test]
    fn scope_classification_from_crd_spec() {
        let namespaced = candidate(crd("example.com", "Widget", "widgets", "Namespaced"))
            .expect("has a version");
        assert_eq!(namespaced.scope, CrdScope::Namespaced);
        assert_eq!(namespaced.plural, "widgets");

        let cluster =
            candidate(crd("example.com", "Gadget", "gadgets", "Cluster")).expect("has a version");
        assert_eq!(cluster.scope, CrdScope::Cluster);
    }

    #[test]
    fn crd_without_versions_is_skipped() {
        let mut c = crd("example.com", "X", "xs", "Namespaced");
        c.spec.versions.clear();
        assert!(candidate(c).is_none());
    }

    /// #36: instances are listed through `versions[0]`, so it must be a served version, and the
    /// storage version when served (no conversion webhook involved).
    #[test]
    fn versions_are_served_only_storage_first() {
        let vs = [
            version("v1alpha1", false, false),
            version("v1beta1", true, false),
            version("v1beta2", true, false),
            version("v1", true, true),
        ];
        assert_eq!(ordered_versions(&vs), ["v1", "v1beta1", "v1beta2"]);

        // Storage version not served: served ones in declared order.
        let vs = [version("v1", true, false), version("v1beta1", false, true)];
        assert_eq!(ordered_versions(&vs), ["v1"]);

        // Nothing marked served: fall back to every declared version.
        let vs = [version("v1", false, true), version("v2", false, false)];
        assert_eq!(ordered_versions(&vs), ["v1", "v2"]);
    }

    /// #36: an instance list that cannot be read says why, instead of an empty list.
    #[test]
    fn list_failures_are_typed() {
        assert_eq!(
            failure_for(&api_error(403, "orders is forbidden")),
            CrdListFailure::Forbidden {
                message: "orders is forbidden".to_owned()
            }
        );
        assert_eq!(
            failure_for(&api_error(
                404,
                "the server could not find the requested resource"
            )),
            CrdListFailure::NotFound {
                message: "the server could not find the requested resource".to_owned()
            }
        );
        assert!(matches!(
            failure_for(&api_error(500, "conversion webhook failed")),
            CrdListFailure::Failed { message } if message.contains("conversion webhook failed")
        ));
    }

    #[test]
    fn only_403_marks_a_probe_denied() {
        assert!(is_forbidden::<()>(&Err(api_error(403, "no"))));
        assert!(!is_forbidden::<()>(&Err(api_error(404, "gone"))));
        assert!(!is_forbidden::<()>(&Err(api_error(500, "boom"))));
        assert!(!is_forbidden(&Ok(())));
    }

    /// T050: a catalog entry's `permission_denied` starts `false` and is only ever set by
    /// `probe_permission_denied` observing a 403 — never by any other error variant, so one
    /// CRD's outage or other API error cannot mark it permission-denied nor abort the scan for
    /// the rest of the catalog (each probe answers a bool; none can fail the scan).
    #[test]
    fn permission_denied_defaults_false_and_is_only_set_by_403() {
        let entry = CrdCatalogEntry {
            group: "example.com".to_owned(),
            kind: "Widget".to_owned(),
            plural: "widgets".to_owned(),
            versions: vec!["v1".to_owned()],
            scope: CrdScope::Namespaced,
            permission_denied: false,
        };
        assert!(!entry.permission_denied);
    }

    #[test]
    fn api_resource_builds_from_group_version_kind() {
        let ar = api_resource("example.com", "v1", "Widget", "widgets");
        assert_eq!(ar.group, "example.com");
        assert_eq!(ar.version, "v1");
        assert_eq!(ar.kind, "Widget");
        assert_eq!(ar.plural, "widgets");
    }
}
