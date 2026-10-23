use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, PersistentVolumeClaimDetails,
    PersistentVolumeClaimSummary, ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_persistentvolumeclaims(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let claims = state
        .cluster
        .list_namespaced::<PersistentVolumeClaim>()
        .await?;
    let summaries = persistentvolumeclaims_to_summaries(claims, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::storage_pvc(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::PersistentVolumeClaimsUpdated(summaries)),
    );
    Ok(())
}

fn persistentvolumeclaims_to_summaries(
    claims: Vec<PersistentVolumeClaim>,
    filter: &[String],
) -> Vec<PersistentVolumeClaimSummary> {
    let mut out: Vec<PersistentVolumeClaimSummary> = Vec::new();

    for claim in claims {
        let namespace = claim.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = claim.name_any();
        let spec = claim.spec.as_ref();
        let status = claim.status.as_ref();

        let phase = status.and_then(|s| s.phase.clone()).unwrap_or_default();

        let volume_name = spec.and_then(|s| s.volume_name.clone());

        let capacity = status
            .and_then(|s| s.capacity.as_ref())
            .and_then(|c| c.get("storage"))
            .map(|q| q.0.clone());

        let access_modes = status
            .and_then(|s| s.access_modes.clone())
            .unwrap_or_default();

        let storage_class = spec.and_then(|s| s.storage_class_name.clone());

        out.push(PersistentVolumeClaimSummary {
            namespace,
            name,
            status: phase,
            volume_name,
            capacity,
            access_modes,
            storage_class,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_persistentvolumeclaim_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client, namespace);
    let claim = api.get(name).await?;

    let details = persistentvolumeclaim_to_details(claim);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::storage_pvc(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::PersistentVolumeClaimDetailsUpdated(
            details,
        )),
    );
    Ok(())
}

fn persistentvolumeclaim_to_details(claim: PersistentVolumeClaim) -> PersistentVolumeClaimDetails {
    let namespace = claim.namespace().unwrap_or_default();
    let name = claim.name_any();
    let creation_timestamp = claim
        .creation_timestamp()
        .map(|ts| super::workload_common::kube_rfc3339(&ts))
        .unwrap_or_default();

    let labels = claim.labels().clone();
    let annotations = claim.annotations().clone();

    let spec = claim.spec.as_ref();
    let status = claim.status.as_ref();

    let phase = status.and_then(|s| s.phase.clone()).unwrap_or_default();

    let volume_name = spec.and_then(|s| s.volume_name.clone());

    let capacity = status
        .and_then(|s| s.capacity.as_ref())
        .and_then(|c| c.get("storage"))
        .map(|q| q.0.clone());

    let access_modes = status
        .and_then(|s| s.access_modes.clone())
        .unwrap_or_default();

    let storage_class = spec.and_then(|s| s.storage_class_name.clone());

    let requested_storage = spec
        .and_then(|s| s.resources.as_ref())
        .and_then(|r| r.requests.as_ref())
        .and_then(|r| r.get("storage"))
        .map(|q| q.0.clone());

    let raw_json = serde_json::to_string_pretty(&claim).unwrap_or_default();

    PersistentVolumeClaimDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        status: phase,
        volume_name,
        capacity,
        access_modes,
        storage_class,
        requested_storage,
        raw_json,
    }
}

pub(super) async fn delete_persistentvolumeclaim(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client, namespace);

    let ts = chrono::Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &api,
        name,
        state.cluster.current_context.as_deref(),
        "delete",
        &ts,
        serde_json::json!({}),
    )
    .await;

    api.delete(name, &DeleteParams::default()).await?;

    refresh_persistentvolumeclaims(state, sender).await?;
    Ok(())
}
