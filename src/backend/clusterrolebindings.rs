use crate::protocol::{
    BackendEvent, BackendEventEnvelope, ClusterRoleBindingDetails, ClusterRoleBindingSummary,
    Correlation, ResourceListEvent, ResourceReadEvent, Route, SubjectSummary, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::rbac::v1::ClusterRoleBinding;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_cluster_role_bindings(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ClusterRoleBinding> = Api::all(client);

    let bindings = api.list(&ListParams::default()).await?;
    let summaries = cluster_role_bindings_to_summaries(bindings.items);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::access_clusterrolebindings(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::ClusterRoleBindingsUpdated(summaries)),
    );
    Ok(())
}

fn cluster_role_bindings_to_summaries(
    bindings: Vec<ClusterRoleBinding>,
) -> Vec<ClusterRoleBindingSummary> {
    let mut out: Vec<ClusterRoleBindingSummary> = Vec::new();

    for binding in bindings {
        let name = binding.name_any();
        let role_ref = format!("{}/{}", binding.role_ref.kind, binding.role_ref.name);
        let subjects_count = binding.subjects.as_ref().map(|s| s.len()).unwrap_or(0);

        out.push(ClusterRoleBindingSummary {
            name,
            role_ref,
            subjects_count,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub(super) async fn fetch_cluster_role_binding_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ClusterRoleBinding> = Api::all(client);
    let binding = api.get(name).await?;

    let details = cluster_role_binding_to_details(binding);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::access_clusterrolebindings(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ClusterRoleBindingDetailsUpdated(details)),
    );
    Ok(())
}

fn cluster_role_binding_to_details(binding: ClusterRoleBinding) -> ClusterRoleBindingDetails {
    let name = binding.name_any();
    let creation_timestamp = binding
        .creation_timestamp()
        .map(|ts| super::workload_common::kube_rfc3339(&ts))
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

    let resource_version = binding.resource_version().unwrap_or_default();
    let raw_json = serde_json::to_string_pretty(&binding).unwrap_or_default();

    ClusterRoleBindingDetails {
        name,
        creation_timestamp,
        labels,
        annotations,
        role_ref,
        subjects,
        raw_json,
        resource_version,
    }
}

pub(super) async fn delete_cluster_role_binding(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ClusterRoleBinding> = Api::all(client);

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

    refresh_cluster_role_bindings(state, sender).await?;
    Ok(())
}
