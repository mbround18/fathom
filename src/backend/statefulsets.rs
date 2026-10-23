use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, ResourceReadEvent, Route,
    StatefulSetDetails, StatefulSetSummary, section_keys,
};
use chrono::Utc;
use crossbeam_channel::Sender;
use k8s_openapi::api::apps::v1::StatefulSet;
use kube::ResourceExt;
use kube::api::{Api, DeleteParams, Patch, PatchParams};
use serde_json::json;
use std::sync::atomic::AtomicU64;

use super::{events, state::BackendState};

use super::interactions;
use super::workload_common;

pub(super) async fn refresh_statefulsets(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let statefulsets = state.cluster.list_namespaced::<StatefulSet>().await?;
    let summaries = statefulsets_to_summaries(statefulsets, &state.cluster.namespace_filter);

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_statefulsets(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::StatefulSetsUpdated(summaries)),
    );
    Ok(())
}

fn statefulsets_to_summaries(
    statefulsets: Vec<StatefulSet>,
    filter: &[String],
) -> Vec<StatefulSetSummary> {
    let mut out: Vec<StatefulSetSummary> = Vec::new();

    for ss in statefulsets {
        let namespace = ss.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = ss.name_any();
        let desired = ss.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
        let ready = ss
            .status
            .as_ref()
            .and_then(|s| s.ready_replicas)
            .unwrap_or(0);
        let updated = ss
            .status
            .as_ref()
            .and_then(|s| s.updated_replicas)
            .unwrap_or(0);

        // Best-effort: StatefulSet doesn't always expose an explicit "available" count.
        // Use ready replicas as a reasonable proxy.
        let available = ready;

        out.push(StatefulSetSummary {
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

pub(super) async fn delete_statefulset(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    delete_statefulset_with_client(
        &client,
        state.cluster.seq.as_ref(),
        sender,
        state.cluster.current_context.as_deref(),
        namespace,
        name,
    )
    .await
}

/// The delete itself, against an explicitly supplied client rather than whatever
/// `BackendState` currently holds.
///
/// Split out for 015-mcp-agent-exposure's `delete_resource` MCP tool
/// (`crate::backend::mcp::tools_mutate`), which runs in a spawned task holding a
/// `kube::Client` but no `&BackendState`. The interaction annotation, the delete and the
/// `Notification` emission all stay together here, so UI-driven and MCP-driven deletes are one
/// code path.
pub(super) async fn delete_statefulset_with_client(
    client: &kube::Client,
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    context: Option<&str>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);

    let ts = Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(&api, name, context, "delete", &ts, json!({})).await;

    api.delete(name, &DeleteParams::default()).await?;
    events::notify_info(
        seq,
        sender,
        format!("Deleted statefulset {}/{}", namespace, name),
    );
    Ok(())
}

pub(super) async fn scale_statefulset(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
    replicas: i32,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    scale_statefulset_with_client(
        &client,
        state.cluster.seq.as_ref(),
        sender,
        state.cluster.current_context.as_deref(),
        namespace,
        name,
        replicas,
    )
    .await
}

/// Client-explicit sibling of [`scale_statefulset`]; see
/// [`delete_statefulset_with_client`] for why it exists.
#[allow(clippy::too_many_arguments)]
pub(super) async fn scale_statefulset_with_client(
    client: &kube::Client,
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    context: Option<&str>,
    namespace: &str,
    name: &str,
    replicas: i32,
) -> anyhow::Result<()> {
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);

    let ts = Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &api,
        name,
        context,
        "scale",
        &ts,
        json!({"replicas": replicas}),
    )
    .await;

    let patch = json!({"spec": {"replicas": replicas }});
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    events::notify_info(
        seq,
        sender,
        format!("Scaled statefulset {}/{} to {}", namespace, name, replicas),
    );
    Ok(())
}

pub(super) async fn restart_statefulset(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<StatefulSet> = Api::namespaced(client, namespace);
    let ts = Utc::now().to_rfc3339();

    let _ = interactions::annotate_action(
        &api,
        name,
        state.cluster.current_context.as_deref(),
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
        state.cluster.seq.as_ref(),
        sender,
        format!("Restart requested for statefulset {}/{}", namespace, name),
    );
    Ok(())
}

pub(super) async fn fetch_statefulset_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<StatefulSet> = Api::namespaced(client, namespace);
    let ss = api.get(name).await?;

    let labels = workload_common::map_opt_btreemap(ss.metadata.labels.clone());
    let annotations = workload_common::map_opt_btreemap(ss.metadata.annotations.clone());
    let creation_timestamp = workload_common::creation_timestamp(&ss.metadata);

    let containers =
        workload_common::containers_from_pod_template(ss.spec.as_ref().map(|s| &s.template));

    let ports = workload_common::ports_from_pod_template(ss.spec.as_ref().map(|s| &s.template));

    let pod_selector = ss
        .spec
        .as_ref()
        .and_then(|s| s.selector.match_labels.as_ref())
        .map(|labels| workload_common::selector_string_from_match_labels(Some(labels)))
        .unwrap_or_default();

    let pods = workload_common::pods_for_selector(state, namespace, &pod_selector).await?;
    let raw_json = serde_json::to_string_pretty(&ss)?;

    let details = StatefulSetDetails {
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
            section_keys::workloads_statefulsets(),
            state.cluster.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::StatefulSetDetailsUpdated(details)),
    );

    Ok(())
}
