//! 034-simultaneous-federation: an opt-in set of contexts held live at once.
//!
//! Each federated context owns one persistent `kube::Client` and four bounded watches (Pods,
//! Nodes, Deployments, Warning Events) that are reduced to counters as they arrive; no object
//! bodies are retained. Per-context state changes are debounced (500 ms) and emitted only when
//! the rollup actually changed. The set is empty by default and, while empty, owns no client
//! and no task (SC-003). Read-only: nothing here mutates a cluster or touches the active
//! context (`ClusterContext.client`).
//!
//! Tier arbitration: a federated context is live-tier, so the 027 fleet poller skips it
//! (`fleet::sync_pollers_with`); un-federating it resumes polling when the dashboard is open.
//! Client construction, the unhealthy-pod classifier, the concurrency cap and the start
//! stagger are shared with `fleet.rs`.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Sender;
use futures::StreamExt;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Event as CoreEvent, Node, Pod};
use kube::Client;
use kube::api::Api;
use kube::runtime::WatchStreamExt;
use kube::runtime::watcher::{self, Config as WatchConfig, Event as WatchEvent};

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, FEDERATION_SOFT_CAP, FederationCombinedRollup,
    FederationConnectionState, FederationContextRollup, FleetContextSummary, FleetHealthState,
    Route, SystemEvent, section_keys,
};

use super::events;
use super::fleet;
use super::persistence::{self, FederationFile};
use super::state::{BackendState, FederationHandle};

/// Per-context emit debounce (FR: rollups coalesce within 500 ms).
pub(super) const DEBOUNCE: Duration = Duration::from_millis(500);
const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_secs(60);
/// Bound on building a client and reading the version during (re)connect.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A running connection failing continuously this long is reported `Unreachable`.
///
/// SC-004 wants the error on screen within 15 s. Live validation (T031) measured the watch
/// error arriving within ~1 s of the control plane dying, so the budget is this threshold plus
/// one `DEGRADED_TICK`; the old 30 s showed the error only after ~30 s.
const UNREACHABLE_AFTER: Duration = Duration::from_secs(10);
/// Recompute cadence: keeps the 1 h warning window and the pending-age rule moving.
const IDLE_TICK: Duration = Duration::from_secs(60);
const DEGRADED_TICK: Duration = Duration::from_secs(2);
const PENDING_UNHEALTHY_AFTER: chrono::Duration = chrono::Duration::minutes(5);

// ---------------------------------------------------------------------------------------
// Backoff and debounce (pure / paused-time testable)
// ---------------------------------------------------------------------------------------

/// Exponential backoff with "equal" jitter: half the capped exponential plus a `jitter`
/// (0.0..=1.0) share of the other half.
pub(super) fn backoff_delay(attempt: u32, jitter: f64) -> Duration {
    let exp = BACKOFF_BASE
        .saturating_mul(1u32 << attempt.min(16))
        .min(BACKOFF_CAP);
    let half = exp / 2;
    half + half.mul_f64(jitter.clamp(0.0, 1.0))
}

fn jitter() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(chrono::Utc::now().timestamp_subsec_nanos().into());
    (h.finish() % 1000) as f64 / 1000.0
}

/// Wait for a change notification, then hold for `window` so a burst collapses into one wake.
pub(super) async fn debounced_wait(notify: &tokio::sync::Notify, window: Duration) {
    notify.notified().await;
    tokio::time::sleep(window).await;
}

// ---------------------------------------------------------------------------------------
// Counter reducers
// ---------------------------------------------------------------------------------------

/// One watched kind's reduced records. A relist (`Init` .. `InitDone`) is staged and swapped in
/// atomically so objects deleted while the watch was down disappear.
pub(super) struct KindStore<V> {
    map: HashMap<String, V>,
    staging: Option<HashMap<String, V>>,
    synced: bool,
}

impl<V> Default for KindStore<V> {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
            staging: None,
            synced: false,
        }
    }
}

impl<V> KindStore<V> {
    fn init(&mut self) {
        self.staging = Some(HashMap::new());
    }
    fn upsert(&mut self, key: String, value: V) {
        match self.staging.as_mut() {
            Some(staging) => staging.insert(key, value),
            None => self.map.insert(key, value),
        };
    }
    fn done(&mut self) {
        if let Some(staging) = self.staging.take() {
            self.map = staging;
        }
        self.synced = true;
    }
    fn delete(&mut self, key: &str) {
        self.map.remove(key);
        if let Some(staging) = self.staging.as_mut() {
            staging.remove(key);
        }
    }
    fn counts(&self) -> Option<&HashMap<String, V>> {
        self.synced.then_some(&self.map)
    }

