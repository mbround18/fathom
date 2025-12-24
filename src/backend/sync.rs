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

use super::state::{BackendState, client_or_err};
use super::{events, pods, state};

pub(super) async fn ensure_background_sync(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = client_or_err(state)?;

    // If tasks already exist, keep them running.
    if state.pod_watch_task.is_some() && state.namespace_sync_task.is_some() {
        return Ok(());
    }

    // Restart cleanly if partially running.
    state::abort_task(&mut state.pod_watch_task);
    state::abort_task(&mut state.namespace_sync_task);

    let sender_for_watch = sender.clone();
    let sender_for_ns = sender.clone();
    let filter_shared = state.namespace_filter_shared.clone();
    let seq_for_watch = state.seq.clone();
    let seq_for_ns = state.seq.clone();
    let context_for_watch = state.current_context.clone();
    let context_for_ns = state.current_context.clone();

    // Pod watcher: updates the list when pods are added/updated/deleted.
    let client_for_watch = client.clone();
    state.pod_watch_task = Some(tokio::spawn(async move {
        events::notify_info(
            seq_for_watch.as_ref(),
            &sender_for_watch,
            "Background sync: watching pods...".to_owned(),
        );

        // Simple retry loop.
        loop {
            let api: Api<Pod> = Api::all(client_for_watch.clone());
            let w = watcher(api, watcher::Config::default());
            futures_util::pin_mut!(w);
            while let Some(ev) = w.next().await {
                match ev {
                    Ok(_event) => {
                        // On any event, refresh pods list.
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
                            format!("Pod watcher error: {}", e),
                        );
                        break;
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }));

    // Namespace refresher: refresh namespaces periodically.
    let client_for_ns = client.clone();
    state.namespace_sync_task = Some(tokio::spawn(async move {
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
    let client = client_or_err(state)?;
    let api: Api<Namespace> = Api::all(client);

    let ns_list = api.list(&ListParams::default()).await?;
    let mut namespaces: Vec<String> = ns_list
        .items
        .into_iter()
        .filter_map(|n| n.metadata.name)
        .collect();
    namespaces.sort();

    events::emit(
        state.seq.as_ref(),
        sender,
        Route::cluster(section_keys::system(), state.current_context.clone()),
        Correlation::None,
        BackendEvent::ResourceList(ResourceListEvent::NamespacesUpdated(namespaces)),
    );
    Ok(())
}
