//! T041, T087, T048: the live stack inventory.
//!
//! [`Watch::inventory`] reads the stacks root and `docker ps` once and joins them
//! ([`crate::inventory::build`]). While anyone is subscribed, two followers feed a debounced
//! re-inventory:
//!
//! - `docker events` ([`ContainerRuntime::follow_events`]) on its own thread, restarted with
//!   backoff when it fails;
//! - a `notify` watcher on the stacks root and on each stack directory (not recursive: data
//!   directories inside a stack can be large and busy), reacting only to compose files, `.env`,
//!   `global.env` and stack directories appearing or going.
//!
//! Changes within [`DEBOUNCE`] become one re-inventory, broadcast as a full [`StackList`] (never
//! a delta) and only when it differs from the last one. Nothing runs, and docker is not called,
//! while nobody is subscribed (SC-008): the first subscription starts the followers and dropping
//! the last stops them.
//!
//! When docker does not answer (FR-010), the list keeps the last containers seen, marked
//! `stale` with the `as_of` of that reading, and [`Update::Unavailable`] carries docker's stderr
//! verbatim. The next good reading clears both.
//!
//! 051 FR-001/FR-002: each reading also rates every compose container's containment. The
//! `docker inspect` inputs are cached by container id: a reading inspects only the ids it has
//! not seen (in one batched call) and forgets the ones that are gone, and the daemon's mode is
//! re-read only when new containers appear or it is not known yet. A container whose inspect
//! failed carries no rating, which the shell gate treats as unknown (refused).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use depths_core::discovery::{ComposeFile, ENV_FILE, GLOBAL_ENV_FILE};
use depths_core::ports::{
    CancelToken, ContainerRuntime, RuntimeError, RuntimeSnapshot, StackListing, StackStore,
};
use depths_core::security::{DaemonMode, Inspect, ShellPolicy};
use depths_protocol::StackList;
use notify::{EventKind, RecursiveMode, Watcher as _};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::inventory;

/// Changes closer together than this become one re-inventory.
pub const DEBOUNCE: Duration = Duration::from_millis(250);
/// The longest wait before `docker events` is tried again.
const EVENTS_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// What subscribers receive.
#[derive(Debug, Clone)]
pub enum Update {
    /// The whole inventory, unfiltered: each socket filters it for its person.
    Stacks(Arc<StackList>),
    /// docker stopped answering; its stderr, verbatim.
    Unavailable(String),
}

/// One reading.
#[derive(Debug, Clone)]
pub struct Inventory {
    pub list: Arc<StackList>,
    /// docker's stderr when it did not answer.
    pub unavailable: Option<String>,
}

#[derive(Clone)]
pub struct Watch {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<dyn StackStore>,
    runtime: Arc<dyn ContainerRuntime>,
    tx: broadcast::Sender<Update>,
    debounce: Duration,
    state: Mutex<State>,
    /// `DEPTHS_SHELL_POLICY`, for the per-container shell rule the inventory carries.
    shell_policy: ShellPolicy,
    /// 051: inspect results by full container id, and the daemon's mode.
    security: Mutex<SecurityCache>,
}

#[derive(Default)]
struct SecurityCache {
    by_id: BTreeMap<String, Inspect>,
    daemon: Option<DaemonMode>,
}

#[derive(Default)]
struct State {
    subscribers: usize,
    running: Option<Running>,
    last: Option<Arc<StackList>>,
    /// The last good `docker ps`, and when it was read.
    last_runtime: Option<(RuntimeSnapshot, String)>,
    unavailable: Option<String>,
}

