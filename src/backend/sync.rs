use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, ResourceListEvent, Route, section_keys,
};
use crossbeam_channel::Sender;
use futures_util::StreamExt;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::{
    api::{Api, ListParams},
    runtime::watcher,
};
use std::time::Duration;

use super::state::BackendState;
use super::{events, pods};

pub(super) async fn ensure_background_sync(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;

    // If tasks already exist, keep them running.
    if state.watch_tasks.pod_watch_task.is_some() && state.watch_tasks.namespace_sync_task.is_some() {
        return Ok(());
    }

    // Restart cleanly if partially running.
    state.watch_tasks.abort_all();

    let sender_for_watch = sender.clone();
    let sender_for_ns = sender.clone();
    let filter_shared = state.cluster.namespace_filter_shared.clone();
    let seq_for_watch = state.cluster.seq.clone();
    let seq_for_ns = state.cluster.seq.clone();
    let context_for_watch = state.cluster.current_context.clone();
    let context_for_ns = state.cluster.current_context.clone();

    // Pod watcher: watch for pod changes with exponential backoff retry
    let client_for_watch = client.clone();
    state.watch_tasks.pod_watch_task = Some(tokio::spawn(async move {
        events::notify_info(
            seq_for_watch.as_ref(),
            &sender_for_watch,
            "Background sync: watching pods...".to_owned(),
        );

        let mut retry_delay = Duration::from_secs(1);
        loop {
            let api: Api<Pod> = Api::all(client_for_watch.clone());
            let w = watcher(api, watcher::Config::default());
            futures_util::pin_mut!(w);

            while let Some(ev) = w.next().await {
                match ev {
                    Ok(_event) => {
                        // Reset retry delay on successful event
                        retry_delay = Duration::from_secs(1);

                        // On any event, refresh pods list
                        let api: Api<Pod> = Api::all(client_for_watch.clone());
                        match api.list(&ListParams::default()).await {
                            Ok(pods_list) => {
                                let filter =
                                    filter_shared.read().map(|f| f.clone()).unwrap_or_default();
                                let summaries = pods::pods_to_summaries(pods_list.items, &filter);
                                events::emit(
                                    seq_for_watch.as_ref(),
                                    &sender_for_watch,
                                    Route::namespaced(
                                        section_keys::workloads_pods(),
                                        context_for_watch.clone(),
                                        None,
                                    ),
                                    Correlation::None,
                                    BackendEvent::ResourceList(ResourceListEvent::PodsUpdated(
                                        summaries,
                                    )),
                                );
                            }
                            Err(e) => {
                                events::notify_error(
                                    seq_for_watch.as_ref(),
                                    &sender_for_watch,
                                    format!("Background pod refresh failed: {}", e),
                                );
                            }
                        }
                    }
                    Err(e) => {
                        events::notify_error(
                            seq_for_watch.as_ref(),
                            &sender_for_watch,
                            format!("Pod watcher error (retrying in {:?}): {}", retry_delay, e),
                        );
                        break;
                    }
                }
            }

            tokio::time::sleep(retry_delay).await;
            retry_delay = std::cmp::min(retry_delay * 2, Duration::from_secs(30));
        }
    }));

    // Namespace refresher: refresh namespaces periodically with improved reliability
    let client_for_ns = client.clone();
    state.watch_tasks.namespace_sync_task = Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(30));
        loop {
            ticker.tick().await;
            let api: Api<Namespace> = Api::all(client_for_ns.clone());
            match api.list(&ListParams::default()).await {
                Ok(ns_list) => {
                    let mut namespaces: Vec<String> = ns_list
                        .items
                        .into_iter()
                        .filter_map(|n| n.metadata.name)
                        .collect();
                    namespaces.sort();
                    events::emit(
                        seq_for_ns.as_ref(),
                        &sender_for_ns,
                        Route::cluster(section_keys::system(), context_for_ns.clone()),
                        Correlation::None,
                        BackendEvent::ResourceList(ResourceListEvent::NamespacesUpdated(
                            namespaces,
                        )),
                    );
                }
                Err(e) => {
                    events::notify_error(
                        seq_for_ns.as_ref(),
                        &sender_for_ns,
                        format!("Background namespace refresh failed: {}", e),
                    );
                }
            }
        }
    }));

    Ok(())
}

pub(super) async fn refresh_namespaces(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let api: Api<Namespace> = Api::all(client);

    let ns_list = api.list(&ListParams::default()).await?;
    let mut namespaces: Vec<String> = ns_list
        .items
        .into_iter()
        .filter_map(|n| n.metadata.name)
        .collect();
    namespaces.sort();

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(section_keys::system(), state.cluster.current_context.clone()),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::NamespacesUpdated(namespaces)),
    );
    Ok(())
}
