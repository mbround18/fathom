//! Reads Helm 3's own release-storage Secret/ConfigMap objects directly (no `helm` binary
//! shellout, research.md R1). A release is a Secret or ConfigMap labeled `owner=helm`, `name=
//! <release>`, `status=<status>`, `version=<revision>` whose `release` data key holds
//! base64(gzip(JSON)) of the full release object; the current release is the storage object
//! with the highest `version` label for a given `(namespace, name)` pair, and every other
//! object sharing that pair is a prior revision (data-model.md).

use std::io::Read;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use flate2::read::GzDecoder;
use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use kube::{
    Client, ResourceExt,
    api::{Api, ListParams},
};
use serde_json::Value;

use crate::protocol::{
    BackendEvent, Correlation, GitOpsController, GitOpsOriginRef, HelmMutationAction,
    HelmMutationApplied, HelmMutationFailed, HelmMutationPreviewReady, HelmMutationStage,
    HelmReleaseDetails, HelmReleaseStatus, HelmReleaseSummary, HelmRevisionSummary,
    ResourceListEvent, ResourceReadEvent, Route, section_keys,
};

use super::{events, flux, state::BackendState};

const HELM_OWNER_LABEL: &str = "owner";
const HELM_OWNER_VALUE: &str = "helm";
const HELM_NAME_LABEL: &str = "name";
const HELM_STATUS_LABEL: &str = "status";
const HELM_VERSION_LABEL: &str = "version";
const HELM_RELEASE_DATA_KEY: &str = "release";

const HELM_RELEASE_NAME_ANNOTATION: &str = "meta.helm.sh/release-name";
const HELM_RELEASE_NAMESPACE_ANNOTATION: &str = "meta.helm.sh/release-namespace";
const FLUX_HELMRELEASE_NAME_LABEL: &str = "helm.toolkit.fluxcd.io/name";
const FLUX_HELMRELEASE_NAMESPACE_LABEL: &str = "helm.toolkit.fluxcd.io/namespace";
const FLUX_KUSTOMIZATION_NAME_LABEL: &str = "kustomize.toolkit.fluxcd.io/name";
const FLUX_KUSTOMIZATION_NAMESPACE_LABEL: &str = "kustomize.toolkit.fluxcd.io/namespace";

/// One Helm release-storage object, decoded far enough to attribute and sort it, with its full
/// payload lazily available for the currently-selected revision only.
struct StorageObject {
    namespace: String,
    release_name: String,
    revision: i64,
    status_label: String,
    /// `Some` once base64/gzip/JSON decode succeeded; `None` means `degraded: true`.
    payload: Option<Value>,
}

/// base64-decode, gunzip, then JSON-parse Helm's release-storage payload. Unit-testable
/// without a live cluster (contracts/protocol.md).
fn decode_release_payload(base64_gzip_json: &str) -> anyhow::Result<Value> {
    let gz_bytes = BASE64.decode(base64_gzip_json.trim())?;
    let mut decoder = GzDecoder::new(&gz_bytes[..]);
    let mut json_bytes = Vec::new();
    decoder.read_to_end(&mut json_bytes)?;
    Ok(serde_json::from_slice(&json_bytes)?)
}

fn status_from_label(status: &str) -> HelmReleaseStatus {
    match status {
        "deployed" => HelmReleaseStatus::Deployed,
        "failed" => HelmReleaseStatus::Failed,
        "pending-install" => HelmReleaseStatus::PendingInstall,
        "pending-upgrade" => HelmReleaseStatus::PendingUpgrade,
        "pending-rollback" => HelmReleaseStatus::PendingRollback,
        "superseded" => HelmReleaseStatus::Superseded,
        "uninstalled" => HelmReleaseStatus::Uninstalled,
        "uninstalling" => HelmReleaseStatus::Uninstalling,
        _ => HelmReleaseStatus::Unknown,
    }
}

/// Deep-merge `overrides` onto `base` (Helm's `config` onto `chart.values`), used to build
/// `HelmReleaseDetails.values_effective`.
fn merge_values(base: &Value, overrides: &Value) -> Value {
    match (base, overrides) {
        (Value::Object(base_map), Value::Object(override_map)) => {
            let mut merged = base_map.clone();
            for (k, v) in override_map {
                let entry = merged
                    .entry(k.clone())
                    .or_insert(Value::Null);
                *entry = merge_values(entry, v);
            }
            Value::Object(merged)
        }
        (_, override_value) => override_value.clone(),
    }
}

fn to_yaml(value: &Value) -> String {
    serde_yaml::to_string(value).unwrap_or_default()
}

async fn list_storage_objects(
    client: &Client,
    namespace: Option<&str>,
) -> anyhow::Result<Vec<StorageObject>> {
    let label_selector = format!("{HELM_OWNER_LABEL}={HELM_OWNER_VALUE}");
    let list_params = ListParams::default().labels(&label_selector);

    let mut out = Vec::new();

    let secrets: Api<Secret> = match namespace {
        Some(ns) => Api::namespaced(client.clone(), ns),
        None => Api::all(client.clone()),
    };
    for secret in secrets.list(&list_params).await?.items {
        let Some(obj) = storage_object_from_secret(&secret) else {
            continue;
        };
        out.push(obj);
    }

    let configmaps: Api<ConfigMap> = match namespace {
        Some(ns) => Api::namespaced(client.clone(), ns),
        None => Api::all(client.clone()),
    };
    for cm in configmaps.list(&list_params).await?.items {
        let Some(obj) = storage_object_from_configmap(&cm) else {
            continue;
        };
        out.push(obj);
    }

    Ok(out)
}

