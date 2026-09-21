use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, ResourceReadEvent,
    RoleBindingDetails, RoleBindingSummary, Route, SubjectSummary, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::rbac::v1::RoleBinding;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_role_bindings(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<RoleBinding> = Api::all(client);

    let bindings = api.list(&ListParams::default()).await?;
    let summaries = role_bindings_to_summaries(bindings.items, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::access_rolebindings(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::RoleBindingsUpdated(summaries)),
    );
    Ok(())
}

fn role_bindings_to_summaries(
    bindings: Vec<RoleBinding>,
    filter: &[String],
) -> Vec<RoleBindingSummary> {
    let mut out: Vec<RoleBindingSummary> = Vec::new();

    for binding in bindings {
        let namespace = binding.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = binding.name_any();
        let role_ref = format!("{}/{}", binding.role_ref.kind, binding.role_ref.name);
        let subjects_count = binding.subjects.as_ref().map(|s| s.len()).unwrap_or(0);

        out.push(RoleBindingSummary {
            name,
            namespace,
            role_ref,
            subjects_count,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_role_binding_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<RoleBinding> = Api::namespaced(client, namespace);
    let binding = api.get(name).await?;

    let details = role_binding_to_details(binding);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::access_rolebindings(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::RoleBindingDetailsUpdated(details)),
    );
    Ok(())
}

fn role_binding_to_details(binding: RoleBinding) -> RoleBindingDetails {
    let namespace = binding.namespace().unwrap_or_default();
    let name = binding.name_any();
    let creation_timestamp = binding
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = binding.labels().clone();
    let annotations = binding.annotations().clone();

    let role_ref = format!("{}/{}", binding.role_ref.kind, binding.role_ref.name);

    let subjects = binding
        .subjects
        .as_ref()
        .map(|subjects| {
            subjects
                .iter()
                .map(|s| SubjectSummary {
                    kind: s.kind.clone(),
                    name: s.name.clone(),
                    namespace: s.namespace.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    let raw_json = serde_json::to_string_pretty(&binding).unwrap_or_default();

    RoleBindingDetails {
        name,
        namespace,
        creation_timestamp,
        labels,
        annotations,
        role_ref,
        subjects,
        raw_json,
    }
}

pub(super) async fn delete_role_binding(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<RoleBinding> = Api::namespaced(client, namespace);

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

    refresh_role_bindings(state, sender).await?;
    Ok(())
}
