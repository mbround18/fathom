use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, PodDisruptionBudgetDetails,
    PodDisruptionBudgetSummary, ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams},
};

use super::interactions;
use super::{events, state::BackendState};

fn int_or_string_to_string(v: &IntOrString) -> String {
    match v {
        IntOrString::Int(n) => n.to_string(),
        IntOrString::String(s) => s.clone(),
    }
}

pub(super) async fn refresh_poddisruptionbudgets(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let pdbs = state
        .cluster
        .list_namespaced::<PodDisruptionBudget>()
        .await?;
    let summaries = poddisruptionbudgets_to_summaries(pdbs, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_poddisruptionbudgets(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::PodDisruptionBudgetsUpdated(summaries)),
    );
    Ok(())
}

fn poddisruptionbudgets_to_summaries(
    pdbs: Vec<PodDisruptionBudget>,
    filter: &[String],
) -> Vec<PodDisruptionBudgetSummary> {
    let mut out: Vec<PodDisruptionBudgetSummary> = Vec::new();

    for pdb in pdbs {
        let namespace = pdb.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = pdb.name_any();

        let min_available = pdb
            .spec
            .as_ref()
            .and_then(|s| s.min_available.as_ref())
            .map(int_or_string_to_string);
        let max_unavailable = pdb
            .spec
            .as_ref()
            .and_then(|s| s.max_unavailable.as_ref())
            .map(int_or_string_to_string);

        let current_healthy = pdb.status.as_ref().map(|s| s.current_healthy);
        let desired_healthy = pdb.status.as_ref().map(|s| s.desired_healthy);

        out.push(PodDisruptionBudgetSummary {
            namespace,
            name,
            min_available,
            max_unavailable,
            current_healthy,
            desired_healthy,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_poddisruptionbudget_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<PodDisruptionBudget> = Api::namespaced(client, namespace);
    let pdb = api.get(name).await?;

    let details = poddisruptionbudget_to_details(pdb);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_poddisruptionbudgets(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::PodDisruptionBudgetDetailsUpdated(
            details,
        )),
    );
    Ok(())
}

fn poddisruptionbudget_to_details(pdb: PodDisruptionBudget) -> PodDisruptionBudgetDetails {
    let namespace = pdb.namespace().unwrap_or_default();
    let name = pdb.name_any();
    let creation_timestamp = pdb
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = pdb.labels().clone();
    let annotations = pdb.annotations().clone();

    let min_available = pdb
        .spec
        .as_ref()
        .and_then(|s| s.min_available.as_ref())
        .map(int_or_string_to_string);
    let max_unavailable = pdb
        .spec
        .as_ref()
        .and_then(|s| s.max_unavailable.as_ref())
        .map(int_or_string_to_string);

    let current_healthy = pdb.status.as_ref().map(|s| s.current_healthy);
    let desired_healthy = pdb.status.as_ref().map(|s| s.desired_healthy);
    let disruptions_allowed = pdb.status.as_ref().map(|s| s.disruptions_allowed);
    let expected_pods = pdb.status.as_ref().map(|s| s.expected_pods);

    let raw_json = serde_json::to_string_pretty(&pdb).unwrap_or_default();

    PodDisruptionBudgetDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        min_available,
        max_unavailable,
        current_healthy,
        desired_healthy,
        disruptions_allowed,
        expected_pods,
        raw_json,
    }
}

pub(super) async fn delete_poddisruptionbudget(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<PodDisruptionBudget> = Api::namespaced(client, namespace);

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

    refresh_poddisruptionbudgets(state, sender).await?;
    Ok(())
}
