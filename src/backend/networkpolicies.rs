use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, NetworkPolicyDetails, NetworkPolicySummary,
    PolicyRule, ResourceListEvent, ResourceReadEvent, Route, RuleSelector, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_network_policies(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<NetworkPolicy> = Api::all(client);

    let policies = api.list(&ListParams::default()).await?;
    let summaries = network_policies_to_summaries(policies.items, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::network_policies(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::NetworkPoliciesUpdated(summaries)),
    );
    Ok(())
}

fn network_policies_to_summaries(
    policies: Vec<NetworkPolicy>,
    filter: &Vec<String>,
) -> Vec<NetworkPolicySummary> {
    let mut out: Vec<NetworkPolicySummary> = Vec::new();

    for policy in policies {
        let namespace = policy.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = policy.name_any();

        let pod_selector = policy
            .spec
            .as_ref()
            .and_then(|s| {
                s.pod_selector
                    .as_ref()
                    .and_then(|ps| ps.match_labels.as_ref())
            })
            .map(|labels| {
                labels
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "{}".to_string());

        let ingress_rules_count = policy
            .spec
            .as_ref()
            .and_then(|s| s.ingress.as_ref())
            .map(|r| r.len())
            .unwrap_or(0);

        let egress_rules_count = policy
            .spec
            .as_ref()
            .and_then(|s| s.egress.as_ref())
            .map(|r| r.len())
            .unwrap_or(0);

        out.push(NetworkPolicySummary {
            name,
            namespace,
            pod_selector,
            ingress_rules_count,
            egress_rules_count,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_network_policy_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<NetworkPolicy> = Api::namespaced(client, namespace);
    let policy = api.get(name).await?;

    let details = network_policy_to_details(policy);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::network_policies(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::NetworkPolicyDetailsUpdated(details)),
    );
    Ok(())
}

fn network_policy_to_details(policy: NetworkPolicy) -> NetworkPolicyDetails {
    let namespace = policy.namespace().unwrap_or_default();
    let name = policy.name_any();
    let creation_timestamp = policy
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = policy.labels().clone();
    let annotations = policy.annotations().clone();

    let spec = policy.spec.as_ref();
    let pod_selector = spec
        .and_then(|s| {
            s.pod_selector
                .as_ref()
                .and_then(|ps| ps.match_labels.as_ref().cloned())
        })
        .unwrap_or_default();

    let policy_types = spec
        .and_then(|s| s.policy_types.as_ref())
        .map(|types| types.clone())
        .unwrap_or_default();

    let ingress_rules = spec
        .and_then(|s| s.ingress.as_ref())
        .map(|rules| {
            rules
                .iter()
                .map(|rule| {
                    let protocols = rule
                        .ports
                        .as_ref()
                        .and_then(|ports| {
                            let protos: Vec<String> = ports
                                .iter()
                                .filter_map(|p| p.protocol.as_ref().map(|proto| proto.clone()))
                                .collect();
                            if protos.is_empty() {
                                None
                            } else {
                                Some(protos)
                            }
                        })
                        .unwrap_or_else(|| vec!["TCP".to_string()]);

                    let ports = rule
                        .ports
                        .as_ref()
                        .map(|port_list| {
                            port_list
                                .iter()
                                .filter_map(|p| p.port.as_ref())
                                .filter_map(|port_obj| match port_obj {
                                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(n) => Some(*n as u16),
                                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::String(_) => None,
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_else(|| vec![]);

                    let from_selectors = rule
                        .from
                        .as_ref()
                        .map(|froms| {
                            froms
                                .iter()
                                .map(|from| {
                                    if let Some(pod_sel) = from.pod_selector.as_ref() {
                                        RuleSelector {
                                            selector_type: "PodSelector".to_string(),
                                            labels: pod_sel.match_labels.as_ref().cloned(),
                                            ip_block: None,
                                            except: vec![],
                                        }
                                    } else if let Some(ns_sel) = from.namespace_selector.as_ref() {
                                        RuleSelector {
                                            selector_type: "NamespaceSelector".to_string(),
                                            labels: ns_sel.match_labels.as_ref().cloned(),
                                            ip_block: None,
                                            except: vec![],
                                        }
                                    } else if let Some(ip_block) = from.ip_block.as_ref() {
                                        RuleSelector {
                                            selector_type: "IPBlock".to_string(),
                                            labels: None,
                                            ip_block: Some(ip_block.cidr.clone()),
                                            except: ip_block.except.as_ref().unwrap_or(&vec![]).clone(),
                                        }
                                    } else {
                                        RuleSelector {
                                            selector_type: "Unknown".to_string(),
                                            labels: None,
                                            ip_block: None,
                                            except: vec![],
                                        }
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    PolicyRule {
                        protocols,
                        ports,
                        from_selectors,
                        to_selectors: vec![],
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let egress_rules = spec
        .and_then(|s| s.egress.as_ref())
        .map(|rules| {
            rules
                .iter()
                .map(|rule| {
                    let protocols = rule
                        .ports
                        .as_ref()
                        .and_then(|ports| {
                            let protos: Vec<String> = ports
                                .iter()
                                .filter_map(|p| p.protocol.as_ref().map(|proto| proto.clone()))
                                .collect();
                            if protos.is_empty() {
                                None
                            } else {
                                Some(protos)
                            }
                        })
                        .unwrap_or_else(|| vec!["TCP".to_string()]);

                    let ports = rule
                        .ports
                        .as_ref()
                        .map(|port_list| {
                            port_list
                                .iter()
                                .filter_map(|p| p.port.as_ref())
                                .filter_map(|port_obj| match port_obj {
                                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(n) => Some(*n as u16),
                                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::String(_) => None,
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_else(|| vec![]);

                    let to_selectors = rule
                        .to
                        .as_ref()
                        .map(|tos| {
                            tos
                                .iter()
                                .map(|to| {
                                    if let Some(pod_sel) = to.pod_selector.as_ref() {
                                        RuleSelector {
                                            selector_type: "PodSelector".to_string(),
                                            labels: pod_sel.match_labels.as_ref().cloned(),
                                            ip_block: None,
                                            except: vec![],
                                        }
                                    } else if let Some(ns_sel) = to.namespace_selector.as_ref() {
                                        RuleSelector {
                                            selector_type: "NamespaceSelector".to_string(),
                                            labels: ns_sel.match_labels.as_ref().cloned(),
                                            ip_block: None,
                                            except: vec![],
                                        }
                                    } else if let Some(ip_block) = to.ip_block.as_ref() {
                                        RuleSelector {
                                            selector_type: "IPBlock".to_string(),
                                            labels: None,
                                            ip_block: Some(ip_block.cidr.clone()),
                                            except: ip_block.except.as_ref().unwrap_or(&vec![]).clone(),
                                        }
                                    } else {
                                        RuleSelector {
                                            selector_type: "Unknown".to_string(),
                                            labels: None,
                                            ip_block: None,
                                            except: vec![],
                                        }
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    PolicyRule {
                        protocols,
                        ports,
                        from_selectors: vec![],
                        to_selectors,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let raw_json = serde_json::to_string_pretty(&policy).unwrap_or_default();

    NetworkPolicyDetails {
        namespace,
        name,
        creation_timestamp,
        labels,
        annotations,
        pod_selector,
        policy_types,
        ingress_rules,
        egress_rules,
        raw_json,
    }
}

pub(super) async fn delete_network_policy(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<NetworkPolicy> = Api::namespaced(client, namespace);

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

    refresh_network_policies(state, sender).await?;
    Ok(())
}
