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
use flate2::write::GzEncoder;
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use kube::{
    Client, ResourceExt,
    api::{Api, ListParams, Patch, PatchParams, PostParams},
    core::ObjectMeta,
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

/// Which Kubernetes object kind backs a release's storage entry — Helm 3 defaults to `Secret`
/// but supports `ConfigMap` storage too; rollback writes must target whichever kind the
/// release already uses (data-model.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StorageKind {
    Secret,
    ConfigMap,
}

/// One Helm release-storage object, decoded far enough to attribute and sort it, with its full
/// payload lazily available for the currently-selected revision only.
struct StorageObject {
    namespace: String,
    release_name: String,
    revision: i64,
    status_label: String,
    /// `Some` once base64/gzip/JSON decode succeeded; `None` means `degraded: true`.
    payload: Option<Value>,
    /// The underlying Secret/ConfigMap's own `metadata.name` (needed to patch it, e.g. when
    /// superseding it on rollback).
    object_name: String,
    kind: StorageKind,
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
        object_name: secret.name_any(),
        kind: StorageKind::Secret,
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
        object_name: cm.name_any(),
        kind: StorageKind::ConfigMap,
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

fn helm_storage_object_name(release: &str, revision: i64) -> String {
    format!("sh.helm.release.v1.{release}.v{revision}")
}

fn infra_failure(
    action: HelmMutationAction,
    stage: HelmMutationStage,
    namespace: &str,
    release: &str,
    err: impl std::fmt::Display,
) -> HelmMutationFailed {
    HelmMutationFailed {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action,
        stage,
        message: format!("{err:#}"),
    }
}

/// `HelmMutationFailed` for the FR-012 "missing `helm` binary" case, shared by every mutate flow
/// that shells out (currently just uninstall — upgrade will reuse this once US2 lands).
fn missing_helm_binary_failure(
    namespace: &str,
    release: &str,
    action: HelmMutationAction,
    stage: HelmMutationStage,
) -> HelmMutationFailed {
    HelmMutationFailed {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action,
        stage,
        message: "The `helm` binary was not found on PATH. Install Helm (https://helm.sh) \
                  to perform this action."
            .to_owned(),
    }
}

/// Finds the current (highest-revision) and requested-target storage objects for a release
/// among `objects` (already filtered/fetched by the caller). Returns `None` for either side
/// that isn't present.
fn find_current_and_target<'a>(
    objects: &'a [StorageObject],
    release: &str,
    target_revision: i64,
) -> (Option<&'a StorageObject>, Option<&'a StorageObject>) {
    let mut matching: Vec<&StorageObject> =
        objects.iter().filter(|o| o.release_name == release).collect();
    matching.sort_by_key(|o| std::cmp::Reverse(o.revision));
    let current = matching.first().copied();
    let target = matching.iter().find(|o| o.revision == target_revision).copied();
    (current, target)
}

/// Pure rollback diff-preview computation (T010/T011): no cluster call, testable directly
/// against `StorageObject` fixtures. Fails at `Preview` stage when the target revision's
/// stored manifest can't be decoded (corrupted payload, FR-012) or is empty.
fn build_rollback_preview(
    namespace: &str,
    release: &str,
    current: &StorageObject,
    target: &StorageObject,
    gitops_warning: Option<GitOpsOriginRef>,
) -> Result<HelmMutationPreviewReady, HelmMutationFailed> {
    let fail = |message: String| HelmMutationFailed {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action: HelmMutationAction::Rollback,
        stage: HelmMutationStage::Preview,
        message,
    };

    let Some(target_payload) = &target.payload else {
        return Err(fail(format!(
            "revision {} of {release} could not be decoded (corrupted release-storage payload)",
            target.revision
        )));
    };
    let target_manifest = payload_str(target_payload, "/manifest").unwrap_or_default();
    if target_manifest.is_empty() {
        return Err(fail(format!(
            "revision {} of {release} has no stored manifest to roll back to",
            target.revision
        )));
    }

    let current_manifest = current
        .payload
        .as_ref()
        .and_then(|p| payload_str(p, "/manifest"))
        .unwrap_or_default();
    if current_manifest.is_empty() {
        return Err(fail(format!(
            "the current revision of {release} could not be decoded (corrupted release-storage payload)"
        )));
    }

    Ok(HelmMutationPreviewReady {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action: HelmMutationAction::Rollback,
        base_revision: current.revision,
        current_manifest,
        target_manifest,
        target_revision: Some(target.revision),
        target_chart_version: None,
        gitops_warning,
    })
}