fn storage_object_from_secret(secret: &Secret) -> Option<StorageObject> {
    let labels = secret.metadata.labels.clone().unwrap_or_default();
    let release_name = labels.get(HELM_NAME_LABEL)?.clone();
    let namespace = secret.namespace().unwrap_or_default();
    let revision: i64 = labels
        .get(HELM_VERSION_LABEL)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let status_label = labels.get(HELM_STATUS_LABEL).cloned().unwrap_or_default();
    let payload = secret
        .data
        .as_ref()
        .and_then(|d| d.get(HELM_RELEASE_DATA_KEY))
        .and_then(|b| String::from_utf8(b.0.clone()).ok())
        .and_then(|s| decode_release_payload(&s).ok());
    Some(StorageObject {
        namespace,
        release_name,
        revision,
        status_label,
        payload,
    })
}

fn storage_object_from_configmap(cm: &ConfigMap) -> Option<StorageObject> {
    let labels = cm.metadata.labels.clone().unwrap_or_default();
    let release_name = labels.get(HELM_NAME_LABEL)?.clone();
    let namespace = cm.namespace().unwrap_or_default();
    let revision: i64 = labels
        .get(HELM_VERSION_LABEL)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let status_label = labels.get(HELM_STATUS_LABEL).cloned().unwrap_or_default();
    let payload = cm
        .data
        .as_ref()
        .and_then(|d| d.get(HELM_RELEASE_DATA_KEY))
        .and_then(|s| decode_release_payload(s).ok());
    Some(StorageObject {
        namespace,
        release_name,
        revision,
        status_label,
        payload,
    })
}

fn payload_str(payload: &Value, pointer: &str) -> Option<String> {
    payload
        .pointer(pointer)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
}

fn summary_from_storage_object(obj: &StorageObject) -> HelmReleaseSummary {
    match &obj.payload {
        Some(payload) => HelmReleaseSummary {
            name: obj.release_name.clone(),
            namespace: obj.namespace.clone(),
            chart: payload_str(payload, "/chart/metadata/name").unwrap_or_default(),
            chart_version: payload_str(payload, "/chart/metadata/version").unwrap_or_default(),
            app_version: payload_str(payload, "/chart/metadata/appVersion"),
            revision: obj.revision,
            status: status_from_label(&obj.status_label),
            updated: payload_str(payload, "/info/last_deployed").unwrap_or_default(),
            degraded: false,
        },
        None => HelmReleaseSummary {
            name: obj.release_name.clone(),
            namespace: obj.namespace.clone(),
            chart: String::new(),
            chart_version: String::new(),
            app_version: None,
            revision: obj.revision,
            status: status_from_label(&obj.status_label),
            updated: String::new(),
            degraded: true,
        },
    }
}

/// The current revision per `(namespace, release_name)`: the storage object with the highest
/// `version` label (data-model.md).
fn current_per_release(objects: &[StorageObject]) -> Vec<&StorageObject> {
    use std::collections::BTreeMap;
    let mut current: BTreeMap<(&str, &str), &StorageObject> = BTreeMap::new();
    for obj in objects {
        let key = (obj.namespace.as_str(), obj.release_name.as_str());
        current
            .entry(key)
            .and_modify(|existing| {
                if obj.revision > existing.revision {
                    *existing = obj;
                }
            })
            .or_insert(obj);
    }
    current.into_values().collect()
}

pub(super) async fn list_releases(
    state: &BackendState,
    sender: &crossbeam_channel::Sender<crate::protocol::BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let objects = list_storage_objects(&client, None).await?;
    let mut summaries: Vec<HelmReleaseSummary> = current_per_release(&objects)
        .into_iter()
        .filter(|obj| namespace_allowed(&state.cluster.namespace_filter, &obj.namespace))
        .map(summary_from_storage_object)
        .collect();
    summaries.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::helm_releases(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::HelmReleasesUpdated(summaries)),
    );
    Ok(())
}

fn revision_summary(obj: &StorageObject) -> HelmRevisionSummary {
    match &obj.payload {
        Some(payload) => HelmRevisionSummary {
            revision: obj.revision,
            status: status_from_label(&obj.status_label),
            description: payload_str(payload, "/info/description"),
            updated: payload_str(payload, "/info/last_deployed").unwrap_or_default(),
            app_version: payload_str(payload, "/chart/metadata/appVersion"),
            chart_version: payload_str(payload, "/chart/metadata/version").unwrap_or_default(),
        },
        None => HelmRevisionSummary {
            revision: obj.revision,
            status: status_from_label(&obj.status_label),
            description: None,
            updated: String::new(),
            app_version: None,
            chart_version: String::new(),
        },
    }
}

/// Origin attribution for a release (Flux ownership labels on its storage object), same
/// precedence family as `applications.rs::attribute_from_meta` but scoped to just the Flux
/// case, since a bare-Helm release has no origin by definition (FR-011).
fn origin_from_labels(
    labels: &std::collections::BTreeMap<String, String>,
    annotations: &std::collections::BTreeMap<String, String>,
    namespace: &str,
) -> Option<(String, String, String)> {
    if let Some(name) = labels.get(FLUX_HELMRELEASE_NAME_LABEL) {
        let ns = labels
            .get(FLUX_HELMRELEASE_NAMESPACE_LABEL)
            .cloned()
            .unwrap_or_else(|| namespace.to_owned());
        return Some(("HelmRelease".to_owned(), ns, name.clone()));
    }
    if let Some(name) = labels.get(FLUX_KUSTOMIZATION_NAME_LABEL) {
        let ns = labels
            .get(FLUX_KUSTOMIZATION_NAMESPACE_LABEL)
            .cloned()
            .unwrap_or_else(|| namespace.to_owned());
        return Some(("Kustomization".to_owned(), ns, name.clone()));
    }
    let _ = annotations;
    None
}

