//! T066: one socket's log follows, shells and stats watch (contracts/protocol.md, "Streams").
//!
//! A socket holds at most [`MAX_STREAMS_PER_SOCKET`] streams, the stats watch and the host watch
//! counting as one each.
//! Every stream belongs to the socket that opened it: ids are per socket, and an id this socket
//! does not hold is `Invalid`. Closing the socket stops all of them ([`Registry`]'s `Drop`).
//!
//! Memory stays bounded however slowly the client reads:
//! - producers put events on one queue of [`STREAM_QUEUE`] entries, and each stream may have at
//!   most [`STREAM_BUDGET`] bytes waiting on it;
//! - a log batch that does not fit is dropped (the follow carries on);
//! - shell output that does not fit closes the shell `failed`, since a terminal with a hole in it
//!   is worse than a closed one;
//! - a stats or host sample is skipped while the previous one is still waiting;
//! - shell input waits in a queue of [`SHELL_INPUT_QUEUE`] writes; a full queue closes the shell.
//!
//! Names are resolved against the inventory, never passed through: a container is found by its
//! exact name, its exact id or an unambiguous id prefix of at least 12 characters, and the full
//! id is what goes into the argv.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use depths_core::plan::{self, LogWindow, StackContext};
use depths_core::ports::{CancelToken, RuntimeError, ShellSession, StreamEvent, StreamSink};
use depths_core::role::{self, Access, Action};
use depths_protocol::{
    AuditAction, AuditOutcome, Container, ContainerRef, Event, EventEnvelope, LogTail, LogTarget,
    MAX_STREAMS_PER_SOCKET, RequestId, Shell, StackList, StatsTarget, StreamCloseReason, StreamId,
    Timestamp,
};
use tokio::sync::mpsc;

use super::{Conn, invalid, reply};
use crate::audit;
use crate::http::AppState;

/// Stream events waiting for the socket, across every stream.
pub const STREAM_QUEUE: usize = 512;
/// Bytes of events one stream may have waiting for the socket.
pub const STREAM_BUDGET: usize = 512 * 1024;
/// Shell writes waiting for the terminal.
pub const SHELL_INPUT_QUEUE: usize = 32;
/// The largest terminal accepted.
pub const MAX_COLS: u16 = 1000;
pub const MAX_ROWS: u16 = 500;
/// The shortest id prefix a container may be named by.
const MIN_ID_PREFIX: usize = 12;

/// What producers hand the socket loop.
pub enum Outgoing {
    /// A `LogChunk` or `ShellOutput`; sent only while the stream is still open.
    Data {
        stream: StreamId,
        env: EventEnvelope,
        _credit: Credit,
    },
    /// A `StatsSample`; sent only while that watch is still the socket's.
    Stats {
        generation: u64,
        env: EventEnvelope,
        _credit: Credit,
    },
    /// A `HostSample`; sent only while that host watch is still the socket's.
    Host {
        generation: u64,
        env: EventEnvelope,
        _credit: Credit,
    },
    /// The stream ended on its own; the loop closes it and says why.
    End {
        stream: StreamId,
        reason: StreamCloseReason,
    },
}

/// Bytes in flight for one stream.
#[derive(Clone)]
pub struct Budget {
    used: Arc<AtomicUsize>,
    cap: usize,
}

/// A share of a [`Budget`], given back once the event is sent (or dropped).
pub struct Credit {
    used: Arc<AtomicUsize>,
    n: usize,
}

impl Drop for Credit {
    fn drop(&mut self) {
        self.used.fetch_sub(self.n, Ordering::AcqRel);
    }
}

impl Budget {
    fn new(cap: usize) -> Self {
        Self {
            used: Arc::default(),
            cap,
        }
    }

    /// `n` more bytes, if they fit. One event always fits an empty budget, so an event larger
    /// than the cap is not starved forever.
    fn take(&self, n: usize) -> Option<Credit> {
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
                (cur == 0 || cur + n <= self.cap).then_some(cur + n)
            })
            .ok()
            .map(|_| Credit {
                used: self.used.clone(),
                n,
            })
    }
}

enum ShellCmd {
    Input(Vec<u8>),
    Resize(u16, u16),
}

enum Kind {
    Logs(CancelToken),
    Shell {
        input: std::sync::mpsc::SyncSender<ShellCmd>,
        stop: Arc<dyn Fn() + Send + Sync>,
        /// 051 FR-009: open only because its person is an admin; closes when they stop being one.
        admin_only: bool,
        /// 051 FR-014: dropping it stops the task that ends the shell when its container goes.
        _follow: tokio::sync::oneshot::Sender<()>,
    },
}

/// One open stream and what keeps it permitted.
struct Entry {
    /// `None`: a container outside every stack.
    stack: Option<depths_core::StackName>,
    action: Action,
    kind: Kind,
}

impl Entry {
    fn stop(self) {
        match self.kind {
            Kind::Logs(cancel) => cancel.cancel(),
            Kind::Shell { input, stop, .. } => {
                // The input thread sees the queue close and reaps the session; the kill gets it
                // out of a write the terminal is not reading.
                drop(input);
                stop();
            }
        }
    }
}

struct StatsWatch {
    generation: u64,
    cancel: CancelToken,
}

/// The socket's `WatchHost`. Shares the generation counter with [`StatsWatch`].
struct HostWatch {
    generation: u64,
    cancel: CancelToken,
}

/// One socket's streams. Dropping it stops every one of them.
pub struct Registry {
    next: u64,
    open: HashMap<StreamId, Entry>,
    stats: Option<StatsWatch>,
    host: Option<HostWatch>,
    generation: u64,
    tx: mpsc::Sender<Outgoing>,
}