/// The followers. Dropping this stops them.
struct Running {
    cancel: CancelToken,
    kick: mpsc::UnboundedSender<()>,
    _stop: oneshot::Sender<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// A live feed of [`Update`]s. The followers run while at least one exists.
pub struct Subscription {
    pub rx: broadcast::Receiver<Update>,
    _guard: Guard,
}

struct Guard(Arc<Inner>);

impl Drop for Guard {
    fn drop(&mut self) {
        let mut st = self.0.state.lock().unwrap();
        st.subscribers = st.subscribers.saturating_sub(1);
        if st.subscribers == 0 {
            st.running = None;
        }
    }
}

impl Watch {
    pub fn new(store: Arc<dyn StackStore>, runtime: Arc<dyn ContainerRuntime>) -> Self {
        Self::with_debounce(store, runtime, DEBOUNCE)
    }

    pub fn with_debounce(
        store: Arc<dyn StackStore>,
        runtime: Arc<dyn ContainerRuntime>,
        debounce: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                runtime,
                tx: broadcast::channel(16).0,
                debounce,
                state: Mutex::new(State::default()),
                shell_policy: ShellPolicy::default(),
                security: Mutex::new(SecurityCache::default()),
            }),
        }
    }

    /// Sets the shell policy the inventory's ratings carry. Call before the watch is shared.
    #[must_use]
    pub fn with_shell_policy(mut self, policy: ShellPolicy) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("with_shell_policy before the watch is shared")
            .shell_policy = policy;
        self
    }

    pub fn shell_policy(&self) -> ShellPolicy {
        self.inner.shell_policy
    }

    /// The daemon's mode as last read; `None` before any reading or when it could not be read.
    pub fn daemon(&self) -> Option<DaemonMode> {
        self.inner.security.lock().unwrap().daemon
    }

    pub fn store(&self) -> &Arc<dyn StackStore> {
        &self.inner.store
    }

    pub fn runtime(&self) -> &Arc<dyn ContainerRuntime> {
        &self.inner.runtime
    }

    /// Reads the inventory now. Subscribers get the result too when it changed.
    pub async fn inventory(&self) -> Inventory {
        refresh(self.inner.clone()).await.0
    }

    /// The last reading, if there has been one.
    pub fn last(&self) -> Option<Arc<StackList>> {
        self.inner.state.lock().unwrap().last.clone()
    }

    /// docker's stderr when the last reading found it unreachable.
    pub fn unavailable(&self) -> Option<String> {
        self.inner.state.lock().unwrap().unavailable.clone()
    }

    /// Asks for a re-inventory (after an operation, say). Ignored while nobody is subscribed.
    pub fn kick(&self) {
        if let Some(r) = &self.inner.state.lock().unwrap().running {
            let _ = r.kick.send(());
        }
    }

    pub fn subscribers(&self) -> usize {
        self.inner.state.lock().unwrap().subscribers
    }

    /// Subscribes, starting the followers if this is the first subscription.
    pub fn subscribe(&self) -> Subscription {
        let rx = self.inner.tx.subscribe();
        let mut st = self.inner.state.lock().unwrap();
        st.subscribers += 1;
        if st.running.is_none() {
            st.running = Some(start(&self.inner));
        }
        Subscription {
            rx,
            _guard: Guard(self.inner.clone()),
        }
    }
}

fn reason_of(e: RuntimeError) -> String {
    let r = match e {
        RuntimeError::Unavailable(r) | RuntimeError::Failed(r) => r,
        RuntimeError::Unsupported => "the container runtime cannot list containers".into(),
    };
    if r.trim().is_empty() {
        "docker did not answer".into()
    } else {
        r
    }
}

/// The list without its reading time, to tell a real change from a re-read.
fn same(a: &StackList, b: &StackList) -> bool {
    let strip = |l: &StackList| StackList {
        as_of: String::new(),
        ..l.clone()
    };
    strip(a) == strip(b)
}