/// Filters owned workloads down to those stamped with this release's Helm
/// release-name/namespace annotations, mirroring `applications.rs`'s attribution.
fn owned_workloads_for_release<'a>(
    workloads: &'a [super::applications::OwnedWorkload],
    namespace: &str,
    name: &str,
) -> Vec<&'a super::applications::OwnedWorkload> {
    workloads
        .iter()
        .filter(|w| annotations_match_release(w.annotations(), namespace, name))
        .collect()
}

/// Pure predicate behind `owned_workloads_for_release`, split out so it can be unit-tested
/// without needing to construct a private `OwnedWorkload`.
fn annotations_match_release(
    annotations: &std::collections::BTreeMap<String, String>,
    namespace: &str,
    name: &str,
) -> bool {
    annotations.get(HELM_RELEASE_NAME_ANNOTATION).map(String::as_str) == Some(name)
        && annotations
            .get(HELM_RELEASE_NAMESPACE_ANNOTATION)
            .map(String::as_str)
            == Some(namespace)
}

/// Origin lookup: the release's own owned workloads carry the Flux ownership labels (a Flux
/// HelmRelease stamps them onto every resource it installs via Helm — research.md R3).
async fn resolve_release_origin(
    client: &Client,
    owned: &[&super::applications::OwnedWorkload],
    namespace: &str,
) -> Option<GitOpsOriginRef> {
    let mut origin_source: Option<(String, String, String)> = None;
    for w in owned {
        if let Some(found) = origin_from_labels(w.labels(), w.annotations(), namespace) {
            origin_source = Some(found);
            break;
        }
    }

    let (kind, origin_ns, origin_name) = origin_source?;
    let flux_status = lookup_flux_resource(client, &kind, &origin_ns, &origin_name).await;
    Some(GitOpsOriginRef {
        controller: GitOpsController::Flux,
        section: section_keys::flux_resource(),
        kind,
        namespace: origin_ns,
        name: origin_name,
        ready: flux_status
            .as_ref()
            .map(|s| s.ready)
            .unwrap_or(crate::protocol::FluxReadyState::Unknown),
        message: flux_status.map(|s| s.message),
    })
}

pub(super) async fn get_release_details(
    state: &BackendState,
    sender: &crossbeam_channel::Sender<crate::protocol::BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let objects = list_storage_objects(&client, Some(namespace)).await?;
    let mut matching: Vec<&StorageObject> = objects
        .iter()
        .filter(|o| o.release_name == name)
        .collect();
    matching.sort_by_key(|o| std::cmp::Reverse(o.revision));

    let Some(current) = matching.first() else {
        return Ok(());
    };

    let revisions: Vec<HelmRevisionSummary> = matching.iter().map(|o| revision_summary(o)).collect();

    let (values_user_supplied, values_effective, manifest, description) = match &current.payload {
        Some(payload) => {
            let config = payload.get("config").cloned().unwrap_or(Value::Null);
            let chart_values = payload
                .pointer("/chart/values")
                .cloned()
                .unwrap_or(Value::Null);
            let effective = merge_values(&chart_values, &config);
            (
                to_yaml(&config),
                to_yaml(&effective),
                payload_str(payload, "/manifest").unwrap_or_default(),
                payload_str(payload, "/info/description"),
            )
        }
        None => (String::new(), String::new(), String::new(), None),
    };

    let workloads = super::applications::list_owned_workloads(state).await?;
    let owned = owned_workloads_for_release(&workloads, namespace, name);
    let origin = resolve_release_origin(&client, &owned, namespace).await;

    let (related, _pods) =
        super::applications::owned_related_resources_and_pods(&client, &owned).await;

    let details = HelmReleaseDetails {
        name: name.to_owned(),
        namespace: namespace.to_owned(),
        chart: current
            .payload
            .as_ref()
            .and_then(|p| payload_str(p, "/chart/metadata/name"))
            .unwrap_or_default(),
        chart_version: current
            .payload
            .as_ref()
            .and_then(|p| payload_str(p, "/chart/metadata/version"))
            .unwrap_or_default(),
        app_version: current
            .payload
            .as_ref()
            .and_then(|p| payload_str(p, "/chart/metadata/appVersion")),
        revision: current.revision,
        status: status_from_label(&current.status_label),
        updated: current
            .payload
            .as_ref()
            .and_then(|p| payload_str(p, "/info/last_deployed"))
            .unwrap_or_default(),
        description,
        values_user_supplied,
        values_effective,
        manifest,
        revisions,
        origin,
        related,
        degraded: current.payload.is_none(),
    };

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::helm_releases(),
            state.cluster.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::HelmReleaseDetailsUpdated(details)),
    );
    Ok(())
}

struct FluxStatus {
    ready: crate::protocol::FluxReadyState,
    message: String,
}

async fn lookup_flux_resource(
    client: &Client,
    kind: &str,
    namespace: &str,
    name: &str,
) -> Option<FluxStatus> {
    let (kustomizations, helmreleases) =
        flux::list_kustomizations_and_helmreleases(client).await.ok()?;
    let candidates = if kind == "Kustomization" {
        &kustomizations
    } else {
        &helmreleases
    };
    let obj = candidates.iter().find(|o| {
        o.metadata.namespace.as_deref() == Some(namespace)
            && o.metadata.name.as_deref() == Some(name)
    })?;
    let summary = flux::summarize(obj, kind, false);
    Some(FluxStatus {
        ready: summary.ready,
        message: summary.message,
    })
}