    /// Apply one watcher event; `reduce` maps an object to its record (or `None` to skip).
    fn apply<K>(
        &mut self,
        event: WatchEvent<K>,
        key: impl Fn(&K) -> String,
        reduce: impl Fn(&K) -> Option<V>,
    ) {
        match event {
            WatchEvent::Init => self.init(),
            WatchEvent::InitApply(obj) | WatchEvent::Apply(obj) => {
                let k = key(&obj);
                match reduce(&obj) {
                    Some(v) => self.upsert(k, v),
                    None => self.delete(&k),
                }
            }
            WatchEvent::InitDone => self.done(),
            WatchEvent::Delete(obj) => self.delete(&key(&obj)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PodPhase {
    Running,
    Pending,
    Failed,
    Other,
}

#[derive(Clone, Debug)]
pub(super) struct PodRec {
    phase: PodPhase,
    /// Unhealthy by the 027 classifier at observation time.
    unhealthy: bool,
    /// A young `Pending` pod is not yet unhealthy; it becomes so once older than 5 minutes.
    pending_since: Option<chrono::DateTime<chrono::Utc>>,
}

fn object_key(namespace: Option<&String>, name: Option<&String>) -> String {
    format!(
        "{}/{}",
        namespace.map_or("", String::as_str),
        name.map_or("", String::as_str)
    )
}

pub(super) fn pod_rec(pod: &Pod, now: chrono::DateTime<chrono::Utc>) -> PodRec {
    let phase = match pod
        .status
        .as_ref()
        .and_then(|s| s.phase.as_deref())
        .unwrap_or("Unknown")
    {
        "Running" => PodPhase::Running,
        "Pending" => PodPhase::Pending,
        "Failed" => PodPhase::Failed,
        _ => PodPhase::Other,
    };
    let unhealthy = fleet::pod_is_unhealthy(pod, now);
    let pending_since = (phase == PodPhase::Pending && !unhealthy)
        .then(|| pod.metadata.creation_timestamp.as_ref().map(|t| t.0))
        .flatten();
    PodRec {
        phase,
        unhealthy,
        pending_since,
    }
}

/// Deployment is unavailable when fewer replicas are available than desired.
pub(super) fn deployment_unavailable(d: &Deployment) -> bool {
    let desired = d.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
    let available = d
        .status
        .as_ref()
        .and_then(|s| s.available_replicas)
        .unwrap_or(0);
    available < desired
}

fn warning_time(e: &CoreEvent) -> Option<chrono::DateTime<chrono::Utc>> {
    if e.type_.as_deref() != Some("Warning") {
        return None;
    }
    e.last_timestamp
        .as_ref()
        .map(|t| t.0)
        .or_else(|| e.event_time.as_ref().map(|t| t.0))
        .or_else(|| e.metadata.creation_timestamp.as_ref().map(|t| t.0))
        .or(Some(chrono::DateTime::<chrono::Utc>::MIN_UTC))
}

/// Everything one live context has reduced so far, plus its connection status.
pub(super) struct Counters {
    pods: KindStore<PodRec>,
    nodes: KindStore<bool>,
    deployments: KindStore<bool>,
    warnings: KindStore<chrono::DateTime<chrono::Utc>>,
    restricted: BTreeSet<&'static str>,
    version: Option<String>,
    state: FederationConnectionState,
    error: Option<String>,
    auth_error: bool,
    /// Set while a watch is failing (non-403); cleared by the next good event.
    degraded_since: Option<std::time::Instant>,
}

impl Counters {
    pub(super) fn new() -> Self {
        Self {
            pods: KindStore::default(),
            nodes: KindStore::default(),
            deployments: KindStore::default(),
            warnings: KindStore::default(),
            restricted: BTreeSet::new(),
            version: None,
            state: FederationConnectionState::Pending,
            error: None,
            auth_error: false,
            degraded_since: None,
        }
    }

    fn on_pod(&mut self, e: WatchEvent<Pod>, now: chrono::DateTime<chrono::Utc>) {
        self.pods.apply(
            e,
            |p| object_key(p.metadata.namespace.as_ref(), p.metadata.name.as_ref()),
            |p| Some(pod_rec(p, now)),
        );
    }
    fn on_node(&mut self, e: WatchEvent<Node>) {
        self.nodes.apply(
            e,
            |n| object_key(None, n.metadata.name.as_ref()),
            |n| Some(fleet::node_is_ready(n)),
        );
    }
    fn on_deployment(&mut self, e: WatchEvent<Deployment>) {
        self.deployments.apply(
            e,
            |d| object_key(d.metadata.namespace.as_ref(), d.metadata.name.as_ref()),
            |d| Some(deployment_unavailable(d)),
        );
    }
    fn on_event(&mut self, e: WatchEvent<CoreEvent>) {
        self.warnings.apply(
            e,
            |ev| object_key(ev.metadata.namespace.as_ref(), ev.metadata.name.as_ref()),
            warning_time,
        );
    }

    /// Build the wire rollup. Numbers are `None` unless the state is `Live`/`Reconnecting`, the
    /// metric is not restricted, and its watch has finished its first list.
    pub(super) fn rollup(
        &self,
        context: &str,
        generation: u64,
        now: chrono::DateTime<chrono::Utc>,
    ) -> FederationContextRollup {
        let numbers = matches!(
            self.state,
            FederationConnectionState::Live | FederationConnectionState::Reconnecting
        );
        let allowed = |name: &str| numbers && !self.restricted.contains(name);
        let nodes = self.nodes.counts().filter(|_| allowed("nodes"));
        let pods = self.pods.counts().filter(|_| allowed("pods"));
        let deployments = self.deployments.counts().filter(|_| allowed("deployments"));
        let warnings = self.warnings.counts().filter(|_| allowed("events"));
        let n = |c: usize| u32::try_from(c).unwrap_or(u32::MAX);
        let pod_count =
            |f: &dyn Fn(&PodRec) -> bool| pods.map(|m| n(m.values().filter(|r| f(r)).count()));
        FederationContextRollup {
            context: context.to_owned(),
            state: self.state,
            generation,
            error: matches!(
                self.state,
                FederationConnectionState::Unreachable | FederationConnectionState::AuthError
            )
            .then(|| self.error.clone())
            .flatten(),
            nodes_ready: nodes.map(|m| n(m.values().filter(|r| **r).count())),
            nodes_total: nodes.map(|m| n(m.len())),
            pods_running: pod_count(&|r| r.phase == PodPhase::Running),
            pods_pending: pod_count(&|r| r.phase == PodPhase::Pending),
            pods_failed: pod_count(&|r| r.phase == PodPhase::Failed),
            pods_unhealthy: pod_count(&|r| {
                r.unhealthy
                    || r.pending_since
                        .is_some_and(|t| now - t > PENDING_UNHEALTHY_AFTER)
            }),
            deployments_unavailable: deployments.map(|m| n(m.values().filter(|u| **u).count())),
            warning_events_1h: warnings.map(|m| {
                n(m.values()
                    .filter(|t| now - **t <= chrono::Duration::hours(1))
                    .count())
            }),
            cluster_version: if numbers { self.version.clone() } else { None },
            restricted: self.restricted.iter().map(|s| (*s).to_owned()).collect(),
            last_updated: now.to_rfc3339(),
        }
    }
}

/// Rollups equal ignoring the timestamp (emit on change only).
fn same_rollup(a: &FederationContextRollup, b: &FederationContextRollup) -> bool {
    let mut a = a.clone();
    a.last_updated = b.last_updated.clone();
    a == *b
}

/// 027-compatible card for a live context (`live: true`).
fn fleet_summary(r: &FederationContextRollup) -> FleetContextSummary {
    let health = match r.state {
        FederationConnectionState::Live | FederationConnectionState::Reconnecting => {
            if r.pods_unhealthy.is_some_and(|n| n > 0)
                || matches!((r.nodes_ready, r.nodes_total), (Some(a), Some(b)) if a < b)
            {
                FleetHealthState::Degraded
            } else {
                FleetHealthState::Healthy
            }
        }
        FederationConnectionState::Pending => FleetHealthState::Pending,
        FederationConnectionState::Unreachable => FleetHealthState::Unreachable,
        FederationConnectionState::AuthError => FleetHealthState::AuthError,
    };
    let restricted: Vec<String> = r
        .restricted
        .iter()
        .filter(|g| g.as_str() == "nodes" || g.as_str() == "pods")
        .cloned()
        .collect();
    FleetContextSummary {
        context: r.context.clone(),
        health,
        error: r.error.clone(),
        node_ready_count: r.nodes_ready,
        node_total_count: r.nodes_total,
        unhealthy_pod_count: r.pods_unhealthy,
        cluster_version: r.cluster_version.clone(),
        flux: None,
        restricted,
        last_updated: r.last_updated.clone(),
        live: true,
        plugin_error: None,
    }
}

// ---------------------------------------------------------------------------------------
// Publishing
// ---------------------------------------------------------------------------------------

#[derive(Clone)]
struct Publisher {
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    latest: Arc<Mutex<HashMap<String, FederationContextRollup>>>,
}

impl Publisher {
    /// Store and emit `rollup` if it differs from the last one (ignoring the timestamp) and its
    /// context is still federated at this generation. Returns whether it was emitted.
    fn publish(&self, rollup: FederationContextRollup) -> bool {
        let combined = {
            let Ok(mut latest) = self.latest.lock() else {
                return false;
            };
            match latest.get(&rollup.context) {
                // Removed (or re-federated at a newer generation): stale, drop it.
                None => return false,
                Some(prev) if prev.generation > rollup.generation => return false,
                Some(prev)
                    if prev.generation == rollup.generation && same_rollup(prev, &rollup) =>
                {
                    return false;
                }
                Some(_) => {}
            }
            latest.insert(rollup.context.clone(), rollup.clone());
            FederationCombinedRollup::combine(
                u32::try_from(latest.len()).unwrap_or(u32::MAX),
                latest.values(),
            )
        };
        fleet::emit_summary(&self.seq, &self.sender, fleet_summary(&rollup));
        emit_rollup(&self.seq, &self.sender, rollup, combined);
        true
    }
}

fn emit_rollup(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    context: FederationContextRollup,
    combined: FederationCombinedRollup,
) {
    events::emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::FederationRollupUpdated { context, combined }),
    );
}

fn emit_state(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let mut federated: Vec<String> = state.federation.handles.keys().cloned().collect();
    federated.sort();
    let over_cap = u32::try_from(federated.len()).unwrap_or(u32::MAX) > FEDERATION_SOFT_CAP;
    events::emit(
        &state.cluster.seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::FederationStateUpdated {
            federated,
            soft_cap: FEDERATION_SOFT_CAP,
            over_cap,
        }),
    );
}

// ---------------------------------------------------------------------------------------
// Per-context live tier
// ---------------------------------------------------------------------------------------

type SharedCounters = Arc<Mutex<Counters>>;

fn with<R>(c: &SharedCounters, f: impl FnOnce(&mut Counters) -> R) -> Option<R> {
    c.lock().ok().map(|mut g| f(&mut g))
}

/// HTTP status of a watcher error, when it carries one.
fn watch_error_code(e: &watcher::Error) -> Option<u16> {
    match e {
        watcher::Error::InitialListFailed(kube::Error::Api(r))
        | watcher::Error::WatchStartFailed(kube::Error::Api(r)) => Some(r.code),
        watcher::Error::WatchError(r) => Some(r.code),
        _ => None,
    }
}

/// Run one watch until the stream ends. `handle` folds an event into the counters.
async fn run_watch<K>(
    client: Client,
    config: WatchConfig,
    kind: &'static str,
    counters: SharedCounters,
    notify: Arc<tokio::sync::Notify>,
    handle: impl Fn(&mut Counters, WatchEvent<K>) + Send + 'static,
) where
    K: kube::Resource<DynamicType = ()>
        + Clone
        + std::fmt::Debug
        + serde::de::DeserializeOwned
        + Send
        + 'static,
{
    let api = Api::<K>::all(client);
    let mut stream = watcher::watcher(api, config).default_backoff().boxed();
    while let Some(item) = stream.next().await {
        match item {
            Ok(event) => {
                with(&counters, |c| {
                    c.degraded_since = None;
                    handle(c, event);
                });
                notify.notify_one();
            }
            Err(e) if watch_error_code(&e) == Some(403) => {
                // No RBAC access to this kind: mark it restricted and stop watching it.
                with(&counters, |c| c.restricted.insert(kind));
                notify.notify_one();
                return;
            }
            Err(e) => {
                with(&counters, |c| {
                    if c.degraded_since.is_none() {
                        c.degraded_since = Some(std::time::Instant::now());
                    }
                    c.auth_error = watch_error_code(&e) == Some(401);
                    c.error = Some(format!("{e}"));
                });
                notify.notify_one();
            }
        }
    }
}

/// Fold connection health into `Counters.state` and return the wire rollup.
fn evaluate(counters: &SharedCounters, context: &str, generation: u64) -> FederationContextRollup {
    let now = chrono::Utc::now();
    with(counters, |c| {
        c.state = match c.degraded_since {
            None => FederationConnectionState::Live,
            Some(t) if t.elapsed() >= UNREACHABLE_AFTER => {
                if c.auth_error {
                    FederationConnectionState::AuthError
                } else {
                    FederationConnectionState::Unreachable
                }
            }
            Some(_) => FederationConnectionState::Reconnecting,
        };
        c.rollup(context, generation, now)
    })
    .unwrap_or_else(|| {
        status_rollup(
            context,
            generation,
            FederationConnectionState::Pending,
            None,
        )
    })
}

/// A rollup with no numbers, for connection-level states.
pub(super) fn status_rollup(
    context: &str,
    generation: u64,
    state: FederationConnectionState,
    error: Option<String>,
) -> FederationContextRollup {
    let mut c = Counters::new();
    c.state = state;
    c.error = error;
    c.rollup(context, generation, chrono::Utc::now())
}

/// Connect (build client + version), retrying with backoff until it works.
async fn connect_with_backoff(
    context: &str,
    generation: u64,
    publisher: &Publisher,
    semaphore: &Arc<tokio::sync::Semaphore>,
) -> Option<(Client, String)> {
    let mut attempt: u32 = 0;
    loop {
        let result = {
            let _permit = semaphore.acquire().await.ok()?;
            tokio::time::timeout(CONNECT_TIMEOUT, async {
                let client = fleet::client_for_context(context).await?;
                let v = client.apiserver_version().await?;
                anyhow::Ok((client, format!("v{}.{}", v.major, v.minor)))
            })
            .await
        };
        let err = match result {
            Ok(Ok(ok)) => return Some(ok),
            Ok(Err(e)) => e,
            Err(_) => anyhow::anyhow!(
                "Timed out after {}s connecting to context",
                CONNECT_TIMEOUT.as_secs()
            ),
        };
        let state = if fleet::is_auth_error(&err) {
            FederationConnectionState::AuthError
        } else {
            FederationConnectionState::Unreachable
        };
        publisher.publish(status_rollup(
            context,
            generation,
            state,
            Some(format!("{err:#}")),
        ));
        tokio::time::sleep(backoff_delay(attempt, jitter())).await;
        attempt = attempt.saturating_add(1);
        publisher.publish(status_rollup(
            context,
            generation,
            FederationConnectionState::Reconnecting,
            None,
        ));
    }
}

/// The supervisor for one federated context: stagger, connect, then run the four watches and
/// the debounced emit loop until aborted.
async fn run_context(
    context: String,
    generation: u64,
    start_delay: Duration,
    publisher: Publisher,
    semaphore: Arc<tokio::sync::Semaphore>,
) {
    if !start_delay.is_zero() {
        tokio::time::sleep(start_delay).await;
    }
    let Some((client, version)) =
        connect_with_backoff(&context, generation, &publisher, &semaphore).await
    else {
        return;
    };

    let counters: SharedCounters = Arc::new(Mutex::new(Counters::new()));
    with(&counters, |c| c.version = Some(version));
    let notify = Arc::new(tokio::sync::Notify::new());

    // Dropping the set (task abort) aborts every watch with it.
    let mut watches = tokio::task::JoinSet::new();
    watches.spawn(run_watch::<Pod>(
        client.clone(),
        WatchConfig::default(),
        "pods",
        counters.clone(),
        notify.clone(),
        |c, e| c.on_pod(e, chrono::Utc::now()),
    ));
    watches.spawn(run_watch::<Node>(
        client.clone(),
        WatchConfig::default(),
        "nodes",
        counters.clone(),
        notify.clone(),
        Counters::on_node,
    ));
    watches.spawn(run_watch::<Deployment>(
        client.clone(),
        WatchConfig::default(),
        "deployments",
        counters.clone(),
        notify.clone(),
        Counters::on_deployment,
    ));
    watches.spawn(run_watch::<CoreEvent>(
        client,
        WatchConfig::default().fields("type=Warning"),
        "events",
        counters.clone(),
        notify.clone(),
        Counters::on_event,
    ));

    publisher.publish(evaluate(&counters, &context, generation));
    loop {
        let degraded = with(&counters, |c| c.degraded_since.is_some()).unwrap_or(false);
        let tick = if degraded { DEGRADED_TICK } else { IDLE_TICK };
        tokio::select! {
            () = debounced_wait(&notify, DEBOUNCE) => {}
            () = tokio::time::sleep(tick) => {}
        }
        publisher.publish(evaluate(&counters, &context, generation));
    }
}

// ---------------------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------------------

fn persist(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let mut contexts: Vec<String> = state.federation.handles.keys().cloned().collect();
    contexts.sort();
    let file = FederationFile {
        contexts,
        ..FederationFile::default()
    };
    let result = match &state.federation.path {
        Some(p) => persistence::save_federation_to(p, &file),
        None => persistence::save_federation(&file),
    };
    if let Err(e) = result {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Failed to save federation.json: {e:#}"),
        );
    }
}

/// Start (or record as unavailable) the live tier for `context`. Idempotent.
fn spawn_context(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
    index: usize,
    exists: bool,
) {
    if state.federation.handles.contains_key(context) {
        return;
    }
    state.federation.next_generation += 1;
    let generation = state.federation.next_generation;
    // The 027 poller must not also run for this context.
    fleet::stop_poller(state, context);

    let publisher = Publisher {
        seq: state.cluster.seq.clone(),
        sender: sender.clone(),
        latest: state.federation.latest.clone(),
    };
    let initial = if exists {
        status_rollup(
            context,
            generation,
            FederationConnectionState::Pending,
            None,
        )
    } else {
        status_rollup(
            context,
            generation,
            FederationConnectionState::Unreachable,
            Some("Context not found in kubeconfig".to_owned()),
        )
    };
    if let Ok(mut latest) = state.federation.latest.lock() {
        latest.insert(context.to_owned(), initial.clone());
    }
    state
        .federation
        .handles
        .insert(context.to_owned(), FederationHandle { tasks: Vec::new() });
    // Announce the (Pending / unavailable) row.
    let combined = current_combined(state);
    fleet::emit_summary(&state.cluster.seq, sender, fleet_summary(&initial));
    emit_rollup(&state.cluster.seq, sender, initial, combined);

    if exists {
        let task = tokio::spawn(run_context(
            context.to_owned(),
            generation,
            fleet::start_offset(index),
            publisher,
            state.fleet.poll_semaphore.clone(),
        ));
        if let Some(h) = state.federation.handles.get_mut(context) {
            h.tasks.push(task);
        }
    }
}

fn current_combined(state: &BackendState) -> FederationCombinedRollup {
    state.federation.latest.lock().map_or_else(
        |_| FederationCombinedRollup::default(),
        |l| {
            FederationCombinedRollup::combine(
                u32::try_from(l.len()).unwrap_or(u32::MAX),
                l.values(),
            )
        },
    )
}

/// `FederateContext`.
pub(super) fn federate(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
) {
    let known = fleet::all_kubeconfig_contexts();
    federate_with(state, sender, context, &known);
}

fn federate_with(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
    known: &[String],
) {
    if state.federation.is_federated(&context) {
        emit_state(state, sender);
        return;
    }
    if !known.contains(&context) {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Cannot federate context \"{context}\": not found in kubeconfig"),
        );
        return;
    }
    let index = state.federation.handles.len();
    spawn_context(state, sender, &context, index, true);
    persist(state, sender);
    emit_state(state, sender);
}

