use crate::protocol::PodSummary;
use k8s_openapi::api::core::v1::Pod;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};
use std::collections::BTreeMap;

use super::pods;
use super::state::BackendState;

pub(super) fn map_opt_btreemap(map: Option<BTreeMap<String, String>>) -> BTreeMap<String, String> {
    map.unwrap_or_default().into_iter().collect()
}

pub(super) fn creation_timestamp(meta: &kube::core::ObjectMeta) -> String {
    meta.creation_timestamp
        .as_ref()
        .map(|t| t.0.to_rfc3339())
        .unwrap_or_default()
}

pub(super) fn containers_from_pod_template(
    template: Option<&k8s_openapi::api::core::v1::PodTemplateSpec>,
) -> Vec<String> {
    template
        .and_then(|t| t.spec.as_ref())
        .map(|ps| {
            ps.containers
                .iter()
                .map(|c| format!("{} ({})", c.name, c.image.clone().unwrap_or_default()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn ports_from_pod_template(
    template: Option<&k8s_openapi::api::core::v1::PodTemplateSpec>,
) -> Vec<crate::protocol::ContainerPort> {
    template
        .and_then(|t| t.spec.as_ref())
        .map(|ps| {
            ps.containers
                .iter()
                .flat_map(|c| {
                    c.ports
                        .iter()
                        .flatten()
                        .map(|p| crate::protocol::ContainerPort {
                            container: c.name.clone(),
                            name: p.name.clone(),
                            port: p.container_port as u16,
                            protocol: p.protocol.clone().unwrap_or_else(|| "TCP".to_owned()),
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn selector_string_from_match_labels(
    match_labels: Option<&BTreeMap<String, String>>,
) -> String {
    match_labels
        .map(|labels| {
            let mut parts: Vec<String> =
                labels.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
            parts.sort();
            parts.join(",")
        })
        .unwrap_or_default()
}

pub(super) fn selector_string_from_map(map: Option<&BTreeMap<String, String>>) -> String {
    selector_string_from_match_labels(map)
}

pub(super) async fn pods_for_selector(
    state: &BackendState,
    namespace: &str,
    selector: &str,
) -> anyhow::Result<Vec<PodSummary>> {
    if selector.is_empty() {
        return Ok(Vec::new());
    }

    let client = state.cluster.client_or_err()?;
    let pod_api: Api<Pod> = Api::namespaced(client, namespace);
    let list = pod_api
        .list(&ListParams::default().labels(selector))
        .await?;
    Ok(pods::pods_to_summaries(list.items, &[]))
}

pub(super) async fn delete_pods_for_selector(
    state: &BackendState,
    namespace: &str,
    selector: &str,
) -> anyhow::Result<u32> {
    if selector.is_empty() {
        return Ok(0);
    }

    let client = state.cluster.client_or_err()?;
    let pod_api: Api<Pod> = Api::namespaced(client, namespace);
    let list = pod_api
        .list(&ListParams::default().labels(selector))
        .await?;

    let mut deleted: u32 = 0;
    for pod in list.items {
        let name = pod.name_any();
        // Ignore per-pod delete errors to be best-effort.
        if pod_api
            .delete(&name, &DeleteParams::default())
            .await
            .is_ok()
        {
            deleted += 1;
        }
    }

    Ok(deleted)
}
