use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, NamespaceSummary, ResourceListEvent, Route,
    section_keys,
};
use crossbeam_channel::Sender;
use futures_util::StreamExt;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::{
    api::{Api, ListParams},
    runtime::watcher,
};
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use super::state::BackendState;
use super::{events, pods};

pub(super) async fn ensure_background_sync(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;

    // If tasks already exist, keep them running.
    if state.watch_tasks.pod_watch_task.is_some() && state.watch_tasks.namespace_sync_task.is_some()
    {
        return Ok(());
    }

    // Restart cleanly if partially running.
    state.watch_tasks.abort_all();

    // Pod watcher: watch for pod changes with exponential backoff retry
    state.watch_tasks.pod_watch_task = Some(tokio::spawn(watch_pods(
        client.clone(),
        state.cluster.namespace_filter_shared.clone(),
        state.cluster.seq.clone(),
        sender.clone(),
        state.cluster.current_context.clone(),
    )));

    // Namespace refresher: refresh namespaces periodically with improved reliability
    state.watch_tasks.namespace_sync_task = Some(tokio::spawn(sync_namespaces_periodically(
        client.clone(),
        state.cluster.seq.clone(),
        sender.clone(),
        state.cluster.current_context.clone(),
    )));

    Ok(())
}

/// Background task: watch pods cluster-wide and re-emit the filtered pod list on
/// every change, reconnecting with exponential backoff (1s doubling to 30s).
async fn watch_pods(
    client: kube::Client,
    filter_shared: Arc<RwLock<Vec<String>>>,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    events::notify_info(
        seq.as_ref(),
        &sender,
        "Background sync: watching pods...".to_owned(),
    );

    let mut retry_delay = Duration::from_secs(1);
    loop {
        let api: Api<Pod> = Api::all(client.clone());
        let w = watcher(api, watcher::Config::default());
        futures_util::pin_mut!(w);

        while let Some(ev) = w.next().await {
            match ev {
                Ok(_event) => {
                    // Reset retry delay on successful event
                    retry_delay = Duration::from_secs(1);

                    // On any event, refresh pods list
                    emit_pods_snapshot(&client, &filter_shared, &seq, &sender, &context).await;
                }
                Err(e) => {
                    events::notify_error(
                        seq.as_ref(),
                        &sender,
                        format!("Pod watcher error (retrying in {:?}): {}", retry_delay, e),
                    );
                    break;
                }
            }
        }

        tokio::time::sleep(retry_delay).await;
        retry_delay = std::cmp::min(retry_delay * 2, Duration::from_secs(30));
    }
}

/// List all pods and emit them (filtered by the shared namespace filter) as `PodsUpdated`.
async fn emit_pods_snapshot(
    client: &kube::Client,
    filter_shared: &RwLock<Vec<String>>,
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    context: &Option<String>,
) {
    let api: Api<Pod> = Api::all(client.clone());
    match api.list(&ListParams::default()).await {
        Ok(pods_list) => {
            let filter = filter_shared.read().map(|f| f.clone()).unwrap_or_default();
            let summaries = pods::pods_to_summaries(pods_list.items, &filter);
            events::emit(
                seq,
                sender,
                Route::namespaced(section_keys::workloads_pods(), context.clone(), None),
                Correlation::None,
                BackendEvent::ResourceList(ResourceListEvent::PodsUpdated(summaries)),
            );
        }
        Err(e) => {
            events::notify_error(seq, sender, format!("Background pod refresh failed: {}", e));
        }
    }
}

/// Background task: re-list namespaces every 30 seconds and emit `NamespacesUpdated`.
async fn sync_namespaces_periodically(
    client: kube::Client,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(30));
    loop {
        ticker.tick().await;
        let api: Api<Namespace> = Api::all(client.clone());
        match api.list(&ListParams::default()).await {
            Ok(ns_list) => {
                let mut namespaces: Vec<String> = ns_list
                    .items
                    .into_iter()
                    .filter_map(|n| n.metadata.name)
                    .collect();
                namespaces.sort();
                emit_namespace_names(seq.as_ref(), &sender, context.clone(), namespaces);
            }
            Err(e) => {
                events::notify_error(
                    seq.as_ref(),
                    &sender,
                    format!("Background namespace refresh failed: {}", e),
                );
            }
        }
    }
}

