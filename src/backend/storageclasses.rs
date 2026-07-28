use crate::protocol::{
    section_keys, BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent,
    ResourceReadEvent, Route, StorageClassDetails, StorageClassSummary,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::storage::v1::StorageClass;
use kube::{
    api::{Api, DeleteParams, ListParams},
    ResourceExt,
};

use super::interactions;
use super::{events, state::BackendState};

const DEFAULT_CLASS_ANNOTATION: &str = "storageclass.kubernetes.io/is-default-class";

pub(super) async fn refresh_storageclasses(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<StorageClass> = Api::all(client);

    let classes = api.list(&ListParams::default()).await?;
    let summaries = storageclasses_to_summaries(classes.items);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::storage_storageclasses(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::StorageClassesUpdated(summaries)),
    );
    Ok(())
}

fn is_default_class(class: &StorageClass) -> bool {
    class
        .annotations()
        .get(DEFAULT_CLASS_ANNOTATION)
        .map(|v| v == "true")
        .unwrap_or(false)
}

fn storageclasses_to_summaries(classes: Vec<StorageClass>) -> Vec<StorageClassSummary> {
    let mut out: Vec<StorageClassSummary> = Vec::new();

    for class in classes {
        let name = class.name_any();
        let is_default = is_default_class(&class);
        let provisioner = class.provisioner.clone();
        let reclaim_policy = class.reclaim_policy.clone();
        let volume_binding_mode = class.volume_binding_mode.clone();

        out.push(StorageClassSummary {
            name,
            provisioner,
            reclaim_policy,
            volume_binding_mode,
            is_default,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub(super) async fn fetch_storageclass_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<StorageClass> = Api::all(client);
    let class = api.get(name).await?;

    let details = storageclass_to_details(class);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::storage_storageclasses(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::StorageClassDetailsUpdated(details)),
    );
    Ok(())
}

fn storageclass_to_details(class: StorageClass) -> StorageClassDetails {
    let name = class.name_any();
    let creation_timestamp = class
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = class.labels().clone();
    let annotations = class.annotations().clone();
    let is_default = is_default_class(&class);

    let provisioner = class.provisioner.clone();
    let reclaim_policy = class.reclaim_policy.clone();
    let volume_binding_mode = class.volume_binding_mode.clone();
    let allow_volume_expansion = class.allow_volume_expansion;
    let parameters = class.parameters.clone().unwrap_or_default();

    let raw_json = serde_json::to_string_pretty(&class).unwrap_or_default();

    StorageClassDetails {
        name,
        creation_timestamp,
        labels,
        annotations,
        provisioner,
        reclaim_policy,
        volume_binding_mode,
        is_default,
        allow_volume_expansion,
        parameters,
        raw_json,
    }
}

pub(super) async fn delete_storageclass(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<StorageClass> = Api::all(client);

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

    refresh_storageclasses(state, sender).await?;
    Ok(())
}