/// One reading, and the directories the file watcher should follow.
async fn refresh(inner: Arc<Inner>) -> (Inventory, Vec<PathBuf>) {
    let task = inner.clone();
    let read = tokio::task::spawn_blocking(move || {
        let listing = task.store.list();
        let runtime = task.runtime.containers_by_project();
        if let Ok(snap) = &runtime {
            inspect_new(&task, snap);
        }
        (listing, runtime)
    })
    .await;
    let (listing, runtime) = match read {
        Ok(r) => r,
        Err(e) => (
            Err(depths_core::ports::StoreError::Io {
                message: e.to_string(),
            }),
            Err(RuntimeError::Failed("the inventory task failed".into())),
        ),
    };
    let mut warnings = Vec::new();
    let listing = listing.unwrap_or_else(|e| {
        warnings.push(format!("cannot read the stacks directory: {e}"));
        StackListing {
            root: inner.store.root().to_path_buf(),
            global_env: false,
            stacks: Vec::new(),
        }
    });
    let dirs: Vec<PathBuf> = std::iter::once(listing.root.clone())
        .chain(listing.stacks.iter().map(|e| listing.root.join(&e.name)))
        .collect();

    let (inv, changed, newly_unavailable) = {
        let mut st = inner.state.lock().unwrap();
        let (list, unavailable) = match runtime {
            Ok(snap) => {
                let as_of = crate::audit::now();
                let list = inventory::build(&listing, &snap, false, as_of.clone());
                st.last_runtime = Some((snap, as_of));
                (list, None)
            }
            Err(e) => {
                let reason = reason_of(e);
                let list = match &st.last_runtime {
                    Some((snap, as_of)) => inventory::build(&listing, snap, true, as_of.clone()),
                    None => inventory::build(
                        &listing,
                        &RuntimeSnapshot::default(),
                        true,
                        crate::audit::now(),
                    ),
                };
                (list, Some(reason))
            }
        };
        let mut list = list;
        list.warnings.extend(warnings);
        rate(
            &mut list,
            &inner.security.lock().unwrap(),
            inner.shell_policy,
        );
        let changed = st.last.as_deref().is_none_or(|l| !same(l, &list));
        let newly_unavailable = unavailable.is_some() && st.unavailable != unavailable;
        let list = Arc::new(list);
        st.last = Some(list.clone());
        st.unavailable.clone_from(&unavailable);
        (Inventory { list, unavailable }, changed, newly_unavailable)
    };
    if changed {
        let _ = inner.tx.send(Update::Stacks(inv.list.clone()));
    }
    if newly_unavailable && let Some(r) = &inv.unavailable {
        tracing::warn!(reason = %r, "container runtime unavailable");
        let _ = inner.tx.send(Update::Unavailable(r.clone()));
    }
    (inv, dirs)
}

/// Inspects the containers this snapshot has that the cache has not seen, in one call, and
/// forgets the ones that are gone. Blocking: runs on the inventory's blocking task.
fn inspect_new(inner: &Inner, snap: &RuntimeSnapshot) {
    let ids: BTreeSet<&str> = snap
        .projects
        .values()
        .flat_map(|p| p.containers.iter().map(|c| c.id.as_str()))
        .collect();
    let new: Vec<String> = {
        let mut cache = inner.security.lock().unwrap();
        cache.by_id.retain(|id, _| ids.contains(id.as_str()));
        ids.iter()
            .filter(|id| !cache.by_id.contains_key(**id))
            .map(|id| (*id).to_owned())
            .collect()
    };
    let daemon_known = inner.security.lock().unwrap().daemon.is_some();
    let daemon = (!new.is_empty() || !daemon_known).then(|| inner.runtime.daemon_mode());
    let inspected = if new.is_empty() {
        Ok(Vec::new())
    } else {
        inner.runtime.inspect_security(&new)
    };
    let mut cache = inner.security.lock().unwrap();
    match daemon {
        Some(Ok(d)) => cache.daemon = Some(d),
        Some(Err(RuntimeError::Unsupported)) | None => {}
        Some(Err(e)) => tracing::debug!(error = ?e, "cannot read the daemon's security options"),
    }
    match inspected {
        Ok(found) => {
            for i in found {
                // `docker inspect` answers with full ids; `docker ps` may have listed a prefix.
                let key = new
                    .iter()
                    .find(|n| i.id == **n || i.id.starts_with(n.as_str()))
                    .cloned()
                    .unwrap_or_else(|| i.id.clone());
                cache.by_id.insert(key, i);
            }
        }
        Err(RuntimeError::Unsupported) => {}
        // Left uncached: those containers carry no rating and the next reading tries again.
        Err(e) => tracing::warn!(error = ?e, "cannot inspect containers for their security"),
    }
}

