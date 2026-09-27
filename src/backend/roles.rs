use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, PolicyRuleSummary, ResourceListEvent,
    ResourceReadEvent, RoleDetails, RoleSummary, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::rbac::v1::Role;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_roles(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let roles = state.cluster.list_namespaced::<Role>().await?;
    let summaries = roles_to_summaries(roles, &state.cluster.namespace_filter);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::access_roles(),
            state.cluster.current_context.clone(),
            None,
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::RolesUpdated(summaries)),
    );
    Ok(())
}

fn roles_to_summaries(roles: Vec<Role>, filter: &[String]) -> Vec<RoleSummary> {
    let mut out: Vec<RoleSummary> = Vec::new();

    for role in roles {
        let namespace = role.namespace().unwrap_or_default();
        if !filter.is_empty() && !filter.iter().any(|ns| ns == &namespace) {
            continue;
        }

        let name = role.name_any();
        let rules_count = role.rules.as_ref().map(|r| r.len()).unwrap_or(0);

        out.push(RoleSummary {
            name,
            namespace,
            rules_count,
        });
    }

    out.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    out
}

pub(super) async fn fetch_role_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Role> = Api::namespaced(client, namespace);
    let role = api.get(name).await?;

    let details = role_to_details(role);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::namespaced(
            section_keys::access_roles(),
            state.cluster.current_context.clone(),
            Some(namespace.to_string()),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::RoleDetailsUpdated(details)),
    );
    Ok(())
}

fn role_to_details(role: Role) -> RoleDetails {
    let namespace = role.namespace().unwrap_or_default();
    let name = role.name_any();
    let creation_timestamp = role
        .creation_timestamp()
        .map(|ts| ts.0.to_rfc3339())
        .unwrap_or_default();

    let labels = role.labels().clone();
    let annotations = role.annotations().clone();

    let rules = role
        .rules
        .as_ref()
        .map(|rules| {
            rules
                .iter()
                .map(|rule| PolicyRuleSummary {
                    api_groups: rule.api_groups.clone().unwrap_or_default(),
                    resources: rule.resources.clone().unwrap_or_default(),
                    verbs: rule.verbs.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    let raw_json = serde_json::to_string_pretty(&role).unwrap_or_default();

    RoleDetails {
        name,
        namespace,
        creation_timestamp,
        labels,
        annotations,
        rules,
        raw_json,
    }
}

pub(super) async fn delete_role(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Role> = Api::namespaced(client, namespace);

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

    refresh_roles(state, sender).await?;
    Ok(())
}
