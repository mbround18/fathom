use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, DaemonMode, NamespaceSummary,
    ResourceListEvent, Route, SystemEvent, section_keys,
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
use super::{
    cert_manager, cluster_version, context_status, daemon_ctl, deployments, events, first_run,
    flux, gatekeeper, hubble, keda, knative, pods, trivy, velero, warm_lists,
};

/// Spawn the background pod watch for the active context. With the daemon preference `auto`
/// it is the snapshot-then-delta watcher (036 US4); otherwise the legacy watcher, unchanged.
fn spawn_pod_watch(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    client: kube::Client,
) {
    let (filter, seq, ctx) = (
        state.cluster.namespace_filter_shared.clone(),
        state.cluster.seq.clone(),
        state.cluster.current_context.clone(),
    );
    let task = if daemon_ctl::active_mode(state) == DaemonMode::Auto {
        tokio::spawn(warm_lists::watch_pods_delta(
            client,
            filter,
            seq,
            sender.clone(),
            ctx,
            state.daemon.snapshot_slot.clone(),
        ))
    } else {
        tokio::spawn(watch_pods(client, filter, seq, sender.clone(), ctx))
    };
    if let Some(old) = state.watch_tasks.pod_watch_task.replace(task) {
        old.abort();
    }
}

/// After the daemon preference changes, switch an already-running pod watch to the matching
/// implementation. No-op when background sync is not running.
pub(super) fn respawn_pod_watch(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    if state.watch_tasks.pod_watch_task.is_none() {
        return;
    }
    if let Some(client) = state.cluster.client.clone() {
        spawn_pod_watch(state, sender, client);
    }
}

