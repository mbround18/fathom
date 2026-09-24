//! 027-fleet-rollup-dashboard: a view-only, all-contexts-at-once health rollup.
//!
//! This is a small, isolated polling subsystem, deliberately kept separate from the ~50
//! existing single-active-client resource modules (research.md R1): it holds its own
//! per-context `kube::Client`s, built fresh each poll attempt using the exact same
//! kubeconfig-resolution/client-construction path `context_status.rs::probe_context` already
//! uses for its one-shot reachability probes, just looped instead of one-shot.
//!
//! Pollers only exist while the Fleet Dashboard view is open (`StartFleetPolling`/
//! `StopFleetPolling`, sent on view mount/unmount) — Constitution V's idle-CPU budget is
//! satisfied by construction when the view is closed, since no poller is ever spawned
//! otherwise. `FleetState.active` gates this: reactive re-sync on `Initialize`/
//! `RefreshContexts`/`SetContext` (mirroring `context_status.rs`'s own triggers) is a no-op
//! unless the dashboard is currently open.
//!
//! "Connected contexts" for this feature means every context listed in the operator's
//! kubeconfig, not `ContextConnectionStatus::Connected` — spec 012 P1 guarantees only one
//! context is ever `Connected` at a time (its own data-model.md says so explicitly), so a
//! status filter would make this dashboard show a single card, defeating its purpose. Polling
//! attempts a real client build per context; unreachable ones just report `Unreachable`.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use crossbeam_channel::Sender;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::Client;
use kube::api::{Api, ListParams};

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, FleetContextSummary, FleetHealthState, Route,
    SystemEvent, section_keys,
};

use super::events;
use super::kube::load_kubeconfig;
use super::state::BackendState;

/// Poll interval while the dashboard is open (research.md R3).
const POLL_INTERVAL: Duration = Duration::from_secs(30);
/// Bound on a single context's poll attempt (research.md R3).
const POLL_TIMEOUT: Duration = Duration::from_secs(10);
/// Maximum number of contexts polled concurrently (FR-012).
pub(super) const MAX_CONCURRENT_POLLS: usize = 8;
/// Per-context stagger applied before a poller's first poll, and its cap (FR-012).
pub(super) const START_OFFSET_STEP: Duration = Duration::from_millis(250);
const START_OFFSET_CAP: Duration = Duration::from_secs(5);
/// A `Pending` pod older than this counts as unhealthy (FR-014).
const PENDING_UNHEALTHY_AFTER: Duration = Duration::from_secs(5 * 60);

/// Every context name currently listed in the operator's kubeconfig, sorted. Deliberately not
/// filtered by connection status — see module doc.
pub(super) fn all_kubeconfig_contexts() -> Vec<String> {
    match load_kubeconfig() {
        Ok(kc) => {
            let mut names: Vec<String> = kc.contexts.iter().map(|c| c.name.clone()).collect();
            names.sort();
            names
        }
        Err(e) => {
            tracing::warn!("fleet: failed to load kubeconfig for polling: {}", e);
            Vec::new()
        }
    }
}

/// `StartFleetPolling`: mark the dashboard active and ensure every current context has a
/// running poller. Idempotent.
pub(super) fn start_polling(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    state.fleet.active = true;
    sync_pollers(state, sender);
}

#[cfg(test)]
fn start_polling_with<F, Fut>(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    contexts: Vec<String>,
    poll: F,
) where
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = FleetContextSummary> + Send + 'static,
{
    state.fleet.active = true;
    sync_pollers_with(state, sender, contexts, poll);
}

/// `StopFleetPolling` (and app shutdown via `cleanup_all_tasks`): abort every running poller
/// and stop reacting to context-list changes until started again.
pub(super) fn stop_polling(state: &mut BackendState) {
    state.fleet.active = false;
    for (_, handle) in state.fleet.pollers.drain() {
        handle.abort();
    }
}

/// Reactive re-sync, called from the same triggers `context_status::spawn_status_probes`
/// already listens to (`Initialize`, `RefreshContexts`, `SetContext`). A no-op while the
/// dashboard is closed (`!state.fleet.active`), per Constitution V.
pub(super) fn sync_pollers(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    if !state.fleet.active {
        return;
    }
    sync_pollers_with(state, sender, all_kubeconfig_contexts(), |c| async move {
        poll_once(&c).await
    });
}

