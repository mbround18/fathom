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
use kube::api::{Api, ListParams};
use kube::{Client, Config, config::KubeConfigOptions};

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

/// Every context name currently listed in the operator's kubeconfig, sorted. Deliberately not
/// filtered by connection status — see module doc.
fn all_kubeconfig_contexts() -> Vec<String> {
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
    let contexts = all_kubeconfig_contexts();

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
    for context in contexts {
        if state.fleet.pollers.contains_key(&context) {
            continue;
        }
        let seq = state.cluster.seq.clone();
        let sender_clone = sender.clone();
        emit_summary(&seq, &sender_clone, pending_summary(context.clone()));
        let handle = spawn_poller(context.clone(), seq, sender_clone);
        state.fleet.pollers.insert(context, handle);
    }
}

fn pending_summary(context: String) -> FleetContextSummary {
    FleetContextSummary {
        context,
        health: FleetHealthState::Pending,
        error: None,
        node_ready_count: None,
        node_total_count: None,
        unhealthy_pod_count: None,
        cluster_version: None,
        flux: None,
        last_updated: now_iso8601(),
    }
}

/// Spawn the recurring poll loop for one context. Emits one `FleetSummaryUpdated` per attempt,
/// success or failure, every `POLL_INTERVAL`, until aborted.
fn spawn_poller(
    context: String,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let summary = poll_once(&context).await;
            emit_summary(&seq, &sender, summary);
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
}

fn emit_summary(
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
            last_updated: now_iso8601(),
        },
    }
}

fn failure_summary(context_name: &str, err: &anyhow::Error) -> FleetContextSummary {
    let message = format!("{err:#}");
    let health = if is_auth_error(&message) {
        FleetHealthState::AuthError
    } else {
        FleetHealthState::Unreachable
    };
    FleetContextSummary {
        context: context_name.to_owned(),
        health,
        error: Some(message),
        node_ready_count: None,
        node_total_count: None,
        unhealthy_pod_count: None,
        cluster_version: None,
        flux: None,
        last_updated: now_iso8601(),
    }
}

/// Best-effort heuristic distinguishing an auth rejection from a generic unreachability
/// failure (FR-005) — mirrors how HTTP client errors surface `kube`'s status text.
fn is_auth_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("401")
        || lower.contains("403")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
}

async fn poll_inner(context_name: &str) -> anyhow::Result<FleetContextSummary> {
    let kubeconfig = load_kubeconfig()?;
    let opts = KubeConfigOptions {
        context: Some(context_name.to_owned()),
        ..Default::default()
    };
    let config = Config::from_custom_kubeconfig(kubeconfig, &opts).await?;
    let client = Client::try_from(config)?;

    let version = client.apiserver_version().await?;

    let (node_ready_count, node_total_count) = fetch_node_counts(&client).await?;
    let unhealthy_pod_count = fetch_unhealthy_pod_count(&client).await?;
    let flux = fetch_flux_rollup(&client).await;

    Ok(FleetContextSummary {
        context: context_name.to_owned(),
        health: classify(unhealthy_pod_count, node_ready_count, node_total_count),
        error: None,
        node_ready_count: Some(node_ready_count),
        node_total_count: Some(node_total_count),
        unhealthy_pod_count: Some(unhealthy_pod_count),
        cluster_version: Some(format!("v{}.{}", version.major, version.minor)),
        flux,
        last_updated: now_iso8601(),
    })
}

fn classify(unhealthy_pods: u32, ready: u32, total: u32) -> FleetHealthState {
    if unhealthy_pods > 0 || ready < total {
        FleetHealthState::Degraded
    } else {
        FleetHealthState::Healthy
    }
}

/// (ready, total) node counts, "Ready" meaning the `Ready` condition's status is `"True"`.
async fn fetch_node_counts(client: &Client) -> anyhow::Result<(u32, u32)> {
    let api = Api::<Node>::all(client.clone());
    let nodes = api.list(&ListParams::default()).await?;
    let total = nodes.items.len() as u32;
    let ready = nodes.items.iter().filter(|n| node_is_ready(n)).count() as u32;
    Ok((ready, total))
}

fn node_is_ready(node: &Node) -> bool {
    node.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conds| conds.iter().find(|c| c.type_ == "Ready"))
        .is_some_and(|c| c.status == "True")
}

/// Count of pods this dashboard classifies as unhealthy: `Failed`/`Unknown` phase, or
/// `Running` with at least one container reporting `ready: false`. No prior "unhealthy pod"
/// concept exists elsewhere in this codebase; this classification is defined fresh for this
/// feature and intentionally treats `Pending` as not-yet-unhealthy (a pod still scheduling is
/// not the same signal as one stuck failing).
async fn fetch_unhealthy_pod_count(client: &Client) -> anyhow::Result<u32> {
    let api = Api::<Pod>::all(client.clone());
    let pods = api.list(&ListParams::default()).await?;
    let count = pods.items.iter().filter(|p| pod_is_unhealthy(p)).count() as u32;
    Ok(count)
}

fn pod_is_unhealthy(pod: &Pod) -> bool {
    let Some(status) = pod.status.as_ref() else {
        return false;
    };
    let phase = status.phase.as_deref().unwrap_or("Unknown");
    match phase {
        "Failed" | "Unknown" => true,
        "Running" => status
            .container_statuses
            .as_ref()
            .is_some_and(|cs| cs.iter().any(|c| !c.ready)),
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

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_healthy_and_degraded() {
        assert_eq!(classify(0, 3, 3), FleetHealthState::Healthy);
        assert_eq!(classify(1, 3, 3), FleetHealthState::Degraded);
        assert_eq!(classify(0, 2, 3), FleetHealthState::Degraded);
    }

    #[test]
    fn failure_maps_to_unreachable_or_auth_with_verbatim_error() {
        let s = failure_summary("c", &anyhow::anyhow!("connection refused"));
        assert_eq!(s.health, FleetHealthState::Unreachable);
        assert_eq!(s.error.as_deref(), Some("connection refused"));
        assert!(s.unhealthy_pod_count.is_none());
        let s = failure_summary("c", &anyhow::anyhow!("401 Unauthorized"));
        assert_eq!(s.health, FleetHealthState::AuthError);
        assert!(s.error.is_some());
    }

    #[test]
    fn auth_error_detected_from_common_status_text() {
        assert!(is_auth_error("Unauthorized"));
        assert!(is_auth_error("HTTP status: 403 Forbidden"));
        assert!(!is_auth_error("connection refused"));
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
        assert!(pod_is_unhealthy(&pod));
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
        assert!(!pod_is_unhealthy(&pod));
    }
}
