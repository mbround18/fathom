use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, DeploymentDetails, DeploymentSummary,
    ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::{api::apps::v1::Deployment, chrono::Utc};
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams, Patch, PatchParams},
};
use serde_json::json;
use std::collections::BTreeMap;

use super::interactions;
use super::pods;
use super::state::client_or_err;
use super::{events, state::BackendState};

pub(super) async fn refresh_deployments(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<Deployment> = Api::all(client);

    let deployments = api.list(&ListParams::default()).await?;
    let summaries = deployments_to_summaries(deployments.items, &state.namespace_filter);
    events::emit(
        state.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_deployments(),
            state.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::DeploymentsUpdated(summaries)),
    );
    Ok(())
}

fn deployments_to_summaries(
    deployments: Vec<Deployment>,
    filter: &Vec<String>,
) -> Vec<DeploymentSummary> {
    let mut out: Vec<DeploymentSummary> = Vec::new();

    for deploy in deployments {
        let namespace = deploy.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = deploy.name_any();
        let desired = deploy.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);

        let ready = deploy
            .status
            .as_ref()
            .and_then(|s| s.ready_replicas)
            .unwrap_or(0);
        let updated = deploy
            .status
            .as_ref()
            .and_then(|s| s.updated_replicas)
            .unwrap_or(0);
        let available = deploy
            .status
            .as_ref()
            .and_then(|s| s.available_replicas)
            .unwrap_or(0);

        out.push(DeploymentSummary {
            name,
            namespace,
            desired,
            ready,
            updated,
            available,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn delete_deployment(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<Deployment> = Api::namespaced(client, namespace);

    let ts = Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &api,
        name,
        state.current_context.as_deref(),
        "delete",
        &ts,
        json!({}),
    )
    .await;

    api.delete(name, &DeleteParams::default()).await?;
    events::notify_info(
        state.seq.as_ref(),
        sender,
        format!("Deleted deployment {}/{}", namespace, name),
    );
    Ok(())
}

pub(super) async fn scale_deployment(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
    replicas: i32,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<Deployment> = Api::namespaced(client, namespace);

    let ts = Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &api,
        name,
        state.current_context.as_deref(),
        "scale",
        &ts,
        json!({"replicas": replicas}),
    )
    .await;

    let patch = json!({"spec": {"replicas": replicas }});
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    events::notify_info(
        state.seq.as_ref(),
        sender,
        format!("Scaled deployment {}/{} to {}", namespace, name, replicas),
    );
    Ok(())
}

pub(super) async fn restart_deployment(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<Deployment> = Api::namespaced(client, namespace);
    let ts = Utc::now().to_rfc3339();

    let _ = interactions::annotate_action(
        &api,
        name,
        state.current_context.as_deref(),
        "restart",
        &ts,
        json!({"restartedAt": ts}),
    )
    .await;

    let patch = json!({
        "spec": {
            "template": {
                "metadata": {
                    "annotations": {"fathom/restartedAt": ts}
                }
            }
        }
    });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    events::notify_info(
        state.seq.as_ref(),
        sender,
        format!("Restart requested for deployment {}/{}", namespace, name),
    );
    Ok(())
}

pub(super) async fn fetch_deployment_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<Deployment> = Api::namespaced(client, namespace);
    let deploy = api.get(name).await?;

    let labels: BTreeMap<String, String> = deploy
        .metadata
        .labels
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let annotations: BTreeMap<String, String> = deploy
        .metadata
        .annotations
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect();

    let creation_timestamp = deploy
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|t| t.0.to_rfc3339())
        .unwrap_or_default();

    let containers = deploy
        .spec
        .as_ref()
        .and_then(|s| s.template.spec.as_ref())
        .map(|ps| {
            ps.containers
                .iter()
                .map(|c| format!("{} ({})", c.name, c.image.clone().unwrap_or_default()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Best-effort association via `.spec.selector.matchLabels`.
    // This won’t cover matchExpressions, but works for the common case.
    let pod_selector = deploy
        .spec
        .as_ref()
        .and_then(|s| s.selector.match_labels.as_ref())
        .map(|labels| {
            let mut parts: Vec<String> =
                labels.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
            parts.sort();
            parts.join(",")
        })
        .unwrap_or_default();

    let pods = if pod_selector.is_empty() {
        Vec::new()
    } else {
        let pod_api: Api<Pod> = Api::namespaced(client_or_err(state)?, namespace);
        let list = pod_api
            .list(&ListParams::default().labels(&pod_selector))
            .await?;
        pods::pods_to_summaries(list.items, &Vec::new())
    };

    let raw_json = serde_json::to_string_pretty(&deploy)?;

    let details = DeploymentDetails {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        creation_timestamp,
        labels,
        annotations,
        containers,
        pod_selector,
        pods,
        raw_json,
    };

    events::emit(
        state.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_deployments(),
            state.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::DeploymentDetailsUpdated(details)),
    );
    Ok(())
}