impl Registry {
    pub fn new() -> (Self, mpsc::Receiver<Outgoing>) {
        let (tx, rx) = mpsc::channel(STREAM_QUEUE);
        let r = Self {
            next: 0,
            open: HashMap::new(),
            stats: None,
            host: None,
            generation: 0,
            tx,
        };
        (r, rx)
    }

    fn in_use(&self) -> usize {
        self.open.len() + usize::from(self.stats.is_some()) + usize::from(self.host.is_some())
    }

    fn has_room(&self) -> bool {
        self.in_use() < MAX_STREAMS_PER_SOCKET
    }

    fn next_id(&mut self) -> StreamId {
        self.next += 1;
        StreamId(self.next)
    }

    /// Whether `stream` is still open, for the loop deciding to send its data.
    pub fn is_open(&self, stream: StreamId) -> bool {
        self.open.contains_key(&stream)
    }

    /// Whether `generation` is the socket's current stats watch.
    pub fn is_watching(&self, generation: u64) -> bool {
        self.stats
            .as_ref()
            .is_some_and(|w| w.generation == generation)
    }

    /// Whether `generation` is the socket's current host watch.
    pub fn is_watching_host(&self, generation: u64) -> bool {
        self.host
            .as_ref()
            .is_some_and(|w| w.generation == generation)
    }

    /// Stops `stream` and forgets it; `false` when this socket does not hold it.
    pub fn close(&mut self, stream: StreamId) -> bool {
        match self.open.remove(&stream) {
            Some(e) => {
                e.stop();
                true
            }
            None => false,
        }
    }

    /// Closes every shell on this socket (spec 052 FR-012); their ids.
    pub fn close_shells(&mut self) -> Vec<StreamId> {
        let shells: Vec<StreamId> = self
            .open
            .iter()
            .filter(|(_, e)| matches!(e.kind, Kind::Shell { .. }))
            .map(|(id, _)| *id)
            .collect();
        for id in &shells {
            self.close(*id);
        }
        shells
    }

    fn unwatch(&mut self) {
        if let Some(w) = self.stats.take() {
            w.cancel.cancel();
        }
    }

    fn unwatch_host(&mut self) {
        if let Some(w) = self.host.take() {
            w.cancel.cancel();
        }
    }