/// `UnfederateContext`: abort tasks, drop the client, forget the row, persist, and let the 027
/// poller resume for this context when the dashboard is open.
pub(super) fn unfederate(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
) {
    let Some(handle) = state.federation.handles.remove(context) else {
        emit_state(state, sender);
        return;
    };
    for task in handle.tasks {
        task.abort();
    }
    if let Ok(mut latest) = state.federation.latest.lock() {
        latest.remove(context);
    }
    persist(state, sender);
    emit_state(state, sender);
    fleet::sync_pollers(state, sender);
}

/// `RequestFederationSnapshot`: re-emit the set and every current rollup.
pub(super) fn snapshot(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    emit_state(state, sender);
    let rollups: Vec<FederationContextRollup> = state
        .federation
        .latest
        .lock()
        .map(|l| l.values().cloned().collect())
        .unwrap_or_default();
    let combined = current_combined(state);
    for r in rollups {
        emit_rollup(&state.cluster.seq, sender, r, combined.clone());
    }
}

/// On `Initialize`: reconnect only the previously chosen contexts, staggered. Contexts that
/// are no longer in the kubeconfig stay in the set as unavailable until the operator removes
/// them. A no-op for an empty set (no client, no task).
pub(super) fn restore(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    let file = match &state.federation.path {
        Some(p) => persistence::load_federation_from(p),
        None => persistence::load_federation(),
    };
    let known = fleet::all_kubeconfig_contexts();
    restore_from(state, sender, &file.contexts, &known);
}

