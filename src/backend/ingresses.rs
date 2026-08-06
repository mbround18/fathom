use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, IngressBackend, IngressDetails,
    IngressLoadBalancerIngress, IngressLoadBalancerPort, IngressPath, IngressRule, IngressSummary,
    ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::networking::v1::Ingress;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_ingresses(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Ingress> = Api::all(client);

    let ingresses = api.list(&ListParams::default()).await?;
    let summaries = ingresses_to_summaries(ingresses.items, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::network_ingress(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::IngressesUpdated(summaries)),
    );
    Ok(())
}

fn ingresses_to_summaries(ingresses: Vec<Ingress>, filter: &Vec<String>) -> Vec<IngressSummary> {
    let mut out: Vec<IngressSummary> = Vec::new();

    for ing in ingresses {
        let namespace = ing.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = ing.name_any();
        let ingress_class = ing.spec.as_ref().and_then(|s| s.ingress_class_name.clone());

        let hosts_count = ing
            .spec
            .as_ref()
            .and_then(|s| s.rules.as_ref())
            .map(|r| r.iter().filter_map(|rule| rule.host.as_ref()).count())
            .unwrap_or(0);

        let load_balancer_ips = ing
            .status
            .as_ref()
            .and_then(|status| status.load_balancer.as_ref())
            .and_then(|lb| lb.ingress.as_ref())
            .map(|ingress| {
                ingress
                    .iter()
                    .filter_map(|ing| ing.ip.as_ref().map(|i| i.clone()))
                    .collect()
            })
            .unwrap_or_default();

        out.push(IngressSummary {
            name,
            namespace,
            ingress_class,
            hosts_count,
            load_balancer_ips,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_ingress_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Ingress> = Api::namespaced(client, namespace);
    let ing = api.get(name).await?;

    let details = ingress_to_details(ing);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::network_ingress(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::IngressDetailsUpdated(details)),
    );
    Ok(())
}

fn ingress_to_details(ing: Ingress) -> IngressDetails {
    let namespace = ing.namespace().unwrap_or_default();
    let name = ing.name_any();
    let creation_timestamp = ing
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = ing.labels().clone();
    let annotations = ing.annotations().clone();

    let spec = ing.spec.as_ref();
    let ingress_class = spec.and_then(|s| s.ingress_class_name.clone());

    let rules = spec
        .and_then(|s| s.rules.as_ref())
        .map(|rule_list| {
            rule_list
                .iter()
                .map(|rule| {
                    let paths = rule
                        .http
                        .as_ref()
                        .map(|http| {
                            http.paths
                                .iter()
                                .map(|path| {
                                    let backend = IngressBackend {
                                        service_name: path
                                            .backend
                                            .service
                                            .as_ref()
                                            .map(|s| s.name.clone()),
                                        service_port: path
                                            .backend
                                            .service
                                            .as_ref()
                                            .and_then(|s| s.port.as_ref())
                                            .and_then(|p| p.number.map(|n| n as u16)),
                                        resource_kind: path
                                            .backend
                                            .resource
                                            .as_ref()
                                            .map(|r| r.kind.clone()),
                                        resource_name: path
                                            .backend
                                            .resource
                                            .as_ref()
                                            .map(|r| r.name.clone()),
                                    };

                                    IngressPath {
                                        path: path.path.clone(),
                                        path_type: path.path_type.clone(),
                                        backend,
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    IngressRule {
                        host: rule.host.clone(),
                        paths,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let default_backend = spec
        .and_then(|s| s.default_backend.as_ref())
        .map(|backend| IngressBackend {
            service_name: backend.service.as_ref().map(|s| s.name.clone()),
            service_port: backend
                .service
                .as_ref()
                .and_then(|s| s.port.as_ref())
                .and_then(|p| p.number.map(|n| n as u16)),
            resource_kind: backend.resource.as_ref().map(|r| r.kind.clone()),
            resource_name: backend.resource.as_ref().map(|r| r.name.clone()),
        });

    let load_balancer_ingress = ing
        .status
        .as_ref()
        .and_then(|status| status.load_balancer.as_ref())
        .and_then(|lb| lb.ingress.as_ref())
        .map(|ingress| {
            ingress
                .iter()
                .map(|ing| IngressLoadBalancerIngress {
                    hostname: ing.hostname.clone(),
                    ip: ing.ip.clone(),
                    ports: ing
                        .ports
                        .as_ref()
                        .map(|ports| {
                            ports
                                .iter()
                                .map(|p| IngressLoadBalancerPort {
                                    port: p.port as u16,
                                    protocol: Some(p.protocol.clone()),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();

    let tls_hosts = spec
        .and_then(|s| s.tls.as_ref())
        .map(|tls_list| {
            tls_list
                .iter()
                .flat_map(|tls| tls.hosts.as_ref().unwrap_or(&vec![]).clone())
                .collect()
        })
        .unwrap_or_default();

    let raw_json = serde_json::to_string_pretty(&ing).unwrap_or_default();

    IngressDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        ingress_class,
        rules,
        default_backend,
        load_balancer_ingress,
        tls_hosts,
        raw_json,
    }
}

pub(super) async fn delete_ingress(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Ingress> = Api::namespaced(client, namespace);

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

    refresh_ingresses(state, sender).await?;
    Ok(())
}
