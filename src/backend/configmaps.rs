use crate::protocol::{
    BackendEvent, BackendEventEnvelope, ConfigMapDetails, ConfigMapSummary, Correlation,
    ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams},
};
use std::collections::BTreeMap;

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_configmaps(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let configmaps = state.cluster.list_namespaced::<ConfigMap>().await?;
    let summaries = configmaps_to_summaries(configmaps, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_configmaps(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::ConfigMapsUpdated(summaries)),
    );
    Ok(())
}

fn configmaps_to_summaries(configmaps: Vec<ConfigMap>, filter: &[String]) -> Vec<ConfigMapSummary> {
    let mut out: Vec<ConfigMapSummary> = Vec::new();

    for cm in configmaps {
        let namespace = cm.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = cm.name_any();

        let keys_count = cm.data.as_ref().map(|d| d.len()).unwrap_or(0)
            + cm.binary_data.as_ref().map(|d| d.len()).unwrap_or(0);

        out.push(ConfigMapSummary {
            name,
            namespace,
            keys_count,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_configmap_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ConfigMap> = Api::namespaced(client, namespace);
    let cm = api.get(name).await?;

    let details = configmap_to_details(cm);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_configmaps(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ConfigMapDetailsUpdated(details)),
    );
    Ok(())
}

fn configmap_to_details(cm: ConfigMap) -> ConfigMapDetails {
    let namespace = cm.namespace().unwrap_or_default();
    let name = cm.name_any();
    let creation_timestamp = cm
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = cm.labels().clone();
    let annotations = cm.annotations().clone();

    let mut data: BTreeMap<String, String> = cm.data.clone().unwrap_or_default();
    if let Some(binary_data) = cm.binary_data.as_ref() {
        for key in binary_data.keys() {
            data.entry(key.clone())
                .or_insert_with(|| "<binary data>".to_string());
        }
    }

    let mut keys: Vec<String> = data.keys().cloned().collect();
    keys.sort();

    let raw_json = serde_json::to_string_pretty(&cm).unwrap_or_default();

    ConfigMapDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        keys,
        data,
        raw_json,
    }
}

pub(super) async fn delete_configmap(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ConfigMap> = Api::namespaced(client, namespace);

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

    refresh_configmaps(state, sender).await?;
    Ok(())
}