    /// Closes the streams `access` no longer permits; their ids. The host watch has no id: it
    /// just stops.
    pub fn revoke(&mut self, access: &Access) -> Vec<StreamId> {
        if !role::permits(access, None, Action::ViewHost) {
            self.unwatch_host();
        }
        let gone: Vec<StreamId> = self
            .open
            .iter()
            .filter(|(_, e)| {
                !role::permits(access, e.stack.as_ref(), e.action) || admin_lost(access, e)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in &gone {
            self.close(*id);
        }
        gone
    }

    #[cfg(test)]
    fn open_count(&self) -> usize {
        self.open.len()
    }
}

/// 051 FR-009: an admin-only shell whose person is no longer an admin.
fn admin_lost(access: &Access, e: &Entry) -> bool {
    matches!(
        e.kind,
        Kind::Shell {
            admin_only: true,
            ..
        }
    ) && access.role != role::Role::Admin
}

impl Drop for Registry {
    fn drop(&mut self) {
        self.unwatch();
        self.unwatch_host();
        for (_, e) in self.open.drain() {
            e.stop();
        }
    }
}

// ---------------------------------------------------------------------------
// Sinks
// ---------------------------------------------------------------------------

struct LogSink {
    stream: StreamId,
    tx: mpsc::Sender<Outgoing>,
    budget: Budget,
    cancel: CancelToken,
}

impl StreamSink for LogSink {
    fn emit(&self, ev: StreamEvent) -> bool {
        if self.cancel.is_cancelled() || self.tx.is_closed() {
            return false;
        }
        if let StreamEvent::ContainerLogBatch { lines, .. } = ev
            && !lines.is_empty()
        {
            let n = lines.iter().map(|l| l.len() + 4).sum::<usize>() + 64;
            // No room: these lines are dropped, the follow goes on.
            if let Some(credit) = self.budget.take(n) {
                let env = reply(
                    None,
                    Event::LogChunk {
                        stream_id: self.stream,
                        lines,
                    },
                );
                let out = Outgoing::Data {
                    stream: self.stream,
                    env,
                    _credit: credit,
                };
                if let Err(mpsc::error::TrySendError::Closed(_)) = self.tx.try_send(out) {
                    return false;
                }
            }
        }
        true
    }
}

const TOO_SLOW: &str = "the browser is not reading the shell's output fast enough";

struct ShellSink {
    stream: StreamId,
    tx: mpsc::Sender<Outgoing>,
    budget: Budget,
    /// Set once the shell was given up on: nothing more is sent.
    failed: std::sync::atomic::AtomicBool,
}

impl ShellSink {
    /// The end goes behind the output already queued. Called from the session's own threads,
    /// never from the runtime's, so waiting for room is allowed; off them it does not wait.
    fn end(&self, reason: StreamCloseReason) {
        let end = Outgoing::End {
            stream: self.stream,
            reason,
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            let _ = self.tx.try_send(end);
        } else {
            let _ = self.tx.blocking_send(end);
        }
    }

    fn give_up(&self) {
        if !self.failed.swap(true, Ordering::AcqRel) {
            self.end(StreamCloseReason::Failed(TOO_SLOW.into()));
        }
    }
}

impl StreamSink for ShellSink {
    fn emit(&self, ev: StreamEvent) -> bool {
        if self.failed.load(Ordering::Acquire) {
            return false;
        }
        match ev {
            StreamEvent::ContainerSessionOutput { data, .. } => {
                let bytes_b64 = STANDARD.encode(data.as_bytes());
                let Some(credit) = self.budget.take(bytes_b64.len() + 64) else {
                    self.give_up();
                    return false;
                };
                let env = reply(
                    None,
                    Event::ShellOutput {
                        stream_id: self.stream,
                        bytes_b64,
                    },
                );
                let out = Outgoing::Data {
                    stream: self.stream,
                    env,
                    _credit: credit,
                };
                match self.tx.try_send(out) {
                    Ok(()) => true,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        self.give_up();
                        false
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => false,
                }
            }
            StreamEvent::ContainerSessionClosed { error, .. } => {
                self.end(error.map_or(StreamCloseReason::Ended, StreamCloseReason::Failed));
                false
            }
            StreamEvent::ContainerLogBatch { .. } => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Where a named container lives.
enum Found<'a> {
    /// In this stack.
    Stack(depths_core::StackName, &'a Container),
    /// In a foreign project, standalone, or in a stack whose name cannot be acted on.
    Outside(&'a Container),
}

impl Found<'_> {
    fn container(&self) -> &Container {
        match self {
            Found::Stack(_, c) | Found::Outside(c) => c,
        }
    }

    fn stack(&self) -> Option<&depths_core::StackName> {
        match self {
            Found::Stack(s, _) => Some(s),
            Found::Outside(_) => None,
        }
    }
}

fn every_container(list: &StackList) -> impl Iterator<Item = (Option<&str>, &Container)> {
    let stacks = list.stacks.iter().flat_map(|s| {
        s.services.iter().flat_map(move |svc| {
            svc.containers
                .iter()
                .map(move |c| (Some(s.name.as_str()), &c.container))
        })
    });
    let foreign = list
        .foreign
        .iter()
        .flat_map(|p| p.containers.iter().map(|c| (None, c)));
    let standalone = list.standalone.iter().map(|c| (None, c));
    stacks.chain(foreign).chain(standalone)
}

fn find<'a>(list: &'a StackList, wanted: &str) -> Option<Found<'a>> {
    let exact = every_container(list).find(|(_, c)| c.name == wanted || c.id == wanted);
    let hit = exact.or_else(|| {
        if wanted.len() < MIN_ID_PREFIX || !wanted.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let mut by_prefix = every_container(list).filter(|(_, c)| c.id.starts_with(wanted));
        let first = by_prefix.next();
        // Ambiguous: refuse rather than pick.
        if by_prefix.next().is_some() {
            None
        } else {
            first
        }
    })?;
    Some(
        match hit.0.and_then(|s| depths_core::StackName::parse(s).ok()) {
            Some(stack) => Found::Stack(stack, hit.1),
            None => Found::Outside(hit.1),
        },
    )
}

/// What `action` on this container needs: the stack's grant, or (outside every stack) admin.
fn needs(found: &Found<'_>, action: Action) -> (Option<depths_core::StackName>, Action) {
    match found {
        Found::Stack(s, _) => (Some(s.clone()), action),
        Found::Outside(_) if action == Action::Shell => (None, Action::Shell),
        Found::Outside(_) => (None, Action::ViewForeign),
    }
}

/// The inventory to resolve against: the live one when a feed keeps it fresh, else a new reading.
async fn current(app: &AppState) -> Arc<StackList> {
    match app.watch.last() {
        Some(l) if app.watch.subscribers() > 0 => l,
        _ => app.watch.inventory().await.list,
    }
}

fn runtime_message(e: RuntimeError) -> String {
    match e {
        RuntimeError::Unavailable(r) | RuntimeError::Failed(r) => r,
        RuntimeError::Unsupported => "the container runtime cannot stream".into(),
    }
}

/// A container the person may act on, or the answer to send instead.
///
/// Not found is `Denied` for anyone who could not see every container anyway, so a name outside
/// someone's stacks cannot be probed for; only an admin learns it does not exist.
fn resolve<'a>(
    conn: &Conn,
    access: &Access,
    id: Option<RequestId>,
    list: &'a StackList,
    container: &ContainerRef,
    action: Action,
    what: &str,
) -> Result<Found<'a>, Box<EventEnvelope>> {
    let Some(found) = find(list, container.as_str()) else {
        return Err(Box::new(
            if role::permits(access, None, Action::ViewForeign) {
                invalid(id, format!("no container named {}", container.as_str()))
            } else {
                conn.deny(id, None, format!("{what} on an unknown container"))
            },
        ));
    };
    let (stack, action) = needs(&found, action);
    if !role::permits(access, stack.as_ref(), action) {
        let s = stack.as_ref().map(depths_core::StackName::as_str);
        return Err(Box::new(conn.deny(
            id,
            s,
            format!("{what} on a container not granted"),
        )));
    }
    Ok(found)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn too_many(id: Option<RequestId>) -> EventEnvelope {
    invalid(
        id,
        format!("too many open streams (at most {MAX_STREAMS_PER_SOCKET} per connection)"),
    )
}

/// `OpenLogs`: `docker compose logs` of a stack or service, or `docker logs` of one container,
/// with timestamps, `tail` lines of history and, when given, only lines `since`.
pub(super) async fn open_logs(
    conn: &mut Conn,
    access: &Access,
    id: Option<RequestId>,
    target: LogTarget,
    tail: LogTail,
    since: Option<Timestamp>,
) -> EventEnvelope {
    let app = conn.app.clone();
    let list = current(&app).await;
    let (step, stack, action) = match &target {
        LogTarget::Stack { stack, service } => {
            let Ok(name) = depths_core::StackName::parse(stack.as_str()) else {
                return conn.deny(id, None, "OpenLogs on an invalid stack name".into());
            };
            if !role::permits(access, Some(&name), Action::View) {
                return conn.deny(
                    id,
                    Some(name.as_str()),
                    "OpenLogs on a stack not granted".into(),
                );
            }
            let Some(found) = list.stacks.iter().find(|s| s.name == name.as_str()) else {
                return invalid(id, format!("no stack named {name}"));
            };
            let service = match service {
                None => None,
                Some(svc) => {
                    let known = found.services.iter().any(|s| s.name == svc.as_str());
                    match depths_core::ServiceName::parse(svc.as_str()) {
                        Ok(s) if known => Some(s),
                        _ => {
                            return invalid(
                                id,
                                format!("{name} has no service named {}", svc.as_str()),
                            );
                        }
                    }
                }
            };
            let (store, n) = (app.store.clone(), name.clone());
            let entry = match tokio::task::spawn_blocking(move || store.entry(&n)).await {
                Ok(Ok(e)) => e,
                Ok(Err(e)) => return invalid(id, e.to_string()),
                Err(_) => return invalid(id, "the server could not complete the request"),
            };
            if !entry.actionable() {
                let why = crate::inventory::invalid_reason(&entry)
                    .or(entry.read_error.clone())
                    .unwrap_or_else(|| "the stack cannot be acted on".into());
                return invalid(id, why);
            }
            let ctx = StackContext {
                stacks_root: app.store.root(),
                stack: &name,
                global_env: app.store.global_env(),
                has_env: entry.has_env,
            };
            let window = LogWindow {
                tail: tail.get(),
                since: since.as_ref().map(Timestamp::as_str),
            };
            let step = plan::stack_logs(&ctx, service.as_ref(), window);
            (step, Some(name), Action::View)
        }
        LogTarget::Container { container } => {
            let found = match resolve(conn, access, id, &list, container, Action::View, "OpenLogs")
            {
                Ok(f) => f,
                Err(e) => return *e,
            };
            let window = LogWindow {
                tail: tail.get(),
                since: since.as_ref().map(Timestamp::as_str),
            };
            let step = plan::container_logs(app.store.root(), &found.container().id, window);
            let (stack, action) = needs(&found, Action::View);
            (step, stack, action)
        }
    };
    if !conn.streams.has_room() {
        return too_many(id);
    }

    let stream = conn.streams.next_id();
    let cancel = CancelToken::new();
    let tx = conn.streams.tx.clone();
    let sink = LogSink {
        stream,
        tx: tx.clone(),
        budget: Budget::new(STREAM_BUDGET),
        cancel: cancel.clone(),
    };
    let runtime = app.runtime.clone();
    let stop = cancel.clone();
    tokio::spawn(async move {
        let ran =
            tokio::task::spawn_blocking(move || runtime.follow_logs(&step, stream, &sink, &stop))
                .await;
        let reason = match ran {
            Ok(Ok(())) => StreamCloseReason::Ended,
            Ok(Err(e)) => StreamCloseReason::Failed(runtime_message(e)),
            Err(_) => StreamCloseReason::Failed("the log follow stopped".into()),
        };
        // Behind the last lines; ignored when the stream was closed meanwhile.
        let _ = tx.send(Outgoing::End { stream, reason }).await;
    });
    conn.streams.open.insert(
        stream,
        Entry {
            stack,
            action,
            kind: Kind::Logs(cancel),
        },
    );
    reply(id, Event::StreamOpened { stream_id: stream })
}

fn size_ok(cols: u16, rows: u16) -> bool {
    (1..=MAX_COLS).contains(&cols) && (1..=MAX_ROWS).contains(&rows)
}

fn shell_path(shell: Option<Shell>) -> &'static str {
    // `/bin/sh` when none is named: present in busybox and distro images alike (see
    // `container_runtime::session`).
    match shell {
        None | Some(Shell::Sh) => "/bin/sh",
        Some(Shell::Bash) => "/bin/bash",
        Some(Shell::Ash) => "/bin/ash",
    }
}

/// `OpenShell`: authorised per stack with `shell`, audited before it opens. The feature flag is
/// checked by the caller.
pub(super) async fn open_shell(
    conn: &mut Conn,
    access: &Access,
    id: Option<RequestId>,
    container: ContainerRef,
    shell: Option<Shell>,
    cols: u16,
    rows: u16,
) -> EventEnvelope {
    if !size_ok(cols, rows) {
        return invalid(
            id,
            format!("the terminal must be 1-{MAX_COLS} columns and 1-{MAX_ROWS} rows"),
        );
    }
    let app = conn.app.clone();
    let list = current(&app).await;
    let found = match resolve(
        conn,
        access,
        id,
        &list,
        &container,
        Action::Shell,
        "OpenShell",
    ) {
        Ok(f) => f,
        Err(e) => return *e,
    };
    // Spec 052: the role allowed it; an untrusted network still may not open a shell.
    // 051 shell guard composes after this
    if let Some(refused) = conn.untrusted(id, depths_core::role::CommandKind::Shell, "OpenShell") {
        return refused;
    }
    if !conn.streams.has_room() {
        return too_many(id);
    }
    let container_id = found.container().id.clone();
    let container_name = found.container().name.clone();
    let shell = shell_path(shell);

    // 051 FR-008 to FR-012: the container gate, on an inspect made now, failing closed.
    let decision = container_gate(&app, &container_id, access.role == role::Role::Admin).await;
    let (security, admin_only) = match decision {
        depths_core::security::ShellDecision::Allow {
            level,
            reasons,
            admin_only,
        } => (audit_security(level, &reasons), admin_only),
        depths_core::security::ShellDecision::Refuse {
            why,
            level,
            reasons,
        } => {
            return refuse_shell(conn, id, found.stack(), why, level, &reasons);
        }
    };

    let mut e = audit::entry(
        conn.actor.clone(),
        AuditAction::ShellOpen,
        conn.ip,
        AuditOutcome::Succeeded,
    );
    e.stack = found.stack().map(|s| s.as_str().to_owned());
    e.argv = vec![
        [
            "docker",
            "exec",
            "--interactive",
            "--tty",
            &container_id,
            shell,
        ]
        .map(str::to_owned)
        .to_vec(),
    ];
    e.security = Some(security);
    if let Err(err) = app.audit.record(&e) {
        tracing::error!(error = %err, "audit write failed; not opening the shell");
        return invalid(id, "the audit log could not be written");
    }
    let (stack, action) = needs(&found, Action::Shell);

    let stream = conn.streams.next_id();
    let opened_id = container_id.clone();
    let sink = Arc::new(ShellSink {
        stream,
        tx: conn.streams.tx.clone(),
        budget: Budget::new(STREAM_BUDGET),
        failed: std::sync::atomic::AtomicBool::new(false),
    });
    let runtime = app.runtime.clone();
    let opened = tokio::task::spawn_blocking(move || {
        runtime.open_shell(&container_id, shell, (cols, rows), stream, sink)
    })
    .await;
    let session = match opened {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return invalid(id, runtime_message(e)),
        Err(_) => return invalid(id, "the server could not complete the request"),
    };
    let stop = session.stopper();
    let (input, rx) = std::sync::mpsc::sync_channel(SHELL_INPUT_QUEUE);
    std::thread::spawn(move || feed(session, &rx));
    let follow = follow_container(
        &app,
        stream,
        opened_id,
        container_name,
        conn.streams.tx.clone(),
    );
    conn.streams.open.insert(
        stream,
        Entry {
            stack,
            action,
            kind: Kind::Shell {
                input,
                stop,
                admin_only,
                _follow: follow,
            },
        },
    );
    reply(id, Event::StreamOpened { stream_id: stream })
}

/// Whether two container ids name the same container: `docker ps` may list a short id where
/// `docker inspect` answers with the full one.
fn same_id(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

/// 051 FR-008/FR-010: inspects the container now, not from the inventory's cache, and asks the
/// one pure decision. An inspect that fails, is unsupported or answers for another container is
/// an unknown profile, which the decision refuses. The daemon's mode comes from the inventory's
/// reading when it has one, else is read now; unreadable counts as rootful.
async fn container_gate(
    app: &AppState,
    container_id: &str,
    admin: bool,
) -> depths_core::security::ShellDecision {
    use depths_core::security::{ShellRequest, profile, shell_decision};
    let runtime = app.runtime.clone();
    let cached = app.watch.daemon();
    let id = container_id.to_owned();
    let read = tokio::task::spawn_blocking(move || {
        let inspect = runtime.inspect_security(std::slice::from_ref(&id));
        let daemon = cached.map_or_else(|| runtime.daemon_mode(), Ok);
        (id, inspect, daemon)
    })
    .await;
    let found = match read {
        Ok((id, Ok(found), daemon)) => found
            .into_iter()
            .find(|i| same_id(&i.id, &id))
            .map(|i| profile(&i, daemon.unwrap_or_default())),
        Ok((_, Err(e), _)) => {
            tracing::warn!(error = ?e, "cannot inspect the container; refusing its shell");
            None
        }
        Err(_) => None,
    };
    shell_decision(ShellRequest {
        policy: app.cfg.shell_policy,
        profile: found.as_ref(),
        role_allows: true,
        admin,
    })
}

fn audit_security(
    level: depths_core::security::Level,
    reasons: &[depths_core::security::Reason],
) -> depths_protocol::AuditSecurity {
    depths_protocol::AuditSecurity {
        level: crate::security::level(level),
        reasons: reasons.iter().map(|r| r.code.as_str().to_owned()).collect(),
        acknowledged: false,
    }
}

/// A refused shell: audited `denied` with the level and reasons that decided it (051 FR-012),
/// and `Denied` with the reason kind and the findings in words.
fn refuse_shell(
    conn: &Conn,
    id: Option<RequestId>,
    stack: Option<&depths_core::StackName>,
    why: depths_core::security::ShellRefusal,
    level: Option<depths_core::security::Level>,
    reasons: &[depths_core::security::Reason],
) -> EventEnvelope {
    use depths_core::security::{Level, Severity, ShellRefusal};
    use depths_protocol::DenyReason;
    let reason = match (why, level) {
        (ShellRefusal::Role, _) => DenyReason::NotPermitted,
        (ShellRefusal::Unknown, _) => DenyReason::ShellUnverifiable,
        (ShellRefusal::Refused, _) | (ShellRefusal::AdminOnly, Some(Level::Escape)) => {
            DenyReason::ShellEscape
        }
        (ShellRefusal::AdminOnly, _) => DenyReason::ShellRootAdminOnly,
    };
    let detail = if why == ShellRefusal::Unknown {
        "the container's configuration could not be read, so its shell is refused".to_owned()
    } else {
        reasons
            .iter()
            .filter(|r| r.severity != Severity::Info)
            .map(|r| r.message.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    };
    let mut e = audit::denied(
        conn.actor.clone(),
        conn.ip,
        stack.map(depths_core::StackName::as_str),
        format!("OpenShell refused ({why:?}): {detail}"),
    );
    e.security = level.map(|l| audit_security(l, reasons));
    if let Err(err) = conn.app.audit.record(&e) {
        tracing::error!(error = %err, "audit write failed for a refused shell");
    }
    reply(
        id,
        Event::Denied {
            reason,
            detail: Some(detail),
        },
    )
}

/// 051 FR-014: ends the shell `stream` when its container is removed, or recreated under its
/// name with a new id, as the inventory sees it. Stops when the returned sender is dropped.
fn follow_container(
    app: &AppState,
    stream: StreamId,
    id: String,
    name: String,
    tx: mpsc::Sender<Outgoing>,
) -> tokio::sync::oneshot::Sender<()> {
    use tokio::sync::broadcast::error::RecvError;
    let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
    let sub = app.watch.subscribe();
    tokio::spawn(async move {
        // The whole subscription, not just its receiver: its guard keeps the watch running.
        let mut sub = sub;
        loop {
            tokio::select! {
                _ = &mut stopped => return,
                u = sub.rx.recv() => match u {
                    Ok(crate::watch::Update::Stacks(list)) => {
                        if let Some(reason) = container_fate(&list, &id, &name) {
                            let _ = tx.send(Outgoing::End { stream, reason }).await;
                            return;
                        }
                    }
                    Ok(crate::watch::Update::Unavailable(_)) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                },
            }
        }
    });
    stop
}

/// What became of a shell's container in this reading: `None` while it is still there (or the
/// reading is stale), else removed, or recreated when its name now belongs to another id.
fn container_fate(list: &StackList, id: &str, name: &str) -> Option<StreamCloseReason> {
    if list.stale || every_container(list).any(|(_, c)| same_id(&c.id, id)) {
        return None;
    }
    Some(if every_container(list).any(|(_, c)| c.name == name) {
        StreamCloseReason::ContainerRecreated
    } else {
        StreamCloseReason::ContainerRemoved
    })
}

/// The session's owner: writes and resizes in order, and the close when the queue closes or the
/// shell is gone.
fn feed(mut session: Box<dyn ShellSession>, rx: &std::sync::mpsc::Receiver<ShellCmd>) {
    while let Ok(cmd) = rx.recv() {
        match cmd {
            ShellCmd::Input(bytes) => {
                if session.write(&bytes).is_err() {
                    break;
                }
            }
            ShellCmd::Resize(cols, rows) => session.resize(cols, rows),
        }
    }
    session.close();
}

fn shell_of(conn: &Conn, stream: StreamId) -> Option<&std::sync::mpsc::SyncSender<ShellCmd>> {
    match conn.streams.open.get(&stream) {
        Some(Entry {
            kind: Kind::Shell { input, .. },
            ..
        }) => Some(input),
        _ => None,
    }
}

/// Queues a write or a resize. No reply, unless something is wrong: `Invalid` for a stream this
/// socket has no shell on, and the stream closed `failed` when the shell has stopped reading.
fn to_shell(
    conn: &mut Conn,
    id: Option<RequestId>,
    stream: StreamId,
    cmd: ShellCmd,
) -> Vec<EventEnvelope> {
    let Some(input) = shell_of(conn, stream) else {
        return vec![invalid(id, "unknown stream")];
    };
    match input.try_send(cmd) {
        Ok(()) => Vec::new(),
        Err(e) => {
            let reason = match e {
                std::sync::mpsc::TrySendError::Full(_) => {
                    StreamCloseReason::Failed("the shell is not reading its input".into())
                }
                std::sync::mpsc::TrySendError::Disconnected(_) => StreamCloseReason::Ended,
            };
            conn.streams.close(stream);
            vec![reply(
                None,
                Event::StreamClosed {
                    stream_id: stream,
                    reason,
                },
            )]
        }
    }
}

pub(super) fn shell_input(
    conn: &mut Conn,
    id: Option<RequestId>,
    stream: StreamId,
    bytes_b64: &str,
) -> Vec<EventEnvelope> {
    let Ok(bytes) = STANDARD.decode(bytes_b64) else {
        return vec![invalid(id, "the shell input is not base64")];
    };
    to_shell(conn, id, stream, ShellCmd::Input(bytes))
}

pub(super) fn shell_resize(
    conn: &mut Conn,
    id: Option<RequestId>,
    stream: StreamId,
    cols: u16,
    rows: u16,
) -> Vec<EventEnvelope> {
    if !size_ok(cols, rows) {
        return vec![invalid(
            id,
            format!("the terminal must be 1-{MAX_COLS} columns and 1-{MAX_ROWS} rows"),
        )];
    }
    to_shell(conn, id, stream, ShellCmd::Resize(cols, rows))
}

/// `CloseStream`: `StreamClosed{closed}`, or `Invalid` for a stream this socket does not hold.
pub(super) fn close_stream(
    conn: &mut Conn,
    id: Option<RequestId>,
    stream: StreamId,
) -> EventEnvelope {
    if conn.streams.close(stream) {
        reply(
            id,
            Event::StreamClosed {
                stream_id: stream,
                reason: StreamCloseReason::Closed,
            },
        )
    } else {
        invalid(id, "unknown stream")
    }
}

/// The full ids `targets` name, as `access` may see them now. Targets no longer permitted, or no
/// longer there, are left out without a word: the watch outlives the containers it names.
fn stats_ids(list: &StackList, access: &Access, targets: &[StatsTarget]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for t in targets {
        match t {
            StatsTarget::Stack { stack } => {
                let Ok(name) = depths_core::StackName::parse(stack.as_str()) else {
                    continue;
                };
                if !role::permits(access, Some(&name), Action::View) {
                    continue;
                }
                if let Some(s) = list.stacks.iter().find(|s| s.name == name.as_str()) {
                    ids.extend(
                        s.services
                            .iter()
                            .flat_map(|svc| svc.containers.iter().map(|c| c.container.id.clone())),
                    );
                }
            }
            StatsTarget::Container { container } => {
                if let Some(found) = find(list, container.as_str()) {
                    let (stack, action) = needs(&found, Action::View);
                    if role::permits(access, stack.as_ref(), action) {
                        ids.push(found.container().id.clone());
                    }
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

/// `WatchStats`: replaces the socket's watch. No reply; `StatsSample` follows at once and then
/// every [`super::WsConfig::stats_every`]. Every target must be permitted when asked for.
pub(super) async fn watch_stats(
    conn: &mut Conn,
    access: &Access,
    id: Option<RequestId>,
    targets: Vec<StatsTarget>,
) -> Vec<EventEnvelope> {
    let app = conn.app.clone();
    let list = current(&app).await;
    for t in &targets {
        match t {
            StatsTarget::Stack { stack } => {
                let ok = depths_core::StackName::parse(stack.as_str())
                    .is_ok_and(|n| role::permits(access, Some(&n), Action::View));
                if !ok {
                    return vec![conn.deny(
                        id,
                        Some(stack.as_str()),
                        "WatchStats on a stack not granted".into(),
                    )];
                }
            }
            StatsTarget::Container { container } => {
                if let Err(e) = resolve(
                    conn,
                    access,
                    id,
                    &list,
                    container,
                    Action::View,
                    "WatchStats",
                ) {
                    return vec![*e];
                }
            }
        }
    }
    conn.streams.unwatch();
    if targets.is_empty() {
        return Vec::new();
    }
    if !conn.streams.has_room() {
        return vec![too_many(id)];
    }
    conn.streams.generation += 1;
    let generation = conn.streams.generation;
    let cancel = CancelToken::new();
    conn.streams.stats = Some(StatsWatch {
        generation,
        cancel: cancel.clone(),
    });
    let tx = conn.streams.tx.clone();
    let user = conn.user.clone();
    let every = app.ws.stats_every;
    tokio::spawn(stats_loop(
        app, user, targets, every, generation, cancel, tx,
    ));
    Vec::new()
}

/// `UnwatchStats`: stops the watch, if there is one. No reply.
pub(super) fn unwatch_stats(conn: &mut Conn) {
    conn.streams.unwatch();
}

async fn stats_loop(
    app: AppState,
    user: String,
    targets: Vec<StatsTarget>,
    every: Duration,
    generation: u64,
    cancel: CancelToken,
    tx: mpsc::Sender<Outgoing>,
) {
    let budget = Budget::new(1);
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        if cancel.is_cancelled() || tx.is_closed() {
            return;
        }
        // The last sample is still waiting for the socket: skip this one.
        let Some(credit) = budget.take(1) else {
            continue;
        };
        let access = app.users.access(&user);
        let list = current(&app).await;
        let ids = stats_ids(&list, &access, &targets);
        if ids.is_empty() {
            continue;
        }
        let runtime = app.runtime.clone();
        let at = audit::now();
        let Ok(Ok(samples)) = tokio::task::spawn_blocking(move || runtime.stats(&ids, &at)).await
        else {
            continue;
        };
        if cancel.is_cancelled() {
            return;
        }
        let env = reply(None, Event::StatsSample { samples });
        let out = Outgoing::Stats {
            generation,
            env,
            _credit: credit,
        };
        if tx.send(out).await.is_err() {
            return;
        }
    }
}

/// How long the first host sample waits after the baseline CPU reading, so it can report a share.
const HOST_PRIME: Duration = Duration::from_millis(250);

/// `WatchHost`, already authorised: starts the socket's host watch, or keeps the one running. No
/// reply; `HostSample` follows shortly and then every [`super::WsConfig::stats_every`].
pub(super) fn watch_host(
    conn: &mut Conn,
    id: Option<RequestId>,
    proc_dir: std::path::PathBuf,
) -> Vec<EventEnvelope> {
    if conn.streams.host.is_some() {
        return Vec::new();
    }
    if !conn.streams.has_room() {
        return vec![too_many(id)];
    }
    conn.streams.generation += 1;
    let generation = conn.streams.generation;
    let cancel = CancelToken::new();
    conn.streams.host = Some(HostWatch {
        generation,
        cancel: cancel.clone(),
    });
    let app = conn.app.clone();
    let every = app.ws.stats_every;
    tokio::spawn(host_loop(
        app,
        conn.user.clone(),
        proc_dir,
        every,
        generation,
        cancel,
        conn.streams.tx.clone(),
    ));
    Vec::new()
}

/// `UnwatchHost`: stops the host watch, if there is one. No reply.
pub(super) fn unwatch_host(conn: &mut Conn) {
    conn.streams.unwatch_host();
}

/// `(running, not running)` over every container in the inventory.
fn container_counts(list: &StackList) -> (u32, u32) {
    let (mut running, mut stopped) = (0u32, 0u32);
    for (_, c) in every_container(list) {
        if c.state == depths_protocol::RuntimeContainerState::Running {
            running = running.saturating_add(1);
        } else {
            stopped = stopped.saturating_add(1);
        }
    }
    (running, stopped)
}

async fn host_loop(
    app: AppState,
    user: String,
    proc_dir: std::path::PathBuf,
    every: Duration,
    generation: u64,
    cancel: CancelToken,
    tx: mpsc::Sender<Outgoing>,
) {
    let mut probe = crate::host::HostProbe::new(proc_dir, app.cfg.stacks_dir.clone());
    probe = match tokio::task::spawn_blocking(move || {
        probe.prime();
        probe
    })
    .await
    {
        Ok(p) => p,
        Err(_) => return,
    };
    tokio::time::sleep(HOST_PRIME.min(every)).await;
    let budget = Budget::new(1);
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        if cancel.is_cancelled() || tx.is_closed() {
            return;
        }
        // Re-read every tick: someone demoted from admin stops getting the host at once.
        if !role::permits(&app.users.access(&user), None, Action::ViewHost) {
            return;
        }
        let Some(credit) = budget.take(1) else {
            continue;
        };
        let (running, stopped) = container_counts(current(&app).await.as_ref());
        let at = audit::now();
        let Ok((p, sample)) = tokio::task::spawn_blocking(move || {
            let s = probe.sample(at, running, stopped);
            (probe, s)
        })
        .await
        else {
            return;
        };
        probe = p;
        if cancel.is_cancelled() {
            return;
        }
        let out = Outgoing::Host {
            generation,
            env: reply(None, Event::HostSample(sample)),
            _credit: credit,
        };
        if tx.send(out).await.is_err() {
            return;
        }
    }
}

/// After an access change: closes the streams no longer permitted and says so.
pub(super) fn revoke(conn: &mut Conn, access: &Access) -> Vec<EventEnvelope> {
    conn.streams
        .revoke(access)
        .into_iter()
        .map(|stream_id| {
            reply(
                None,
                Event::StreamClosed {
                    stream_id,
                    reason: StreamCloseReason::Failed("access was removed".into()),
                },
            )
        })
        .collect()
}

/// Whether a frame is shell traffic, which has its own (larger) rate limit: key repeat alone
/// sends 30 a second.
pub(super) fn is_shell_traffic(text: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Peek {
        cmd: PeekCmd,
    }
    #[derive(serde::Deserialize)]
    struct PeekCmd {
        #[serde(rename = "type")]
        kind: String,
    }
    serde_json::from_str::<Peek>(text)
        .is_ok_and(|p| matches!(p.cmd.kind.as_str(), "ShellInput" | "ShellResize"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, name: &str) -> Container {
        use depths_protocol::{AdoptionState, RuntimeContainerState};
        Container {
            id: id.into(),
            name: name.into(),
            image: "i".into(),
            state: RuntimeContainerState::Running,
            health: None,
            status: "Up".into(),
            created_at: String::new(),
            command: String::new(),
            ports: vec![],
            project: None,
            service: None,
            cluster: None,
            adoption: AdoptionState::NotAdopted,
        }
    }

    #[test]
    fn a_budget_lets_one_oversize_event_through_and_refills_on_drop() {
        let b = Budget::new(10);
        let big = b.take(100).expect("an empty budget takes anything");
        assert!(b.take(1).is_none());
        drop(big);
        let a = b.take(6).unwrap();
        assert!(b.take(5).is_none());
        let _c = b.take(4).unwrap();
        drop(a);
        assert_eq!(b.used.load(Ordering::Acquire), 4);
    }

    #[test]
    fn shell_traffic_is_told_apart_from_other_commands() {
        assert!(is_shell_traffic(
            r#"{"id":1,"cmd":{"type":"ShellInput","data":{"stream_id":1,"bytes_b64":"YQ=="}}}"#
        ));
        assert!(!is_shell_traffic(r#"{"id":1,"cmd":{"type":"ListStacks"}}"#));
        assert!(!is_shell_traffic("not json"));
    }

    #[test]
    fn sizes_are_bounded() {
        assert!(size_ok(1, 1) && size_ok(MAX_COLS, MAX_ROWS));
        assert!(!size_ok(0, 24) && !size_ok(80, 0));
        assert!(!size_ok(MAX_COLS + 1, 24) && !size_ok(80, MAX_ROWS + 1));
    }

    #[test]
    fn the_registry_starts_empty() {
        let (r, _rx) = Registry::new();
        assert_eq!(r.open_count(), 0);
        assert!(r.has_room());
    }

    #[test]
    fn containers_resolve_by_name_id_or_a_long_unique_prefix() {
        let a = "a".repeat(12) + &"1".repeat(52);
        let b = "a".repeat(12) + &"2".repeat(52);
        let list = StackList {
            root: String::new(),
            stacks: Vec::new(),
            foreign: Vec::new(),
            standalone: vec![c(&a, "one"), c(&b, "two")],
            warnings: Vec::new(),
            stale: false,
            as_of: String::new(),
        };
        let id_of = |w: &str| find(&list, w).map(|f| f.container().id.clone());
        assert_eq!(id_of("one"), Some(a.clone()));
        assert_eq!(id_of(&b), Some(b.clone()));
        assert_eq!(id_of(&b[..13]), Some(b.clone()));
        assert_eq!(id_of(&a[..12]), None, "ambiguous");
        assert_eq!(id_of(&a[..11]), None, "too short");
        assert_eq!(id_of("nope"), None);
        assert!(matches!(find(&list, "one"), Some(Found::Outside(_))));
    }
}
