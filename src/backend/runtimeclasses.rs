use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, ResourceReadEvent, Route,
    RuntimeClassDetails, RuntimeClassSummary, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::node::v1::RuntimeClass;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_runtimeclasses(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<RuntimeClass> = Api::all(client);

    let classes = api.list(&ListParams::default()).await?;
    let summaries = runtimeclasses_to_summaries(classes.items);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::config_runtimeclasses(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::RuntimeClassesUpdated(summaries)),
    );
    Ok(())
}

fn runtimeclasses_to_summaries(classes: Vec<RuntimeClass>) -> Vec<RuntimeClassSummary> {
    let mut out: Vec<RuntimeClassSummary> = Vec::new();

    for class in classes {
        let name = class.name_any();
        let handler = class.handler.clone();

        out.push(RuntimeClassSummary { name, handler });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub(super) async fn fetch_runtimeclass_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<RuntimeClass> = Api::all(client);
    let class = api.get(name).await?;

    let details = runtimeclass_to_details(class);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::config_runtimeclasses(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::RuntimeClassDetailsUpdated(details)),
    );
    Ok(())
}

fn runtimeclass_to_details(class: RuntimeClass) -> RuntimeClassDetails {
    let name = class.name_any();
    let creation_timestamp = class
        .creation_timestamp()
        .map(|ts| super::workload_common::kube_rfc3339(&ts))
        .unwrap_or_default();

    let labels = class.labels().clone();
    let annotations = class.annotations().clone();

    let handler = class.handler.clone();

    let scheduling = class.scheduling.as_ref().map(|s| {
        let node_selector_count = s.node_selector.as_ref().map(|m| m.len()).unwrap_or(0);
        let tolerations_count = s.tolerations.as_ref().map(|t| t.len()).unwrap_or(0);
        format!(
            "node selectors: {}, tolerations: {}",
            node_selector_count, tolerations_count
        )
    });

    let raw_json = serde_json::to_string_pretty(&class).unwrap_or_default();

    RuntimeClassDetails {
        name,
        creation_timestamp,
        labels,
        annotations,
        handler,
        scheduling,
        raw_json,
    }
}

pub(super) async fn delete_runtimeclass(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<RuntimeClass> = Api::all(client);

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

    refresh_runtimeclasses(state, sender).await?;
    Ok(())
}
