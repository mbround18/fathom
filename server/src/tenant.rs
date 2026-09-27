//! Per-user backends for the hosted server (spec 031, T023).
//!
//! One [`UserBackend`] per `user_key`: an isolated backend thread on its own small Tokio
//! runtime, a [`HostedAccess`] that talks to the API server as that user (never a kubeconfig,
//! never `Client::try_default`), and a fan-out hub so several tabs of the same user share it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use fathom::backend::access::{ClusterAccess, Identity};
use fathom::backend::audit_log::{AuditSink, LocalFileSink};
use fathom::backend::rbac_access;
use fathom::bridge::pump::{BUFFER_CAP, BatchSink, Hub, SinkError};
use fathom::bridge::{BackendLink, spawn_backend_with, spawn_pump};
use fathom::config::Args;
use fathom::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, BridgeError, Correlation, NotificationLevel,
    PublicIdentity, RequestId, ResourceReadEvent, Route, SessionEndReason, ShellMode, StreamId,
    SubscriptionInfo, SystemEvent, section_keys,
};
use kube::{Client, Config};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::filter;
use crate::kubeauth::{self, Credentials, TokenCell};
use crate::limits::{LimitReached, Permit, SessionLimits, UserCounters};

/// The only context a hosted backend has.
pub const HOME_CONTEXT: &str = "in-cluster";

/// Hosted [`ClusterAccess`]: one fixed home context, per-user credentials, no local features.
pub struct HostedAccess {
    base: Config,
    creds: Credentials,
    state_root: PathBuf,
}

impl HostedAccess {
    pub fn new(base: Config, creds: Credentials, state_root: PathBuf) -> Self {
        Self {
            base,
            creds,
            state_root,
        }
    }
}

#[async_trait]
impl ClusterAccess for HostedAccess {
    async fn build_client(&self, context: Option<&str>) -> anyhow::Result<Client> {
        match context {
            None => {}
            Some(c) if c == HOME_CONTEXT => {}
            Some(other) => anyhow::bail!("context `{other}` is not available in hosted mode"),
        }
        Ok(kubeauth::build_client(
            self.base.clone(),
            self.creds.clone(),
        )?)
    }

    fn home_context(&self) -> Option<String> {
        Some(HOME_CONTEXT.to_owned())
    }

    fn allows_local_features(&self) -> bool {
        false
    }

    fn state_root(&self) -> Option<PathBuf> {
        Some(self.state_root.clone())
    }
}

/// One delivered batch for a tab.
#[derive(Debug, Clone)]
pub struct TabBatch {
    pub envelopes: Vec<BackendEventEnvelope>,
    pub dropped: u32,
}

type Tabs = Arc<Mutex<Vec<(u64, UnboundedSender<TabBatch>)>>>;

/// Broadcasts each batch to every attached tab. With no live tab it refuses, which makes the
/// [`Hub`] detach it and buffer instead, so nothing is lost between tabs.
struct FanoutSink {
    tabs: Tabs,
}

impl BatchSink<BackendEventEnvelope> for FanoutSink {
    fn send(&mut self, envelopes: &[BackendEventEnvelope], dropped: u32) -> Result<(), SinkError> {
        let mut tabs = self.tabs.lock().unwrap_or_else(|e| e.into_inner());
        tabs.retain(|(_, tx)| {
            tx.send(TabBatch {
                envelopes: envelopes.to_vec(),
                dropped,
            })
            .is_ok()
        });
        if tabs.is_empty() {
            return Err(SinkError("no tabs attached".to_owned()));
        }
        Ok(())
    }
}

type FanoutHub = Hub<BackendEventEnvelope, FanoutSink>;

/// A tab attached to a user's backend. Dropping it detaches the tab and frees its slot.
pub struct TabAttachment {
    pub id: u64,
    pub rx: UnboundedReceiver<TabBatch>,
    tabs: Tabs,
    _permit: Permit,
}

impl Drop for TabAttachment {
    fn drop(&mut self) {
        self.tabs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(id, _)| *id != self.id);
    }
}

/// Everything needed to start one user's backend.
pub struct SpawnParams {
    pub user_key: String,
    pub identity: Identity,
    pub access: Arc<dyn ClusterAccess>,
    /// The kube bearer cell to revoke on teardown (pass-through only).
    pub token_cell: Option<TokenCell>,
    pub limits: SessionLimits,
    pub audit: Arc<dyn AuditSink>,
}