pub(super) async fn ensure_background_sync(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;

    // If tasks already exist, keep them running.
    if state.watch_tasks.pod_watch_task.is_some()
        && state.watch_tasks.deployment_watch_task.is_some()
        && state.watch_tasks.namespace_sync_task.is_some()
    {
        return Ok(());
    }

    // Restart cleanly if partially running.
    state.watch_tasks.abort_all();

    // Pod watcher: watch for pod changes with exponential backoff retry
    spawn_pod_watch(state, sender, client.clone());

    // Deployment watcher: the Deployments list converges after scale/restart without a refresh.
    state.watch_tasks.deployment_watch_task = Some(tokio::spawn(watch_deployments(
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

/// Best-effort: if a `hubble-relay` Service is present, start (or keep running) a
/// background poll of `Observer.GetFlows` every 20s, emitting `HubbleTrafficUpdated`.
/// Does nothing if a poll task is already running.
///
/// Detection (a cluster-wide Service LIST) runs inside the spawned task, not under the command
/// lock, so `Initialize`/`SetContext` do not hold the lock through it. When Hubble is absent the
/// task ends at once, and the next call detects again.
pub(super) async fn ensure_hubble_poll(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    if state
        .watch_tasks
        .hubble_poll_task
        .as_ref()
        .is_some_and(|task| !task.is_finished())
    {
        return;
    }
    let Some(client) = state.cluster.client.clone() else {
        return;
    };

    let seq = state.cluster.seq.clone();
    let sender = sender.clone();
    let context = state.cluster.current_context.clone();
    state.watch_tasks.hubble_poll_task = Some(tokio::spawn(async move {
        let Some(cfg) = hubble::detect_hubble(&client).await else {
            return;
        };
        poll_hubble_traffic(client, cfg, seq, sender, context).await;
    }));
}

/// Best-effort: start (or keep running) a background scan of Flux GitOps Toolkit
/// resources every 20s, emitting `FluxOverviewUpdated`. Does nothing if a poll task
/// is already running; `scan_flux` itself reports `installed: false` when Flux's
/// CRDs are absent, so there is no separate detection gate here.
pub(super) async fn ensure_flux_poll(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    if state.watch_tasks.flux_poll_task.is_some() {
        return;
    }
    let Some(client) = state.cluster.client.clone() else {
        return;
    };

    state.watch_tasks.flux_poll_task = Some(tokio::spawn(poll_flux_overview(
        client,
        state.cluster.seq.clone(),
        sender.clone(),
        state.cluster.current_context.clone(),
    )));
}

/// Best-effort: start (or keep running) a background scan of OPA Gatekeeper constraints
/// every 20s, emitting `GatekeeperOverviewUpdated`. Does nothing if a poll task is already
/// running; `scan_gatekeeper` itself reports `GatekeeperInstallState::NotInstalled` when
/// Gatekeeper's CRDs are absent, so there is no separate detection gate here.
pub(super) async fn ensure_gatekeeper_poll(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    if state.watch_tasks.gatekeeper_poll_task.is_some() {
        return;
    }
    let Some(client) = state.cluster.client.clone() else {
        return;
    };

    state.watch_tasks.gatekeeper_poll_task = Some(tokio::spawn(poll_gatekeeper_overview(
        client,
        state.cluster.seq.clone(),
        sender.clone(),
        state.cluster.current_context.clone(),
    )));
}

/// Background task: rescan Gatekeeper constraints every 20s and emit the aggregated overview.
async fn poll_gatekeeper_overview(
    client: kube::Client,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        // A failure reaches the pane as a `ScanFailed` overview (with a retry) rather than a
        // toast every 20s; the log keeps the history.
        let overview = gatekeeper::scan_gatekeeper_overview(&client)
            .await
            .unwrap_or_else(|(overview, message)| {
                tracing::warn!("Background Gatekeeper scan failed: {message}");
                overview
            });
        events::emit(
            seq.as_ref(),
            &sender,
            Route::cluster(section_keys::integrations_gatekeeper(), context.clone()),
            Correlation::None,
            BackendEvent::System(SystemEvent::GatekeeperOverviewUpdated(overview)),
        );
    }
}

/// Best-effort: start (or keep running) a background scan of Velero backup resources every
/// 20s, emitting `VeleroOverviewUpdated`. Does nothing if a poll task is already running;
/// `scan_velero` itself reports `installed: false` when Velero's CRDs are absent, so there is no
/// separate detection gate here (mirrors `ensure_flux_poll`).
pub(super) async fn ensure_velero_poll(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    if state.watch_tasks.velero_poll_task.is_some() {
        return;
    }
    let Some(client) = state.cluster.client.clone() else {
        return;
    };

    state.watch_tasks.velero_poll_task = Some(tokio::spawn(poll_velero_overview(
        client,
        state.cluster.seq.clone(),
        sender.clone(),
        state.cluster.current_context.clone(),
    )));
}

/// Background task: rescan Velero resources every 20s and emit the aggregated overview.
async fn poll_velero_overview(
    client: kube::Client,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        match velero::scan_velero(&client).await {
            Ok(overview) => {
                events::emit(
                    seq.as_ref(),
                    &sender,
                    Route::cluster(section_keys::integrations_velero(), context.clone()),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::VeleroOverviewUpdated(overview)),
                );
            }
            Err(e) => {
                notify_poll_error(
                    seq.as_ref(),
                    &sender,
                    format!("Background Velero scan failed: {}", e),
                );
            }
        }
    }
}

/// Best-effort: start (or keep running) a background scan of cert-manager Certificates
/// every 20s, emitting `CertManagerOverviewUpdated`. Does nothing if a poll task is already
/// running; `scan_cert_manager` itself reports `installed: false` when the Certificate CRD
/// is absent, so there is no separate detection gate here (mirrors `ensure_flux_poll`).
pub(super) async fn ensure_cert_manager_poll(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    if state.watch_tasks.cert_manager_poll_task.is_some() {
        return;
    }
    let Some(client) = state.cluster.client.clone() else {
        return;
    };

    state.watch_tasks.cert_manager_poll_task = Some(tokio::spawn(poll_cert_manager_overview(
        client,
        state.cluster.seq.clone(),
        sender.clone(),
        state.cluster.current_context.clone(),
    )));
}

/// Background task: rescan cert-manager Certificates every 20s and emit the aggregated overview.
async fn poll_cert_manager_overview(
    client: kube::Client,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        match cert_manager::scan_cert_manager(&client).await {
            Ok(overview) => {
                events::emit(
                    seq.as_ref(),
                    &sender,
                    Route::cluster(section_keys::integrations_cert_manager(), context.clone()),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::CertManagerOverviewUpdated(overview)),
                );
            }
            Err(e) => {
                notify_poll_error(
                    seq.as_ref(),
                    &sender,
                    format!("Background cert-manager scan failed: {}", e),
                );
            }
        }
    }
}

