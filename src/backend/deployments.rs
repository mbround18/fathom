use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, DeploymentDetails, DeploymentSummary,
    ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use chrono::Utc;
use crossbeam_channel::Sender;
use k8s_openapi::api::apps::v1::Deployment;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, Patch, PatchParams},
};
use serde_json::json;
use std::sync::atomic::AtomicU64;

use super::workload_common;
use super::{events, state::BackendState};

pub(super) async fn refresh_deployments(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let filter = &state.cluster.namespace_filter;
    let deployments = state.cluster.list_namespaced::<Deployment>().await?;
    let summaries = deployments_to_summaries(deployments, filter);

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_deployments(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::DeploymentsUpdated(summaries)),
    );
    Ok(())
}

/// List all deployments and emit them (filtered by `filter`) as `DeploymentsUpdated`, tagged
/// with the context they were listed from. Used by the background deployment watcher.
pub(super) async fn emit_deployments_snapshot(
    client: &kube::Client,
    filter: &[String],
    scope: &super::ns_scope::ScopeState,
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    context: &Option<String>,
) {
    match super::ns_scope::list_scoped::<Deployment>(client.clone(), filter, scope).await {
        Ok(list) => {
            let summaries = deployments_to_summaries(list, filter);
            events::emit(
                seq,
                sender,
                Route::namespaced(section_keys::workloads_deployments(), context.clone(), None),
                Correlation::None,
                BackendEvent::ResourceList(ResourceListEvent::DeploymentsUpdated(summaries)),
            );
        }
        Err(e) => events::report_error(
            seq,
            sender,
            format!("Background deployment refresh failed: {e}"),
            super::ns_scope::is_forbidden(&e),
        ),
    }
}

fn deployments_to_summaries(
    deployments: Vec<Deployment>,
    filter: &[String],
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
    let client = state.cluster.client_or_err()?;
    delete_deployment_with_client(&client, state.cluster.seq.as_ref(), sender, namespace, name)
        .await
}

/// The delete itself, against an explicitly supplied client rather than whatever
/// `BackendState` currently holds.
///
/// Split out for 015-mcp-agent-exposure's `delete_resource` MCP tool
/// (`crate::backend::mcp::tools_mutate`), which serves external MCP clients from a spawned task
/// that has a `kube::Client` but no `&BackendState` borrow. The write *and* its
/// `Notification` emission both live here, so a UI-driven delete and an MCP-driven one are the
/// same code path and produce the same event — not two parallel implementations.
pub(super) async fn delete_deployment_with_client(
    client: &kube::Client,
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    api.delete(name, &DeleteParams::default()).await?;
    events::notify_info(
        seq,
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
    let client = state.cluster.client_or_err()?;
    scale_deployment_with_client(
        &client,
        state.cluster.seq.as_ref(),
        sender,
        namespace,
        name,
        replicas,
    )
    .await
}

/// Client-explicit sibling of [`scale_deployment`]; see
/// [`delete_deployment_with_client`] for why it exists.
pub(super) async fn scale_deployment_with_client(
    client: &kube::Client,
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
    replicas: i32,
) -> anyhow::Result<()> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    let patch = json!({"spec": {"replicas": replicas }});
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    events::notify_info(
        seq,
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
    let client = state.cluster.client_or_err()?;
    let api: Api<Deployment> = Api::namespaced(client, namespace);
    let ts = Utc::now().to_rfc3339();
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
        state.cluster.seq.as_ref(),
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
    let client = state.cluster.client_or_err()?;
    let api: Api<Deployment> = Api::namespaced(client, namespace);
    let deploy = api.get(name).await?;

    let labels = workload_common::map_opt_btreemap(deploy.metadata.labels.clone());
    let annotations = workload_common::map_opt_btreemap(deploy.metadata.annotations.clone());
    let creation_timestamp = workload_common::creation_timestamp(&deploy.metadata);

    let containers =
        workload_common::containers_from_pod_template(deploy.spec.as_ref().map(|s| &s.template));
    let ports = workload_common::ports_from_pod_template(deploy.spec.as_ref().map(|s| &s.template));

    // Best-effort association via `.spec.selector.matchLabels`.
    // This won’t cover matchExpressions, but works for the common case.
    let pod_selector = deploy
        .spec
        .as_ref()
        .and_then(|s| s.selector.match_labels.as_ref())
        .map(|labels| workload_common::selector_string_from_match_labels(Some(labels)))
        .unwrap_or_default();

    let pods = workload_common::pods_for_selector(state, namespace, &pod_selector).await?;
    let raw_json = serde_json::to_string_pretty(&deploy)?;

    let details = DeploymentDetails {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        creation_timestamp,
        labels,
        annotations,
        containers,
        ports,
        pod_selector,
        pods,
        raw_json,
    };

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_deployments(),
            state.cluster.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::DeploymentDetailsUpdated(details)),
    );
    Ok(())
}