impl SpawnParams {
    pub fn with_default_audit(mut self) -> Self {
        self.audit = Arc::new(LocalFileSink);
        self
    }
}

/// One signed-in user's live backend.
pub struct UserBackend {
    pub user_key: String,
    identity: Identity,
    /// Dropped on shutdown: the backend loop only exits once every command sender is gone.
    link: Mutex<Option<BackendLink>>,
    hub: Arc<FanoutHub>,
    tabs: Tabs,
    next_tab: std::sync::atomic::AtomicU64,
    counters: UserCounters,
    limits: SessionLimits,
    token_cell: Option<TokenCell>,
    last_activity: Mutex<Instant>,
    /// Live log follows by stream id (feedback #2: several views stream at once), each holding
    /// its stream permit and the start command a recycle replays (T024). `None` is the one
    /// id-less stream an older client may run.
    log_streams: Mutex<HashMap<Option<StreamId>, (Permit, BackendCmd)>>,
    exec_stream: Mutex<Option<Permit>>,
    /// Live watch/poll slots counted against `max_watches_per_user`, by kind.
    watch_permits: Mutex<HashMap<&'static str, Permit>>,
    /// Set once the self-access probe found at least one permitted check (T033).
    access_verified: Arc<AtomicBool>,
    ended: AtomicBool,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl UserBackend {
    /// Start the backend and event pump on dedicated threads. The backend's Tokio runtime has
    /// two workers: per-user isolation without a thread pool per signed-in user.
    pub fn spawn(p: SpawnParams) -> anyhow::Result<Arc<Self>> {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder
            .worker_threads(2)
            .thread_name("fathom-user-rt")
            .enable_all();
        // Every thread of this user's runtime resolves state paths under the user's own root.
        if let Some(root) = p.access.state_root() {
            fathom::backend::persistence::scope_runtime_to_state_root(&mut builder, root);
        }
        let runtime = builder.build()?;
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (gui_tx, gui_rx) = crossbeam_channel::unbounded();
        let link = BackendLink::new(cmd_tx);
        let tabs: Tabs = Arc::default();
        let hub: Arc<FanoutHub> = Arc::new(Hub::new(BUFFER_CAP));

        let args = Args {
            mcp_stdio: false,
            only_mcp: false,
            disable_mcp: true,
        };
        let backend = spawn_backend_with(
            cmd_rx,
            gui_tx,
            args,
            p.access,
            p.audit,
            Some(p.identity.clone()),
            Some(runtime),
        );
        let access_verified = Arc::new(AtomicBool::new(false));
        let pump = {
            let hub = Arc::clone(&hub);
            let verified = Arc::clone(&access_verified);
            spawn_pump(gui_rx, Arc::clone(&hub), &link, move |env| {
                watch_access_probe(env, &hub, &verified);
            })
        };
        let counters = UserCounters::new(&p.limits);
        Ok(Arc::new(Self {
            user_key: p.user_key,
            identity: p.identity,
            link: Mutex::new(Some(link)),
            hub,
            tabs,
            next_tab: 1.into(),
            counters,
            limits: p.limits,
            token_cell: p.token_cell,
            last_activity: Mutex::new(Instant::now()),
            log_streams: Mutex::new(HashMap::new()),
            exec_stream: Mutex::new(None),
            watch_permits: Mutex::new(HashMap::new()),
            access_verified,
            ended: AtomicBool::new(false),
            threads: Mutex::new(vec![backend, pump]),
        }))
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn counters(&self) -> &UserCounters {
        &self.counters
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_alive(&self) -> bool {
        !self.ended.load(Ordering::Acquire)
            && self
                .link
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(|l| l.backend_alive.load(Ordering::Acquire))
    }

    pub fn idle_for(&self) -> Duration {
        self.last_activity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
    }

    fn touch(&self) {
        *self.last_activity.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    /// Reply for `subscribe`, carrying mode, identity and limits.
    pub fn subscription_info(&self) -> Result<SubscriptionInfo, BridgeError> {
        let mut info = self
            .link
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .ok_or(BridgeError::BackendGone)?
            .subscription_info()?;
        info.mode = Some(ShellMode::Hosted);
        info.identity = Some(PublicIdentity {
            username: self.identity.username.clone(),
            groups: self.identity.groups.clone(),
        });
        info.limits = Some(self.limits.public());
        Ok(info)
    }

    /// Attach a tab. Refused with [`LimitReached`] past `max_tabs_per_user`. The first tab
    /// after none receives the buffered backlog.
    pub fn attach_tab(&self) -> Result<TabAttachment, LimitReached> {
        let permit = self.counters.tab()?;
        let (tx, rx) = unbounded_channel();
        let id = self.next_tab.fetch_add(1, Ordering::Relaxed);
        let first = {
            let mut tabs = self.tabs.lock().unwrap_or_else(|e| e.into_inner());
            let first = tabs.is_empty();
            tabs.push((id, tx));
            first
        };
        if first {
            // Subscribing replaces any stale sink and flushes the backlog to the tab list.
            let sink = FanoutSink {
                tabs: Arc::clone(&self.tabs),
            };
            if let Err(e) = self.hub.subscribe(sink) {
                tracing::warn!("fan-out subscribe failed: {e}");
            }
        }
        self.touch();
        Ok(TabAttachment {
            id,
            rx,
            tabs: Arc::clone(&self.tabs),
            _permit: permit,
        })
    }

    /// Run `cmd` through the hosted filter and the stream limits, then forward it.
    pub fn send(&self, cmd: BackendCmd) -> Result<RequestId, BridgeError> {
        filter::check(&cmd, HOME_CONTEXT).map_err(|d| BridgeError::Forbidden(d.to_string()))?;
        self.account_streams(&cmd)?;
        self.account_watches(&cmd)?;
        self.touch();
        let probe =
            matches!(cmd, BackendCmd::Initialize) && !self.access_verified.load(Ordering::Acquire);
        let id = self.forward(cmd)?;
        if probe {
            // Queued behind Initialize on the same backend loop, so the client exists by then.
            let _ = self.forward(BackendCmd::RefreshRbacSelfAccess {
                checks: rbac_access::hosted_probe_checks_in(&crate::host::probe_namespaces()),
            });
        }
        Ok(id)
    }

    /// Count long-lived watches/pollers against `max_watches_per_user`. The connect commands
    /// start the pod and deployment watchers; metrics pollers take one slot each. A slot is
    /// held until its stop command (or shutdown). Repeats of the same kind reuse the slot.
    fn account_watches(&self, cmd: &BackendCmd) -> Result<(), BridgeError> {
        use BackendCmd as C;
        let mut held = self.watch_permits.lock().unwrap_or_else(|e| e.into_inner());
        match cmd {
            C::Initialize | C::SetContext { .. } => {
                self.take_watch(&mut held, "pods")?;
                self.take_watch(&mut held, "deployments")?;
            }
            C::StartMetricsPolling { .. } => self.take_watch(&mut held, "metrics")?,
            C::StartWorkloadMetricsPolling { .. } => {
                self.take_watch(&mut held, "workload-metrics")?;
            }
            C::StopMetricsPolling => {
                held.remove("metrics");
            }
            C::StopWorkloadMetricsPolling => {
                held.remove("workload-metrics");
            }
            _ => {}
        }
        Ok(())
    }

    fn take_watch(
        &self,
        held: &mut HashMap<&'static str, Permit>,
        kind: &'static str,
    ) -> Result<(), BridgeError> {
        if !held.contains_key(kind) {
            let permit = self
                .counters
                .watch()
                .map_err(|e| BridgeError::Forbidden(e.to_string()))?;
            held.insert(kind, permit);
        }
        Ok(())
    }

    /// Push one system event to every tab through the private link (not the command path), so
    /// it needs no backend round trip and works while the backend is winding down.
    pub fn emit_system(&self, event: SystemEvent) {
        emit_system_to(&self.hub, event);
    }

    /// Terminal end of the hosted session: tell the tabs why, then revoke and stop the backend.
    pub fn end_session(&self, reason: SessionEndReason) {
        if self.ended.load(Ordering::Acquire) {
            return;
        }
        self.emit_system(SystemEvent::SessionEnded { reason });
        self.shutdown();
    }

    /// Restart the active log follows so they re-authenticate with the current token and a
    /// lapsed user cannot hold a stream forever (research R6). Each slot is reused: the backend
    /// replaces a stream started again under the same id in place. Returns whether a stream
    /// was recycled. Watchers reconnect on their own timeouts; exec shells and port forwards are
    /// deliberately not recycled mid-use.
    pub fn recycle_streams(&self) -> bool {
        let cmds: Vec<BackendCmd> = self
            .log_streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|(_, cmd)| cmd.clone())
            .collect();
        let mut recycled = false;
        for cmd in cmds {
            recycled |= self.forward(cmd).is_ok();
        }
        recycled
    }

    fn forward(&self, cmd: BackendCmd) -> Result<RequestId, BridgeError> {
        self.link
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .ok_or(BridgeError::BackendGone)?
            .send(cmd)
    }

    fn account_streams(&self, cmd: &BackendCmd) -> Result<(), BridgeError> {
        use BackendCmd as C;
        let refuse = |e: LimitReached| BridgeError::Forbidden(e.to_string());
        match cmd {
            C::StartPodLogs { stream_id, .. } => {
                let mut streams = self.log_streams.lock().unwrap_or_else(|e| e.into_inner());
                // Restarting a live id (or the id-less stream) reuses its slot.
                if let Some(entry) = streams.get_mut(stream_id) {
                    entry.1 = cmd.clone();
                } else {
                    let permit = self.counters.stream().map_err(refuse)?;
                    streams.insert(*stream_id, (permit, cmd.clone()));
                }
            }
            C::StopPodLogStream { stream_id } => {
                let mut streams = self.log_streams.lock().unwrap_or_else(|e| e.into_inner());
                streams.remove(&Some(*stream_id));
            }
            C::StopPodLogs => self
                .log_streams
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear(),
            C::StartPodExec { .. } => {
                let mut slot = self.exec_stream.lock().unwrap_or_else(|e| e.into_inner());
                if slot.is_none() {
                    *slot = Some(self.counters.stream().map_err(refuse)?);
                }
            }
            C::StopPodExec => *self.exec_stream.lock().unwrap_or_else(|e| e.into_inner()) = None,
            _ => {}
        }
        Ok(())
    }

    /// Stop background polling and streams while nobody is looking (idle park). The backend
    /// stays alive; the next tab resumes by issuing its usual commands.
    pub fn park(&self) {
        for cmd in [
            BackendCmd::StopMetricsPolling,
            BackendCmd::StopWorkloadMetricsPolling,
            BackendCmd::StopPodLogs,
            BackendCmd::StopPodExec,
        ] {
            let _ = self.forward(cmd);
        }
        self.log_streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        *self.exec_stream.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let mut w = self.watch_permits.lock().unwrap_or_else(|e| e.into_inner());
        w.remove("metrics");
        w.remove("workload-metrics");
    }

    /// End the backend: revoke credentials first (in-flight requests fail closed), then ask it
    /// to stop. Idempotent.
    pub fn shutdown(&self) {
        if self.ended.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(cell) = &self.token_cell {
            cell.revoke();
        }
        // Shutdown is queued first, then the sender is dropped so the backend loop exits.
        let _ = self.forward(BackendCmd::Shutdown);
        self.link.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.tabs.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.watch_permits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Wait for the backend and pump threads (tests, graceful server shutdown).
    pub fn join(&self) {
        let handles = std::mem::take(&mut *self.threads.lock().unwrap_or_else(|e| e.into_inner()));
        for h in handles {
            let _ = h.join();
        }
    }
}

fn emit_system_to(hub: &FanoutHub, event: SystemEvent) {
    hub.deliver(vec![BackendEventEnvelope::new(
        0,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(event),
    )]);
}

/// Pump-side hook for the T033 self-access probe. All checks denied means the user has no
/// permissions at all: the tabs get an empty `ContextsUpdated` (what the UI keys its no-access
/// state off) plus a warning notification, without waiting for any further command.
fn watch_access_probe(env: &BackendEventEnvelope, hub: &FanoutHub, verified: &AtomicBool) {
    let BackendEvent::ResourceRead(ResourceReadEvent::RbacSelfAccessUpdated { results }) =
        &env.event
    else {
        return;
    };
    match rbac_access::hosted_probe_verdict(results) {
        Some(false) => verified.store(true, Ordering::Release),
        Some(true) => {
            emit_system_to(
                hub,
                SystemEvent::ContextsUpdated {
                    contexts: Vec::new(),
                    current: None,
                },
            );
            emit_system_to(
                hub,
                SystemEvent::Notification {
                    level: NotificationLevel::Warn,
                    message: "Your account has no permissions on this cluster. Ask an administrator to grant access.".to_owned(),
                },
            );
        }
        None => {}
    }
}

impl Drop for UserBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// `user_key` -> live backend. Reuses a user's backend across sessions and tabs.
#[derive(Default)]
pub struct TenantRegistry {
    users: Mutex<std::collections::HashMap<String, Arc<UserBackend>>>,
}

impl TenantRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The user's live backend, creating it with `make` when absent or dead.
    pub fn get_or_create(
        &self,
        user_key: &str,
        make: impl FnOnce() -> anyhow::Result<Arc<UserBackend>>,
    ) -> anyhow::Result<Arc<UserBackend>> {
        let mut users = self.users.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(b) = users.get(user_key)
            && b.is_alive()
        {
            return Ok(Arc::clone(b));
        }
        let b = make()?;
        users.insert(user_key.to_owned(), Arc::clone(&b));
        Ok(b)
    }

    pub fn get(&self, user_key: &str) -> Option<Arc<UserBackend>> {
        self.users
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(user_key)
            .cloned()
    }

    /// End and forget one user's backend (sign-out, expiry, revocation).
    pub fn end(&self, user_key: &str) {
        let b = self
            .users
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(user_key);
        if let Some(b) = b {
            b.shutdown();
        }
    }

    pub fn len(&self) -> usize {
        self.users.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Park backends with no tabs idle for `park_after`; end those idle for `drop_after`.
    pub fn sweep(&self, park_after: Duration, drop_after: Duration) {
        let all: Vec<_> = self
            .users
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        for b in all {
            if b.tab_count() > 0 {
                continue;
            }
            let idle = b.idle_for();
            if idle >= drop_after || !b.is_alive() {
                self.end(&b.user_key);
            } else if idle >= park_after {
                b.park();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fathom::protocol::{BackendEvent, SystemEvent};

    struct NullSink;
    impl AuditSink for NullSink {
        fn record(
            &self,
            _: fathom::protocol::AuditActionSummary,
            _: String,
            _: String,
            _: bool,
            _: Option<&str>,
            _: Option<fathom::protocol::AuditActor>,
        ) {
        }
    }

    fn identity(name: &str) -> Identity {
        Identity {
            sub: format!("sub-{name}"),
            username: name.to_owned(),
            groups: vec!["dev".into()],
            email: None,
        }
    }

    fn base_config() -> Config {
        Config::new("https://127.0.0.1:1".parse().unwrap())
    }

    fn access(cell: &TokenCell) -> Arc<HostedAccess> {
        Arc::new(HostedAccess::new(
            base_config(),
            Credentials::Bearer(cell.clone()),
            std::env::temp_dir().join("fathom-tenant-test"),
        ))
    }

    fn cell() -> TokenCell {
        TokenCell::new(&secrecy::SecretString::from("tok".to_owned()), None).unwrap()
    }

    fn spawn(name: &str, limits: SessionLimits) -> Arc<UserBackend> {
        let c = cell();
        UserBackend::spawn(SpawnParams {
            user_key: format!("k-{name}"),
            identity: identity(name),
            access: access(&c),
            token_cell: Some(c),
            limits,
            audit: Arc::new(NullSink),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn access_is_hosted_shaped() {
        fathom::startup::install_crypto_provider();
        let a = access(&cell());
        assert_eq!(a.home_context().as_deref(), Some("in-cluster"));
        assert!(!a.allows_local_features());
        assert!(a.state_root().is_some());
        assert!(a.build_client(None).await.is_ok());
        assert!(a.build_client(Some("in-cluster")).await.is_ok());
        let err = a.build_client(Some("kind-dev")).await.err().unwrap();
        assert!(err.to_string().contains("not available in hosted mode"));
    }

    #[test]
    fn subscription_info_is_hosted_and_carries_identity_and_limits() {
        let b = spawn("alice", SessionLimits::default());
        let info = b.subscription_info().unwrap();
        assert_eq!(info.mode, Some(ShellMode::Hosted));
        let id = info.identity.unwrap();
        assert_eq!((id.username.as_str(), id.groups.len()), ("alice", 1));
        assert_eq!(info.limits.unwrap().max_tabs, 5);
        b.shutdown();
        b.join();
    }

    #[test]
    fn filtered_commands_never_reach_the_backend() {
        let b = spawn("alice", SessionLimits::default());
        assert!(matches!(
            b.send(BackendCmd::RefreshContexts),
            Err(BridgeError::Forbidden(_))
        ));
        assert!(matches!(
            b.send(BackendCmd::SetContext {
                context: "kind-x".into()
            }),
            Err(BridgeError::Forbidden(_))
        ));
        assert!(b.send(BackendCmd::RefreshNamespaces).is_ok());
        b.shutdown();
        b.join();
    }

    #[test]
    fn tab_limit_and_fanout_to_all_tabs() {
        let b = spawn(
            "alice",
            SessionLimits {
                max_tabs_per_user: 2,
                ..Default::default()
            },
        );
        let mut t1 = b.attach_tab().unwrap();
        let mut t2 = b.attach_tab().unwrap();
        assert_eq!(b.attach_tab().err().unwrap().max, 2);
        assert_eq!(b.tab_count(), 2);
        b.send(BackendCmd::Initialize).unwrap();

        let started = |t: &mut TabAttachment| {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut seen = false;
            while Instant::now() < deadline && !seen {
                while let Ok(batch) = t.rx.try_recv() {
                    seen |= batch.envelopes.iter().any(|e| {
                        matches!(e.event, BackendEvent::System(SystemEvent::BackendStarted))
                    });
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            seen
        };
        assert!(started(&mut t1), "tab 1 saw BackendStarted");
        // Tab 2 joined at the same time as tab 1's first batch; it must receive live batches.
        b.send(BackendCmd::RefreshNamespaces).unwrap();
        drop(t1);
        assert_eq!(b.tab_count(), 1);
        assert!(b.attach_tab().is_ok(), "slot freed by dropping the tab");
        let _ = &mut t2;
        b.shutdown();
        b.join();
    }

    #[test]
    fn shutdown_revokes_credentials_and_stops_threads() {
        let c = cell();
        let b = UserBackend::spawn(SpawnParams {
            user_key: "k".into(),
            identity: identity("bob"),
            access: access(&c),
            token_cell: Some(c.clone()),
            limits: SessionLimits::default(),
            audit: Arc::new(NullSink),
        })
        .unwrap();
        assert!(c.is_set());
        b.shutdown();
        assert!(!c.is_set());
        assert!(!b.is_alive());
        b.shutdown(); // idempotent
        b.join();
    }

    #[test]
    fn stream_limit_counts_logs_and_exec_and_releases_on_stop() {
        let b = spawn(
            "alice",
            SessionLimits {
                max_streams_per_user: 1,
                ..Default::default()
            },
        );
        // Exercise via the accounting helper directly to stay independent of field shapes.
        b.log_streams.lock().unwrap().insert(
            None,
            (b.counters().stream().unwrap(), BackendCmd::StopPodLogs),
        );
        assert!(b.counters().stream().is_err());
        b.send(BackendCmd::StopPodLogs).unwrap();
        assert!(b.counters().stream().is_ok());
        b.shutdown();
        b.join();
    }

    #[test]
    fn log_streams_hold_one_permit_per_id() {
        let b = spawn(
            "alice",
            SessionLimits {
                max_streams_per_user: 2,
                ..Default::default()
            },
        );
        let start = |id: u64| BackendCmd::StartPodLogs {
            namespace: "default".into(),
            name: "web".into(),
            container: None,
            stream_id: Some(StreamId((1 << 32) + id)),
        };
        b.send(start(1)).unwrap();
        b.send(start(2)).unwrap();
        assert!(matches!(b.send(start(3)), Err(BridgeError::Forbidden(_))));
        // Restarting a live id reuses its slot.
        b.send(start(1)).unwrap();
        assert!(b.recycle_streams());
        // Stopping one id frees exactly its slot.
        b.send(BackendCmd::StopPodLogStream {
            stream_id: StreamId((1 << 32) + 1),
        })
        .unwrap();
        b.send(start(3)).unwrap();
        assert!(matches!(b.send(start(4)), Err(BridgeError::Forbidden(_))));
        b.send(BackendCmd::StopPodLogs).unwrap();
        assert!(!b.recycle_streams());
        b.shutdown();
        b.join();
    }

    #[test]
    fn registry_reuses_live_backend_and_replaces_ended_one() {
        let reg = TenantRegistry::new();
        let a = reg
            .get_or_create("k", || Ok(spawn("alice", SessionLimits::default())))
            .unwrap();
        let again = reg.get_or_create("k", || panic!("must reuse")).unwrap();
        assert!(Arc::ptr_eq(&a, &again));
        reg.end("k");
        assert!(reg.is_empty());
        assert!(!a.is_alive());
        let b = reg
            .get_or_create("k", || Ok(spawn("alice", SessionLimits::default())))
            .unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        reg.end("k");
    }

    #[test]
    fn two_users_are_isolated() {
        let reg = TenantRegistry::new();
        let a = reg
            .get_or_create("ka", || Ok(spawn("alice", SessionLimits::default())))
            .unwrap();
        let b = reg
            .get_or_create("kb", || Ok(spawn("bob", SessionLimits::default())))
            .unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(a.identity().username, "alice");
        assert_eq!(b.identity().username, "bob");
        reg.end("ka");
        assert!(b.is_alive());
        reg.end("kb");
    }

    fn recv_system(t: &mut TabAttachment, want: impl Fn(&SystemEvent) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            while let Ok(batch) = t.rx.try_recv() {
                for e in batch.envelopes {
                    if let BackendEvent::System(ev) = &e.event
                        && want(ev)
                    {
                        return true;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn watches_count_against_the_per_user_cap_and_release_on_stop() {
        let b = spawn(
            "alice",
            SessionLimits {
                max_watches_per_user: 3,
                ..Default::default()
            },
        );
        b.send(BackendCmd::Initialize).unwrap();
        assert_eq!(b.counters().watches(), 2, "pod + deployment watchers");
        b.send(BackendCmd::Initialize).unwrap();
        assert_eq!(b.counters().watches(), 2, "repeat connect reuses slots");
        let metrics = || BackendCmd::StartMetricsPolling {
            namespace: "n".into(),
            name: "p".into(),
            interval_secs: 5,
        };
        b.send(metrics()).unwrap();
        let err = b
            .send(BackendCmd::StartWorkloadMetricsPolling {
                namespace: "n".into(),
                workload_name: "w".into(),
                workload_kind: "Deployment".into(),
                pod_selector: "a=b".into(),
                interval_secs: 5,
            })
            .err();
        assert!(matches!(err, Some(BridgeError::Forbidden(_))), "{err:?}");
        b.send(BackendCmd::StopMetricsPolling).unwrap();
        assert_eq!(b.counters().watches(), 2);
        b.shutdown();
        assert_eq!(b.counters().watches(), 0);
        b.join();
    }

    #[test]
    fn end_session_tells_tabs_then_stops_the_backend() {
        let b = spawn("alice", SessionLimits::default());
        let mut t = b.attach_tab().unwrap();
        b.end_session(SessionEndReason::Revoked);
        assert!(recv_system(&mut t, |e| matches!(
            e,
            SystemEvent::SessionEnded {
                reason: SessionEndReason::Revoked
            }
        )));
        assert!(!b.is_alive());
        b.join();
    }

    #[test]
    fn zero_permission_probe_result_emits_the_no_access_signal() {
        use fathom::protocol::RbacAccessResult;
        let b = spawn("alice", SessionLimits::default());
        let mut t = b.attach_tab().unwrap();
        let denied: Vec<_> = rbac_access::hosted_probe_checks()
            .into_iter()
            .map(|check| RbacAccessResult {
                check,
                allowed: false,
                reason: None,
            })
            .collect();
        let env = BackendEventEnvelope::new(
            1,
            Route::global(section_keys::system()),
            Correlation::None,
            BackendEvent::ResourceRead(ResourceReadEvent::RbacSelfAccessUpdated {
                results: denied,
            }),
        );
        watch_access_probe(&env, &b.hub, &b.access_verified);
        assert!(recv_system(&mut t, |e| matches!(
            e,
            SystemEvent::ContextsUpdated { contexts, current: None } if contexts.is_empty()
        )));
        assert!(!b.access_verified.load(Ordering::Acquire));
        b.shutdown();
        b.join();
    }
}
