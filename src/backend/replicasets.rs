use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ReplicaSetDetails, ReplicaSetSummary,
    ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::apps::v1::ReplicaSet;
use kube::ResourceExt;
use kube::api::{Api, DeleteParams, Patch, PatchParams};
use serde_json::json;

use super::{events, state::BackendState};

use super::interactions;
use super::workload_common;

pub(super) async fn refresh_replicasets(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let replicasets = state.cluster.list_namespaced::<ReplicaSet>().await?;
    let summaries = replicasets_to_summaries(replicasets, &state.cluster.namespace_filter);

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_replicasets(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::ReplicaSetsUpdated(summaries)),
    );
    Ok(())
}

fn replicasets_to_summaries(
    replicasets: Vec<ReplicaSet>,
    filter: &[String],
) -> Vec<ReplicaSetSummary> {
    let mut out: Vec<ReplicaSetSummary> = Vec::new();

    for rs in replicasets {
        let namespace = rs.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = rs.name_any();
        let desired = rs.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
        let ready = rs
            .status
            .as_ref()
            .and_then(|s| s.ready_replicas)
            .unwrap_or(0);
        let updated = rs.status.as_ref().map(|s| s.replicas).unwrap_or(0);
        let available = ready;

        out.push(ReplicaSetSummary {
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

pub(super) async fn delete_replicaset(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ReplicaSet> = Api::namespaced(client, namespace);

    let ts = chrono::Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &api,
        name,
        state.cluster.current_context.as_deref(),
        "delete",
        &ts,
        json!({}),
    )
    .await;

    api.delete(name, &DeleteParams::default()).await?;
    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        format!("Deleted replicaset {}/{}", namespace, name),
    );
    Ok(())
}

pub(super) async fn scale_replicaset(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
    replicas: i32,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ReplicaSet> = Api::namespaced(client, namespace);

    let ts = chrono::Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &api,
        name,
        state.cluster.current_context.as_deref(),
        "scale",
        &ts,
        json!({"replicas": replicas}),
    )
    .await;

    let patch = json!({"spec": {"replicas": replicas }});
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        format!("Scaled replicaset {}/{} to {}", namespace, name, replicas),
    );
    Ok(())
}

pub(super) async fn restart_replicaset(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    // Best-effort "restart": delete pods matching the ReplicaSet selector.
    // Unlike Deployments, changing the RS template alone won't reliably restart existing pods.
    let client = state.cluster.client_or_err()?;
    let api: Api<ReplicaSet> = Api::namespaced(client, namespace);
    let rs = api.get(name).await?;

    let selector = rs
        .spec
        .as_ref()
        .and_then(|s| s.selector.match_labels.as_ref())
        .map(|labels| workload_common::selector_string_from_match_labels(Some(labels)))
        .unwrap_or_default();

    let deleted = workload_common::delete_pods_for_selector(state, namespace, &selector).await?;

    let ts = chrono::Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &api,
        name,
        state.cluster.current_context.as_deref(),
        "restart",
        &ts,
        json!({"deletedPods": deleted}),
    )
    .await;

    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        format!(
            "Restart requested for replicaset {}/{} (deleted {} matching pods)",
            namespace, name, deleted
        ),
    );
    Ok(())
}

pub(super) async fn fetch_replicaset_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ReplicaSet> = Api::namespaced(client, namespace);
    let rs = api.get(name).await?;

    let labels = workload_common::map_opt_btreemap(rs.metadata.labels.clone());
    let annotations = workload_common::map_opt_btreemap(rs.metadata.annotations.clone());
    let creation_timestamp = workload_common::creation_timestamp(&rs.metadata);

    let containers = workload_common::containers_from_pod_template(
        rs.spec.as_ref().and_then(|s| s.template.as_ref()),
    );

    let ports = workload_common::ports_from_pod_template(
        rs.spec.as_ref().and_then(|s| s.template.as_ref()),
    );

    let pod_selector = rs
        .spec
        .as_ref()
        .and_then(|s| s.selector.match_labels.as_ref())
        .map(|labels| workload_common::selector_string_from_match_labels(Some(labels)))
        .unwrap_or_default();

    let pods = workload_common::pods_for_selector(state, namespace, &pod_selector).await?;
    let raw_json = serde_json::to_string_pretty(&rs)?;

    let details = ReplicaSetDetails {
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
            section_keys::workloads_replicasets(),
            state.cluster.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ReplicaSetDetailsUpdated(details)),
    );

    Ok(())
}