/// Stale-preview guard (T012): the release's live revision must still match the
/// `base_revision` the preview was computed against, or the apply fails clearly rather than
/// mutating against a moved target (FR-012).
fn check_base_revision(
    action: HelmMutationAction,
    namespace: &str,
    release: &str,
    live_revision: i64,
    base_revision: i64,
) -> Result<(), HelmMutationFailed> {
    if live_revision != base_revision {
        return Err(HelmMutationFailed {
            namespace: namespace.to_owned(),
            release: release.to_owned(),
            action,
            stage: HelmMutationStage::Apply,
            message: format!(
                "{release} has moved to revision {live_revision} since the preview was computed \
                 (expected revision {base_revision}); refresh and try again"
            ),
        });
    }
    Ok(())
}

/// Pure precondition guard for [`uninstall_helm_release`], split out so the FR-012
/// missing-binary and stale-`base_revision` edge cases (T029/T030) are unit-testable without a
/// live cluster or subprocess.
fn check_uninstall_preconditions(
    namespace: &str,
    release: &str,
    helm_available: bool,
    live_revision: i64,
    base_revision: i64,
) -> Result<(), HelmMutationFailed> {
    if !helm_available {
        return Err(missing_helm_binary_failure(
            namespace,
            release,
            HelmMutationAction::Uninstall,
            HelmMutationStage::Apply,
        ));
    }
    if live_revision != base_revision {
        return Err(HelmMutationFailed {
            namespace: namespace.to_owned(),
            release: release.to_owned(),
            action: HelmMutationAction::Uninstall,
            stage: HelmMutationStage::Apply,
            message: format!(
                "This release has changed since the confirm dialog was opened (now at \
                 revision {live_revision}, expected {base_revision}). Reopen the release and \
                 try again."
            ),
        });
    }
    Ok(())
}

/// Clones `source`'s decoded release payload for a new revision (rollback semantics: chart,
/// values/config, and manifest carry over verbatim from the target revision; only `version`
/// and `info` are updated) — pure, unit-testable without a cluster.
fn payload_for_new_revision(source: &Value, new_revision: i64, description: String) -> Value {
    let mut payload = source.clone();
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("version".to_owned(), serde_json::json!(new_revision));
        let now = k8s_openapi::chrono::Utc::now().to_rfc3339();
        let info = obj
            .entry("info")
            .or_insert_with(|| Value::Object(Default::default()));
        if let Some(info_obj) = info.as_object_mut() {
            info_obj.insert("status".to_owned(), serde_json::json!("deployed"));
            info_obj.insert("last_deployed".to_owned(), serde_json::json!(now));
            info_obj.insert("description".to_owned(), serde_json::json!(description));
        }
    }
    payload
}

fn encode_release_payload(payload: &Value) -> anyhow::Result<String> {
    let json_bytes = serde_json::to_vec(payload)?;
    let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, &json_bytes)?;
    let gz_bytes = encoder.finish()?;
    Ok(BASE64.encode(gz_bytes))
}