/// Best-effort: start (or keep running) a background scan of Trivy Operator vulnerability
/// reports every 20s, emitting `TrivyOverviewUpdated`. Does nothing if a poll task is already
/// running; `scan_trivy` itself reports `installed: false` when the Trivy Operator's CRDs are
/// absent, so there is no separate detection gate here (mirrors `ensure_flux_poll`).
pub(super) async fn ensure_trivy_poll(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    if state.watch_tasks.trivy_poll_task.is_some() {
        return;
    }
    let Some(client) = state.cluster.client.clone() else {
        return;
    };

    state.watch_tasks.trivy_poll_task = Some(tokio::spawn(poll_trivy_overview(
        client,
        state.cluster.seq.clone(),
        sender.clone(),
        state.cluster.current_context.clone(),
    )));
}

/// Background task: rescan Trivy Operator vulnerability reports every 20s and emit the
/// aggregated overview.
async fn poll_trivy_overview(
    client: kube::Client,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        match trivy::scan_trivy(&client).await {
            Ok(overview) => {
                events::emit(
                    seq.as_ref(),
                    &sender,
                    Route::cluster(
                        section_keys::integrations_vulnerabilities(),
                        context.clone(),
                    ),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::TrivyOverviewUpdated(overview)),
                );
            }
            Err(e) => {
                notify_poll_error(
                    seq.as_ref(),
                    &sender,
                    format!("Background Trivy scan failed: {}", e),
                );
            }
        }
    }
}

/// Best-effort: start (or keep running) a background scan of Knative Serving resources every
/// 20s, emitting `KnativeOverviewUpdated`. Does nothing if a poll task is already running;
/// `scan_knative` itself reports `installed: false` when Knative's CRDs are absent, so there is
/// no separate detection gate here (mirrors `ensure_flux_poll`).
pub(super) async fn ensure_knative_poll(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    if state.watch_tasks.knative_poll_task.is_some() {
        return;
    }
    let Some(client) = state.cluster.client.clone() else {
        return;
    };

    state.watch_tasks.knative_poll_task = Some(tokio::spawn(poll_knative_overview(
        client,
        state.cluster.seq.clone(),
        sender.clone(),
        state.cluster.current_context.clone(),
    )));
}

/// Background task: rescan Knative Serving resources every 20s and emit the aggregated overview.
async fn poll_knative_overview(
    client: kube::Client,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        match knative::scan_knative(&client).await {
            Ok(overview) => {
                events::emit(
                    seq.as_ref(),
                    &sender,
                    Route::cluster(section_keys::integrations_knative(), context.clone()),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::KnativeOverviewUpdated(overview)),
                );
            }
            Err(e) => {
                notify_poll_error(
                    seq.as_ref(),
                    &sender,
                    format!("Background Knative scan failed: {}", e),
                );
            }
        }
    }
}

/// Best-effort: start (or keep running) a background KEDA scan every 20s, emitting
/// `KedaOverviewUpdated`. `scan_keda` reports `installed: false` when KEDA is absent
/// (mirrors `ensure_flux_poll`).
pub(super) async fn ensure_keda_poll(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    if state.watch_tasks.keda_poll_task.is_some() {
        return;
    }
    let Some(client) = state.cluster.client.clone() else {
        return;
    };
    state.watch_tasks.keda_poll_task = Some(tokio::spawn(poll_keda_overview(
        client,
        state.cluster.seq.clone(),
        sender.clone(),
        state.cluster.current_context.clone(),
    )));
}

