use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, LimitRangeDetails, LimitRangeItemSummary,
    LimitRangeSummary, ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::LimitRange;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};
use std::collections::BTreeMap;

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_limitranges(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<LimitRange> = Api::all(client);

    let limitranges = api.list(&ListParams::default()).await?;
    let summaries = limitranges_to_summaries(limitranges.items, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_limitranges(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::LimitRangesUpdated(summaries)),
    );
    Ok(())
}

fn limitranges_to_summaries(
    limitranges: Vec<LimitRange>,
    filter: &Vec<String>,
) -> Vec<LimitRangeSummary> {
    let mut out: Vec<LimitRangeSummary> = Vec::new();

    for lr in limitranges {
        let namespace = lr.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = lr.name_any();

        let limits_count = lr
            .spec
            .as_ref()
            .map(|s| s.limits.len())
            .unwrap_or(0);

        out.push(LimitRangeSummary {
            name,
            namespace,
            limits_count,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_limitrange_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<LimitRange> = Api::namespaced(client, namespace);
    let lr = api.get(name).await?;

    let details = limitrange_to_details(lr);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_limitranges(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::LimitRangeDetailsUpdated(details)),
    );
    Ok(())
}

fn limitrange_to_details(lr: LimitRange) -> LimitRangeDetails {
    let namespace = lr.namespace().unwrap_or_default();
    let name = lr.name_any();
    let creation_timestamp = lr
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = lr.labels().clone();
    let annotations = lr.annotations().clone();

    let limits: Vec<LimitRangeItemSummary> = lr
        .spec
        .as_ref()
        .map(|s| {
            s.limits
                .iter()
                .map(|item| {
                    let max: BTreeMap<String, String> = item
                        .max
                        .as_ref()
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.0.clone())).collect())
                        .unwrap_or_default();
                    let min: BTreeMap<String, String> = item
                        .min
                        .as_ref()
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.0.clone())).collect())
                        .unwrap_or_default();
                    let default: BTreeMap<String, String> = item
                        .default
                        .as_ref()
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.0.clone())).collect())
                        .unwrap_or_default();

                    LimitRangeItemSummary {
                        limit_type: item.type_.clone(),
                        max,
                        min,
                        default,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let raw_json = serde_json::to_string_pretty(&lr).unwrap_or_default();

    LimitRangeDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        limits,
        raw_json,
    }
}

pub(super) async fn delete_limitrange(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<LimitRange> = Api::namespaced(client, namespace);

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

    refresh_limitranges(state, sender).await?;
    Ok(())
}