/// Computes the rollback diff-preview for `release`/`target_revision` without mutating
/// anything (FR-002). Reuses `resolve_release_origin` unmodified for `gitops_warning`
/// (research.md R5).
pub(super) async fn preview_helm_rollback(
    state: &BackendState,
    namespace: &str,
    release: &str,
    target_revision: i64,
) -> Result<HelmMutationPreviewReady, HelmMutationFailed> {
    let action = HelmMutationAction::Rollback;
    let stage = HelmMutationStage::Preview;

    let client = state
        .cluster
        .client_or_err()
        .map_err(|e| infra_failure(action, stage, namespace, release, e))?;
    let objects = list_storage_objects(&client, Some(namespace))
        .await
        .map_err(|e| infra_failure(action, stage, namespace, release, e))?;

    let (current, target) = find_current_and_target(&objects, release, target_revision);
    let current = current.ok_or_else(|| HelmMutationFailed {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action,
        stage,
        message: format!("release {release} not found in {namespace}"),
    })?;
    let target = target.ok_or_else(|| HelmMutationFailed {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action,
        stage,
        message: format!("revision {target_revision} of {release} not found"),
    })?;

    let workloads = super::applications::list_owned_workloads(state)
        .await
        .map_err(|e| infra_failure(action, stage, namespace, release, e))?;
    let owned = owned_workloads_for_release(&workloads, namespace, release);
    let gitops_warning = resolve_release_origin(&client, &owned, namespace).await;

    build_rollback_preview(namespace, release, current, target, gitops_warning)
}

/// Applies a previously previewed rollback (FR-001/FR-002): re-checks `base_revision`
/// against the release's live revision (stale-preview guard, T012), then writes a new
/// release-storage revision entry cloned from the target revision's payload — standard Helm
/// rollback semantics (creates a new revision, does not delete intervening ones).
pub(super) async fn apply_helm_rollback(
    state: &BackendState,
    namespace: &str,
    release: &str,
    target_revision: i64,
    base_revision: i64,
) -> Result<HelmMutationApplied, HelmMutationFailed> {
    let action = HelmMutationAction::Rollback;
    let apply_stage = HelmMutationStage::Apply;

    let client = state
        .cluster
        .client_or_err()
        .map_err(|e| infra_failure(action, apply_stage, namespace, release, e))?;
    let objects = list_storage_objects(&client, Some(namespace))
        .await
        .map_err(|e| infra_failure(action, apply_stage, namespace, release, e))?;

    let (current, target) = find_current_and_target(&objects, release, target_revision);
    let current = current.ok_or_else(|| HelmMutationFailed {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action,
        stage: apply_stage,
        message: format!("release {release} not found in {namespace}"),
    })?;
    let target = target.ok_or_else(|| HelmMutationFailed {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action,
        stage: apply_stage,
        message: format!("revision {target_revision} of {release} not found"),
    })?;

    check_base_revision(action, namespace, release, current.revision, base_revision)?;

    let Some(target_payload) = &target.payload else {
        return Err(HelmMutationFailed {
            namespace: namespace.to_owned(),
            release: release.to_owned(),
            action,
            stage: apply_stage,
            message: format!(
                "revision {target_revision} of {release} could not be decoded (corrupted \
                 release-storage payload)"
            ),
        });
    };

    let new_revision = current.revision + 1;
    let description = format!("Rollback to {target_revision}");
    let new_payload = payload_for_new_revision(target_payload, new_revision, description);
    let encoded = encode_release_payload(&new_payload)
        .map_err(|e| infra_failure(action, apply_stage, namespace, release, e))?;

    write_new_revision(
        &client,
        namespace,
        release,
        current.kind,
        &current.object_name,
        new_revision,
        &encoded,
    )
    .await
    .map_err(|e| infra_failure(action, apply_stage, namespace, release, e))?;

    Ok(HelmMutationApplied {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action,
        new_revision: Some(new_revision),
    })
}