/// Background task: rescan KEDA ScaledObjects/ScaledJobs every 20s.
async fn poll_keda_overview(
    client: kube::Client,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        match keda::scan_keda(&client).await {
            Ok(overview) => {
                events::emit(
                    seq.as_ref(),
                    &sender,
                    Route::cluster(section_keys::autoscalers(), context.clone()),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::KedaOverviewUpdated(overview)),
                );
            }
            Err(e) => {
                notify_poll_error(
                    seq.as_ref(),
                    &sender,
                    format!("Background KEDA scan failed: {}", e),
                );
            }
        }
    }
}

/// Background task: rescan Flux resources every 20s and emit the aggregated overview.
async fn poll_flux_overview(
    client: kube::Client,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        match flux::scan_flux(&client).await {
            Ok(overview) => {
                events::emit(
                    seq.as_ref(),
                    &sender,
                    Route::cluster(section_keys::integrations_flux(), context.clone()),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::FluxOverviewUpdated(overview)),
                );
            }
            Err(e) => {
                notify_poll_error(
                    seq.as_ref(),
                    &sender,
                    format!("Background Flux scan failed: {}", e),
                );
            }
        }
    }
}

/// Background task: sample Hubble traffic every 20s and emit the aggregated summary.
async fn poll_hubble_traffic(
    client: kube::Client,
    cfg: hubble::HubbleConfig,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        match hubble::fetch_traffic_summary(&client, &cfg).await {
            Ok(summary) => {
                events::emit(
                    seq.as_ref(),
                    &sender,
                    Route::cluster(section_keys::integrations_hubble(), context.clone()),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::HubbleTrafficUpdated(summary)),
                );
            }
            Err(e) => {
                notify_poll_error(
                    seq.as_ref(),
                    &sender,
                    format!("Background Hubble traffic poll failed: {}", e),
                );
            }
        }
    }
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
    let (c, s, tx) = (client.clone(), seq.clone(), sender.clone());
    let watch_ctx = context.clone();
    watch_and_emit::<Pod, _, _>(client, "Pod", watch_ctx, seq, sender, move || {
        let (c, f, s, tx, ctx) = (
            c.clone(),
            filter_shared.clone(),
            s.clone(),
            tx.clone(),
            context.clone(),
        );
        async move { emit_pods_snapshot(&c, &f, &s, &tx, &ctx).await }
    })
    .await;
}

/// Background task: watch deployments cluster-wide and re-emit the filtered list on every
/// change (replica counts after a scale, rollouts after a restart), with the same backoff.
async fn watch_deployments(
    client: kube::Client,
    filter_shared: Arc<RwLock<Vec<String>>>,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    let (c, s, tx) = (client.clone(), seq.clone(), sender.clone());
    let watch_ctx = context.clone();
    watch_and_emit::<Deployment, _, _>(client, "Deployment", watch_ctx, seq, sender, move || {
        let filter = filter_shared.read().map(|f| f.clone()).unwrap_or_default();
        let (c, s, tx, ctx) = (c.clone(), s.clone(), tx.clone(), context.clone());
        async move { deployments::emit_deployments_snapshot(&c, &filter, &s, &tx, &ctx).await }
    })
    .await;
}

/// Watch every `K` (`kind`, e.g. "Pod") in the cluster and call `on_change` after each watch event, reconnecting
/// with exponential backoff (1s doubling to 30s) when the watch fails.
///
/// The watchers double as the mid-session health check: when a watch fails because the cluster
/// is gone (connection refused, DNS, TLS, timeout) or now rejects the credentials, the first
/// watcher to notice marks the active context down, reports it once (`ActiveConnectionProblem`
/// plus one notification) and every background poll goes quiet until a watch succeeds again.
/// Before this, a deleted cluster produced a fresh error notification from every poller every
/// few seconds, forever.
async fn watch_and_emit<K, F, Fut>(
    client: kube::Client,
    kind: &str,
    context: Option<String>,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    mut on_change: F,
) where
    K: kube::Resource<DynamicType = ()>
        + Clone
        + serde::de::DeserializeOwned
        + std::fmt::Debug
        + Send
        + 'static,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    events::notify_info(
        seq.as_ref(),
        &sender,
        format!("Background sync: watching {}s...", kind.to_lowercase()),
    );

    let mut retry_delay = Duration::from_secs(1);
    loop {
        let api: Api<K> = Api::all(client.clone());
        let w = watcher(api, watcher::Config::default());
        futures_util::pin_mut!(w);

        while let Some(ev) = w.next().await {
            match ev {
                Ok(_event) => {
                    retry_delay = Duration::from_secs(1);
                    report_watch_recovered(&seq, &sender, context.as_deref());
                    on_change().await;
                }
                Err(e) => {
                    report_watch_error(&seq, &sender, context.as_deref(), kind, retry_delay, &e);
                    break;
                }
            }
        }

        tokio::time::sleep(retry_delay).await;
        retry_delay = std::cmp::min(retry_delay * 2, Duration::from_secs(30));
    }
}

