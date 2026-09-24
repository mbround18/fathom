use std::collections::BTreeSet;
use std::time::Duration;

use k8s_openapi::api::core::v1::Node;
use kube::api::{Api, ListParams};

use crate::protocol::v1::ClusterVersionSummary;

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Fetch the server version and distinct node kubelet versions. Failures degrade to
/// `None` / empty (neutral "unknown" state) instead of erroring (spec FR-005).
pub(super) async fn fetch_cluster_version(
    client: &kube::Client,
) -> anyhow::Result<ClusterVersionSummary> {
    let server_version = match tokio::time::timeout(FETCH_TIMEOUT, client.apiserver_version()).await
    {
        Ok(Ok(info)) => Some(info.git_version),
        _ => None,
    };

    let nodes = match tokio::time::timeout(
        FETCH_TIMEOUT,
        Api::<Node>::all(client.clone()).list(&ListParams::default()),
    )
    .await
    {
        Ok(Ok(list)) => list.items,
        _ => Vec::new(),
    };
    let versions: BTreeSet<String> = nodes
        .iter()
        .filter_map(|n| n.status.as_ref()?.node_info.as_ref())
        .map(|i| i.kubelet_version.clone())
        .collect();
    let node_kubelet_versions: Vec<String> = versions.into_iter().collect();

    let skew_detected = server_version
        .as_ref()
        .is_some_and(|s| node_kubelet_versions.iter().any(|v| v != s));

    Ok(ClusterVersionSummary {
        server_version,
        node_kubelet_versions,
        skew_detected,
    })
}
