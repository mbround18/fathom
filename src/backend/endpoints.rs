use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, EndpointAddress, EndpointDetails,
    EndpointPort, EndpointSubset, EndpointSummary, ResourceListEvent, ResourceReadEvent, Route,
    section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::Endpoints;
use kube::{ResourceExt, api::Api};

use super::{events, state::BackendState};

pub(super) async fn refresh_endpoints(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let endpoints = state.cluster.list_namespaced::<Endpoints>().await?;
    let summaries = endpoints_to_summaries(endpoints, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::network_endpoints(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::EndpointsUpdated(summaries)),
    );
    Ok(())
}

fn endpoints_to_summaries(endpoints: Vec<Endpoints>, filter: &[String]) -> Vec<EndpointSummary> {
    let mut out: Vec<EndpointSummary> = Vec::new();

    for ep in endpoints {
        let namespace = ep.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = ep.name_any();
        let mut ready_addresses_count = 0;
        let mut not_ready_addresses_count = 0;
        let mut port_count = 0;

        if let Some(subsets) = ep.subsets.as_ref() {
            for subset in subsets {
                ready_addresses_count += subset.addresses.as_ref().map(|v| v.len()).unwrap_or(0);
                not_ready_addresses_count += subset
                    .not_ready_addresses
                    .as_ref()
                    .map(|v| v.len())
                    .unwrap_or(0);
                port_count = subset.ports.as_ref().map(|v| v.len()).unwrap_or(0);
            }
        }

        out.push(EndpointSummary {
            name,
            namespace,
            ready_addresses_count,
            not_ready_addresses_count,
            port_count,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_endpoint_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Endpoints> = Api::namespaced(client, namespace);
    let ep = api.get(name).await?;

    let details = endpoint_to_details(ep);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::network_endpoints(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::EndpointDetailsUpdated(details)),
    );
    Ok(())
}

fn endpoint_to_details(ep: Endpoints) -> EndpointDetails {
    let namespace = ep.namespace().unwrap_or_default();
    let name = ep.name_any();
    let creation_timestamp = ep
        .creation_timestamp()
        .map(|ts| super::workload_common::kube_rfc3339(&ts))
        .unwrap_or_default();

    let labels = ep.labels().clone();
    let annotations = ep.annotations().clone();

    let subsets = ep
        .subsets
        .as_ref()
        .map(|subset_list| {
            subset_list
                .iter()
                .map(|subset| {
                    let ready_addresses = subset
                        .addresses
                        .as_ref()
                        .map(|addrs| {
                            addrs
                                .iter()
                                .map(|addr| EndpointAddress {
                                    ip: addr.ip.clone(),
                                    hostname: addr.hostname.clone(),
                                    node_name: addr.node_name.clone(),
                                    target_ref: addr.target_ref.as_ref().map(|tr| {
                                        format!(
                                            "{}/{}/{}",
                                            tr.kind.as_deref().unwrap_or("Unknown"),
                                            tr.namespace.as_deref().unwrap_or(""),
                                            tr.name.as_deref().unwrap_or("")
                                        )
                                    }),
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    let not_ready_addresses = subset
                        .not_ready_addresses
                        .as_ref()
                        .map(|addrs| {
                            addrs
                                .iter()
                                .map(|addr| EndpointAddress {
                                    ip: addr.ip.clone(),
                                    hostname: addr.hostname.clone(),
                                    node_name: addr.node_name.clone(),
                                    target_ref: addr.target_ref.as_ref().map(|tr| {
                                        format!(
                                            "{}/{}/{}",
                                            tr.kind.as_deref().unwrap_or("Unknown"),
                                            tr.namespace.as_deref().unwrap_or(""),
                                            tr.name.as_deref().unwrap_or("")
                                        )
                                    }),
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    let ports = subset
                        .ports
                        .as_ref()
                        .map(|port_list| {
                            port_list
                                .iter()
                                .map(|p| EndpointPort {
                                    name: p.name.clone(),
                                    port: p.port as u16,
                                    protocol: p.protocol.as_deref().unwrap_or("TCP").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    EndpointSubset {
                        ready_addresses,
                        not_ready_addresses,
                        ports,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let raw_json = serde_json::to_string_pretty(&ep).unwrap_or_default();

    EndpointDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        subsets,
        raw_json,
    }
}