/// `RefreshRelatedResources` dispatch target for `kind == "HelmRelease"`: owned-workload
/// `"owns"` relations (Helm's own `meta.helm.sh/*` tracking annotations, since Helm-installed
/// resources typically carry no `ownerReferences` back to the release) plus, when the release
/// has a Flux origin, one `"reconciled-by"` relation (data-model.md).
pub(super) async fn related_for_helm_release(
    state: &BackendState,
    namespace: &str,
    name: &str,
) -> anyhow::Result<Vec<crate::protocol::RelatedResourceRef>> {
    let client = state.cluster.client_or_err()?;
    let workloads = super::applications::list_owned_workloads(state).await?;
    let owned = owned_workloads_for_release(&workloads, namespace, name);
    let (mut related, _pods) =
        super::applications::owned_related_resources_and_pods(&client, &owned).await;

    for w in &owned {
        if let Some((kind, origin_ns, origin_name)) =
            origin_from_labels(w.labels(), w.annotations(), namespace)
        {
            let flux_status = lookup_flux_resource(&client, &kind, &origin_ns, &origin_name).await;
            related.push(reconciled_by_ref(
                kind,
                origin_ns,
                origin_name,
                flux_status.map(|s| s.ready.label().to_owned()),
            ));
            break;
        }
    }

    Ok(related)
}

fn reconciled_by_ref(
    kind: String,
    origin_namespace: String,
    origin_name: String,
    status_hint: Option<String>,
) -> crate::protocol::RelatedResourceRef {
    crate::protocol::RelatedResourceRef {
        section: section_keys::flux_resource(),
        kind,
        namespace: Some(origin_namespace),
        name: origin_name,
        relation: "reconciled-by".to_owned(),
        status_hint,
    }
}

fn namespace_allowed(filter: &[String], namespace: &str) -> bool {
    filter.is_empty() || filter.iter().any(|ns| ns == namespace)
}

// --- Helm mutate (006): upgrade preview/apply and uninstall shell out to the operator's local
// `helm` binary (research.md R4 — no Go-template-rendering crate exists in this workspace).
// Rollback needs no rendering (the target revision's manifest is already stored verbatim) and
// is implemented directly against release-storage reads/writes, like the rest of this module.

/// Whether the `helm` binary is reachable on `PATH` (FR-012's "missing `helm` binary" error
/// path for upgrade/uninstall — rollback never needs this).
pub async fn helm_binary_available() -> bool {
    tokio::process::Command::new("helm")
        .arg("version")
        .output()
        .await
        .is_ok_and(|out| out.status.success())
}

fn mutation_failed(
    namespace: &str,
    release: &str,
    action: HelmMutationAction,
    stage: HelmMutationStage,
    message: impl Into<String>,
) -> HelmMutationFailed {
    HelmMutationFailed {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action,
        stage,
        message: message.into(),
    }
}

/// FR-012's "missing `helm` binary" guard, shared by preview and apply. Takes the already-
/// evaluated `helm_available` flag (rather than calling `helm_binary_available()` itself) so it
/// stays a pure, synchronous, directly unit-testable function (T020): with `helm_available:
/// false` it returns `Err` without anything upstream ever spawning a subprocess.
fn guard_helm_available(
    namespace: &str,
    release: &str,
    action: HelmMutationAction,
    stage: HelmMutationStage,
    helm_available: bool,
) -> Result<(), HelmMutationFailed> {
    if helm_available {
        return Ok(());
    }
    Err(mutation_failed(
        namespace,
        release,
        action,
        stage,
        "The `helm` binary was not found on PATH. Upgrade requires helm to be installed and reachable.",
    ))
}

/// The stale-`base_revision` guard shared by rollback/upgrade/uninstall apply (FR-012): the
/// release must still be at the revision the preview/confirm dialog was computed against. Pure
/// and synchronous (T022) so it is unit-testable without a cluster or a `helm` subprocess.
fn check_base_revision_fresh(
    namespace: &str,
    release: &str,
    action: HelmMutationAction,
    live_revision: i64,
    base_revision: i64,
) -> Result<(), HelmMutationFailed> {
    if live_revision == base_revision {
        return Ok(());
    }
    Err(mutation_failed(
        namespace,
        release,
        action,
        HelmMutationStage::Apply,
        format!(
            "The release has changed since this was previewed (was revision {base_revision}, is now revision {live_revision}). Preview again before applying."
        ),
    ))
}

/// Snapshot of a release's current live state, read directly from release storage (no `helm`
/// shellout — same read path as `get_release_details`): supplies `base_revision`/
/// `current_manifest`/`gitops_warning` for the upgrade preview, and the chart payload to
/// materialize for rendering.
struct CurrentReleaseState {
    revision: i64,
    manifest: String,
    chart_payload: Option<Value>,
    origin: Option<GitOpsOriginRef>,
}

async fn current_release_state(
    state: &BackendState,
    namespace: &str,
    release: &str,
) -> anyhow::Result<Option<CurrentReleaseState>> {
    let client = state.cluster.client_or_err()?;
    let objects = list_storage_objects(&client, Some(namespace)).await?;
    let mut matching: Vec<&StorageObject> =
        objects.iter().filter(|o| o.release_name == release).collect();
    matching.sort_by_key(|o| std::cmp::Reverse(o.revision));
    let Some(current) = matching.first() else {
        return Ok(None);
    };

    let manifest = current
        .payload
        .as_ref()
        .and_then(|p| payload_str(p, "/manifest"))
        .unwrap_or_default();
    let chart_payload = current.payload.as_ref().and_then(|p| p.pointer("/chart")).cloned();

    let workloads = super::applications::list_owned_workloads(state).await?;
    let owned = owned_workloads_for_release(&workloads, namespace, release);
    let origin = resolve_release_origin(&client, &owned, namespace).await;

    Ok(Some(CurrentReleaseState {
        revision: current.revision,
        manifest,
        chart_payload,
        origin,
    }))
}

