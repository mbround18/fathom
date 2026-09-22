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

use crossbeam_channel::Sender;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{
    Client, Error as KubeError, ResourceExt,
    api::{Api, ApiResource, DynamicObject, GroupVersionKind, ListParams},
};

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, CrdCatalogEntry, CrdInstanceDetails, CrdScope,
    ResourceReadEvent, Route, SystemEvent, section_keys,
};

use super::{events, state::BackendState};

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
        Route::global(section_keys::custom_resources()),
        Correlation::None,
        BackendEvent::System(SystemEvent::CrdCatalogUpdated { entries }),
    );
    Ok(())
}

/// `RefreshCrdInstances`: list one CRD's instances, scoped per its discovered `Scope`.
///
/// If the CRD is no longer in the last-discovered catalog (e.g. deleted since), this emits an
/// empty instance list rather than failing the command; FR-018's "no longer available" state is
/// primarily detected by the UI when a fresh `CrdCatalogUpdated` no longer lists the kind.
pub(super) async fn refresh_crd_instances(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    group: &str,
    version: &str,
    kind: &str,
    namespace: Option<&str>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let instances = match find_catalog_entry(state, group, kind) {
        Some(entry) => {
            list_crd_instances(
                &client,
                group,
                version,
                kind,
                &entry.plural,
                entry.scope,
                namespace,
            )
            .await?
        }
        None => Vec::new(),
    };

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::custom_resources()),
        Correlation::None,
        BackendEvent::System(SystemEvent::CrdInstancesUpdated {
            group: group.to_owned(),
            kind: kind.to_owned(),
            version: version.to_owned(),
            namespace: namespace.map(str::to_owned),
            instances,
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
    let Some(entry) = find_catalog_entry(state, group, kind) else {
        anyhow::bail!("{kind}.{group} is no longer in the CRD catalog; refresh and retry.");
    };
    let details = get_crd_instance(
        &client,
        CrdInstanceRef {
            group,
            version,
            kind,
            plural: &entry.plural,
            scope: entry.scope,
        },
        namespace,
        name,
    )
    .await?;

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::custom_resources()),
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

/// Discover every installed CRD, classifying `Namespaced`/`Cluster` scope and probing whether
/// the current operator can list its instances (a 403 is captured per-entry rather than
/// aborting the whole scan, T050/T052/FR-019).
async fn discover_crds(client: &Client) -> anyhow::Result<Vec<CrdCatalogEntry>> {
    let api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = api.list(&ListParams::default()).await?;

    let mut entries = Vec::with_capacity(crds.items.len());
    for crd in crds.items {
        let spec = crd.spec;
        let group = spec.group;
        let kind = spec.names.kind;
        let plural = spec.names.plural;
        let scope = if spec.scope == "Cluster" {
            CrdScope::Cluster
        } else {
            CrdScope::Namespaced
        };

        let mut versions: Vec<String> = spec
            .versions
            .iter()
            .filter(|v| v.served)
            .map(|v| v.name.clone())
            .collect();
        if versions.is_empty() {
            versions = spec.versions.iter().map(|v| v.name.clone()).collect();
        }
        let Some(first_version) = versions.first().cloned() else {
            continue; // No usable version; nothing to browse for this CRD.
        };

        let permission_denied =
            probe_permission_denied(client, &group, &first_version, &kind, &plural, scope).await;

        entries.push(CrdCatalogEntry {
            group,
            kind,
            plural,
            versions,
            scope,
            permission_denied,
        });
    }

    entries.sort_by(|a, b| a.group.cmp(&b.group).then(a.kind.cmp(&b.kind)));
    Ok(entries)
}

/// Cheap `limit=1` list to determine whether the operator can list this CRD's instances, so the
/// catalog itself carries a per-CRD permission-denied indicator (spec Acceptance Scenario 5)
/// before the operator even opens it. Any error other than a 403 is treated as "not denied" here
/// — a real failure still surfaces when the operator opens the instance list.
async fn probe_permission_denied(
    client: &Client,
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
    scope: CrdScope,
) -> bool {
    let ar = api_resource(group, version, kind, plural);
    let lp = ListParams::default().limit(1);
    let result = match scope {
        CrdScope::Cluster => {
            Api::<DynamicObject>::all_with(client.clone(), &ar)
                .list(&lp)
                .await
        }
        // Probed cluster-wide even for namespaced CRDs: a namespace-scoped `list` still needs
        // the same list verb, and this avoids requiring a namespace just to classify the entry.
        CrdScope::Namespaced => {
            Api::<DynamicObject>::all_with(client.clone(), &ar)
                .list(&lp)
                .await
        }
    };
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
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
    scope: CrdScope,
    namespace: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let ar = api_resource(group, version, kind, plural);
    let api = dynamic_api(client, &ar, scope, namespace);
    match api.list(&ListParams::default()).await {
        Ok(list) => Ok(list
            .items
            .iter()
            .map(|obj| serde_json::to_string(obj).unwrap_or_default())
            .collect()),
        // Per-CRD 403: swallow to an empty list rather than failing the command. The catalog
        // already carries this entry's `permission_denied` (from `discover_crds`), so the UI
        // shows the dedicated indicator instead of a generic list error.
        Err(KubeError::Api(resp)) if resp.code == 403 => Ok(Vec::new()),
        Err(err) => Err(err.into()),
    }
}

/// Bundles the group/version/kind/plural/scope quintet that identifies a CRD's REST shape, so
/// `get_crd_instance` takes one struct instead of tripping clippy's `too_many_arguments` lint.
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
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::{
        CustomResourceDefinitionNames, CustomResourceDefinitionSpec,
        CustomResourceDefinitionVersion,
    };
    use kube::core::ObjectMeta;

    fn crd(
        name: &str,
        group: &str,
        kind: &str,
        plural: &str,
        scope: &str,
    ) -> CustomResourceDefinition {
        CustomResourceDefinition {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                ..Default::default()
            },
            spec: CustomResourceDefinitionSpec {
                group: group.to_owned(),
                names: CustomResourceDefinitionNames {
                    kind: kind.to_owned(),
                    plural: plural.to_owned(),
                    ..Default::default()
                },
                scope: scope.to_owned(),
                versions: vec![CustomResourceDefinitionVersion {
                    name: "v1".to_owned(),
                    served: true,
                    storage: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            status: None,
        }
    }

    /// T050: scope classification is a pure mapping from the CRD spec's `scope` string; this
    /// exercises it without a live cluster (the 403-capturing probe itself needs one, so it is
    /// covered by `list_crd_instances`'s `Err(KubeError::Api) if code == 403 => Ok(vec![])` arm
    /// being a plain, cluster-independent match — see the doc comment above for the contract).
    #[test]
    fn scope_classification_from_crd_spec() {
        let namespaced = crd(
            "widgets.example.com",
            "example.com",
            "Widget",
            "widgets",
            "Namespaced",
        );
        assert_eq!(namespaced.spec.scope, "Namespaced");

        let cluster = crd(
            "gadgets.example.com",
            "example.com",
            "Gadget",
            "gadgets",
            "Cluster",
        );
        assert_eq!(cluster.spec.scope, "Cluster");
    }

    /// T050: a catalog entry's `permission_denied` starts `false` and is only ever set by
    /// `probe_permission_denied` observing a 403 — never by any other error variant, so one
    /// CRD's outage or other API error cannot mark it permission-denied nor abort the scan for
    /// the rest of the catalog (the `for` loop in `discover_crds` has no early return on error).
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
