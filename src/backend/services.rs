use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, ResourceReadEvent, Route,
    ServiceDetails, ServicePort, ServiceSummary, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::Service;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};

use super::interactions;
use super::state::client_or_err;
use super::{events, state::BackendState};

pub(super) async fn refresh_services(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<Service> = Api::all(client);

    let services = api.list(&ListParams::default()).await?;
    let summaries = services_to_summaries(services.items, &state.namespace_filter);
    events::emit(
        state.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::network_services(),
            state.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::ServicesUpdated(summaries)),
    );
    Ok(())
}

fn services_to_summaries(services: Vec<Service>, filter: &Vec<String>) -> Vec<ServiceSummary> {
    let mut out: Vec<ServiceSummary> = Vec::new();

    for svc in services {
        let namespace = svc.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = svc.name_any();
        let service_type = svc
            .spec
            .as_ref()
            .and_then(|s| s.type_.as_deref())
            .unwrap_or("ClusterIP")
            .to_string();

        let cluster_ip = svc
            .spec
            .as_ref()
            .and_then(|s| s.cluster_ip.as_deref())
            .unwrap_or("")
            .to_string();

        let external_ips = svc
            .spec
            .as_ref()
            .and_then(|s| s.external_ips.as_ref())
            .map(|v| v.clone())
            .unwrap_or_default();

        let port_count = svc
            .spec
            .as_ref()
            .and_then(|s| s.ports.as_ref())
            .map(|p| p.len())
            .unwrap_or(0);

        out.push(ServiceSummary {
            name,
            namespace,
            service_type,
            cluster_ip,
            external_ips,
            port_count,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_service_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<Service> = Api::namespaced(client, namespace);
    let svc = api.get(name).await?;

    let details = service_to_details(svc);
    events::emit(
        state.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::network_services(),
            state.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ServiceDetailsUpdated(details)),
    );
    Ok(())
}

fn service_to_details(svc: Service) -> ServiceDetails {
    let namespace = svc.namespace().unwrap_or_default();
    let name = svc.name_any();
    let creation_timestamp = svc
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = svc.labels().clone();
    let annotations = svc.annotations().clone();

    let spec = svc.spec.as_ref();
    let service_type = spec
        .and_then(|s| s.type_.as_deref())
        .unwrap_or("ClusterIP")
        .to_string();
    let cluster_ip = spec
        .and_then(|s| s.cluster_ip.as_deref())
        .unwrap_or("")
        .to_string();
    let cluster_ips = spec
        .and_then(|s| s.cluster_ips.as_ref())
        .map(|v| v.clone())
        .unwrap_or_default();
    let external_ips = spec
        .and_then(|s| s.external_ips.as_ref())
        .map(|v| v.clone())
        .unwrap_or_default();
    let external_name = spec
        .and_then(|s| s.external_name.as_deref())
        .map(|s| s.to_string());
    let selector = spec
        .and_then(|s| s.selector.as_ref())
        .map(|m| m.clone())
        .unwrap_or_default();

    let ports = spec
        .and_then(|s| s.ports.as_ref())
        .map(|port_list| {
            port_list
                .iter()
                .map(|p| ServicePort {
                    name: p.name.clone(),
                    port: p.port as u16,
                    target_port: p.target_port.as_ref().map(|tp| match tp {
                        k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(n) => {
                            n.to_string()
                        }
                        k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::String(s) => {
                            s.clone()
                        }
                    }),
                    protocol: p.protocol.as_deref().unwrap_or("TCP").to_string(),
                    node_port: p.node_port.map(|np| np as u16),
                })
                .collect()
        })
        .unwrap_or_default();

    let session_affinity = spec
        .and_then(|s| s.session_affinity.as_deref())
        .unwrap_or("None")
        .to_string();

    let load_balancer_ip = spec
        .and_then(|s| s.load_balancer_ip.as_deref())
        .map(|s| s.to_string());
    let load_balancer_ingress_hosts = svc
        .status
        .as_ref()
        .and_then(|status| status.load_balancer.as_ref())
        .and_then(|lb| lb.ingress.as_ref())
        .map(|ingress| {
            ingress
                .iter()
                .filter_map(|ing| ing.hostname.as_ref().map(|h| h.clone()))
                .collect()
        })
        .unwrap_or_default();

    let raw_json = serde_json::to_string_pretty(&svc).unwrap_or_default();

    ServiceDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        service_type,
        cluster_ip,
        cluster_ips,
        external_ips,
        external_name,
        selector,
        ports,
        session_affinity,
        load_balancer_ip,
        load_balancer_ingress_hosts,
        raw_json,
    }
}

pub(super) async fn delete_service(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;
    let api: Api<Service> = Api::namespaced(client, namespace);

    let ts = k8s_openapi::chrono::Utc::now().to_rfc3339();
    let _ = interactions::annotate_action(
        &api,
        name,
        state.current_context.as_deref(),
        "delete",
        &ts,
        serde_json::json!({}),
    )
    .await;

    api.delete(name, &DeleteParams::default()).await?;

    refresh_services(state, sender).await?;
    Ok(())
}
