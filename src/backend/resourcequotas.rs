use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, ResourceQuotaDetails,
    ResourceQuotaSummary, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::ResourceQuota;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};
use std::collections::BTreeMap;

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_resourcequotas(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ResourceQuota> = Api::all(client);

    let quotas = api.list(&ListParams::default()).await?;
    let summaries = resourcequotas_to_summaries(quotas.items, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_resourcequotas(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::ResourceQuotasUpdated(summaries)),
    );
    Ok(())
}

fn resourcequotas_to_summaries(
    quotas: Vec<ResourceQuota>,
    filter: &Vec<String>,
) -> Vec<ResourceQuotaSummary> {
    let mut out: Vec<ResourceQuotaSummary> = Vec::new();

    for rq in quotas {
        let namespace = rq.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = rq.name_any();

        let hard_limits_count = rq
            .spec
            .as_ref()
            .and_then(|s| s.hard.as_ref())
            .map(|h| h.len())
            .unwrap_or(0);

        out.push(ResourceQuotaSummary {
            name,
            namespace,
            hard_limits_count,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_resourcequota_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ResourceQuota> = Api::namespaced(client, namespace);
    let rq = api.get(name).await?;

    let details = resourcequota_to_details(rq);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_resourcequotas(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ResourceQuotaDetailsUpdated(details)),
    );
    Ok(())
}

fn resourcequota_to_details(rq: ResourceQuota) -> ResourceQuotaDetails {
    let namespace = rq.namespace().unwrap_or_default();
    let name = rq.name_any();
    let creation_timestamp = rq
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = rq.labels().clone();
    let annotations = rq.annotations().clone();

    let hard: BTreeMap<String, String> = rq
        .spec
        .as_ref()
        .and_then(|s| s.hard.as_ref())
        .map(|h| {
            h.iter()
                .map(|(k, v)| (k.clone(), v.0.clone()))
                .collect()
        })
        .unwrap_or_default();

    let used: BTreeMap<String, String> = rq
        .status
        .as_ref()
        .and_then(|s| s.used.as_ref())
        .map(|u| {
            u.iter()
                .map(|(k, v)| (k.clone(), v.0.clone()))
                .collect()
        })
        .unwrap_or_default();

    let raw_json = serde_json::to_string_pretty(&rq).unwrap_or_default();

    ResourceQuotaDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        hard,
        used,
        raw_json,
    }
}

pub(super) async fn delete_resourcequota(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ResourceQuota> = Api::namespaced(client, namespace);

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

    refresh_resourcequotas(state, sender).await?;
    Ok(())
}
