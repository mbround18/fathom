//! API discovery for the Role/ClusterRole rules editor (042-role-create).
//!
//! `RefreshApiResources` walks `/api` and `/apis` once and replies with every group's resource
//! names (plurals and subresources such as `pods/log`) at the group's preferred version, so the
//! editor can offer real API groups and resources, CRDs included. Read-only: the only calls are
//! discovery GETs.
//!
//! Unlike `kube::discovery::Discovery::run`, one broken group (an aggregated API whose backing
//! service is down, typically `metrics.k8s.io`) does not fail the whole catalog: it is named in
//! `failed_groups` and the rest is still returned.

use crossbeam_channel::Sender;
use futures::future::join_all;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::APIResourceList;

use crate::protocol::{
    ApiGroupResources, ApiResourceCatalog, ApiResourceName, BackendEvent, BackendEventEnvelope,
    Correlation, ResourceReadEvent, Route, section_keys,
};

use super::{events, state::BackendState};

/// Handle `RefreshApiResources`.
pub(super) async fn handle_refresh_api_resources(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let catalog = discover(&client).await?;
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(
            section_keys::access_roles(),
            state.cluster.current_context.clone(),
        ),
        Correlation::None,
        BackendEvent::ResourceRead(ResourceReadEvent::ApiResourcesUpdated(catalog)),
    );
    Ok(())
}

async fn discover(client: &kube::Client) -> anyhow::Result<ApiResourceCatalog> {
    // The group list itself must succeed; without it there is nothing to offer.
    let groups = client.list_api_groups().await?;
    let core_versions = client.list_core_api_versions().await?;

    let core = async {
        let version = core_versions
            .versions
            .first()
            .cloned()
            .unwrap_or_else(|| "v1".to_owned());
        (
            String::new(),
            client.list_core_api_resources(&version).await,
        )
    };
    let named = groups.groups.into_iter().filter_map(|g| {
        let group_version = g
            .preferred_version
            .as_ref()
            .map(|v| v.group_version.clone())
            .or_else(|| g.versions.first().map(|v| v.group_version.clone()))?;
        let client = client.clone();
        Some(async move {
            let list = client.list_api_group_resources(&group_version).await;
            (g.name, list)
        })
    });

    let (core, named) = futures::join!(core, join_all(named));
    let mut results = vec![core];
    results.extend(named);
    Ok(flatten(
        results
            .into_iter()
            .map(|(group, list)| (group, list.ok()))
            .collect(),
    ))
}

/// Pure: fold per-group discovery results into the catalog. `None` marks a failed group.
/// Groups and resource names are sorted so the pickers are stable.
fn flatten(results: Vec<(String, Option<APIResourceList>)>) -> ApiResourceCatalog {
    let mut groups = Vec::new();
    let mut failed_groups = Vec::new();
    for (group, list) in results {
        let Some(list) = list else {
            failed_groups.push(group);
            continue;
        };
        let mut resources: Vec<ApiResourceName> = list
            .resources
            .into_iter()
            .map(|r| ApiResourceName {
                name: r.name,
                namespaced: r.namespaced,
                verbs: r.verbs,
            })
            .collect();
        resources.sort_by(|a, b| a.name.cmp(&b.name));
        resources.dedup_by(|a, b| a.name == b.name);
        groups.push(ApiGroupResources { group, resources });
    }
    groups.sort_by(|a, b| a.group.cmp(&b.group));
    failed_groups.sort();
    ApiResourceCatalog {
        groups,
        failed_groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::APIResource;

    fn resource(name: &str, namespaced: bool) -> APIResource {
        APIResource {
            name: name.to_owned(),
            namespaced,
            verbs: vec!["get".to_owned(), "list".to_owned()],
            kind: "X".to_owned(),
            singular_name: String::new(),
            ..Default::default()
        }
    }

    fn list(group_version: &str, names: &[(&str, bool)]) -> APIResourceList {
        APIResourceList {
            group_version: group_version.to_owned(),
            resources: names.iter().map(|(n, ns)| resource(n, *ns)).collect(),
        }
    }

    #[test]
    fn flattens_sorts_and_keeps_subresources() {
        let catalog = flatten(vec![
            (
                "apps".to_owned(),
                Some(list(
                    "apps/v1",
                    &[("statefulsets", true), ("deployments", true)],
                )),
            ),
            (
                String::new(),
                Some(list(
                    "v1",
                    &[("pods/log", true), ("pods", true), ("nodes", false)],
                )),
            ),
        ]);
        assert_eq!(catalog.groups.len(), 2);
        assert_eq!(catalog.groups[0].group, "");
        let core: Vec<&str> = catalog.groups[0]
            .resources
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(core, ["nodes", "pods", "pods/log"]);
        assert!(!catalog.groups[0].resources[0].namespaced);
        assert_eq!(catalog.groups[1].resources[0].name, "deployments");
        assert!(catalog.failed_groups.is_empty());
    }

    #[test]
    fn a_failed_group_is_named_not_fatal() {
        let catalog = flatten(vec![
            ("metrics.k8s.io".to_owned(), None),
            (
                "rbac.authorization.k8s.io".to_owned(),
                Some(list("rbac.authorization.k8s.io/v1", &[("roles", true)])),
            ),
        ]);
        assert_eq!(catalog.failed_groups, ["metrics.k8s.io"]);
        assert_eq!(catalog.groups.len(), 1);
        assert_eq!(catalog.groups[0].resources[0].name, "roles");
    }
}
