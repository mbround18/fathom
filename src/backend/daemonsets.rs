use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, DaemonSetDetails, DaemonSetSummary,
    ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::{api::apps::v1::DaemonSet, chrono::Utc};
use kube::ResourceExt;
use kube::api::{Api, DeleteParams, Patch, PatchParams};
use serde_json::json;

use super::{events, state::BackendState};

use super::interactions;
use super::workload_common;

pub(super) async fn refresh_daemonsets(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let daemonsets = state.cluster.list_namespaced::<DaemonSet>().await?;
    let summaries = daemonsets_to_summaries(daemonsets, &state.cluster.namespace_filter);

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_daemonsets(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::DaemonSetsUpdated(summaries)),
    );
    Ok(())
}

fn daemonsets_to_summaries(daemonsets: Vec<DaemonSet>, filter: &[String]) -> Vec<DaemonSetSummary> {
    let mut out: Vec<DaemonSetSummary> = Vec::new();

    for ds in daemonsets {
        let namespace = ds.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = ds.name_any();
        let desired = ds
            .status
            .as_ref()
            .map(|s| s.desired_number_scheduled)
            .unwrap_or(0);
        let ready = ds.status.as_ref().map(|s| s.number_ready).unwrap_or(0);
        let updated = ds
            .status
            .as_ref()
            .and_then(|s| s.updated_number_scheduled)
            .unwrap_or(0);
        let available = ds
            .status
            .as_ref()
            .and_then(|s| s.number_available)
            .unwrap_or(0);

        out.push(DaemonSetSummary {
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

pub(super) async fn delete_daemonset(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<DaemonSet> = Api::namespaced(client, namespace);

    let ts = Utc::now().to_rfc3339();
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
        format!("Deleted daemonset {}/{}", namespace, name),
    );
    Ok(())
}

pub(super) async fn restart_daemonset(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<DaemonSet> = Api::namespaced(client, namespace);
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
        format!("Restart requested for daemonset {}/{}", namespace, name),
    );
    Ok(())
}

pub(super) async fn fetch_daemonset_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<DaemonSet> = Api::namespaced(client, namespace);
    let ds = api.get(name).await?;

    let labels = workload_common::map_opt_btreemap(ds.metadata.labels.clone());
    let annotations = workload_common::map_opt_btreemap(ds.metadata.annotations.clone());
    let creation_timestamp = workload_common::creation_timestamp(&ds.metadata);

    let containers =
        workload_common::containers_from_pod_template(ds.spec.as_ref().map(|s| &s.template));

    let ports = workload_common::ports_from_pod_template(ds.spec.as_ref().map(|s| &s.template));

    let pod_selector = ds
        .spec
        .as_ref()
        .and_then(|s| s.selector.match_labels.as_ref())
        .map(|labels| workload_common::selector_string_from_match_labels(Some(labels)))
        .unwrap_or_default();

    let pods = workload_common::pods_for_selector(state, namespace, &pod_selector).await?;
    let raw_json = serde_json::to_string_pretty(&ds)?;

    let details = DaemonSetDetails {
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
            section_keys::workloads_daemonsets(),
            state.cluster.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::DaemonSetDetailsUpdated(details)),
    );

    Ok(())
}