/// Writes a new release-storage object for `new_revision` (matching the release's existing
/// `Secret`/`ConfigMap` storage kind) and marks the previous current object as `superseded`.
async fn write_new_revision(
    client: &Client,
    namespace: &str,
    release: &str,
    kind: StorageKind,
    current_object_name: &str,
    new_revision: i64,
    encoded_payload: &str,
) -> anyhow::Result<()> {
    let object_name = helm_storage_object_name(release, new_revision);
    let mut labels = std::collections::BTreeMap::new();
    labels.insert(HELM_OWNER_LABEL.to_owned(), HELM_OWNER_VALUE.to_owned());
    labels.insert(HELM_NAME_LABEL.to_owned(), release.to_owned());
    labels.insert(HELM_STATUS_LABEL.to_owned(), "deployed".to_owned());
    labels.insert(HELM_VERSION_LABEL.to_owned(), new_revision.to_string());

    let supersede_patch = serde_json::json!({
        "metadata": { "labels": { HELM_STATUS_LABEL: "superseded" } }
    });

    match kind {
        StorageKind::Secret => {
            let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
            let mut data = std::collections::BTreeMap::new();
            data.insert(
                HELM_RELEASE_DATA_KEY.to_owned(),
                ByteString(encoded_payload.as_bytes().to_vec()),
            );
            let secret = Secret {
                metadata: ObjectMeta {
                    name: Some(object_name),
                    namespace: Some(namespace.to_owned()),
                    labels: Some(labels),
                    ..Default::default()
                },
                type_: Some("helm.sh/release.v1".to_owned()),
                data: Some(data),
                ..Default::default()
            };
            api.create(&PostParams::default(), &secret).await?;
            api.patch(
                current_object_name,
                &PatchParams::default(),
                &Patch::Merge(&supersede_patch),
            )
            .await?;
        }
        StorageKind::ConfigMap => {
            let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
            let mut data = std::collections::BTreeMap::new();
            data.insert(HELM_RELEASE_DATA_KEY.to_owned(), encoded_payload.to_owned());
            let cm = ConfigMap {
                metadata: ObjectMeta {
                    name: Some(object_name),
                    namespace: Some(namespace.to_owned()),
                    labels: Some(labels),
                    ..Default::default()
                },
                data: Some(data),
                ..Default::default()
            };
            api.create(&PostParams::default(), &cm).await?;
            api.patch(
                current_object_name,
                &PatchParams::default(),
                &Patch::Merge(&supersede_patch),
            )
            .await?;
        }
    }
    Ok(())
}

/// The current (highest-`version`) revision for `(namespace, release)`, or `None` if no
/// release-storage object exists for it any more.
async fn current_release_revision(
    client: &Client,
    namespace: &str,
    release: &str,
) -> anyhow::Result<Option<i64>> {
    let objects = list_storage_objects(client, Some(namespace)).await?;
    Ok(current_per_release(&objects)
        .into_iter()
        .find(|o| o.release_name == release)
        .map(|o| o.revision))
}