/// FR-004's "different chart version" edge case: Fathom has no chart-repository integration, so
/// only the release's own currently-installed chart (materialized from its stored payload) can
/// be re-rendered locally — a requested version that differs from what's stored is rejected
/// with a clear error (research.md R4) rather than silently rendering the wrong chart.
fn check_chart_version(
    namespace: &str,
    release: &str,
    action: HelmMutationAction,
    stage: HelmMutationStage,
    chart_payload: &Value,
    requested: Option<&str>,
) -> Result<(), HelmMutationFailed> {
    let Some(requested) = requested else {
        return Ok(());
    };
    let current_chart_version = chart_payload
        .pointer("/metadata/version")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if requested == current_chart_version {
        return Ok(());
    }
    Err(mutation_failed(
        namespace,
        release,
        action,
        stage,
        format!(
            "Cannot upgrade to chart version {requested}: Fathom has no configured chart repository to resolve it from (only the release's currently-installed chart, version {current_chart_version}, can be re-rendered locally). Use the helm CLI directly for a chart-version change."
        ),
    ))
}

/// Writes a release's stored chart (metadata/values/templates/files, decoded from its own
/// release-storage payload — the same payload `get_release_details` already reads
/// `/chart/metadata`, `/chart/values` from) to a temp directory so `helm upgrade [--dry-run]`
/// can render it without needing network access to a chart repository (FR-004, research.md R4).
fn materialize_chart(chart_payload: &Value) -> anyhow::Result<tempfile::TempDir> {
    let dir = tempfile::Builder::new().prefix("fathom-helm-upgrade-").tempdir()?;
    let root = dir.path();

    let metadata = chart_payload.get("metadata").cloned().unwrap_or(Value::Null);
    std::fs::write(root.join("Chart.yaml"), to_yaml(&metadata))?;

    let values = chart_payload.get("values").cloned().unwrap_or(Value::Null);
    std::fs::write(root.join("values.yaml"), to_yaml(&values))?;

    if let Some(templates) = chart_payload.get("templates").and_then(Value::as_array) {
        for file in templates {
            write_chart_file(root, file)?;
        }
    }
    if let Some(files) = chart_payload.get("files").and_then(Value::as_array) {
        for file in files {
            write_chart_file(root, file)?;
        }
    }

    Ok(dir)
}

fn write_chart_file(root: &std::path::Path, file: &Value) -> anyhow::Result<()> {
    let name = file
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("chart file entry missing a name"))?;
    let data_b64 = file.get("data").and_then(Value::as_str).unwrap_or_default();
    let bytes = BASE64.decode(data_b64.as_bytes()).unwrap_or_default();
    let path = root.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Runs `helm upgrade <release> <chart_dir> -n <namespace> [--dry-run] -f -`, piping
/// `values_yaml` via stdin (integrations.rs:352-style async subprocess shellout — spawn, no
/// blocking calls). `dry_run` selects preview vs. real apply.
async fn run_helm_upgrade(
    release: &str,
    chart_dir: &std::path::Path,
    namespace: &str,
    values_yaml: &str,
    dry_run: bool,
) -> anyhow::Result<std::process::Output> {
    use tokio::io::AsyncWriteExt;

    let mut cmd = tokio::process::Command::new("helm");
    cmd.arg("upgrade")
        .arg(release)
        .arg(chart_dir)
        .arg("-n")
        .arg(namespace)
        .arg("-f")
        .arg("-");
    if dry_run {
        cmd.arg("--dry-run");
    }
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(values_yaml.as_bytes()).await?;
    }
    Ok(child.wait_with_output().await?)
}

/// `helm`'s own stderr, verbatim (Constitution Principle I); falls back to stdout for the rare
/// case a `helm` failure writes nothing to stderr.
fn helm_error_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        stderr.trim().to_owned()
    } else {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}

/// Extracts the `MANIFEST:` section from `helm upgrade [--dry-run]`'s stdout (everything after
/// the `MANIFEST:` line, up to the next top-level `NOTES:` section or end of output).
fn extract_manifest_section(stdout: &str) -> String {
    let Some(start) = stdout.find("MANIFEST:") else {
        return stdout.trim().to_owned();
    };
    let after = &stdout[start + "MANIFEST:".len()..];
    let manifest = match after.find("\nNOTES:") {
        Some(end) => &after[..end],
        None => after,
    };
    manifest.trim().to_owned()
}

/// Extracts the new revision number from `helm upgrade`'s stdout (`REVISION: <n>` line).
fn extract_revision(stdout: &str) -> Option<i64> {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("REVISION:")?.trim().parse().ok())
}

