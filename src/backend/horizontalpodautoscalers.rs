use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, HorizontalPodAutoscalerDetails,
    HorizontalPodAutoscalerSummary, ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_horizontalpodautoscalers(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let hpas = state
        .cluster
        .list_namespaced::<HorizontalPodAutoscaler>()
        .await?;
    let summaries = horizontalpodautoscalers_to_summaries(hpas, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_horizontalpodautoscalers(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::HorizontalPodAutoscalersUpdated(
            summaries,
        )),
    );
    Ok(())
}

fn horizontalpodautoscalers_to_summaries(
    hpas: Vec<HorizontalPodAutoscaler>,
    filter: &[String],
) -> Vec<HorizontalPodAutoscalerSummary> {
    let mut out: Vec<HorizontalPodAutoscalerSummary> = Vec::new();

    for hpa in hpas {
        let namespace = hpa.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = hpa.name_any();

        let target_ref = hpa
            .spec
            .as_ref()
            .map(|s| format!("{}/{}", s.scale_target_ref.kind, s.scale_target_ref.name))
            .unwrap_or_default();

        let min_replicas = hpa.spec.as_ref().and_then(|s| s.min_replicas);
        let max_replicas = hpa.spec.as_ref().map(|s| s.max_replicas).unwrap_or(0);
        let current_replicas = hpa.status.as_ref().and_then(|s| s.current_replicas);

        out.push(HorizontalPodAutoscalerSummary {
            namespace,
            name,
            target_ref,
            min_replicas,
            max_replicas,
            current_replicas,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_horizontalpodautoscaler_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client, namespace);
    let hpa = api.get(name).await?;

    let details = horizontalpodautoscaler_to_details(hpa);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_horizontalpodautoscalers(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::HorizontalPodAutoscalerDetailsUpdated(
            details,
        )),
    );
    Ok(())
}

fn horizontalpodautoscaler_to_details(
    hpa: HorizontalPodAutoscaler,
) -> HorizontalPodAutoscalerDetails {
    let namespace = hpa.namespace().unwrap_or_default();
    let name = hpa.name_any();
    let creation_timestamp = hpa
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = hpa.labels().clone();
    let annotations = hpa.annotations().clone();

    let target_ref = hpa
        .spec
        .as_ref()
        .map(|s| format!("{}/{}", s.scale_target_ref.kind, s.scale_target_ref.name))
        .unwrap_or_default();

    let min_replicas = hpa.spec.as_ref().and_then(|s| s.min_replicas);
    let max_replicas = hpa.spec.as_ref().map(|s| s.max_replicas).unwrap_or(0);
    let current_replicas = hpa.status.as_ref().and_then(|s| s.current_replicas);
    let desired_replicas = hpa.status.as_ref().map(|s| s.desired_replicas).unwrap_or(0);

    let conditions = hpa
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|conds| conds.iter().map(|c| c.type_.clone()).collect())
        .unwrap_or_default();

    let raw_json = serde_json::to_string_pretty(&hpa).unwrap_or_default();

    HorizontalPodAutoscalerDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        target_ref,
        min_replicas,
        max_replicas,
        current_replicas,
        desired_replicas,
        conditions,
        raw_json,
    }
}

pub(super) async fn delete_horizontalpodautoscaler(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client, namespace);

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

    refresh_horizontalpodautoscalers(state, sender).await?;
    Ok(())
}