fn restore_from(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    contexts: &[String],
    known: &[String],
) {
    for (index, context) in contexts.iter().enumerate() {
        spawn_context(state, sender, context, index, known.contains(context));
    }
    emit_state(state, sender);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};

    fn test_sender() -> (
        Sender<BackendEventEnvelope>,
        crossbeam_channel::Receiver<BackendEventEnvelope>,
    ) {
        unbounded()
    }

    fn pod(name: &str, phase: &str, ready: Option<bool>, age_min: i64) -> Pod {
        use k8s_openapi::api::core::v1::{ContainerStatus, PodStatus};
        Pod {
            metadata: ObjectMeta {
                name: Some(name.into()),
                namespace: Some("d".into()),
                creation_timestamp: Some(Time(
                    chrono::Utc::now() - chrono::Duration::minutes(age_min),
                )),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some(phase.into()),
                container_statuses: ready.map(|r| {
                    vec![ContainerStatus {
                        ready: r,
                        ..Default::default()
                    }]
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn live_counters() -> Counters {
        let mut c = Counters::new();
        c.state = FederationConnectionState::Live;
        c
    }

    #[test]
    fn pod_counters_by_phase_and_unhealthy() {
        let now = chrono::Utc::now();
        let mut c = live_counters();
        c.on_pod(WatchEvent::Init, now);
        c.on_pod(
            WatchEvent::InitApply(pod("a", "Running", Some(true), 60)),
            now,
        );
        c.on_pod(
            WatchEvent::InitApply(pod("b", "Running", Some(false), 60)),
            now,
        );
        c.on_pod(WatchEvent::InitApply(pod("c", "Pending", None, 1)), now);
        c.on_pod(WatchEvent::InitApply(pod("d", "Pending", None, 30)), now);
        c.on_pod(WatchEvent::InitApply(pod("e", "Failed", None, 30)), now);
        // Not synced yet: no numbers.
        assert_eq!(c.rollup("x", 1, now).pods_running, None);
        c.on_pod(WatchEvent::InitDone, now);
        let r = c.rollup("x", 1, now);
        assert_eq!(r.pods_running, Some(2));
        assert_eq!(r.pods_pending, Some(2));
        assert_eq!(r.pods_failed, Some(1));
        // b (not ready), d (pending > 5m), e (failed); c is a young pending pod.
        assert_eq!(r.pods_unhealthy, Some(3));
        // The young pending pod turns unhealthy as time passes, without a new event.
        let later = now + chrono::Duration::minutes(10);
        assert_eq!(c.rollup("x", 1, later).pods_unhealthy, Some(4));
        c.on_pod(
            WatchEvent::Delete(pod("b", "Running", Some(false), 60)),
            now,
        );
        assert_eq!(c.rollup("x", 1, now).pods_running, Some(1));
    }

    #[test]
    fn relist_drops_objects_deleted_during_gap() {
        let now = chrono::Utc::now();
        let mut c = live_counters();
        c.on_pod(WatchEvent::Init, now);
        c.on_pod(
            WatchEvent::InitApply(pod("a", "Running", Some(true), 9)),
            now,
        );
        c.on_pod(
            WatchEvent::InitApply(pod("b", "Running", Some(true), 9)),
            now,
        );
        c.on_pod(WatchEvent::InitDone, now);
        // Relist after a 410: only "a" comes back.
        c.on_pod(WatchEvent::Init, now);
        c.on_pod(
            WatchEvent::InitApply(pod("a", "Running", Some(true), 9)),
            now,
        );
        // Staged: the previous view is still served until InitDone.
        assert_eq!(c.rollup("x", 1, now).pods_running, Some(2));
        c.on_pod(WatchEvent::InitDone, now);
        assert_eq!(c.rollup("x", 1, now).pods_running, Some(1));
    }

    #[test]
    fn node_deployment_and_warning_reducers() {
        use k8s_openapi::api::apps::v1::{DeploymentSpec, DeploymentStatus};
        use k8s_openapi::api::core::v1::{NodeCondition, NodeStatus};
        let now = chrono::Utc::now();
        let mut c = live_counters();
        let node = |n: &str, ready: &str| Node {
            metadata: ObjectMeta {
                name: Some(n.into()),
                ..Default::default()
            },
            status: Some(NodeStatus {
                conditions: Some(vec![NodeCondition {
                    type_: "Ready".into(),
                    status: ready.into(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        c.on_node(WatchEvent::Init);
        c.on_node(WatchEvent::InitApply(node("n1", "True")));
        c.on_node(WatchEvent::InitApply(node("n2", "False")));
        c.on_node(WatchEvent::InitDone);
        let dep = |n: &str, want: i32, have: i32| Deployment {
            metadata: ObjectMeta {
                name: Some(n.into()),
                namespace: Some("d".into()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(want),
                ..Default::default()
            }),
            status: Some(DeploymentStatus {
                available_replicas: Some(have),
                ..Default::default()
            }),
        };
        c.on_deployment(WatchEvent::Init);
        c.on_deployment(WatchEvent::InitApply(dep("ok", 2, 2)));
        c.on_deployment(WatchEvent::InitApply(dep("bad", 3, 1)));
        c.on_deployment(WatchEvent::InitDone);
        let ev = |n: &str, ty: &str, mins: i64| CoreEvent {
            metadata: ObjectMeta {
                name: Some(n.into()),
                namespace: Some("d".into()),
                ..Default::default()
            },
            type_: Some(ty.into()),
            last_timestamp: Some(Time(now - chrono::Duration::minutes(mins))),
            ..Default::default()
        };
        c.on_event(WatchEvent::Init);
        c.on_event(WatchEvent::InitApply(ev("w1", "Warning", 5)));
        c.on_event(WatchEvent::InitApply(ev("w2", "Warning", 120)));
        c.on_event(WatchEvent::InitApply(ev("n1", "Normal", 1)));
        c.on_event(WatchEvent::InitDone);
        let r = c.rollup("x", 1, now);
        assert_eq!((r.nodes_ready, r.nodes_total), (Some(1), Some(2)));
        assert_eq!(r.deployments_unavailable, Some(1));
        assert_eq!(r.warning_events_1h, Some(1));
    }

    #[test]
    fn restricted_metric_is_none_and_listed() {
        let now = chrono::Utc::now();
        let mut c = live_counters();
        c.on_node(WatchEvent::Init);
        c.on_node(WatchEvent::InitDone);
        c.restricted.insert("nodes");
        let r = c.rollup("x", 1, now);
        assert_eq!(r.nodes_total, None);
        assert_eq!(r.restricted, vec!["nodes".to_owned()]);
    }

    #[test]
    fn non_live_states_carry_no_numbers() {
        let now = chrono::Utc::now();
        let mut c = live_counters();
        c.on_node(WatchEvent::Init);
        c.on_node(WatchEvent::InitDone);
        c.state = FederationConnectionState::Unreachable;
        c.error = Some("boom".into());
        let r = c.rollup("x", 1, now);
        assert_eq!(r.nodes_total, None);
        assert_eq!(r.error.as_deref(), Some("boom"));
        c.state = FederationConnectionState::Live;
        assert_eq!(c.rollup("x", 1, now).error, None);
        // Reconnecting keeps last-known numbers.
        c.state = FederationConnectionState::Reconnecting;
        assert_eq!(c.rollup("x", 1, now).nodes_total, Some(0));
    }

    #[test]
    fn backoff_is_exponential_capped_and_jittered() {
        assert_eq!(backoff_delay(0, 0.0), Duration::from_millis(500));
        assert_eq!(backoff_delay(0, 1.0), Duration::from_secs(1));
        assert_eq!(backoff_delay(2, 1.0), Duration::from_secs(4));
        assert_eq!(backoff_delay(3, 0.0), Duration::from_secs(4));
        assert_eq!(backoff_delay(30, 1.0), BACKOFF_CAP);
        assert_eq!(backoff_delay(30, 0.0), BACKOFF_CAP / 2);
        assert!(backoff_delay(4, 0.3) > backoff_delay(4, 0.0));
    }

    #[tokio::test(start_paused = true)]
    async fn burst_of_notifications_wakes_once_after_window() {
        let notify = Arc::new(tokio::sync::Notify::new());
        let n2 = notify.clone();
        let start = tokio::time::Instant::now();
        let waiter = tokio::spawn(async move {
            debounced_wait(&n2, DEBOUNCE).await;
            start.elapsed()
        });
        for _ in 0..5 {
            notify.notify_one();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let elapsed = waiter.await.unwrap();
        assert_eq!(elapsed, DEBOUNCE);
    }

    #[test]
    fn publish_emits_on_change_only_and_drops_stale_generations() {
        let (tx, rx) = test_sender();
        let publisher = Publisher {
            seq: Arc::new(AtomicU64::new(0)),
            sender: tx,
            latest: Arc::new(Mutex::new(HashMap::new())),
        };
        let mk = |generation, state| status_rollup("a", generation, state, None);
        // Unknown (never federated / already removed) context: dropped.
        assert!(!publisher.publish(mk(1, FederationConnectionState::Live)));
        publisher
            .latest
            .lock()
            .unwrap()
            .insert("a".into(), mk(2, FederationConnectionState::Pending));
        assert!(!publisher.publish(mk(1, FederationConnectionState::Live)));
        assert!(publisher.publish(mk(2, FederationConnectionState::Live)));
        assert!(!publisher.publish(mk(2, FederationConnectionState::Live)));
        // Fleet card + rollup, once.
        assert_eq!(rx.try_iter().count(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn empty_set_spawns_no_clients_or_tasks() {
        let (tx, rx) = test_sender();
        let mut state = BackendState::new();
        restore_from(&mut state, &tx, &[], &["a".to_owned()]);
        assert!(state.federation.handles.is_empty());
        assert!(state.federation.latest.lock().unwrap().is_empty());
        assert!(state.fleet.pollers.is_empty());
        // Only the initial state event.
        assert_eq!(rx.try_iter().count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn federate_validates_persists_and_unfederate_removes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("federation.json");
        let (tx, rx) = test_sender();
        let mut state = BackendState::new();
        state.federation.path = Some(path.clone());
        let known = vec!["a".to_owned(), "b".to_owned()];

        federate_with(&mut state, &tx, "nope".into(), &known);
        assert!(state.federation.handles.is_empty());
        assert!(!path.exists());
        assert!(rx.try_iter().any(|e| matches!(
            e.event,
            BackendEvent::System(SystemEvent::Notification { .. })
        )));

        federate_with(&mut state, &tx, "a".into(), &known);
        assert_eq!(state.federation.handles["a"].tasks.len(), 1);
        assert_eq!(persistence::load_federation_from(&path).contexts, vec!["a"]);

        unfederate(&mut state, &tx, "a");
        assert!(state.federation.handles.is_empty());
        assert!(state.federation.latest.lock().unwrap().is_empty());
        assert!(persistence::load_federation_from(&path).contexts.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn restore_keeps_missing_context_as_unavailable_without_task() {
        let (tx, rx) = test_sender();
        let mut state = BackendState::new();
        restore_from(&mut state, &tx, &["gone".to_owned()], &["a".to_owned()]);
        assert!(state.federation.handles["gone"].tasks.is_empty());
        let r = state.federation.latest.lock().unwrap()["gone"].clone();
        assert_eq!(r.state, FederationConnectionState::Unreachable);
        assert!(r.error.unwrap().contains("not found"));
        assert!(rx.try_iter().count() >= 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tier_arbiter_never_double_polls() {
        let (tx, _rx) = test_sender();
        let mut state = BackendState::new();
        let hang = |c: String| async move {
            std::future::pending::<()>().await;
            fleet::pending_summary(c)
        };
        state.fleet.active = true;
        // "a" is polled; promoting it to live stops its poller and keeps it out.
        state.fleet.pollers.insert(
            "a".to_owned(),
            tokio::spawn(async { std::future::pending::<()>().await }),
        );
        state
            .federation
            .handles
            .insert("a".to_owned(), FederationHandle { tasks: vec![] });
        fleet::stop_poller(&mut state, "a");
        fleet::sync_pollers_with(&mut state, &tx, vec!["a".to_owned(), "b".to_owned()], hang);
        let mut polled: Vec<_> = state.fleet.pollers.keys().cloned().collect();
        polled.sort();
        assert_eq!(polled, vec!["b"]);
        // Un-federate "a": the next sync polls it again.
        state.federation.handles.remove("a");
        fleet::sync_pollers_with(&mut state, &tx, vec!["a".to_owned(), "b".to_owned()], hang);
        let mut polled: Vec<_> = state.fleet.pollers.keys().cloned().collect();
        polled.sort();
        assert_eq!(polled, vec!["a", "b"]);
        state.fleet.abort_all();
    }

    #[test]
    fn unreachable_error_surfaces_within_sc004_budget() {
        // Worst case: the failure starts just after a tick, so the state flips on the first
        // degraded tick at or after `UNREACHABLE_AFTER` (SC-004: error within 15 s).
        assert!(UNREACHABLE_AFTER + DEGRADED_TICK <= Duration::from_secs(15));
    }
}