/// Rates each compose container from the cache, and each stack by its worst known container.
fn rate(list: &mut StackList, cache: &SecurityCache, policy: ShellPolicy) {
    let daemon = cache.daemon.unwrap_or_default();
    for stack in &mut list.stacks {
        let mut known = Vec::new();
        for c in stack
            .services
            .iter_mut()
            .flat_map(|s| s.containers.iter_mut())
        {
            c.security = cache.by_id.get(&c.container.id).map(|i| {
                let p = depths_core::security::profile(i, daemon);
                known.push(p.level);
                crate::security::container(&p, policy)
            });
        }
        stack.security = (!known.is_empty())
            .then(|| crate::security::level(depths_core::security::rollup(known)));
    }
}

fn start(inner: &Arc<Inner>) -> Running {
    let cancel = CancelToken::new();
    let (kick, kicks) = mpsc::unbounded_channel();
    let (stop, stopped) = oneshot::channel();
    {
        let runtime = inner.runtime.clone();
        let kick = kick.clone();
        let cancel = cancel.clone();
        // A plain thread: it blocks on `docker events` and must not hold up runtime shutdown.
        let spawned = std::thread::Builder::new()
            .name("depths-events".into())
            .spawn(move || follow(runtime.as_ref(), &kick, &cancel));
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "cannot start the docker events follower");
        }
    }
    tokio::spawn(debouncer(inner.clone(), kicks, kick.clone(), stopped));
    Running {
        cancel,
        kick,
        _stop: stop,
    }
}

/// Follows `docker events` until cancelled, retrying with backoff.
fn follow(runtime: &dyn ContainerRuntime, kick: &mpsc::UnboundedSender<()>, cancel: &CancelToken) {
    let mut backoff = Duration::from_secs(1);
    let mut failed = false;
    while !cancel.is_cancelled() {
        let seen = AtomicBool::new(false);
        if failed {
            // Coming back: re-read, so a recovered daemon clears the banner.
            let _ = kick.send(());
        }
        let on_event = || {
            seen.store(true, Ordering::Relaxed);
            let _ = kick.send(());
        };
        match runtime.follow_events(&on_event, cancel) {
            Ok(()) => return,
            Err(RuntimeError::Unsupported) => return,
            Err(e) => {
                if seen.load(Ordering::Relaxed) {
                    backoff = Duration::from_secs(1);
                }
                tracing::warn!(error = %e, retry_in = ?backoff, "docker events stopped");
                failed = true;
                let _ = kick.send(());
                sleep_unless_cancelled(backoff, cancel);
                backoff = (backoff * 2).min(EVENTS_BACKOFF_MAX);
            }
        }
    }
}

fn sleep_unless_cancelled(d: Duration, cancel: &CancelToken) {
    let step = Duration::from_millis(50);
    let mut left = d;
    while !left.is_zero() && !cancel.is_cancelled() {
        let s = left.min(step);
        std::thread::sleep(s);
        left -= s;
    }
}

/// Whether a file event at `path` can change the inventory.
fn relevant(root: &Path, path: &Path) -> bool {
    if path.parent() == Some(root) {
        // A stack directory appearing, going or being renamed, or `global.env`.
        return true;
    }
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        n == ENV_FILE
            || n == GLOBAL_ENV_FILE
            || ComposeFile::ORDER.iter().any(|f| f.file_name() == n)
    })
}

