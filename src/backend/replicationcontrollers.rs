use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ReplicationControllerDetails,
    ReplicationControllerSummary, ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::ReplicationController;
use kube::ResourceExt;
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams};
use serde_json::json;

use super::{events, state::BackendState};

use super::state::client_or_err;
use super::workload_common;

pub(super) async fn refresh_replicationcontrollers(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<ReplicationController> = Api::all(client);

    let rcs = api.list(&ListParams::default()).await?;
    let summaries = replicationcontrollers_to_summaries(rcs.items, &state.namespace_filter);

    events::emit(
        state.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::workloads_replicationcontrollers(),
            state.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::ReplicationControllersUpdated(summaries)),
    );
    Ok(())
}

fn replicationcontrollers_to_summaries(
    rcs: Vec<ReplicationController>,
    filter: &Vec<String>,
) -> Vec<ReplicationControllerSummary> {
    let mut out: Vec<ReplicationControllerSummary> = Vec::new();

    for rc in rcs {
        let namespace = rc.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = rc.name_any();
        let desired = rc.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
        let replicas = rc.status.as_ref().map(|s| s.replicas).unwrap_or(0);

        out.push(ReplicationControllerSummary {
            name,
            namespace,
            desired,
            ready: replicas,
            updated: replicas,
            available: replicas,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn delete_replicationcontroller(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<ReplicationController> = Api::namespaced(client, namespace);
    api.delete(name, &DeleteParams::default()).await?;
    events::notify_info(
        state.seq.as_ref(),
        sender,
        format!("Deleted replicationcontroller {}/{}", namespace, name),
    );
    Ok(())
}

pub(super) async fn scale_replicationcontroller(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
    replicas: i32,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<ReplicationController> = Api::namespaced(client, namespace);
    let patch = json!({"spec": {"replicas": replicas }});
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    events::notify_info(
        state.seq.as_ref(),
        sender,
        format!(
            "Scaled replicationcontroller {}/{} to {}",
            namespace, name, replicas
        ),
    );
    Ok(())
}

pub(super) async fn restart_replicationcontroller(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    // Best-effort "restart": delete pods matching the RC selector.
    let client = client_or_err(state)?;
    let api: Api<ReplicationController> = Api::namespaced(client, namespace);
    let rc = api.get(name).await?;

    let selector = rc
        .spec
        .as_ref()
        .and_then(|s| s.selector.as_ref())
        .map(|m| workload_common::selector_string_from_map(Some(m)))
        .unwrap_or_default();

    let deleted = workload_common::delete_pods_for_selector(state, namespace, &selector).await?;
    events::notify_info(
        state.seq.as_ref(),
        sender,
        format!(
            "Restart requested for replicationcontroller {}/{} (deleted {} matching pods)",
            namespace, name, deleted
        ),
    );
    Ok(())
}

pub(super) async fn fetch_replicationcontroller_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<ReplicationController> = Api::namespaced(client, namespace);
    let rc = api.get(name).await?;

    let labels = workload_common::map_opt_btreemap(rc.metadata.labels.clone());
    let annotations = workload_common::map_opt_btreemap(rc.metadata.annotations.clone());
    let creation_timestamp = workload_common::creation_timestamp(&rc.metadata);

    let containers = workload_common::containers_from_pod_template(
        rc.spec.as_ref().and_then(|s| s.template.as_ref()),
    );

    let pod_selector = rc
        .spec
        .as_ref()
        .and_then(|s| s.selector.as_ref())
        .map(|m| workload_common::selector_string_from_map(Some(m)))
        .unwrap_or_default();

    let pods = workload_common::pods_for_selector(state, namespace, &pod_selector).await?;
    let raw_json = serde_json::to_string_pretty(&rc)?;

    let details = ReplicationControllerDetails {
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
            section_keys::workloads_replicationcontrollers(),
            state.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ReplicationControllerDetailsUpdated(
            details,
        )),
    );

    Ok(())
}
