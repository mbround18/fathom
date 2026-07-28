use crate::protocol::{
    section_keys, BackendEvent, BackendEventEnvelope, Correlation, PersistentVolumeDetails,
    PersistentVolumeSummary, ResourceListEvent, ResourceReadEvent, Route,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::PersistentVolume;
use kube::{
    api::{Api, DeleteParams, ListParams},
    ResourceExt,
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_persistentvolumes(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<PersistentVolume> = Api::all(client);

    let volumes = api.list(&ListParams::default()).await?;
    let summaries = persistentvolumes_to_summaries(volumes.items);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::storage_pv(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::PersistentVolumesUpdated(summaries)),
    );
    Ok(())
}

fn persistentvolumes_to_summaries(volumes: Vec<PersistentVolume>) -> Vec<PersistentVolumeSummary> {
    let mut out: Vec<PersistentVolumeSummary> = Vec::new();

    for volume in volumes {
        let name = volume.name_any();
        let spec = volume.spec.as_ref();

        let capacity = spec
            .and_then(|s| s.capacity.as_ref())
            .and_then(|c| c.get("storage"))
            .map(|q| q.0.clone())
            .unwrap_or_default();

        let access_modes = spec
            .and_then(|s| s.access_modes.clone())
            .unwrap_or_default();

        let status = volume
            .status
            .as_ref()
            .and_then(|s| s.phase.clone())
            .unwrap_or_default();

        let storage_class = spec.and_then(|s| s.storage_class_name.clone());

        let reclaim_policy = spec
            .and_then(|s| s.persistent_volume_reclaim_policy.clone())
            .unwrap_or_default();

        let claim_ref = spec.and_then(|s| s.claim_ref.as_ref()).map(|r| {
            format!(
                "{}/{}",
                r.namespace.clone().unwrap_or_default(),
                r.name.clone().unwrap_or_default()
            )
        });

        out.push(PersistentVolumeSummary {
            name,
            capacity,
            access_modes,
            status,
            storage_class,
            reclaim_policy,
            claim_ref,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub(super) async fn fetch_persistentvolume_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<PersistentVolume> = Api::all(client);
    let volume = api.get(name).await?;

    let details = persistentvolume_to_details(volume);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::storage_pv(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::PersistentVolumeDetailsUpdated(details)),
    );
    Ok(())
}

fn persistentvolume_to_details(volume: PersistentVolume) -> PersistentVolumeDetails {
    let name = volume.name_any();
    let creation_timestamp = volume
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = volume.labels().clone();
    let annotations = volume.annotations().clone();

    let spec = volume.spec.as_ref();

    let capacity = spec
        .and_then(|s| s.capacity.as_ref())
        .and_then(|c| c.get("storage"))
        .map(|q| q.0.clone())
        .unwrap_or_default();

    let access_modes = spec
        .and_then(|s| s.access_modes.clone())
        .unwrap_or_default();

    let status = volume
        .status
        .as_ref()
        .and_then(|s| s.phase.clone())
        .unwrap_or_default();

    let storage_class = spec.and_then(|s| s.storage_class_name.clone());

    let reclaim_policy = spec
        .and_then(|s| s.persistent_volume_reclaim_policy.clone())
        .unwrap_or_default();

    let claim_ref = spec.and_then(|s| s.claim_ref.as_ref()).map(|r| {
        format!(
            "{}/{}",
            r.namespace.clone().unwrap_or_default(),
            r.name.clone().unwrap_or_default()
        )
    });

    let volume_mode = spec.and_then(|s| s.volume_mode.clone());

    let csi_driver = spec
        .and_then(|s| s.csi.as_ref())
        .map(|csi| csi.driver.clone());

    let raw_json = serde_json::to_string_pretty(&volume).unwrap_or_default();

    PersistentVolumeDetails {
        name,
        creation_timestamp,
        labels,
        annotations,
        capacity,
        access_modes,
        status,
        storage_class,
        reclaim_policy,
        claim_ref,
        volume_mode,
        csi_driver,
        raw_json,
    }
}

pub(super) async fn delete_persistentvolume(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<PersistentVolume> = Api::all(client);

    let ts = k8s_openapi::chrono::Utc::now().to_rfc3339();
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

    refresh_persistentvolumes(state, sender).await?;
    Ok(())
}