pub(super) async fn refresh_namespaces(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    // Capture the context at request start so a late result keeps its origin (FR-016).
    let context = state.cluster.current_context.clone();
    let ns_api: Api<Namespace> = Api::all(client.clone());
    let pod_api: Api<Pod> = Api::all(client.clone());
    let deployment_api: Api<Deployment> = Api::all(client);

    let lp = ListParams::default();
    let (ns_result, pods_result, deployments_result) = tokio::join!(
        ns_api.list(&lp),
        pod_api.list(&lp),
        deployment_api.list(&lp)
    );
    let ns_list = ns_result?;

    let mut pod_counts: HashMap<String, u32> = HashMap::new();
    if let Ok(pods) = pods_result {
        for pod in pods.items {
            if let Some(ns) = pod.metadata.namespace {
                *pod_counts.entry(ns).or_default() += 1;
            }
        }
    }

    let mut deployment_counts: HashMap<String, u32> = HashMap::new();
    if let Ok(deployments) = deployments_result {
        for deployment in deployments.items {
            if let Some(ns) = deployment.metadata.namespace {
                *deployment_counts.entry(ns).or_default() += 1;
            }
        }
    }

    let mut summaries: Vec<NamespaceSummary> = ns_list
        .items
        .into_iter()
        .filter_map(|ns| {
            let name = ns.metadata.name?;
            let status = ns
                .status
                .and_then(|s| s.phase)
                .unwrap_or_else(|| "Unknown".to_owned());
            let creation_timestamp = ns
                .metadata
                .creation_timestamp
                .map(|t| t.0.to_rfc3339())
                .unwrap_or_default();
            Some(NamespaceSummary {
                pod_count: pod_counts.get(&name).copied().unwrap_or(0),
                deployment_count: deployment_counts.get(&name).copied().unwrap_or(0),
                name,
                status,
                creation_timestamp,
            })
        })
        .collect();
    summaries.sort_by(|a, b| a.name.cmp(&b.name));

    let mut namespaces: Vec<String> = summaries.iter().map(|s| s.name.clone()).collect();
    namespaces.sort();

    let seq = state.cluster.seq.as_ref();
    emit_namespace_names(seq, sender, context.clone(), namespaces);
    events::emit(
        seq,
        sender,
        namespaces_route(context),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::NamespaceSummariesUpdated(summaries)),
    );
    Ok(())
}

/// Route for cluster-wide namespace data fetched from `context`.
fn namespaces_route(context: Option<String>) -> Route {
    Route::cluster(section_keys::system(), context)
}

/// Emit `NamespacesUpdated` tagged with the context the namespaces were listed from.
fn emit_namespace_names(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    context: Option<String>,
    namespaces: Vec<String>,
) {
    events::emit(
        seq,
        sender,
        namespaces_route(context),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::NamespacesUpdated(namespaces)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ScopeKind;

    #[test]
    fn namespaces_list_carries_originating_context() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let seq = AtomicU64::new(0);
        emit_namespace_names(
            &seq,
            &tx,
            Some("kind-a".to_owned()),
            vec!["default".to_owned()],
        );

        let env = rx.try_recv().expect("a NamespacesUpdated envelope");
        assert_eq!(env.route.context.as_deref(), Some("kind-a"));
        assert_eq!(env.route.scope, ScopeKind::Cluster);
        assert!(matches!(
            env.event,
            BackendEvent::ResourceList(ResourceListEvent::NamespacesUpdated(ref ns)) if ns == &["default"]
        ));
    }

    #[test]
    fn namespaces_route_carries_originating_context() {
        let route = namespaces_route(Some("kind-b".to_owned()));
        assert_eq!(route.context.as_deref(), Some("kind-b"));
        assert_eq!(route.scope, ScopeKind::Cluster);
    }
}
