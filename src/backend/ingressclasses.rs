use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, IngressClassDetails, IngressClassParameters,
    IngressClassSummary, ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::networking::v1::IngressClass;
use kube::{
    ResourceExt,
    api::{Api, ListParams},
};

use super::{events, state::BackendState};

pub(super) async fn refresh_ingress_classes(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<IngressClass> = Api::all(client);

    let classes = api.list(&ListParams::default()).await?;
    let summaries = ingress_classes_to_summaries(classes.items);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::network_ingressclasses(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::IngressClassesUpdated(summaries)),
    );
    Ok(())
}

fn ingress_classes_to_summaries(classes: Vec<IngressClass>) -> Vec<IngressClassSummary> {
    let mut out: Vec<IngressClassSummary> = Vec::new();

    for class in classes {
        let name = class.name_any();
        let controller = class
            .spec
            .as_ref()
            .and_then(|s| s.controller.clone())
            .unwrap_or_default();

        out.push(IngressClassSummary { name, controller });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub(super) async fn fetch_ingress_class_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<IngressClass> = Api::all(client);
    let class = api.get(name).await?;

    let details = ingress_class_to_details(class);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::network_ingressclasses(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::IngressClassDetailsUpdated(details)),
    );
    Ok(())
}

fn ingress_class_to_details(class: IngressClass) -> IngressClassDetails {
    let name = class.name_any();
    let creation_timestamp = class
        .creation_timestamp()
        .map(|ts| super::workload_common::kube_rfc3339(&ts))
        .unwrap_or_default();

    let labels = class.labels().clone();
    let annotations = class.annotations().clone();

    let controller = class
        .spec
        .as_ref()
        .and_then(|s| s.controller.clone())
        .unwrap_or_default();

    let parameters = class
        .spec
        .as_ref()
        .and_then(|s| s.parameters.as_ref())
        .map(|params| IngressClassParameters {
            api_group: params.api_group.clone(),
            kind: params.kind.clone(),
            namespace: params.namespace.clone(),
            name: params.name.clone(),
        });

    let raw_json = serde_json::to_string_pretty(&class).unwrap_or_default();

    IngressClassDetails {
        name,
        creation_timestamp,
        labels,
        annotations,
        controller,
        parameters,
        raw_json,
    }
}
