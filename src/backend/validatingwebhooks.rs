use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, ResourceReadEvent, Route,
    ValidatingWebhookConfigurationDetails, ValidatingWebhookConfigurationSummary,
    ValidatingWebhookSummary, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::admissionregistration::v1::ValidatingWebhookConfiguration;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_validatingwebhooks(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ValidatingWebhookConfiguration> = Api::all(client);

    let configs = api.list(&ListParams::default()).await?;
    let summaries = validatingwebhooks_to_summaries(configs.items);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::config_validatingwebhooks(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::ValidatingWebhookConfigurationsUpdated(
            summaries,
        )),
    );
    Ok(())
}

fn validatingwebhooks_to_summaries(
    configs: Vec<ValidatingWebhookConfiguration>,
) -> Vec<ValidatingWebhookConfigurationSummary> {
    let mut out: Vec<ValidatingWebhookConfigurationSummary> = Vec::new();

    for config in configs {
        let name = config.name_any();
        let webhooks_count = config.webhooks.as_ref().map(|w| w.len()).unwrap_or(0);

        out.push(ValidatingWebhookConfigurationSummary {
            name,
            webhooks_count,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub(super) async fn fetch_validatingwebhook_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ValidatingWebhookConfiguration> = Api::all(client);
    let config = api.get(name).await?;

    let details = validatingwebhook_to_details(config);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::config_validatingwebhooks(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(
            ResourceReadEvent::ValidatingWebhookConfigurationDetailsUpdated(details),
        ),
    );
    Ok(())
}

fn validatingwebhook_to_details(
    config: ValidatingWebhookConfiguration,
) -> ValidatingWebhookConfigurationDetails {
    let name = config.name_any();
    let creation_timestamp = config
        .creation_timestamp()
        .map(|ts| super::workload_common::kube_rfc3339(&ts))
        .unwrap_or_default();

    let labels = config.labels().clone();
    let annotations = config.annotations().clone();

    let webhooks = config
        .webhooks
        .as_ref()
        .map(|whs| {
            whs.iter()
                .map(|wh| ValidatingWebhookSummary {
                    name: wh.name.clone(),
                    failure_policy: wh.failure_policy.clone(),
                    side_effects: Some(wh.side_effects.clone()),
                })
                .collect()
        })
        .unwrap_or_default();

    let raw_json = serde_json::to_string_pretty(&config).unwrap_or_default();

    ValidatingWebhookConfigurationDetails {
        name,
        creation_timestamp,
        labels,
        annotations,
        webhooks,
        raw_json,
    }
}

pub(super) async fn delete_validatingwebhook(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ValidatingWebhookConfiguration> = Api::all(client);

    let ts = chrono::Utc::now().to_rfc3339();
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

    refresh_validatingwebhooks(state, sender).await?;
    Ok(())
}