async fn debouncer(
    inner: Arc<Inner>,
    mut kicks: mpsc::UnboundedReceiver<()>,
    kick: mpsc::UnboundedSender<()>,
    mut stopped: oneshot::Receiver<()>,
) {
    let root = inner.store.root().to_path_buf();
    let fs_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        // Reads (including Depths' own) are not changes; a closed-after-write file is.
        if let EventKind::Access(a) = ev.kind
            && a != notify::event::AccessKind::Close(notify::event::AccessMode::Write)
        {
            return;
        }
        if ev.paths.iter().any(|p| relevant(&fs_root, p)) {
            let _ = kick.send(());
        }
    })
    .inspect_err(|e| tracing::warn!(error = %e, "cannot watch the stacks directory"))
    .ok();
    let mut watched: BTreeSet<PathBuf> = BTreeSet::new();
    let mut sync = |watcher: &mut Option<notify::RecommendedWatcher>, dirs: Vec<PathBuf>| {
        let Some(w) = watcher.as_mut() else { return };
        let want: BTreeSet<PathBuf> = dirs.into_iter().collect();
        for gone in watched.difference(&want) {
            let _ = w.unwatch(gone);
        }
        for new in want.difference(&watched) {
            if let Err(e) = w.watch(new, RecursiveMode::NonRecursive) {
                tracing::debug!(dir = %new.display(), error = %e, "cannot watch");
            }
        }
        watched = want;
    };
    // Follow at least the root from the start; stack directories join after the first reading.
    sync(&mut watcher, vec![root]);

    loop {
        tokio::select! {
            _ = &mut stopped => return,
            k = kicks.recv() => if k.is_none() { return },
        }
        tokio::select! {
            _ = &mut stopped => return,
            () = tokio::time::sleep(inner.debounce) => {}
        }
        while kicks.try_recv().is_ok() {}
        let (_, dirs) = refresh(inner.clone()).await;
        sync(&mut watcher, dirs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use depths_core::plan::RunStep;
    use depths_core::ports::{OutputSink, ProjectContainers, StepOutcome, Validation};

    #[derive(Default)]
    struct Mock {
        ps: AtomicUsize,
        following: AtomicUsize,
        follows_started: AtomicUsize,
        emit: AtomicBool,
        fail: Mutex<Option<String>>,
    }

    impl ContainerRuntime for Mock {
        fn containers_by_project(&self) -> Result<RuntimeSnapshot, RuntimeError> {
            self.ps.fetch_add(1, Ordering::SeqCst);
            if let Some(r) = self.fail.lock().unwrap().clone() {
                return Err(RuntimeError::Unavailable(r));
            }
            let mut s = RuntimeSnapshot::default();
            s.projects
                .insert("other".into(), ProjectContainers::default());
            Ok(s)
        }
        fn run_step(
            &self,
            _: &RunStep,
            _: Option<&[u8]>,
            _: &dyn OutputSink,
            _: &CancelToken,
        ) -> Result<StepOutcome, RuntimeError> {
            Err(RuntimeError::Unsupported)
        }
        fn validate(&self, _: &RunStep, _: &[u8]) -> Result<Validation, RuntimeError> {
            Err(RuntimeError::Unsupported)
        }
        fn follow_events(
            &self,
            on_event: &(dyn Fn() + Sync),
            cancel: &CancelToken,
        ) -> Result<(), RuntimeError> {
            self.follows_started.fetch_add(1, Ordering::SeqCst);
            self.following.fetch_add(1, Ordering::SeqCst);
            while !cancel.is_cancelled() {
                if self.emit.swap(false, Ordering::SeqCst) {
                    for _ in 0..5 {
                        on_event();
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.following.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn setup() -> (tempfile::TempDir, Arc<Mock>, Watch) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("web")).unwrap();
        std::fs::write(
            dir.path().join("web/compose.yaml"),
            "services:\n  app:\n    image: nginx\n",
        )
        .unwrap();
        let store = depths_stacks_fs::FsStackStore::open(dir.path()).unwrap();
        let mock = Arc::new(Mock::default());
        let w = Watch::with_debounce(Arc::new(store), mock.clone(), Duration::from_millis(50));
        (dir, mock, w)
    }

    async fn until(what: &str, f: impl Fn() -> bool) {
        for _ in 0..400 {
            if f() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nothing_calls_docker_while_nobody_is_subscribed() {
        let (_d, mock, w) = setup();
        tokio::time::sleep(Duration::from_millis(200)).await;
        w.kick();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(mock.ps.load(Ordering::SeqCst), 0);
        assert_eq!(mock.follows_started.load(Ordering::SeqCst), 0);

        let a = w.subscribe();
        let b = w.subscribe();
        until("events follower", || {
            mock.following.load(Ordering::SeqCst) == 1
        })
        .await;
        drop(a);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            mock.following.load(Ordering::SeqCst),
            1,
            "one subscriber is left"
        );
        drop(b);
        until("the follower to stop", || {
            mock.following.load(Ordering::SeqCst) == 0
        })
        .await;
        assert_eq!(mock.follows_started.load(Ordering::SeqCst), 1);
        assert_eq!(mock.ps.load(Ordering::SeqCst), 0, "no event, no reading");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn events_are_debounced_into_one_full_list() {
        let (_d, mock, w) = setup();
        let mut sub = w.subscribe();
        until("events follower", || {
            mock.following.load(Ordering::SeqCst) == 1
        })
        .await;
        mock.emit.store(true, Ordering::SeqCst);
        let u = tokio::time::timeout(Duration::from_secs(5), sub.rx.recv())
            .await
            .unwrap()
            .unwrap();
        let Update::Stacks(list) = u else {
            panic!("{u:?}")
        };
        assert_eq!(list.stacks.len(), 1);
        assert_eq!(
            list.foreign.len(),
            1,
            "the full list, foreign projects included"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            mock.ps.load(Ordering::SeqCst),
            1,
            "five events, one reading"
        );

        // The same again is not broadcast.
        mock.emit.store(true, Ordering::SeqCst);
        until("second reading", || mock.ps.load(Ordering::SeqCst) == 2).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(sub.rx.try_recv().is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_compose_file_change_is_seen() {
        let (d, mock, w) = setup();
        let mut sub = w.subscribe();
        // The first reading also arms the per-stack watch.
        w.inventory().await;
        let _ = sub.rx.recv().await;
        w.kick();
        until("reading", || mock.ps.load(Ordering::SeqCst) >= 2).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::fs::create_dir(d.path().join("db")).unwrap();
        std::fs::write(d.path().join("db/compose.yaml"), "services: {}\n").unwrap();
        let list = loop {
            let u = tokio::time::timeout(Duration::from_secs(5), sub.rx.recv())
                .await
                .unwrap()
                .unwrap();
            if let Update::Stacks(l) = u
                && l.stacks.len() == 2
            {
                break l;
            }
        };
        assert!(list.stacks.iter().any(|s| s.name == "db"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unreachable_runtime_keeps_the_last_list_stale() {
        let (_d, mock, w) = setup();
        let good = w.inventory().await;
        assert!(!good.list.stale && good.unavailable.is_none());

        let stderr = "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?";
        *mock.fail.lock().unwrap() = Some(stderr.into());
        let mut sub = w.subscribe();
        let bad = w.inventory().await;
        assert_eq!(bad.unavailable.as_deref(), Some(stderr), "verbatim");
        assert!(bad.list.stale);
        assert_eq!(
            bad.list.as_of, good.list.as_of,
            "as of the last good reading"
        );
        assert_eq!(bad.list.foreign.len(), 1, "the last containers are kept");
        assert_eq!(w.unavailable().as_deref(), Some(stderr));
        let Update::Stacks(l) = sub.rx.recv().await.unwrap() else {
            panic!()
        };
        assert!(l.stale);
        let Update::Unavailable(r) = sub.rx.recv().await.unwrap() else {
            panic!()
        };
        assert_eq!(r, stderr);

        // Still down: no second banner.
        w.inventory().await;
        assert!(sub.rx.try_recv().is_err());

        *mock.fail.lock().unwrap() = None;
        let back = w.inventory().await;
        assert!(!back.list.stale && back.unavailable.is_none());
        assert_eq!(w.unavailable(), None);
        let Update::Stacks(l) = sub.rx.recv().await.unwrap() else {
            panic!()
        };
        assert!(!l.stale);
    }

    #[test]
    fn only_stack_files_are_relevant() {
        let root = Path::new("/stacks");
        assert!(relevant(root, Path::new("/stacks/web")));
        assert!(relevant(root, Path::new("/stacks/global.env")));
        assert!(relevant(root, Path::new("/stacks/web/compose.yaml")));
        assert!(relevant(root, Path::new("/stacks/web/.env")));
        assert!(relevant(root, Path::new("/stacks/web/docker-compose.yml")));
        assert!(!relevant(root, Path::new("/stacks/web/app.log")));
        assert!(!relevant(root, Path::new("/stacks/web/data")));
    }

    /// 051 FR-001/FR-002: a runtime with rated containers, counting its inspects.
    #[derive(Default)]
    struct Rated {
        /// `(id, inspect JSON)`; `None` JSON: present in `ps`, missing from inspect.
        containers: Mutex<Vec<(String, Option<serde_json::Value>)>>,
        inspected: Mutex<Vec<Vec<String>>>,
        daemon_reads: AtomicUsize,
        inspect_fails: AtomicBool,
    }

    impl ContainerRuntime for Rated {
        fn containers_by_project(&self) -> Result<RuntimeSnapshot, RuntimeError> {
            let containers = self
                .containers
                .lock()
                .unwrap()
                .iter()
                .map(|(id, _)| depths_protocol::Container {
                    id: id.clone(),
                    name: format!("web-{id}-1"),
                    image: "nginx".into(),
                    state: depths_protocol::RuntimeContainerState::Running,
                    health: None,
                    status: "Up".into(),
                    created_at: String::new(),
                    command: String::new(),
                    ports: vec![],
                    project: Some("web".into()),
                    service: Some("app".into()),
                    cluster: None,
                    adoption: depths_protocol::AdoptionState::NotAdopted,
                })
                .collect();
            let mut s = RuntimeSnapshot::default();
            s.projects.insert(
                "web".into(),
                ProjectContainers {
                    working_dir: None,
                    containers,
                },
            );
            Ok(s)
        }
        fn run_step(
            &self,
            _: &RunStep,
            _: Option<&[u8]>,
            _: &dyn OutputSink,
            _: &CancelToken,
        ) -> Result<StepOutcome, RuntimeError> {
            Err(RuntimeError::Unsupported)
        }
        fn validate(&self, _: &RunStep, _: &[u8]) -> Result<Validation, RuntimeError> {
            Err(RuntimeError::Unsupported)
        }
        fn inspect_security(&self, ids: &[String]) -> Result<Vec<Inspect>, RuntimeError> {
            self.inspected.lock().unwrap().push(ids.to_vec());
            if self.inspect_fails.load(Ordering::SeqCst) {
                return Err(RuntimeError::Failed("inspect failed".into()));
            }
            let all = self.containers.lock().unwrap();
            Ok(ids
                .iter()
                .filter_map(|id| all.iter().find(|(c, _)| c == id))
                .filter_map(|(_, j)| j.clone())
                .map(|j| serde_json::from_value(j).unwrap())
                .collect())
        }
        fn daemon_mode(&self) -> Result<DaemonMode, RuntimeError> {
            self.daemon_reads.fetch_add(1, Ordering::SeqCst);
            Ok(DaemonMode::Rootful)
        }
    }

    fn rated_setup(rt: Rated) -> (tempfile::TempDir, Arc<Rated>, Watch) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("web")).unwrap();
        std::fs::write(
            dir.path().join("web/compose.yaml"),
            "services:\n  app:\n    image: nginx\n",
        )
        .unwrap();
        let store = depths_stacks_fs::FsStackStore::open(dir.path()).unwrap();
        let rt = Arc::new(rt);
        let w = Watch::new(Arc::new(store), rt.clone()).with_shell_policy(ShellPolicy::Strict);
        (dir, rt, w)
    }

    fn ratings(
        list: &StackList,
    ) -> (
        Option<depths_protocol::SecurityLevel>,
        Vec<(String, Option<depths_protocol::SecurityLevel>)>,
    ) {
        let s = list.stacks.iter().find(|s| s.name == "web").unwrap();
        let cs = s
            .services
            .iter()
            .flat_map(|v| v.containers.iter())
            .map(|c| (c.container.id.clone(), c.security.as_ref().map(|x| x.level)))
            .collect();
        (s.security, cs)
    }

    #[tokio::test]
    async fn containers_are_rated_once_by_id_and_stacks_by_their_worst() {
        use depths_protocol::SecurityLevel as L;
        use serde_json::json;
        let rt = Rated::default();
        *rt.containers.lock().unwrap() = vec![
            (
                "aaa".into(),
                Some(json!({ "Id": "aaa", "Config": { "User": "1000" } })),
            ),
            (
                "bbb".into(),
                Some(
                    json!({ "Id": "bbb", "Config": { "User": "" }, "HostConfig": { "Privileged": true } }),
                ),
            ),
        ];
        let (_d, rt, w) = rated_setup(rt);
        let inv = w.inventory().await;
        let (stack, cs) = ratings(&inv.list);
        assert_eq!(stack, Some(L::Escape));
        assert_eq!(
            cs,
            vec![("aaa".into(), Some(L::Ok)), ("bbb".into(), Some(L::Escape))]
        );
        let c = &inv.list.stacks[0].services[0].containers[1];
        assert_eq!(
            c.security.as_ref().unwrap().shell,
            depths_protocol::ShellRule::Refused
        );
        assert_eq!(w.daemon(), Some(DaemonMode::Rootful));

        // A second reading inspects nothing new and leaves the daemon alone.
        let _ = w.inventory().await;
        assert_eq!(rt.inspected.lock().unwrap().len(), 1);
        assert_eq!(rt.daemon_reads.load(Ordering::SeqCst), 1);

        // A new container is inspected alone; a gone one is forgotten.
        rt.containers.lock().unwrap().remove(1);
        rt.containers.lock().unwrap().push((
            "ccc".into(),
            Some(json!({ "Id": "ccc", "Config": { "User": "root" } })),
        ));
        let inv = w.inventory().await;
        assert_eq!(
            rt.inspected.lock().unwrap().last().unwrap(),
            &vec!["ccc".to_owned()]
        );
        let (stack, cs) = ratings(&inv.list);
        assert_eq!(stack, Some(L::Root), "root on a rootful daemon");
        assert_eq!(
            cs,
            vec![("aaa".into(), Some(L::Ok)), ("ccc".into(), Some(L::Root))]
        );
        assert_eq!(w.inner.security.lock().unwrap().by_id.len(), 2);
    }

    #[tokio::test]
    async fn a_failed_inspect_leaves_containers_unrated_and_is_retried() {
        use serde_json::json;
        let rt = Rated::default();
        *rt.containers.lock().unwrap() = vec![(
            "aaa".into(),
            Some(json!({ "Id": "aaa", "Config": { "User": "1000" } })),
        )];
        rt.inspect_fails.store(true, Ordering::SeqCst);
        let (_d, rt, w) = rated_setup(rt);
        let inv = w.inventory().await;
        assert_eq!(ratings(&inv.list), (None, vec![("aaa".into(), None)]));
        rt.inspect_fails.store(false, Ordering::SeqCst);
        let inv = w.inventory().await;
        assert_eq!(rt.inspected.lock().unwrap().len(), 2, "retried");
        assert_eq!(
            ratings(&inv.list).0,
            Some(depths_protocol::SecurityLevel::Ok)
        );
    }
}
