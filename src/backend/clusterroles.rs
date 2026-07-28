use crate::protocol::{
    BackendEvent, BackendEventEnvelope, ClusterRoleDetails, ClusterRoleSummary, Correlation,
    PolicyRuleSummary, ResourceListEvent, ResourceReadEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use k8s_openapi::api::rbac::v1::ClusterRole;
use kube::{
    ResourceExt,
    api::{Api, DeleteParams, ListParams},
};

use super::interactions;
use super::{events, state::BackendState};

pub(super) async fn refresh_cluster_roles(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ClusterRole> = Api::all(client);

    let roles = api.list(&ListParams::default()).await?;
    let summaries = cluster_roles_to_summaries(roles.items);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::access_clusterroles(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::ClusterRolesUpdated(summaries)),
    );
    Ok(())
}

fn cluster_roles_to_summaries(roles: Vec<ClusterRole>) -> Vec<ClusterRoleSummary> {
    let mut out: Vec<ClusterRoleSummary> = Vec::new();

    for role in roles {
        let name = role.name_any();
        let rules_count = role.rules.as_ref().map(|r| r.len()).unwrap_or(0);

        out.push(ClusterRoleSummary { name, rules_count });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub(super) async fn fetch_cluster_role_details(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ClusterRole> = Api::all(client);
    let role = api.get(name).await?;

    let details = cluster_role_to_details(role);
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::access_clusterroles(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ClusterRoleDetailsUpdated(details)),
    );
    Ok(())
}

fn cluster_role_to_details(role: ClusterRole) -> ClusterRoleDetails {
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

    ClusterRoleDetails {
        name,
        creation_timestamp,
        labels,
        annotations,
        rules,
        raw_json,
    }
}

pub(super) async fn delete_cluster_role(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<ClusterRole> = Api::all(client);

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

    refresh_cluster_roles(state, sender).await?;
    Ok(())
}