/// A watch failed: report a lost cluster once, anything else as before.
fn report_watch_error(
    seq: &Arc<AtomicU64>,
    sender: &Sender<BackendEventEnvelope>,
    context: Option<&str>,
    kind: &str,
    retry_delay: Duration,
    err: &watcher::Error,
) {
    let (problem_kind, _) = first_run::classify_std(err);
    if !first_run::is_connection_level(problem_kind) {
        events::notify_error(
            seq.as_ref(),
            sender,
            format!("{kind} watcher error (retrying in {retry_delay:?}): {err}"),
        );
        return;
    }
    if first_run::set_active_down(true) {
        // Already reported; keep retrying quietly.
        tracing::debug!("{kind} watcher still failing: {err}");
        return;
    }
    let Some(ctx) = context else {
        return;
    };
    let err = anyhow::anyhow!("{err}");
    context_status::emit_active_problem(seq, sender, ctx, &err);
    events::notify_error(
        seq.as_ref(),
        sender,
        format!("Lost the connection to {ctx}. Fathom keeps retrying in the background."),
    );
}

/// A watch delivered an event: if the active context was marked down, it is back.
fn report_watch_recovered(
    seq: &Arc<AtomicU64>,
    sender: &Sender<BackendEventEnvelope>,
    context: Option<&str>,
) {
    if !first_run::active_down() || !first_run::set_active_down(false) {
        return;
    }
    let Some(ctx) = context else {
        return;
    };
    context_status::emit_status(
        seq,
        sender,
        ctx,
        crate::protocol::ContextConnectionStatus::Connected,
        None,
    );
    context_status::emit_active_problem_value(seq, sender, ctx, None);
    events::notify_info(seq.as_ref(), sender, format!("Reconnected to {ctx}."));
}

/// A background poll failed. While the active cluster is known to be down the failure is the
/// same outage the watcher already reported, so it is logged instead of notified.
fn notify_poll_error(seq: &AtomicU64, sender: &Sender<BackendEventEnvelope>, message: String) {
    if first_run::active_down() {
        tracing::debug!("{message} (cluster unreachable; not notified)");
        return;
    }
    events::notify_error(seq, sender, message);
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
            notify_poll_error(seq, sender, format!("Background pod refresh failed: {}", e));
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
                notify_poll_error(
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

/// Start the cluster/node Kubernetes version poll (019) if not already running.
pub(super) async fn ensure_cluster_version_poll(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    if state.watch_tasks.cluster_version_poll_task.is_some() {
        return;
    }
    let Some(client) = state.cluster.client.clone() else {
        return;
    };

    state.watch_tasks.cluster_version_poll_task = Some(tokio::spawn(poll_cluster_version(
        client,
        state.cluster.seq.clone(),
        sender.clone(),
    )));
}

/// Background task: fetch immediately, then every 20s, emitting `ClusterVersionUpdated`.
async fn poll_cluster_version(
    client: kube::Client,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await; // first tick completes immediately
        match cluster_version::fetch_cluster_version(&client).await {
            Ok(summary) => {
                events::emit(
                    seq.as_ref(),
                    &sender,
                    Route::global(section_keys::system()),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::ClusterVersionUpdated(summary)),
                );
            }
            Err(e) => {
                notify_poll_error(
                    seq.as_ref(),
                    &sender,
                    format!("Background cluster version poll failed: {}", e),
                );
            }
        }
    }
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