/// T024: compute an upgrade diff-preview via `helm upgrade --dry-run` against an already-known
/// current release state, without mutating anything (FR-004). Split from `preview_helm_upgrade`
/// so the shellout/render logic is unit-testable (T021) without a live cluster.
async fn render_upgrade_preview(
    namespace: &str,
    release: &str,
    current: CurrentReleaseState,
    values_yaml: &str,
    chart_version: Option<&str>,
) -> Result<HelmMutationPreviewReady, HelmMutationFailed> {
    let action = HelmMutationAction::Upgrade;
    let stage = HelmMutationStage::Preview;

    let Some(chart_payload) = &current.chart_payload else {
        return Err(mutation_failed(
            namespace,
            release,
            action,
            stage,
            "The release's stored chart payload is corrupted or unreadable; cannot render an upgrade preview.",
        ));
    };
    check_chart_version(namespace, release, action, stage, chart_payload, chart_version)?;

    let chart_dir = materialize_chart(chart_payload).map_err(|e| {
        mutation_failed(
            namespace,
            release,
            action,
            stage,
            format!("Failed to materialize the release's chart for rendering: {e}"),
        )
    })?;

    let output = run_helm_upgrade(release, chart_dir.path(), namespace, values_yaml, true)
        .await
        .map_err(|e| mutation_failed(namespace, release, action, stage, format!("Failed to run helm: {e}")))?;

    if !output.status.success() {
        return Err(mutation_failed(namespace, release, action, stage, helm_error_message(&output)));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let target_manifest = extract_manifest_section(&stdout);
    if target_manifest.is_empty() {
        return Err(mutation_failed(
            namespace,
            release,
            action,
            stage,
            "helm upgrade --dry-run produced an empty rendered manifest.",
        ));
    }

    Ok(HelmMutationPreviewReady {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action,
        base_revision: current.revision,
        current_manifest: current.manifest,
        target_manifest,
        target_revision: None,
        target_chart_version: chart_version.map(str::to_owned),
        gitops_warning: current.origin,
    })
}

/// `PreviewHelmUpgrade` command handler: guards on `helm_binary_available()` (T001/FR-012),
/// reads the release's current state, then renders the preview.
pub(super) async fn preview_helm_upgrade(
    state: &BackendState,
    namespace: &str,
    release: &str,
    values_yaml: &str,
    chart_version: Option<&str>,
) -> Result<HelmMutationPreviewReady, HelmMutationFailed> {
    let action = HelmMutationAction::Upgrade;
    let stage = HelmMutationStage::Preview;
    guard_helm_available(namespace, release, action, stage, helm_binary_available().await)?;

    let current = current_release_state(state, namespace, release)
        .await
        .map_err(|e| {
            mutation_failed(namespace, release, action, stage, format!("Failed to read release state: {e}"))
        })?
        .ok_or_else(|| {
            mutation_failed(
                namespace,
                release,
                action,
                stage,
                format!("Release {release} not found in namespace {namespace}."),
            )
        })?;

    render_upgrade_preview(namespace, release, current, values_yaml, chart_version).await
}

/// T025: apply a previously previewed upgrade for real (no `--dry-run`) against an already-known
/// current release state. Split out like `render_upgrade_preview` for the same testability
/// reason.
async fn apply_upgrade_with_state(
    namespace: &str,
    release: &str,
    current: CurrentReleaseState,
    values_yaml: &str,
    chart_version: Option<&str>,
) -> Result<HelmMutationApplied, HelmMutationFailed> {
    let action = HelmMutationAction::Upgrade;
    let stage = HelmMutationStage::Apply;

    let Some(chart_payload) = &current.chart_payload else {
        return Err(mutation_failed(
            namespace,
            release,
            action,
            stage,
            "The release's stored chart payload is corrupted or unreadable; cannot apply the upgrade.",
        ));
    };
    check_chart_version(namespace, release, action, stage, chart_payload, chart_version)?;

    let chart_dir = materialize_chart(chart_payload).map_err(|e| {
        mutation_failed(
            namespace,
            release,
            action,
            stage,
            format!("Failed to materialize the release's chart for the upgrade: {e}"),
        )
    })?;

    let output = run_helm_upgrade(release, chart_dir.path(), namespace, values_yaml, false)
        .await
        .map_err(|e| mutation_failed(namespace, release, action, stage, format!("Failed to run helm: {e}")))?;

    if !output.status.success() {
        return Err(mutation_failed(namespace, release, action, stage, helm_error_message(&output)));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(HelmMutationApplied {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action,
        new_revision: extract_revision(&stdout),
    })
}

/// `ApplyHelmUpgrade` command handler: guards on `helm_binary_available()`, re-checks
/// `base_revision` (stale-preview guard, FR-012), then applies.
pub(super) async fn apply_helm_upgrade(
    state: &BackendState,
    namespace: &str,
    release: &str,
    values_yaml: &str,
    chart_version: Option<&str>,
    base_revision: i64,
) -> Result<HelmMutationApplied, HelmMutationFailed> {
    let action = HelmMutationAction::Upgrade;
    let stage = HelmMutationStage::Apply;
    guard_helm_available(namespace, release, action, stage, helm_binary_available().await)?;

    let current = current_release_state(state, namespace, release)
        .await
        .map_err(|e| {
            mutation_failed(namespace, release, action, stage, format!("Failed to read release state: {e}"))
        })?
        .ok_or_else(|| {
            mutation_failed(
                namespace,
                release,
                action,
                stage,
                format!("Release {release} not found in namespace {namespace}."),
            )
        })?;

    check_base_revision_fresh(namespace, release, action, current.revision, base_revision)?;

    apply_upgrade_with_state(namespace, release, current, values_yaml, chart_version).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    fn encode_release_fixture(json: &str) -> String {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(json.as_bytes()).unwrap();
        let gz_bytes = encoder.finish().unwrap();
        BASE64.encode(gz_bytes)
    }

    #[test]
    fn decode_release_payload_roundtrips_a_fixture_blob() {
        let json = r#"{
            "name": "redis",
            "info": {"status": "deployed", "last_deployed": "2024-01-01T00:00:00Z", "description": "Install complete"},
            "chart": {"metadata": {"name": "redis", "version": "1.2.3", "appVersion": "7.0"}, "values": {"replicas": 1}},
            "config": {"replicas": 3},
            "manifest": "apiVersion: v1\nkind: Service",
            "version": 4
        }"#;
        let blob = encode_release_fixture(json);
        let decoded = decode_release_payload(&blob).expect("decode");
        assert_eq!(decoded["name"], "redis");
        assert_eq!(decoded["chart"]["metadata"]["version"], "1.2.3");
        assert_eq!(decoded["config"]["replicas"], 3);
    }

    #[test]
    fn decode_release_payload_rejects_corrupted_blob() {
        assert!(decode_release_payload("not-valid-base64!!!").is_err());
    }

    #[test]
    fn merge_values_overlays_config_onto_chart_defaults() {
        let base = serde_json::json!({"replicas": 1, "nested": {"a": 1, "b": 2}});
        let overrides = serde_json::json!({"replicas": 3, "nested": {"b": 5}});
        let merged = merge_values(&base, &overrides);
        assert_eq!(merged["replicas"], 3);
        assert_eq!(merged["nested"]["a"], 1);
        assert_eq!(merged["nested"]["b"], 5);
    }

    #[test]
    fn current_per_release_picks_highest_revision() {
        let objects = vec![
            StorageObject {
                namespace: "default".to_owned(),
                release_name: "redis".to_owned(),
                revision: 1,
                status_label: "superseded".to_owned(),
                payload: None,
            },
            StorageObject {
                namespace: "default".to_owned(),
                release_name: "redis".to_owned(),
                revision: 3,
                status_label: "deployed".to_owned(),
                payload: None,
            },
            StorageObject {
                namespace: "default".to_owned(),
                release_name: "redis".to_owned(),
                revision: 2,
                status_label: "superseded".to_owned(),
                payload: None,
            },
        ];
        let current = current_per_release(&objects);
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].revision, 3);
    }

    #[test]
    fn summary_from_storage_object_is_degraded_when_payload_missing() {
        let obj = StorageObject {
            namespace: "default".to_owned(),
            release_name: "redis".to_owned(),
            revision: 1,
            status_label: "deployed".to_owned(),
            payload: None,
        };
        let summary = summary_from_storage_object(&obj);
        assert!(summary.degraded);
        assert_eq!(summary.name, "redis");
        assert_eq!(summary.namespace, "default");
    }

    #[test]
    fn origin_from_labels_prefers_flux_helmrelease() {
        let mut labels = std::collections::BTreeMap::new();
        labels.insert(FLUX_HELMRELEASE_NAME_LABEL.to_owned(), "redis".to_owned());
        labels.insert(
            FLUX_HELMRELEASE_NAMESPACE_LABEL.to_owned(),
            "flux-system".to_owned(),
        );
        let annotations = std::collections::BTreeMap::new();
        let origin = origin_from_labels(&labels, &annotations, "default").expect("origin");
        assert_eq!(origin.0, "HelmRelease");
        assert_eq!(origin.1, "flux-system");
        assert_eq!(origin.2, "redis");
    }

    #[test]
    fn origin_from_labels_none_for_bare_helm_release() {
        let labels = std::collections::BTreeMap::new();
        let annotations = std::collections::BTreeMap::new();
        assert!(origin_from_labels(&labels, &annotations, "default").is_none());
    }

    #[test]
    fn namespace_allowed_with_empty_filter_allows_everything() {
        assert!(namespace_allowed(&[], "cache"));
    }

    #[test]
    fn namespace_allowed_restricts_to_filtered_namespaces() {
        let filter = vec!["cache".to_owned()];
        assert!(namespace_allowed(&filter, "cache"));
        assert!(!namespace_allowed(&filter, "default"));
    }

    /// Covers the "owns" relation's underlying filter: `related_for_helm_release`/
    /// `get_release_details` both narrow owned workloads to this release's own via the same
    /// `meta.helm.sh/*` annotations `applications.rs` attributes from.
    #[test]
    fn annotations_match_release_requires_both_name_and_namespace() {
        let mut annotations = std::collections::BTreeMap::new();
        annotations.insert(HELM_RELEASE_NAME_ANNOTATION.to_owned(), "redis".to_owned());
        annotations.insert(
            HELM_RELEASE_NAMESPACE_ANNOTATION.to_owned(),
            "cache".to_owned(),
        );
        assert!(annotations_match_release(&annotations, "cache", "redis"));
        assert!(!annotations_match_release(&annotations, "default", "redis"));
        assert!(!annotations_match_release(&annotations, "cache", "other"));
    }

    #[test]
    fn annotations_match_release_false_when_annotations_absent() {
        let annotations = std::collections::BTreeMap::new();
        assert!(!annotations_match_release(&annotations, "cache", "redis"));
    }

    /// Covers the "reconciled-by" relation shape `related_for_helm_release` builds once an
    /// origin is found, independent of the async Flux lookup that supplies `status_hint`.
    #[test]
    fn reconciled_by_ref_has_reconciled_by_relation_and_flux_resource_section() {
        let r = reconciled_by_ref(
            "HelmRelease".to_owned(),
            "flux-system".to_owned(),
            "redis".to_owned(),
            Some("Ready".to_owned()),
        );
        assert_eq!(r.relation, "reconciled-by");
        assert_eq!(r.section, section_keys::flux_resource());
        assert_eq!(r.kind, "HelmRelease");
        assert_eq!(r.namespace.as_deref(), Some("flux-system"));
        assert_eq!(r.name, "redis");
        assert_eq!(r.status_hint.as_deref(), Some("Ready"));
    }

    // ---- 006-helm-mutate US2 (upgrade): T020, T021, T022 ----

    /// T020: `helm_available: false` short-circuits with a clear `HelmMutationFailed` — and
    /// since `guard_helm_available` is a pure, synchronous function that never calls
    /// `tokio::process::Command`, no subprocess is spawned to produce that flag.
    #[test]
    fn guard_helm_available_short_circuits_when_helm_missing() {
        let err = guard_helm_available(
            "default",
            "redis",
            HelmMutationAction::Upgrade,
            HelmMutationStage::Preview,
            false,
        )
        .expect_err("missing helm must fail");
        assert_eq!(err.namespace, "default");
        assert_eq!(err.release, "redis");
        assert_eq!(err.stage, HelmMutationStage::Preview);
        assert!(err.message.contains("helm"), "message: {}", err.message);
    }

    #[test]
    fn guard_helm_available_passes_when_helm_present() {
        assert!(
            guard_helm_available(
                "default",
                "redis",
                HelmMutationAction::Upgrade,
                HelmMutationStage::Preview,
                true,
            )
            .is_ok()
        );
    }

    /// T022: a `base_revision` that no longer matches the release's live revision fails at the
    /// `Apply` stage — this must be checked (and must fail) before any `helm upgrade` is ever
    /// invoked, so it is asserted here as a pure function with no subprocess involved at all.
    #[test]
    fn check_base_revision_fresh_rejects_stale_base_revision() {
        let err = check_base_revision_fresh("default", "redis", HelmMutationAction::Upgrade, 5, 4)
            .expect_err("stale base_revision must fail");
        assert_eq!(err.stage, HelmMutationStage::Apply);
        assert!(err.message.contains('4') && err.message.contains('5'), "message: {}", err.message);
    }

    #[test]
    fn check_base_revision_fresh_passes_when_matching() {
        assert!(check_base_revision_fresh("default", "redis", HelmMutationAction::Upgrade, 5, 5).is_ok());
    }

    /// Guards the parallel PATH-manipulating tests in this module against interleaving on a
    /// multi-threaded test runner (`cargo test` runs `#[test]`s in parallel by default).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Installs a fake `helm` shell script that always fails, prepends its directory to `PATH`,
    /// and returns a guard that restores the original `PATH` (and releases `ENV_LOCK`) on drop.
    struct FakeHelmGuard {
        _dir: tempfile::TempDir,
        original_path: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for FakeHelmGuard {
        fn drop(&mut self) {
            // SAFETY: serialized by `ENV_LOCK`, held for the guard's whole lifetime.
            unsafe {
                match &self.original_path {
                    Some(p) => std::env::set_var("PATH", p),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    fn install_failing_fake_helm(stderr_message: &str) -> FakeHelmGuard {
        let lock = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::Builder::new().prefix("fathom-fake-helm-").tempdir().unwrap();
        let script_path = dir.path().join("helm");
        std::fs::write(
            &script_path,
            format!("#!/bin/sh\necho '{stderr_message}' 1>&2\nexit 1\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }

        let original_path = std::env::var_os("PATH");
        let mut new_path = std::ffi::OsString::from(dir.path());
        if let Some(existing) = &original_path {
            new_path.push(":");
            new_path.push(existing);
        }
        // SAFETY: serialized by `ENV_LOCK`, held for the guard's whole lifetime (restored on drop).
        unsafe {
            std::env::set_var("PATH", &new_path);
        }

        FakeHelmGuard {
            _dir: dir,
            original_path,
            _lock: lock,
        }
    }

    /// T021: an invalid values YAML (surfaced by `helm upgrade --dry-run` failing) produces a
    /// `HelmMutationFailed { stage: Preview, .. }` whose `message` is the failing `helm`
    /// invocation's stderr verbatim (Constitution Principle I) — exercised through
    /// `render_upgrade_preview` with a fake `helm` binary on `PATH`, so no live cluster is
    /// needed (only the already-fetched `CurrentReleaseState` fixture below).
    #[tokio::test]
    async fn render_upgrade_preview_surfaces_helm_dry_run_error_verbatim() {
        let _guard = install_failing_fake_helm("Error: YAML parse error on values.yaml");

        let current = CurrentReleaseState {
            revision: 3,
            manifest: "apiVersion: v1\nkind: Service".to_owned(),
            chart_payload: Some(serde_json::json!({
                "metadata": {"name": "redis", "version": "1.2.3"},
                "values": {"replicas": 1},
                "templates": [],
                "files": [],
            })),
            origin: None,
        };

        let err = render_upgrade_preview("default", "redis", current, "not: [valid", None)
            .await
            .expect_err("invalid values must fail preview");

        assert_eq!(err.namespace, "default");
        assert_eq!(err.release, "redis");
        assert_eq!(err.stage, HelmMutationStage::Preview);
        assert!(
            err.message.contains("YAML parse error"),
            "expected helm's stderr verbatim, got: {}",
            err.message
        );
    }

    #[test]
    fn check_chart_version_rejects_a_different_version() {
        let chart_payload = serde_json::json!({"metadata": {"version": "1.2.3"}});
        let err = check_chart_version(
            "default",
            "redis",
            HelmMutationAction::Upgrade,
            HelmMutationStage::Preview,
            &chart_payload,
            Some("2.0.0"),
        )
        .expect_err("different chart version must fail");
        assert!(err.message.contains("1.2.3"), "message: {}", err.message);
        assert!(err.message.contains("2.0.0"), "message: {}", err.message);
    }

    #[test]
    fn check_chart_version_allows_same_version_or_none_requested() {
        let chart_payload = serde_json::json!({"metadata": {"version": "1.2.3"}});
        assert!(
            check_chart_version(
                "default",
                "redis",
                HelmMutationAction::Upgrade,
                HelmMutationStage::Preview,
                &chart_payload,
                Some("1.2.3"),
            )
            .is_ok()
        );
        assert!(
            check_chart_version(
                "default",
                "redis",
                HelmMutationAction::Upgrade,
                HelmMutationStage::Preview,
                &chart_payload,
                None,
            )
            .is_ok()
        );
    }

    #[test]
    fn extract_manifest_section_pulls_manifest_between_markers() {
        let stdout = "NAME: redis\n\nMANIFEST:\n---\napiVersion: v1\nkind: Service\n\nNOTES:\nsome notes here\n";
        assert_eq!(
            extract_manifest_section(stdout),
            "---\napiVersion: v1\nkind: Service"
        );
    }

    #[test]
    fn extract_revision_reads_the_revision_line() {
        let stdout = "NAME: redis\nREVISION: 7\nSTATUS: deployed\n";
        assert_eq!(extract_revision(stdout), Some(7));
    }

    #[test]
    fn extract_revision_none_when_absent() {
        assert_eq!(extract_revision("NAME: redis\n"), None);
    }
}