/// `sync_pollers` with the context list and per-context poll function injected (testability).
pub(super) fn sync_pollers_with<F, Fut>(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    contexts: Vec<String>,
    poll: F,
) where
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = FleetContextSummary> + Send + 'static,
{
    // 034: a context in the federated set is served by its live tier, never also polled.
    let contexts: Vec<String> = contexts
        .into_iter()
        .filter(|c| !state.federation.is_federated(c))
        .collect();

    // Stop pollers for contexts that disappeared from the kubeconfig.
    let to_remove: Vec<String> = state
        .fleet
        .pollers
        .keys()
        .filter(|c| !contexts.contains(c))
        .cloned()
        .collect();
    for context in to_remove {
        if let Some(handle) = state.fleet.pollers.remove(&context) {
            handle.abort();
        }
    }

    // Start pollers for new contexts.
    for (index, context) in contexts.into_iter().enumerate() {
        if state.fleet.pollers.contains_key(&context) {
            continue;
        }
        let seq = state.cluster.seq.clone();
        let sender_clone = sender.clone();
        emit_summary(&seq, &sender_clone, pending_summary(context.clone()));
        let handle = spawn_poller(
            context.clone(),
            start_offset(index),
            seq,
            sender_clone,
            state.fleet.poll_semaphore.clone(),
            poll.clone(),
        );
        state.fleet.pollers.insert(context, handle);
    }
}

/// 034: abort and forget `context`'s 027 poller (it is being promoted to the live tier).
pub(super) fn stop_poller(state: &mut BackendState, context: &str) {
    if let Some(handle) = state.fleet.pollers.remove(context) {
        handle.abort();
    }
}

/// 034: build a client for `context_name` via the same kubeconfig path the poller uses.
pub(super) async fn client_for_context(context_name: &str) -> anyhow::Result<Client> {
    super::cloud_auth::build_client(context_name).await
}

/// Deterministic first-poll delay for the `index`th context: index x 250 ms, capped at 5 s.
pub(super) fn start_offset(index: usize) -> Duration {
    START_OFFSET_STEP
        .saturating_mul(u32::try_from(index).unwrap_or(u32::MAX))
        .min(START_OFFSET_CAP)
}

pub(super) fn pending_summary(context: String) -> FleetContextSummary {
    FleetContextSummary {
        context,
        health: FleetHealthState::Pending,
        error: None,
        node_ready_count: None,
        node_total_count: None,
        unhealthy_pod_count: None,
        cluster_version: None,
        flux: None,
        restricted: Vec::new(),
        last_updated: now_iso8601(),
        live: false,
        plugin_error: None,
    }
}

