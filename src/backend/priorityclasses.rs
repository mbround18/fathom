use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, PriorityClassDetails, PriorityClassSummary,
    ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::scheduling::v1::PriorityClass;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_priorityclasses(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<PriorityClass> = Api::all(client);

    let classes = api.list(&ListParams::default()).await?;
    let summaries = priorityclasses_to_summaries(classes.items);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::config_priorityclasses(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::PriorityClassesUpdated(summaries)),
    );
    Ok(())
}

fn priorityclasses_to_summaries(classes: Vec<PriorityClass>) -> Vec<PriorityClassSummary> {
    let mut out: Vec<PriorityClassSummary> = Vec::new();

    for class in classes {
        let name = class.name_any();
        let value = class.value;
        let global_default = class.global_default.unwrap_or(false);
        let description = class.description.clone();

        out.push(PriorityClassSummary {
            name,
            value,
            global_default,
            description,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub(super) async fn fetch_priorityclass_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<PriorityClass> = Api::all(client);
    let class = api.get(name).await?;

    let details = priorityclass_to_details(class);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::config_priorityclasses(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::PriorityClassDetailsUpdated(details)),
    );
    Ok(())
}

fn priorityclass_to_details(class: PriorityClass) -> PriorityClassDetails {
    let name = class.name_any();
    let creation_timestamp = class
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = class.labels().clone();
    let annotations = class.annotations().clone();

    let value = class.value;
    let global_default = class.global_default.unwrap_or(false);
    let description = class.description.clone();
    let preemption_policy = class.preemption_policy.clone();

    let raw_json = serde_json::to_string_pretty(&class).unwrap_or_default();

    PriorityClassDetails {
        name,
        creation_timestamp,
        labels,
        annotations,
        value,
        global_default,
        description,
        preemption_policy,
        raw_json,
    }
}

pub(super) async fn delete_priorityclass(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<PriorityClass> = Api::all(client);

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

    refresh_priorityclasses(state, sender).await?;
    Ok(())
}