/// Uninstalls a Helm release (T032): guards on `helm_binary_available()` (FR-012) and a stale
/// `base_revision` (the operator's confirm dialog no longer matches the release's live
/// revision) before shelling out to `helm uninstall`, which performs the actual release-storage
/// removal (constitution: the backend performs the mutation, never the frontend). No preview
/// step exists for uninstall (data-model.md), so this goes straight from guard to subprocess.
pub(super) async fn uninstall_helm_release(
    client: &Client,
    namespace: &str,
    release: &str,
    base_revision: i64,
) -> Result<HelmMutationApplied, HelmMutationFailed> {
    let helm_available = helm_binary_available().await;
    if !helm_available {
        return Err(missing_helm_binary_failure(
            namespace,
            release,
            HelmMutationAction::Uninstall,
            HelmMutationStage::Apply,
        ));
    }

    let live_revision = match current_release_revision(client, namespace, release).await {
        Ok(Some(rev)) => rev,
        Ok(None) => {
            return Err(HelmMutationFailed {
                namespace: namespace.to_owned(),
                release: release.to_owned(),
                action: HelmMutationAction::Uninstall,
                stage: HelmMutationStage::Apply,
                message: format!(
                    "Release {release} was not found in namespace {namespace}; it may have \
                     already been uninstalled."
                ),
            });
        }
        Err(e) => {
            return Err(HelmMutationFailed {
                namespace: namespace.to_owned(),
                release: release.to_owned(),
                action: HelmMutationAction::Uninstall,
                stage: HelmMutationStage::Apply,
                message: e.to_string(),
            });
        }
    };

    check_uninstall_preconditions(namespace, release, helm_available, live_revision, base_revision)?;

    let output = tokio::process::Command::new("helm")
        .args(["uninstall", release, "-n", namespace])
        .output()
        .await
        .map_err(|e| HelmMutationFailed {
            namespace: namespace.to_owned(),
            release: release.to_owned(),
            action: HelmMutationAction::Uninstall,
            stage: HelmMutationStage::Apply,
            message: format!("Failed to run `helm uninstall`: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let message = if stderr.is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        } else {
            stderr
        };
        return Err(HelmMutationFailed {
            namespace: namespace.to_owned(),
            release: release.to_owned(),
            action: HelmMutationAction::Uninstall,
            stage: HelmMutationStage::Apply,
            message,
        });
    }

    Ok(HelmMutationApplied {
        namespace: namespace.to_owned(),
        release: release.to_owned(),
        action: HelmMutationAction::Uninstall,
        new_revision: None,
    })
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
                object_name: "sh.helm.release.v1.redis.v1".to_owned(),
                kind: StorageKind::Secret,
            },
            StorageObject {
                namespace: "default".to_owned(),
                release_name: "redis".to_owned(),
                revision: 3,
                status_label: "deployed".to_owned(),
                payload: None,
                object_name: "sh.helm.release.v1.redis.v3".to_owned(),
                kind: StorageKind::Secret,
            },
            StorageObject {
                namespace: "default".to_owned(),
                release_name: "redis".to_owned(),
                revision: 2,
                status_label: "superseded".to_owned(),
                payload: None,
                object_name: "sh.helm.release.v1.redis.v2".to_owned(),
                kind: StorageKind::Secret,
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
            object_name: "sh.helm.release.v1.redis.v1".to_owned(),
            kind: StorageKind::Secret,
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


    // --- US2: Upgrade -----------------------------------------------------------------


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
    // --- US1: Rollback (T010-T012) ---------------------------------------------------

    fn storage_object_fixture(revision: i64, manifest: &str) -> StorageObject {
        let payload = serde_json::json!({
            "name": "redis",
            "info": {"status": "deployed", "last_deployed": "2024-01-01T00:00:00Z", "description": "Install complete"},
            "chart": {"metadata": {"name": "redis", "version": "1.2.3", "appVersion": "7.0"}, "values": {}},
            "config": {},
            "manifest": manifest,
            "version": revision,
        });
        StorageObject {
            namespace: "default".to_owned(),
            release_name: "redis".to_owned(),
            revision,
            status_label: "deployed".to_owned(),
            payload: Some(payload),
            object_name: helm_storage_object_name("redis", revision),
            kind: StorageKind::Secret,
        }
    }

    fn corrupted_storage_object_fixture(revision: i64) -> StorageObject {
        StorageObject {
            namespace: "default".to_owned(),
            release_name: "redis".to_owned(),
            revision,
            status_label: "superseded".to_owned(),
            payload: None,
            object_name: helm_storage_object_name("redis", revision),
            kind: StorageKind::Secret,
        }
    }

    /// T010: rollback diff-preview computation against fixture storage-object data — asserts
    /// `current_manifest`/`target_manifest` populate correctly, with no cluster call (this is
    /// a pure function, so "no cluster call" is structural, not merely asserted).
    #[test]
    fn build_rollback_preview_populates_manifests_from_fixtures() {
        let current = storage_object_fixture(3, "apiVersion: v1\nkind: Service\nrevision: 3");
        let target = storage_object_fixture(1, "apiVersion: v1\nkind: Service\nrevision: 1");

        let preview = build_rollback_preview("default", "redis", &current, &target, None)
            .expect("preview should succeed");

        assert_eq!(preview.namespace, "default");
        assert_eq!(preview.release, "redis");
        assert_eq!(preview.action, HelmMutationAction::Rollback);
        assert_eq!(preview.base_revision, 3);
        assert_eq!(preview.target_revision, Some(1));
        assert!(preview.current_manifest.contains("revision: 3"));
        assert!(preview.target_manifest.contains("revision: 1"));
        assert!(preview.gitops_warning.is_none());
    }

    /// T010 (GitOps-warning path): `gitops_warning` is carried through verbatim when present.
    #[test]
    fn build_rollback_preview_carries_gitops_warning_through_verbatim() {
        let current = storage_object_fixture(2, "kind: Service");
        let target = storage_object_fixture(1, "kind: Service");
        let warning = GitOpsOriginRef {
            controller: GitOpsController::Flux,
            section: section_keys::flux_resource(),
            kind: "HelmRelease".to_owned(),
            namespace: "flux-system".to_owned(),
            name: "redis".to_owned(),
            ready: crate::protocol::FluxReadyState::Ready,
            message: None,
        };

        let preview =
            build_rollback_preview("default", "redis", &current, &target, Some(warning.clone()))
                .expect("preview should succeed");

        assert_eq!(preview.gitops_warning.map(|w| w.name), Some(warning.name));
    }

    /// T011: rollback against a target revision whose stored manifest fails to decode
    /// (corrupted payload edge case) — asserts `HelmMutationFailed { stage: Preview, .. }`
    /// with a clear message, no panic.
    #[test]
    fn build_rollback_preview_fails_at_preview_stage_for_corrupted_target() {
        let current = storage_object_fixture(3, "kind: Service");
        let target = corrupted_storage_object_fixture(1);

        let err = build_rollback_preview("default", "redis", &current, &target, None)
            .expect_err("corrupted target manifest must fail the preview");

        assert_eq!(err.action, HelmMutationAction::Rollback);
        assert_eq!(err.stage, HelmMutationStage::Preview);
        assert!(err.message.contains("revision 1"));
        assert!(!err.message.is_empty());
    }

    /// T012: `ApplyHelmRollback` with a `base_revision` that no longer matches the release's
    /// live revision (stale-preview edge case) — asserts `HelmMutationFailed { stage: Apply,
    /// .. }`. This is checked before any write is attempted (the check function itself never
    /// touches a client), so "no write attempted" is structural.
    #[test]
    fn check_base_revision_fails_at_apply_stage_when_stale() {
        let err = check_base_revision(HelmMutationAction::Rollback, "default", "redis", 4, 3)
            .expect_err("stale base_revision must fail");

        assert_eq!(err.action, HelmMutationAction::Rollback);
        assert_eq!(err.stage, HelmMutationStage::Apply);
        assert!(err.message.contains("revision 4"));
        assert!(err.message.contains("revision 3"));
    }

    #[test]
    fn check_base_revision_succeeds_when_matching() {
        assert!(
            check_base_revision(HelmMutationAction::Rollback, "default", "redis", 3, 3).is_ok()
        );
    }

    #[test]
    fn find_current_and_target_picks_highest_revision_and_requested_target() {
        let objects = vec![
            storage_object_fixture(1, "kind: Service"),
            storage_object_fixture(3, "kind: Service"),
            storage_object_fixture(2, "kind: Service"),
        ];
        let (current, target) = find_current_and_target(&objects, "redis", 1);
        assert_eq!(current.map(|o| o.revision), Some(3));
        assert_eq!(target.map(|o| o.revision), Some(1));
    }

    #[test]
    fn find_current_and_target_none_for_missing_release_or_revision() {
        let objects = vec![storage_object_fixture(1, "kind: Service")];
        let (current, target) = find_current_and_target(&objects, "other-release", 1);
        assert!(current.is_none());
        assert!(target.is_none());

        let (current, target) = find_current_and_target(&objects, "redis", 99);
        assert!(current.is_some());
        assert!(target.is_none());
    }

    #[test]
    fn payload_for_new_revision_updates_version_and_status_keeps_manifest() {
        let source = serde_json::json!({
            "name": "redis",
            "info": {"status": "superseded", "last_deployed": "2024-01-01T00:00:00Z", "description": "old"},
            "chart": {"metadata": {"name": "redis", "version": "1.2.3"}, "values": {}},
            "config": {},
            "manifest": "kind: Service",
            "version": 1,
        });
        let new_payload = payload_for_new_revision(&source, 4, "Rollback to 1".to_owned());
        assert_eq!(new_payload["version"], 4);
        assert_eq!(new_payload["info"]["status"], "deployed");
        assert_eq!(new_payload["info"]["description"], "Rollback to 1");
        assert_eq!(new_payload["manifest"], "kind: Service");
        assert_eq!(new_payload["chart"]["metadata"]["version"], "1.2.3");
    }

    #[test]
    fn encode_release_payload_roundtrips_through_decode_release_payload() {
        let payload = serde_json::json!({"name": "redis", "version": 4});
        let encoded = encode_release_payload(&payload).expect("encode");
        let decoded = decode_release_payload(&encoded).expect("decode");
        assert_eq!(decoded["name"], "redis");
        assert_eq!(decoded["version"], 4);
    }

    // --- Helm mutate (006) US3: uninstall precondition guards (T029/T030). These exercise the
    // pure `check_uninstall_preconditions` guard directly rather than the async
    // `uninstall_helm_release` orchestrator, mirroring this module's existing convention of unit
    // testing the sync logic extracted from cluster/subprocess-touching async functions (there is
    // no mockable `kube::Client`/subprocess seam in this codebase to drive the orchestrator
    // itself without a live cluster and a real `helm` binary). Because the guard function never
    // touches the filesystem or network, these tests also demonstrate T029's "no subprocess
    // spawned" and "no release-storage record removed" assertions by construction: nothing here
    // can do either.

    #[test]
    fn uninstall_fails_clearly_when_helm_binary_missing() {
        let result = check_uninstall_preconditions("cache", "redis", false, 5, 5);
        let err = result.expect_err("missing helm binary must fail");
        assert_eq!(err.namespace, "cache");
        assert_eq!(err.release, "redis");
        assert_eq!(err.action, HelmMutationAction::Uninstall);
        assert_eq!(err.stage, HelmMutationStage::Apply);
        assert!(
            err.message.to_lowercase().contains("helm"),
            "message should clearly mention the missing helm binary: {}",
            err.message
        );
    }

    #[test]
    fn uninstall_fails_before_checking_revision_when_helm_binary_missing() {
        // Even when the (hypothetical) live revision matches base_revision, a missing binary
        // must still fail first (FR-012 takes priority; nothing downstream should run).
        let result = check_uninstall_preconditions("cache", "redis", false, 5, 5);
        assert!(result.is_err());
    }

    #[test]
    fn uninstall_fails_on_stale_base_revision() {
        let result = check_uninstall_preconditions("cache", "redis", true, 5, 3);
        let err = result.expect_err("stale base_revision must fail");
        assert_eq!(err.stage, HelmMutationStage::Apply);
        assert_eq!(err.action, HelmMutationAction::Uninstall);
        assert!(
            err.message.contains('5') && err.message.contains('3'),
            "message should surface both the live and expected revisions: {}",
            err.message
        );
    }

    #[test]
    fn uninstall_succeeds_precondition_when_binary_present_and_revision_matches() {
        let result = check_uninstall_preconditions("cache", "redis", true, 5, 5);
        assert!(result.is_ok());
    }
}