/// Spawn the recurring poll loop for one context. Waits `start_delay` before the first poll,
/// then emits one `FleetSummaryUpdated` per attempt, success or failure, every `POLL_INTERVAL`,
/// until aborted. Each poll holds a permit from the shared `semaphore` (FR-012).
fn spawn_poller<F, Fut>(
    context: String,
    start_delay: Duration,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    semaphore: Arc<tokio::sync::Semaphore>,
    poll: F,
) -> tokio::task::JoinHandle<()>
where
    F: Fn(String) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = FleetContextSummary> + Send + 'static,
{
    tokio::spawn(async move {
        if !start_delay.is_zero() {
            tokio::time::sleep(start_delay).await;
        }
        loop {
            let summary = {
                let Ok(_permit) = semaphore.acquire().await else {
                    return;
                };
                poll(context.clone()).await
            };
            emit_summary(&seq, &sender, summary);
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
}

pub(super) fn emit_summary(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    summary: FleetContextSummary,
) {
    debug_assert!(
        summary.error.is_none()
            || matches!(
                summary.health,
                FleetHealthState::Unreachable | FleetHealthState::AuthError
            ),
        "FleetContextSummary.error must only be set when health is Unreachable/AuthError"
    );
    events::emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::FleetSummaryUpdated(summary)),
    );
}

/// Build a fresh client for `context_name` (same construction path as
/// `context_status::probe_context`) and gather one poll attempt's worth of data, bounded by
/// `POLL_TIMEOUT`.
async fn poll_once(context_name: &str) -> FleetContextSummary {
    match tokio::time::timeout(POLL_TIMEOUT, poll_inner(context_name)).await {
        Ok(Ok(summary)) => summary,
        Ok(Err(e)) => failure_summary(context_name, &e),
        Err(_) => FleetContextSummary {
            context: context_name.to_owned(),
            health: FleetHealthState::Unreachable,
            error: Some(format!(
                "Timed out after {}s polling context",
                POLL_TIMEOUT.as_secs()
            )),
            node_ready_count: None,
            node_total_count: None,
            unhealthy_pod_count: None,
            cluster_version: None,
            flux: None,
            restricted: Vec::new(),
            last_updated: now_iso8601(),
            live: false,
            plugin_error: None,
        },
    }
}

fn failure_summary(context_name: &str, err: &anyhow::Error) -> FleetContextSummary {
    let plugin_error = super::cloud_auth::plugin_error_of(err).cloned();
    // A plugin failure is a credential problem, but stays distinguishable from an API 401/403
    // by `plugin_error` (035).
    let health = if plugin_error.is_some() || is_auth_error(err) {
        FleetHealthState::AuthError
    } else {
        FleetHealthState::Unreachable
    };
    FleetContextSummary {
        context: context_name.to_owned(),
        health,
        error: Some(format!("{err:#}")),
        node_ready_count: None,
        node_total_count: None,
        unhealthy_pod_count: None,
        cluster_version: None,
        flux: None,
        restricted: Vec::new(),
        last_updated: now_iso8601(),
        live: false,
        plugin_error,
    }
}

/// The HTTP status of a `kube::Error::Api` anywhere in the error chain, if present.
fn api_status_code(err: &anyhow::Error) -> Option<u16> {
    err.chain()
        .find_map(|e| match e.downcast_ref::<kube::Error>() {
            Some(kube::Error::Api(resp)) => Some(resp.code),
            _ => None,
        })
}

/// Whether the error is an auth rejection (401/403 from the API server) (FR-005).
pub(super) fn is_auth_error(err: &anyhow::Error) -> bool {
    matches!(api_status_code(err), Some(401 | 403))
        || err
            .chain()
            .any(|e| matches!(e.downcast_ref::<kube::Error>(), Some(kube::Error::Auth(_))))
}

async fn poll_inner(context_name: &str) -> anyhow::Result<FleetContextSummary> {
    // 035: credentials come through the shared cloud-auth cache, so a fresh client per poll does
    // not re-run the plugin. A 401 with a cached credential drops it and retries exactly once.
    let mut client = super::cloud_auth::build_client(context_name).await?;
    let version = match client.apiserver_version().await {
        Ok(v) => v,
        Err(e)
            if matches!(&e, kube::Error::Api(r) if r.code == 401)
                && super::cloud_auth::invalidate_context(context_name) =>
        {
            client = super::cloud_auth::build_client(context_name).await?;
            match client.apiserver_version().await {
                Ok(v) => v,
                Err(e) => return Err(super::cloud_auth::rejected_after_retry(context_name, e)),
            }
        }
        Err(e) => return Err(e.into()),
    };

    let (nodes, pods, flux) = tokio::join!(
        fetch_node_counts(&client),
        fetch_unhealthy_pod_count(&client),
        fetch_flux_rollup(&client),
    );
    assemble_summary(
        context_name,
        format!("v{}.{}", version.major, version.minor),
        nodes,
        pods,
        flux,
    )
}

/// Combine independently fetched groups into a summary. A 403 (`Ok(None)`, see
/// `restrict_forbidden`) marks the group restricted; any other error fails the poll (FR-013).
fn assemble_summary(
    context_name: &str,
    cluster_version: String,
    nodes: anyhow::Result<Option<(u32, u32)>>,
    pods: anyhow::Result<Option<u32>>,
    flux: Option<crate::protocol::FluxRollupStatus>,
) -> anyhow::Result<FleetContextSummary> {
    let nodes = nodes?;
    let pods = pods?;
    let mut restricted = Vec::new();
    if nodes.is_none() {
        restricted.push("nodes".to_owned());
    }
    if pods.is_none() {
        restricted.push("pods".to_owned());
    }
    Ok(FleetContextSummary {
        context: context_name.to_owned(),
        health: classify(pods, nodes),
        error: None,
        node_ready_count: nodes.map(|n| n.0),
        node_total_count: nodes.map(|n| n.1),
        unhealthy_pod_count: pods,
        cluster_version: Some(cluster_version),
        flux,
        restricted,
        last_updated: now_iso8601(),
        live: false,
        plugin_error: None,
    })
}

/// Health from whatever was readable; restricted groups (`None`) contribute nothing.
fn classify(unhealthy_pods: Option<u32>, nodes: Option<(u32, u32)>) -> FleetHealthState {
    let pods_bad = unhealthy_pods.is_some_and(|n| n > 0);
    let nodes_bad = nodes.is_some_and(|(ready, total)| ready < total);
    if pods_bad || nodes_bad {
        FleetHealthState::Degraded
    } else {
        FleetHealthState::Healthy
    }
}

/// Map a list result: a 403 becomes `Ok(None)` (restricted), other errors stay errors.
fn restrict_forbidden<T>(res: Result<T, kube::Error>) -> anyhow::Result<Option<T>> {
    match res {
        Ok(v) => Ok(Some(v)),
        Err(kube::Error::Api(resp)) if resp.code == 403 => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// (ready, total) node counts, "Ready" meaning the `Ready` condition's status is `"True"`.
/// `None` when the list is forbidden.
async fn fetch_node_counts(client: &Client) -> anyhow::Result<Option<(u32, u32)>> {
    let api = Api::<Node>::all(client.clone());
    let Some(nodes) = restrict_forbidden(api.list(&ListParams::default()).await)? else {
        return Ok(None);
    };
    let total = nodes.items.len() as u32;
    let ready = nodes.items.iter().filter(|n| node_is_ready(n)).count() as u32;
    Ok(Some((ready, total)))
}

pub(super) fn node_is_ready(node: &Node) -> bool {
    node.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conds| conds.iter().find(|c| c.type_ == "Ready"))
        .is_some_and(|c| c.status == "True")
}

/// Count of pods this dashboard classifies as unhealthy (FR-014): `Failed`/`Unknown` phase;
/// `Running` with at least one container not ready; any container (or init container) waiting
/// with `ImagePullBackOff`, `ErrImagePull`, `CrashLoopBackOff` or `CreateContainerError`; or
/// `Pending` for more than 5 minutes since creation. A young `Pending` pod is still
/// scheduling and is not counted. `None` when the pod list is forbidden.
async fn fetch_unhealthy_pod_count(client: &Client) -> anyhow::Result<Option<u32>> {
    let api = Api::<Pod>::all(client.clone());
    let Some(pods) = restrict_forbidden(api.list(&ListParams::default()).await)? else {
        return Ok(None);
    };
    let now = chrono::Utc::now();
    let count = pods
        .items
        .iter()
        .filter(|p| pod_is_unhealthy(p, now))
        .count() as u32;
    Ok(Some(count))
}

const BAD_WAITING_REASONS: [&str; 4] = [
    "ImagePullBackOff",
    "ErrImagePull",
    "CrashLoopBackOff",
    "CreateContainerError",
];

pub(super) fn pod_is_unhealthy(pod: &Pod, now: chrono::DateTime<chrono::Utc>) -> bool {
    let Some(status) = pod.status.as_ref() else {
        return false;
    };
    let waiting_bad = status
        .container_statuses
        .iter()
        .chain(status.init_container_statuses.iter())
        .flatten()
        .any(|c| {
            c.state
                .as_ref()
                .and_then(|s| s.waiting.as_ref())
                .and_then(|w| w.reason.as_deref())
                .is_some_and(|r| BAD_WAITING_REASONS.contains(&r))
        });
    if waiting_bad {
        return true;
    }
    let phase = status.phase.as_deref().unwrap_or("Unknown");
    match phase {
        "Failed" | "Unknown" => true,
        "Running" => status
            .container_statuses
            .as_ref()
            .is_some_and(|cs| cs.iter().any(|c| !c.ready)),
        "Pending" => pod.metadata.creation_timestamp.as_ref().is_some_and(|t| {
            (now - t.0)
                .to_std()
                .is_ok_and(|age| age > PENDING_UNHEALTHY_AFTER)
        }),
        _ => false,
    }
}

async fn fetch_flux_rollup(client: &Client) -> Option<crate::protocol::FluxRollupStatus> {
    match super::flux::scan_flux(client).await {
        Ok(overview) => Some(crate::protocol::reduce_flux_overview(&overview)),
        Err(e) => {
            tracing::debug!("fleet: flux scan failed, treating as not applicable: {}", e);
            None
        }
    }
}

pub(super) fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_healthy_and_degraded() {
        assert_eq!(classify(Some(0), Some((3, 3))), FleetHealthState::Healthy);
        assert_eq!(classify(Some(1), Some((3, 3))), FleetHealthState::Degraded);
        assert_eq!(classify(Some(0), Some((2, 3))), FleetHealthState::Degraded);
        assert_eq!(classify(None, None), FleetHealthState::Healthy);
        assert_eq!(classify(None, Some((2, 3))), FleetHealthState::Degraded);
    }

    #[test]
    fn failure_maps_to_unreachable_or_auth_with_verbatim_error() {
        let s = failure_summary("c", &anyhow::anyhow!("connection refused"));
        assert_eq!(s.health, FleetHealthState::Unreachable);
        assert_eq!(s.error.as_deref(), Some("connection refused"));
        assert!(s.unhealthy_pod_count.is_none());
        let s = failure_summary("c", &anyhow::Error::new(api_err(401)));
        assert_eq!(s.health, FleetHealthState::AuthError);
        assert!(s.error.is_some());
    }

    fn api_err(code: u16) -> kube::Error {
        kube::Error::Api(kube::core::ErrorResponse {
            status: "Failure".to_owned(),
            message: "nope".to_owned(),
            reason: "x".to_owned(),
            code,
        })
    }

    #[test]
    fn auth_error_uses_typed_status_not_text() {
        assert!(is_auth_error(&anyhow::Error::new(api_err(401))));
        assert!(is_auth_error(&anyhow::Error::new(api_err(403))));
        assert!(is_auth_error(
            &anyhow::Error::new(api_err(401)).context("version call")
        ));
        assert!(!is_auth_error(&anyhow::Error::new(api_err(500))));
        assert!(!is_auth_error(&anyhow::anyhow!("401 Unauthorized")));
    }

    #[test]
    fn nodes_denied_pods_ok() {
        let s = assemble_summary("c", "v1.30".into(), Ok(None), Ok(Some(2)), None).unwrap();
        assert_eq!(s.restricted, vec!["nodes"]);
        assert!(s.node_ready_count.is_none() && s.node_total_count.is_none());
        assert_eq!(s.unhealthy_pod_count, Some(2));
        assert_eq!(s.health, FleetHealthState::Degraded);
    }

    #[test]
    fn pods_denied_nodes_ok() {
        let s = assemble_summary("c", "v1.30".into(), Ok(Some((3, 3))), Ok(None), None).unwrap();
        assert_eq!(s.restricted, vec!["pods"]);
        assert!(s.unhealthy_pod_count.is_none());
        assert_eq!(s.node_ready_count, Some(3));
        assert_eq!(s.health, FleetHealthState::Healthy);
    }

    #[test]
    fn both_denied_is_healthy_when_version_ok() {
        let s = assemble_summary("c", "v1.30".into(), Ok(None), Ok(None), None).unwrap();
        assert_eq!(s.restricted, vec!["nodes", "pods"]);
        assert_eq!(s.health, FleetHealthState::Healthy);
        assert!(s.error.is_none());
    }

    #[test]
    fn non_403_list_error_fails_poll() {
        let e = restrict_forbidden::<()>(Err(api_err(500))).unwrap_err();
        assert!(assemble_summary("c", "v".into(), Err(e), Ok(Some(0)), None).is_err());
        assert!(matches!(restrict_forbidden(Ok(1)), Ok(Some(1))));
        assert!(matches!(
            restrict_forbidden::<()>(Err(api_err(403))),
            Ok(None)
        ));
    }

    #[test]
    fn pending_summary_has_no_numbers_or_error() {
        let s = pending_summary("dev".to_owned());
        assert_eq!(s.health, FleetHealthState::Pending);
        assert!(s.error.is_none());
        assert!(s.node_ready_count.is_none());
        assert!(s.node_total_count.is_none());
        assert!(s.unhealthy_pod_count.is_none());
        assert!(s.cluster_version.is_none());
    }

    #[test]
    fn pod_running_with_unready_container_is_unhealthy() {
        use k8s_openapi::api::core::v1::{ContainerStatus, PodStatus};

        let pod = Pod {
            status: Some(PodStatus {
                phase: Some("Running".to_owned()),
                container_statuses: Some(vec![ContainerStatus {
                    ready: false,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(pod_is_unhealthy(&pod, chrono::Utc::now()));
    }

    #[test]
    fn pod_pending_is_not_unhealthy() {
        use k8s_openapi::api::core::v1::PodStatus;

        let pod = Pod {
            status: Some(PodStatus {
                phase: Some("Pending".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!pod_is_unhealthy(&pod, chrono::Utc::now()));
    }

    fn waiting_pod(reason: &str, init: bool) -> Pod {
        use k8s_openapi::api::core::v1::{
            ContainerState, ContainerStateWaiting, ContainerStatus, PodStatus,
        };
        let cs = ContainerStatus {
            ready: false,
            state: Some(ContainerState {
                waiting: Some(ContainerStateWaiting {
                    reason: Some(reason.to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut status = PodStatus {
            phase: Some("Pending".to_owned()),
            ..Default::default()
        };
        if init {
            status.init_container_statuses = Some(vec![cs]);
        } else {
            status.container_statuses = Some(vec![cs]);
        }
        Pod {
            status: Some(status),
            ..Default::default()
        }
    }

    #[test]
    fn bad_waiting_reasons_are_unhealthy() {
        let now = chrono::Utc::now();
        for reason in BAD_WAITING_REASONS {
            assert!(
                pod_is_unhealthy(&waiting_pod(reason, false), now),
                "{reason}"
            );
            assert!(
                pod_is_unhealthy(&waiting_pod(reason, true), now),
                "init {reason}"
            );
        }
        assert!(!pod_is_unhealthy(
            &waiting_pod("ContainerCreating", false),
            now
        ));
    }

    #[test]
    fn pending_pod_unhealthy_only_after_five_minutes() {
        use k8s_openapi::api::core::v1::PodStatus;
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
        let now = chrono::Utc::now();
        let pending = |age_secs: i64| Pod {
            metadata: ObjectMeta {
                creation_timestamp: Some(Time(now - chrono::Duration::seconds(age_secs))),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Pending".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!pod_is_unhealthy(&pending(60), now));
        assert!(pod_is_unhealthy(&pending(301), now));
    }

    fn test_sender() -> (
        Sender<BackendEventEnvelope>,
        crossbeam_channel::Receiver<BackendEventEnvelope>,
    ) {
        crossbeam_channel::unbounded()
    }

    /// A poll that never completes, so no cluster is contacted and handles stay live.
    async fn hang(context: String) -> FleetContextSummary {
        std::future::pending::<()>().await;
        pending_summary(context)
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[tokio::test(start_paused = true)]
    async fn start_twice_leaves_pollers_unchanged() {
        let (tx, _rx) = test_sender();
        let mut state = BackendState::new();
        start_polling_with(&mut state, &tx, names(&["a", "b"]), hang);
        assert_eq!(state.fleet.pollers.len(), 2);
        start_polling_with(&mut state, &tx, names(&["a", "b"]), hang);
        assert_eq!(state.fleet.pollers.len(), 2);
        state.fleet.abort_all();
    }

    #[tokio::test(start_paused = true)]
    async fn adding_context_gains_one_poller_and_keeps_prior() {
        let (tx, _rx) = test_sender();
        let mut state = BackendState::new();
        start_polling_with(&mut state, &tx, names(&["a", "b"]), hang);
        let a_id = state.fleet.pollers["a"].id();
        let b_id = state.fleet.pollers["b"].id();
        sync_pollers_with(&mut state, &tx, names(&["a", "b", "c"]), hang);
        assert_eq!(state.fleet.pollers.len(), 3);
        assert_eq!(state.fleet.pollers["a"].id(), a_id);
        assert_eq!(state.fleet.pollers["b"].id(), b_id);
        state.fleet.abort_all();
    }

    #[tokio::test(start_paused = true)]
    async fn stop_polling_emits_no_further_events() {
        let (tx, rx) = test_sender();
        let mut state = BackendState::new();
        let poll = |c: String| async move { pending_summary(c) };
        start_polling_with(&mut state, &tx, names(&["a", "b"]), poll);
        tokio::time::sleep(Duration::from_secs(1)).await;
        stop_polling(&mut state);
        assert!(state.fleet.pollers.is_empty());
        while rx.try_recv().is_ok() {}
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert!(
            rx.try_recv().is_err(),
            "event emitted after StopFleetPolling"
        );
    }

    #[test]
    fn start_offset_is_staggered_and_capped() {
        assert_eq!(start_offset(0), Duration::ZERO);
        assert_eq!(start_offset(4), Duration::from_secs(1));
        assert_eq!(start_offset(100), Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn at_most_eight_polls_run_concurrently() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (tx, _rx) = test_sender();
        let mut state = BackendState::new();
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let poll = {
            let (running, peak) = (running.clone(), peak.clone());
            move |context: String| {
                let (running, peak) = (running.clone(), peak.clone());
                async move {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    running.fetch_sub(1, Ordering::SeqCst);
                    pending_summary(context)
                }
            }
        };
        let contexts: Vec<String> = (0..20).map(|i| format!("ctx-{i:02}")).collect();
        start_polling_with(&mut state, &tx, contexts, poll);
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(peak.load(Ordering::SeqCst), MAX_CONCURRENT_POLLS);
        state.fleet.abort_all();
    }
}
