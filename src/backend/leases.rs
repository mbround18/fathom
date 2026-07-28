use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, LeaseDetails, LeaseSummary, ResourceListEvent,
    ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::coordination::v1::Lease;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_leases(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Lease> = Api::all(client);

    let leases = api.list(&ListParams::default()).await?;
    let summaries = leases_to_summaries(leases.items, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_leases(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::LeasesUpdated(summaries)),
    );
    Ok(())
}

fn leases_to_summaries(leases: Vec<Lease>, filter: &Vec<String>) -> Vec<LeaseSummary> {
    let mut out: Vec<LeaseSummary> = Vec::new();

    for lease in leases {
        let namespace = lease.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = lease.name_any();

        let holder_identity = lease.spec.as_ref().and_then(|s| s.holder_identity.clone());

        let renew_time = lease
            .spec
            .as_ref()
            .and_then(|s| s.renew_time.as_ref())
            .map(|t| t.0.to_rfc3339());

        out.push(LeaseSummary {
            namespace,
            name,
            holder_identity,
            renew_time,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_lease_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Lease> = Api::namespaced(client, namespace);
    let lease = api.get(name).await?;

    let details = lease_to_details(lease);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::config_leases(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::LeaseDetailsUpdated(details)),
    );
    Ok(())
}

fn lease_to_details(lease: Lease) -> LeaseDetails {
    let namespace = lease.namespace().unwrap_or_default();
    let name = lease.name_any();
    let creation_timestamp = lease
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = lease.labels().clone();
    let annotations = lease.annotations().clone();

    let spec = lease.spec.as_ref();
    let holder_identity = spec.and_then(|s| s.holder_identity.clone());
    let lease_duration_seconds = spec.and_then(|s| s.lease_duration_seconds);
    let acquire_time = spec
        .and_then(|s| s.acquire_time.as_ref())
        .map(|t| t.0.to_rfc3339());
    let renew_time = spec
        .and_then(|s| s.renew_time.as_ref())
        .map(|t| t.0.to_rfc3339());

    let raw_json = serde_json::to_string_pretty(&lease).unwrap_or_default();

    LeaseDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        holder_identity,
        lease_duration_seconds,
        acquire_time,
        renew_time,
        raw_json,
    }
}

pub(super) async fn delete_lease(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Lease> = Api::namespaced(client, namespace);

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

    refresh_leases(state, sender).await?;
    Ok(())
}
