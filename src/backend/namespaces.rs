//! Namespace read path: `SelectNamespace` -> `NamespaceDetailsUpdated`.

use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::{LimitRange, Namespace, ResourceQuota};
use kube::ResourceExt;
use kube::api::{Api, ListParams};

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, NamespaceDetails, ResourceReadEvent, Route,
    section_keys,
};

use super::events;
use super::state::BackendState;

/// Build the detail payload from a Namespace plus the names of its quotas and limit ranges.
pub(super) fn namespace_details(
    ns: &Namespace,
    mut resource_quota_names: Vec<String>,
    mut limit_range_names: Vec<String>,
) -> NamespaceDetails {
    resource_quota_names.sort();
    limit_range_names.sort();
    NamespaceDetails {
        name: ns.name_any(),
        status: ns
            .status
            .as_ref()
            .and_then(|s| s.phase.clone())
            .unwrap_or_else(|| "Unknown".to_owned()),
        labels: ns.metadata.labels.clone().unwrap_or_default(),
        annotations: ns.metadata.annotations.clone().unwrap_or_default(),
        creation_timestamp: ns
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|t| t.0.to_rfc3339())
            .unwrap_or_default(),
        resource_quota_names,
        limit_range_names,
        raw_json: raw_json(ns),
    }
}

/// Pretty JSON of the namespace with `managedFields` dropped (noise; no secret data lives here).
fn raw_json(ns: &Namespace) -> String {
    let mut clean = ns.clone();
    clean.metadata.managed_fields = None;
    serde_json::to_string_pretty(&clean).unwrap_or_default()
}

pub(super) async fn select_namespace(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    name: &str,
) -> anyhow::Result<()> {
    let Some(client) = &state.cluster.client else {
        return Ok(());
    };
    let context = state.cluster.current_context.clone();
    let api: Api<Namespace> = Api::all(client.clone());
    match api.get(name).await {
        Ok(ns) => {
            // Best-effort: a denied list simply yields no names (RBAC is never escalated).
            let lp = ListParams::default();
            let quotas: Api<ResourceQuota> = Api::namespaced(client.clone(), name);
            let ranges: Api<LimitRange> = Api::namespaced(client.clone(), name);
            let (q, r) = tokio::join!(quotas.list(&lp), ranges.list(&lp));
            let q = q
                .map(|l| l.items.iter().map(ResourceExt::name_any).collect())
                .unwrap_or_default();
            let r = r
                .map(|l| l.items.iter().map(ResourceExt::name_any).collect())
                .unwrap_or_default();
            let details = namespace_details(&ns, q, r);
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                Route::cluster(section_keys::namespaces(), context),
                Correlation::None,
                BackendEvent::ResourceRead(ResourceReadEvent::NamespaceDetailsUpdated(details)),
            );
        }
        Err(e) => {
            tracing::warn!("Failed to get namespace details: {}", e);
            events::notify_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Failed to get namespace details: {e}"),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::NamespaceStatus;
    use kube::api::ObjectMeta;
    use std::collections::BTreeMap;

    #[test]
    fn details_carry_labels_annotations_phase() {
        let ns = Namespace {
            metadata: ObjectMeta {
                name: Some("team-a".into()),
                labels: Some(BTreeMap::from([("env".into(), "dev".into())])),
                annotations: Some(BTreeMap::from([("owner".into(), "a".into())])),
                ..Default::default()
            },
            status: Some(NamespaceStatus {
                phase: Some("Active".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let d = namespace_details(&ns, vec!["b".into(), "a".into()], vec![]);
        assert_eq!(d.name, "team-a");
        assert_eq!(d.status, "Active");
        assert_eq!(d.labels["env"], "dev");
        assert_eq!(d.annotations["owner"], "a");
        assert_eq!(d.resource_quota_names, vec!["a", "b"]);
        assert!(d.raw_json.contains("team-a"));
    }
}
