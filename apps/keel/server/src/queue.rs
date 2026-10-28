//! The run queue and the runner hub (spec 055 T070, contracts/runner.md).
//!
//! A runner session ([`attach`]) starts with the runner's `Hello`. The hub then keeps a link to
//! it, pings it every [`RunSettings::ping_interval`] and declares it lost after
//! [`RunSettings::missed_pings`] unanswered pings (any frame counts as an answer). A lost or
//! closed session marks its in-flight hosts `lost`.
//!
//! [`kick`] dispatches: it claims queued `run_hosts` of connected hosts with
//! `FOR UPDATE SKIP LOCKED` (so concurrent kicks never claim one twice), moves their runs from
//! `queued` to `running`, and sends each host `Assign`, `Secrets` and the `Pack` frames, the
//! last `Pack` being the runner's signal to start. Secret values go to the runner only: never
//! into a log line, a run event or an error.
//!
//! What the runner reports is batched ([`RunSettings::flush_rows`] or
//! [`RunSettings::flush_interval`]) into `run_events` and `run_log_chunks` under one run-wide
//! sequence number. Numbers are assigned, written and committed under the run's lock, and only
//! then broadcast on [`Runs::live`], so the database always holds a gap-free prefix of what
//! subscribers see and a subscriber can resume from any number.
//!
//! Output past `--run-log-cap` keeps the first half of the cap as it arrives and the last half
//! in memory; when the run ends, a `log_truncated` event, a marker chunk and the tail are
//! written. Live output pauses between the two.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use keel_core::WorkspaceId;
use keel_core::ports::CommitId;
use keel_core::run::{self as core_run, HostState, RunEvent, RunState};
use keel_exec::codec;
use keel_exec::events::{self, HostStatus, KeelEvent};
use keel_exec::frames::{self, Frame, Peer, Refusal, SecretItem, SecretString, Subsystem};
use keel_git::Mirror;
use keel_protocol::{Event, LogStream};
use serde_json::{Value, json};
use sqlx::PgConnection;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Notify, broadcast, mpsc};
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::serve::AppState;

/// The largest `data` kept for one run event; a larger result is replaced by a note.
const MAX_EVENT_DATA: usize = 64 * 1024;
/// A job without `timeout` gets this long.
const DEFAULT_TIMEOUT_SECS: u64 = 3600;

/// The hub's tunables. Tests shorten the intervals.
#[derive(Debug, Clone, Copy)]
pub struct RunSettings {
    /// `--run-log-cap`: the most output bytes stored per run.
    pub log_cap: u64,
    pub ping_interval: Duration,
    pub missed_pings: u32,
    pub flush_interval: Duration,
    pub flush_rows: usize,
    pub hello_timeout: Duration,
}

impl Default for RunSettings {
    fn default() -> Self {
        Self {
            log_cap: 64 * 1024 * 1024,
            ping_interval: Duration::from_secs(15),
            missed_pings: 3,
            flush_interval: Duration::from_millis(100),
            flush_rows: 64,
            hello_timeout: Duration::from_secs(10),
        }
    }
}

struct Link {
    session: u64,
    tx: mpsc::UnboundedSender<Frame>,
    /// Ends the session (revocation).
    close: Arc<Notify>,
}

/// One run's output accounting.
#[derive(Default)]
struct RunLog {
    next_seq: u64,
    head_bytes: u64,
    tail: VecDeque<(LogStream, Vec<u8>)>,
    tail_bytes: u64,
    dropped: u64,
}

struct Inner {
    settings: RunSettings,
    links: Mutex<HashMap<Uuid, Link>>,
    logs: Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<Option<RunLog>>>>>,
    live: broadcast::Sender<(Uuid, Event)>,
    sessions: AtomicU64,
}

/// The runner hub: connected runners, run output and its live broadcast.
#[derive(Clone)]
pub struct Runs {
    inner: Arc<Inner>,
}

impl Default for Runs {
    fn default() -> Self {
        Self::new(RunSettings::default())
    }
}

impl Runs {
    #[must_use]
    pub fn new(settings: RunSettings) -> Self {
        let (live, _) = broadcast::channel(4096);
        Self {
            inner: Arc::new(Inner {
                settings,
                links: Mutex::new(HashMap::new()),
                logs: Mutex::new(HashMap::new()),
                live,
                sessions: AtomicU64::new(0),
            }),
        }
    }

    #[must_use]
    pub fn settings(&self) -> RunSettings {
        self.inner.settings
    }

    /// Every run's `RunEvent` and `RunLog`, in sequence order per run, after they are stored.
    #[must_use]
    pub fn live(&self) -> broadcast::Receiver<(Uuid, Event)> {
        self.inner.live.subscribe()
    }

    fn connected(&self) -> Vec<Uuid> {
        self.inner
            .links
            .lock()
            .map(|l| l.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Sends `frame` to `host`'s runner; false if it is not connected.
    fn send(&self, host: Uuid, frame: Frame) -> bool {
        self.inner
            .links
            .lock()
            .ok()
            .and_then(|l| l.get(&host).map(|link| link.tx.send(frame).is_ok()))
            .unwrap_or(false)
    }

    fn register(&self, host: Uuid, tx: mpsc::UnboundedSender<Frame>, close: Arc<Notify>) -> u64 {
        let session = self.inner.sessions.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut l) = self.inner.links.lock() {
            // A reconnect replaces the old link; the old session ends on its own.
            l.insert(host, Link { session, tx, close });
        }
        session
    }

    /// Ends `host`'s session, if it has one: its in-flight hosts become `lost`.
    pub fn close(&self, host: Uuid) {
        if let Ok(l) = self.inner.links.lock()
            && let Some(link) = l.get(&host)
        {
            link.close.notify_one();
        }
    }

    /// Sends `frame` to every connected runner (the KRL, on change).
    pub fn broadcast(&self, frame: &Frame) {
        if let Ok(l) = self.inner.links.lock() {
            for link in l.values() {
                let _ = link.tx.send(frame.clone());
            }
        }
    }

    fn unregister(&self, host: Uuid, session: u64) {
        if let Ok(mut l) = self.inner.links.lock()
            && l.get(&host).is_some_and(|link| link.session == session)
        {
            l.remove(&host);
        }
    }

    fn log_slot(&self, run: Uuid) -> Arc<tokio::sync::Mutex<Option<RunLog>>> {
        let mut logs = self
            .inner
            .logs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        logs.entry(run).or_default().clone()
    }

    fn forget_log(&self, run: Uuid) {
        if let Ok(mut logs) = self.inner.logs.lock() {
            logs.remove(&run);
        }
    }
}

/// Something a runner reported, waiting to be written.
#[derive(Debug, Clone)]
pub(crate) enum Rec {
    Event {
        kind: String,
        host: Option<String>,
        task: Option<String>,
        data: Value,
    },
    Log {
        stream: LogStream,
        bytes: Vec<u8>,
    },
}

/// A row on its way to `run_events` or `run_log_chunks`.
enum Row {
    Event {
        seq: u64,
        kind: String,
        host: Option<String>,
        task: Option<String>,
        data: Value,
    },
    Log {
        seq: u64,
        stream: LogStream,
        bytes: Vec<u8>,
    },
}

const fn stream_str(s: LogStream) -> &'static str {
    match s {
        LogStream::Stdout => "stdout",
        LogStream::Stderr => "stderr",
    }
}

fn stream_of(s: &str) -> LogStream {
    if s == "stderr" {
        LogStream::Stderr
    } else {
        LogStream::Stdout
    }
}

fn now_rfc3339() -> String {
    crate::bindings::ts(time::OffsetDateTime::now_utc())
}

impl Row {
    fn event(&self, run: Uuid, at: &str) -> Event {
        match self {
            Row::Event {
                seq,
                kind,
                host,
                task,
                data,
            } => Event::RunEvent {
                run,
                seq: *seq,
                at: at.to_owned(),
                kind: kind.clone(),
                host: host.clone(),
                task: task.clone(),
                data: data.to_string(),
            },
            Row::Log { seq, stream, bytes } => Event::RunLog {
                run,
                seq: *seq,
                stream: *stream,
                text: String::from_utf8_lossy(bytes).into_owned(),
            },
        }
    }
}

async fn load_log(app: &AppState, run: Uuid) -> Result<RunLog, sqlx::Error> {
    let (seq, bytes): (i64, i64) = sqlx::query_as(
        "select greatest( \
           coalesce((select max(seq) from run_events where run_id = $1), 0), \
           coalesce((select max(seq) from run_log_chunks where run_id = $1), 0)), \
         coalesce((select sum(length(bytes)) from run_log_chunks where run_id = $1), 0)::bigint",
    )
    .bind(run)
    .fetch_one(&app.db)
    .await?;
    Ok(RunLog {
        next_seq: u64::try_from(seq).unwrap_or(0),
        head_bytes: u64::try_from(bytes).unwrap_or(0),
        ..RunLog::default()
    })
}

/// Writes `rows` in one transaction.
async fn write_rows(app: &AppState, run: Uuid, rows: &[Row]) -> Result<(), sqlx::Error> {
    let mut ev = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut lg = (Vec::new(), Vec::new(), Vec::new());
    for r in rows {
        match r {
            Row::Event {
                seq,
                kind,
                host,
                task,
                data,
            } => {
                ev.0.push(i64::try_from(*seq).unwrap_or(i64::MAX));
                ev.1.push(kind.clone());
                ev.2.push(host.clone());
                ev.3.push(task.clone());
                ev.4.push(data.clone());
            }
            Row::Log { seq, stream, bytes } => {
                lg.0.push(i64::try_from(*seq).unwrap_or(i64::MAX));
                lg.1.push(stream_str(*stream).to_owned());
                lg.2.push(bytes.clone());
            }
        }
    }
    let mut tx = app.db.begin().await?;
    if !ev.0.is_empty() {
        sqlx::query(
            "insert into run_events (run_id, seq, kind, host, task, data) \
             select $1, * from unnest($2::bigint[], $3::text[], $4::text[], $5::text[], $6::jsonb[])",
        )
        .bind(run)
        .bind(&ev.0)
        .bind(&ev.1)
        .bind(&ev.2)
        .bind(&ev.3)
        .bind(&ev.4)
        .execute(&mut *tx)
        .await?;
    }
    if !lg.0.is_empty() {
        sqlx::query(
            "insert into run_log_chunks (run_id, seq, stream, bytes) \
             select $1, * from unnest($2::bigint[], $3::text[], $4::bytea[])",
        )
        .bind(run)
        .bind(&lg.0)
        .bind(&lg.1)
        .bind(&lg.2)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Numbers, stores and broadcasts what a runner reported for `run`.
pub(crate) async fn record(app: &AppState, run: Uuid, recs: Vec<Rec>) {
    if recs.is_empty() {
        return;
    }
    let slot = app.runs.log_slot(run);
    let mut guard = slot.lock().await;
    if guard.is_none() {
        match load_log(app, run).await {
            Ok(l) => *guard = Some(l),
            Err(e) => {
                tracing::error!(error = %e, run = %run, "loading run output state failed");
                return;
            }
        }
    }
    let Some(log) = guard.as_mut() else { return };
    let cap = app.runs.settings().log_cap;
    let tail_cap = cap / 2;
    let head_cap = cap - tail_cap;
    let (seq0, head0, tail0, tail_bytes0, dropped0) = (
        log.next_seq,
        log.head_bytes,
        log.tail.clone(),
        log.tail_bytes,
        log.dropped,
    );
    let mut rows = Vec::new();
    for rec in recs {
        match rec {
            Rec::Event {
                kind,
                host,
                task,
                data,
            } => {
                log.next_seq += 1;
                rows.push(Row::Event {
                    seq: log.next_seq,
                    kind,
                    host,
                    task,
                    data,
                });
            }
            Rec::Log { stream, bytes } => {
                let room = usize::try_from(head_cap.saturating_sub(log.head_bytes))
                    .unwrap_or(usize::MAX)
                    .min(bytes.len());
                let (head, rest) = bytes.split_at(room);
                if !head.is_empty() {
                    log.next_seq += 1;
                    log.head_bytes += head.len() as u64;
                    rows.push(Row::Log {
                        seq: log.next_seq,
                        stream,
                        bytes: head.to_vec(),
                    });
                }
                if !rest.is_empty() {
                    push_tail(log, stream, rest, tail_cap);
                }
            }
        }
    }
    if rows.is_empty() {
        return;
    }
    if let Err(e) = write_rows(app, run, &rows).await {
        tracing::error!(error = %e, run = %run, "writing run output failed");
        (
            log.next_seq,
            log.head_bytes,
            log.tail,
            log.tail_bytes,
            log.dropped,
        ) = (seq0, head0, tail0, tail_bytes0, dropped0);
        return;
    }
    let at = now_rfc3339();
    for r in &rows {
        let _ = app.runs.inner.live.send((run, r.event(run, &at)));
    }
}

fn push_tail(log: &mut RunLog, stream: LogStream, bytes: &[u8], tail_cap: u64) {
    log.tail.push_back((stream, bytes.to_vec()));
    log.tail_bytes += bytes.len() as u64;
    while log.tail_bytes > tail_cap {
        let excess = log.tail_bytes - tail_cap;
        let Some(front) = log.tail.front_mut() else {
            break;
        };
        let len = front.1.len() as u64;
        if len <= excess {
            log.tail.pop_front();
            log.tail_bytes -= len;
            log.dropped += len;
        } else {
            let n = usize::try_from(excess).unwrap_or(usize::MAX);
            front.1.drain(..n);
            log.tail_bytes -= excess;
            log.dropped += excess;
        }
    }
}

/// Writes the truncation marker and the kept tail of `run`'s output, then forgets its state.
async fn finish_log(app: &AppState, run: Uuid) {
    let slot = app.runs.log_slot(run);
    let mut guard = slot.lock().await;
    if guard.is_none() {
        match load_log(app, run).await {
            Ok(l) => *guard = Some(l),
            Err(e) => {
                tracing::error!(error = %e, run = %run, "loading run output state failed");
                return;
            }
        }
    }
    let Some(log) = guard.as_mut() else { return };
    let mut rows = Vec::new();
    if log.dropped > 0 {
        log.next_seq += 1;
        rows.push(Row::Event {
            seq: log.next_seq,
            kind: "log_truncated".into(),
            host: None,
            task: None,
            data: json!({ "bytes": log.dropped }),
        });
        log.next_seq += 1;
        rows.push(Row::Log {
            seq: log.next_seq,
            stream: LogStream::Stderr,
            bytes: format!("\n[keel: {} bytes truncated]\n", log.dropped).into_bytes(),
        });
    }
    for (stream, bytes) in std::mem::take(&mut log.tail) {
        log.next_seq += 1;
        rows.push(Row::Log {
            seq: log.next_seq,
            stream,
            bytes,
        });
    }
    if !rows.is_empty() {
        if let Err(e) = write_rows(app, run, &rows).await {
            tracing::error!(error = %e, run = %run, "writing the run's log tail failed");
        } else {
            let at = now_rfc3339();
            for r in &rows {
                let _ = app.runs.inner.live.send((run, r.event(run, &at)));
            }
        }
    }
    *guard = None;
    drop(guard);
    app.runs.forget_log(run);
}

// ---- runner sessions -------------------------------------------------------------------------

/// Why a runner session was refused before it started.
#[derive(Debug)]
pub enum AttachError {
    /// The runner sent no `Hello` in time.
    Timeout,
    /// The runner's first frame was not a `Hello`.
    NoHello,
    /// The runner's `Hello` names another host.
    WrongHost,
    /// Unknown or revoked host.
    UnknownHost,
    /// The runner's `ansible_core` is below [`keel_exec::versions::MIN_ANSIBLE_CORE`].
    VersionTooOld,
    Codec(codec::CodecError),
    Db(sqlx::Error),
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => f.write_str("the runner sent no Hello in time"),
            Self::NoHello => f.write_str("the runner's first frame was not a Hello"),
            Self::WrongHost => f.write_str("the runner's Hello names another host"),
            Self::UnknownHost => f.write_str("unknown or revoked host"),
            Self::VersionTooOld => f.write_str("the runner's ansible-core is too old"),
            Self::Codec(e) => write!(f, "runner channel: {e}"),
            Self::Db(e) => write!(f, "database: {e}"),
        }
    }
}

impl std::error::Error for AttachError {}

impl From<codec::CodecError> for AttachError {
    fn from(e: codec::CodecError) -> Self {
        Self::Codec(e)
    }
}

impl From<sqlx::Error> for AttachError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

/// Starts a session for `host` on `stream` (already authenticated as that host). Returns once
/// the `Hello` is checked; the session runs in the background.
///
/// # Errors
///
/// The runner did not identify itself as `host`, or `host` is unknown or revoked.
pub async fn attach<S>(app: AppState, host: Uuid, mut stream: S) -> Result<(), AttachError>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let settings = app.runs.settings();
    let hello = tokio::time::timeout(settings.hello_timeout, codec::read_frame(&mut stream))
        .await
        .map_err(|_| AttachError::Timeout)??;
    let Some(Frame::Hello {
        host_id,
        version,
        ansible_core,
        python,
        override_quorum,
        ledger_head,
        ..
    }) = hello
    else {
        return Err(AttachError::NoHello);
    };
    if host_id != host.to_string() {
        return Err(AttachError::WrongHost);
    }
    if !keel_exec::versions::meets_min_ansible_core(&ansible_core) {
        let _ = codec::write_frame(
            &mut stream,
            &Frame::Refused {
                run_id: String::new(),
                reason: Refusal::VersionTooOld,
                detail: format!(
                    "ansible-core {} is below {}",
                    truncate(&ansible_core, 64),
                    keel_exec::versions::MIN_ANSIBLE_CORE
                ),
            },
        )
        .await;
        return Err(AttachError::VersionTooOld);
    }
    let row: Option<(Uuid,)> = sqlx::query_as(
        "update hosts set state = 'online', runner_version = $2, ansible_version = $3, \
         python_version = $4, override_quorum = $5, ledger_head = $6, last_seen_at = now() \
         where id = $1 and state <> 'revoked' returning workspace_id",
    )
    .bind(host)
    .bind(truncate(&version, 64))
    .bind(truncate(&ansible_core, 64))
    .bind(truncate(&python, 64))
    .bind(i32::try_from(override_quorum).unwrap_or(i32::MAX))
    .bind(ledger_head.map(|h| truncate(&h, 128)))
    .fetch_optional(&app.db)
    .await?;
    let Some((workspace,)) = row else {
        return Err(AttachError::UnknownHost);
    };
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Frame>();
    let writer_task = tokio::spawn(async move {
        while let Some(f) = out_rx.recv().await {
            if codec::write_frame(&mut writer, &f).await.is_err() {
                break;
            }
        }
    });
    let (in_tx, in_rx) = mpsc::channel::<Frame>(256);
    let reader_task = tokio::spawn(async move {
        while let Ok(Some(f)) = codec::read_frame(&mut reader).await {
            // A frame the runner may not send on this subsystem ends the session.
            if !(f.may_be_sent_by(Peer::Runner) && f.subsystem() == Some(Subsystem::Runner)) {
                break;
            }
            if in_tx.send(f).await.is_err() {
                break;
            }
        }
    });
    let close = Arc::new(Notify::new());
    let session = app.runs.register(host, out_tx.clone(), Arc::clone(&close));
    if let Some(cas) = app.ssh.get() {
        let _ = out_tx.send(Frame::Krl {
            bytes: cas.krl.to_bytes(),
        });
    }
    app.repos.push(
        workspace,
        Event::HostState {
            host,
            state: keel_protocol::HostStatus::Online,
        },
    );
    tracing::info!(host = %host, "runner connected");
    let app2 = app.clone();
    tokio::spawn(async move {
        let mut s = Session {
            app: app2.clone(),
            host,
            out: out_tx,
            active: HashMap::new(),
            buffer: Vec::new(),
        };
        let lost = s.run(in_rx, close).await;
        s.flush().await;
        app2.runs.unregister(host, session);
        reader_task.abort();
        drop(s.out);
        let _ = writer_task.await;
        tracing::info!(host = %host, lost, "runner disconnected");
        disconnected(&app2, host).await;
    });
    kick(&app);
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

struct Active {
    host_name: String,
    first_failure: Option<keel_protocol::FirstFailure>,
}

struct Session {
    app: AppState,
    host: Uuid,
    out: mpsc::UnboundedSender<Frame>,
    active: HashMap<Uuid, Active>,
    buffer: Vec<(Uuid, Rec)>,
}

impl Session {
    /// Runs until the stream ends (false) or the runner is lost (true).
    async fn run(&mut self, mut frames: mpsc::Receiver<Frame>, close: Arc<Notify>) -> bool {
        let settings = self.app.runs.settings();
        let mut ping = tokio::time::interval(settings.ping_interval);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ping.tick().await;
        let mut flush = tokio::time::interval(settings.flush_interval);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut missed = 0u32;
        loop {
            tokio::select! {
                f = frames.recv() => {
                    let Some(frame) = f else { return false };
                    missed = 0;
                    self.on_frame(frame).await;
                    if self.buffer.len() >= settings.flush_rows {
                        self.flush().await;
                    }
                }
                _ = ping.tick() => {
                    if missed >= settings.missed_pings {
                        return true;
                    }
                    missed += 1;
                    if self.out.send(Frame::Ping).is_err() {
                        return false;
                    }
                }
                _ = flush.tick() => self.flush().await,
                () = close.notified() => return true,
            }
        }
    }

    async fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        let mut by_run: BTreeMap<Uuid, Vec<Rec>> = BTreeMap::new();
        for (run, rec) in self.buffer.drain(..) {
            by_run.entry(run).or_default().push(rec);
        }
        for (run, recs) in by_run {
            record(&self.app, run, recs).await;
        }
    }

    /// The host name this session runs `run` as, if the run is assigned to this host.
    async fn active(&mut self, run_id: &str) -> Option<Uuid> {
        let run = Uuid::parse_str(run_id).ok()?;
        if self.active.contains_key(&run) {
            return Some(run);
        }
        let row: Option<(String,)> = sqlx::query_as(
            "select host_name from run_hosts where run_id = $1 and host_id = $2 \
             and state in ('sent', 'running')",
        )
        .bind(run)
        .bind(self.host)
        .fetch_optional(&self.app.db)
        .await
        .ok()
        .flatten();
        let (host_name,) = row?;
        self.active.insert(
            run,
            Active {
                host_name,
                first_failure: None,
            },
        );
        Some(run)
    }

    async fn on_frame(&mut self, frame: Frame) {
        match frame {
            Frame::Started { run_id } => {
                let Some(run) = self.active(&run_id).await else {
                    return;
                };
                self.flush().await;
                let r = sqlx::query(
                    "update run_hosts set state = 'running', started_at = now() \
                     where run_id = $1 and host_id = $2 and state = 'sent'",
                )
                .bind(run)
                .bind(self.host)
                .execute(&self.app.db)
                .await;
                if let Err(e) = r {
                    tracing::error!(error = %e, "recording Started failed");
                }
            }
            Frame::Refused {
                run_id,
                reason,
                detail,
            } => {
                let Some(run) = self.active(&run_id).await else {
                    return;
                };
                let name = self.active.get(&run).map(|a| a.host_name.clone());
                self.buffer.push((
                    run,
                    Rec::Event {
                        kind: "refused".into(),
                        host: name,
                        task: None,
                        data: json!({ "reason": reason.as_str(), "detail": truncate(&detail, 1024) }),
                    },
                ));
                self.flush().await;
                let r = sqlx::query(
                    "update run_hosts set state = 'refused', refusal = $3, ended_at = now() \
                     where run_id = $1 and host_id = $2 and state in ('sent', 'running')",
                )
                .bind(run)
                .bind(self.host)
                .bind(reason.as_str())
                .execute(&self.app.db)
                .await;
                if let Err(e) = r {
                    tracing::error!(error = %e, "recording Refused failed");
                }
                self.active.remove(&run);
                maybe_finish(&self.app, run).await;
            }
            Frame::Event { run_id, json, .. } => {
                let Some(run) = self.active(&run_id).await else {
                    return;
                };
                let rec = match events::parse_line(&json) {
                    Ok(ev) => {
                        if let KeelEvent::HostResult(r) = &ev
                            && let Some(ff) = r.first_failure()
                            && let Some(a) = self.active.get_mut(&run)
                            && a.first_failure.is_none()
                        {
                            a.first_failure = Some(keel_protocol::FirstFailure {
                                task: ff.task,
                                module: Some(ff.module).filter(|m| !m.is_empty()),
                                message: truncate(&ff.message, 4096),
                                rc: ff.rc,
                                unreachable: r.status == HostStatus::Unreachable,
                            });
                        }
                        Rec::Event {
                            kind: ev.kind().to_owned(),
                            host: ev.host().map(|h| truncate(h, 253)),
                            task: ev.task().map(|t| truncate(t, 1024)),
                            data: event_data(&ev),
                        }
                    }
                    Err(e) => Rec::Event {
                        kind: "invalid_event".into(),
                        host: None,
                        task: None,
                        data: json!({ "error": truncate(&e.to_string(), 256) }),
                    },
                };
                self.buffer.push((run, rec));
            }
            Frame::Log {
                run_id,
                stream,
                bytes,
                ..
            } => {
                let Some(run) = self.active(&run_id).await else {
                    return;
                };
                let stream = match stream {
                    frames::LogStream::Stdout => LogStream::Stdout,
                    frames::LogStream::Stderr => LogStream::Stderr,
                };
                self.buffer.push((run, Rec::Log { stream, bytes }));
            }
            Frame::Finished {
                run_id,
                exit_code,
                stats,
                ledger_line,
            } => {
                let Some(run) = self.active(&run_id).await else {
                    return;
                };
                self.flush().await;
                let active = self.active.remove(&run);
                let name = active
                    .as_ref()
                    .map(|a| a.host_name.clone())
                    .unwrap_or_default();
                let ff = active.and_then(|a| a.first_failure);
                let s = stats.get(&name).copied().unwrap_or_else(|| {
                    stats
                        .values()
                        .fold(events::HostStats::default(), |a, b| events::HostStats {
                            ok: a.ok + b.ok,
                            changed: a.changed + b.changed,
                            failures: a.failures + b.failures,
                            unreachable: a.unreachable + b.unreachable,
                            skipped: a.skipped + b.skipped,
                            rescued: a.rescued + b.rescued,
                            ignored: a.ignored + b.ignored,
                        })
                });
                if let Err(e) =
                    finished(&self.app, run, self.host, exit_code, s, ff, &ledger_line).await
                {
                    tracing::error!(error = %e, run = %run, "recording Finished failed");
                }
                maybe_finish(&self.app, run).await;
            }
            // A runner asking for a fresh certificate.
            Frame::RenewCert {} => match crate::hosts::renew(&self.app, self.host).await {
                Ok(cert) => {
                    let _ = self.out.send(Frame::Cert { cert });
                }
                Err(e) => {
                    tracing::warn!(error = %e, host = %self.host, "renewing a runner certificate failed")
                }
            },
            // Any frame already counted as liveness.
            _ => {}
        }
    }
}

/// What `run_events.data` keeps of a parsed event (results already censored by the parser).
fn event_data(ev: &KeelEvent) -> Value {
    let v = match ev {
        KeelEvent::PlaybookStart { playbook } => json!({ "playbook": playbook }),
        KeelEvent::PlayStart { play } => json!({ "play": play }),
        KeelEvent::TaskStart {
            task_id,
            action,
            handler,
            ..
        } => json!({ "task_id": task_id, "action": action, "handler": handler }),
        KeelEvent::HostResult(r) => json!({
            "task_id": r.task_id,
            "action": r.action,
            "ignore_errors": r.ignore_errors,
            "result": r.result,
        }),
        KeelEvent::Stats { stats } => json!({ "stats": stats }),
    };
    if v.to_string().len() > MAX_EVENT_DATA {
        json!({ "truncated": true })
    } else {
        v
    }
}

fn clamp(n: u64) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// The exit code a runner reports for a run cancelled before or while it ran.
const CANCELLED_EXIT: i32 = 130;

async fn finished(
    app: &AppState,
    run: Uuid,
    host: Uuid,
    exit_code: i32,
    s: events::HostStats,
    ff: Option<keel_protocol::FirstFailure>,
    ledger_line: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let (run_state,): (String,) =
        sqlx::query_as("select state from runs where id = $1 for no key update")
            .bind(run)
            .fetch_one(&mut *tx)
            .await?;
    // A runner may end a run with `Finished` and no `Started`: 125 is a setup failure (a short
    // or bad pack, or a refusal frames.rs has no variant for), 130 a cancel before start.
    let state = if exit_code == 0 {
        "succeeded"
    } else if run_state == "cancelled" || exit_code == CANCELLED_EXIT {
        "cancelled"
    } else {
        "failed"
    };
    let ledger = (!ledger_line.is_empty())
        .then(|| keel_exec::pack_builder::sha256_hex(ledger_line.as_bytes()));
    let n = sqlx::query(
        "update run_hosts set state = $3, ended_at = now(), ledger_hash = $4 \
         where run_id = $1 and host_id = $2 and state in ('sent', 'running')",
    )
    .bind(run)
    .bind(host)
    .bind(state)
    .bind(&ledger)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 1 {
        sqlx::query(
            "insert into run_host_results (run_id, host_id, ok, changed, failed, unreachable, \
             skipped, rescued, ignored, first_failure) \
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) on conflict do nothing",
        )
        .bind(run)
        .bind(host)
        .bind(clamp(s.ok))
        .bind(clamp(s.changed))
        .bind(clamp(s.failures))
        .bind(clamp(s.unreachable))
        .bind(clamp(s.skipped))
        .bind(clamp(s.rescued))
        .bind(clamp(s.ignored))
        .bind(ff.map(|f| json!(f)))
        .execute(&mut *tx)
        .await?;
        if state != "cancelled" {
            host_revision(&mut tx, run, host, state, ledger.as_deref(), ledger_line).await?;
        }
    }
    tx.commit().await
}

/// Records what the host applied (spec 055 US8, T101): its new base for the job.
///
/// Outside signed mode the host keeps no ledger and its base never moves, so the row is
/// `unchecked` and never counts as a base. In signed mode the host's own `end` ledger line
/// decides: only `succeeded` or `failed` moves the base (a failed run has partly applied the
/// target); a missing, unreadable or `cancelled` end line writes nothing, so Keel's belief stays
/// where the host's ledger does and the next `Assign` carries the base the host will check.
///
/// The authorization is Keel's own record: `override` or `baseline` when an approved review
/// group covers the host (with the approvals behind it), `signed` otherwise. The `end` line
/// does not name signers, so `signers` stays empty.
async fn host_revision(
    tx: &mut sqlx::PgConnection,
    run: Uuid,
    host: Uuid,
    outcome: &str,
    ledger_hash: Option<&str>,
    ledger_line: &str,
) -> Result<(), sqlx::Error> {
    let (job, commit, signed, name): (Option<Uuid>, String, bool, String) = sqlx::query_as(
        "select r.job_id, r.commit, r.signed_mode, rh.host_name from runs r \
         join run_hosts rh on rh.run_id = r.id and rh.host_id = $2 where r.id = $1",
    )
    .bind(run)
    .bind(host)
    .fetch_one(&mut *tx)
    .await?;
    let outcome = if signed {
        match ledger_outcome(ledger_line, &run.to_string()) {
            Some(o) => o,
            None => return Ok(()),
        }
    } else if outcome == "succeeded" {
        "succeeded"
    } else {
        "failed"
    };
    let group: Option<(Uuid, Option<String>)> = if signed {
        sqlx::query_as(
            "select id, base_commit from review_groups \
             where run_id = $1 and state = 'approved' and $2 = any(hosts) limit 1",
        )
        .bind(run)
        .bind(&name)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        None
    };
    let (kind, approvals): (&str, Vec<Uuid>) = match &group {
        None if !signed => ("unchecked", Vec::new()),
        None => ("signed", Vec::new()),
        Some((g, base)) => {
            let ids: Vec<(Uuid,)> = sqlx::query_as(
                "select id from approvals where review_group_id = $1 and kind = 'override' \
                 and decision = 'approve' order by at",
            )
            .bind(g)
            .fetch_all(&mut *tx)
            .await?;
            let kind = if base.is_none() {
                "baseline"
            } else {
                "override"
            };
            (kind, ids.into_iter().map(|r| r.0).collect())
        }
    };
    sqlx::query(
        "insert into host_revisions (id, host_id, job_id, commit, run_id, authorization_kind, \
         signers, approval_ids, ledger_hash, outcome) \
         values ($1, $2, $3, $4, $5, $6, '{}', $7, $8, $9)",
    )
    .bind(Uuid::now_v7())
    .bind(host)
    .bind(job)
    .bind(commit)
    .bind(run)
    .bind(kind)
    .bind(&approvals)
    .bind(ledger_hash)
    .bind(outcome)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The outcome of a host's `end` ledger line for `run`, when it moves the base.
fn ledger_outcome(line: &str, run: &str) -> Option<&'static str> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("event")?.as_str()? != "end" || v.get("run")?.as_str()? != run {
        return None;
    }
    match v.get("outcome")?.as_str()? {
        "succeeded" => Some("succeeded"),
        "failed" => Some("failed"),
        _ => None,
    }
}

/// A session ended: the host is offline and its in-flight hosts are lost.
async fn disconnected(app: &AppState, host: Uuid) {
    let mut offline = None;
    let result: Result<Vec<(Uuid, String)>, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        // A reconnect may already have brought the host back.
        if !app.runs.connected().contains(&host) {
            let ws: Option<(Uuid,)> = sqlx::query_as(
                "update hosts set state = 'offline' where id = $1 and state = 'online' \
                 returning workspace_id",
            )
            .bind(host)
            .fetch_optional(&mut *tx)
            .await?;
            offline = ws.map(|w| w.0);
        }
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "update run_hosts set state = 'lost', ended_at = now() \
             where host_id = $1 and state in ('sent', 'running') returning run_id, host_name",
        )
        .bind(host)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows)
    }
    .await;
    if result.is_ok()
        && let Some(ws) = offline
    {
        app.repos.push(
            ws,
            Event::HostState {
                host,
                state: keel_protocol::HostStatus::Offline,
            },
        );
    }
    match result {
        Ok(rows) => {
            for (run, name) in rows {
                record(
                    app,
                    run,
                    vec![Rec::Event {
                        kind: "runner_lost".into(),
                        host: Some(name),
                        task: None,
                        data: json!({}),
                    }],
                )
                .await;
                maybe_finish(app, run).await;
            }
        }
        Err(e) => tracing::error!(error = %e, host = %host, "recording a lost runner failed"),
    }
}

/// Ends `run` once every host is terminal: its state, summary, log tail, audit and push.
pub(crate) async fn maybe_finish(app: &AppState, run: Uuid) {
    if let Err(e) = try_finish(app, run).await {
        tracing::error!(error = %e, run = %run, "finishing a run failed");
    }
}

async fn try_finish(app: &AppState, run: Uuid) -> Result<(), sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let row: Option<(String, Option<String>, Uuid, bool)> = sqlx::query_as(
        "select state, state_reason, workspace_id, ended_at is not null from runs \
         where id = $1 for no key update",
    )
    .bind(run)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((state, reason, workspace, ended)) = row else {
        return Ok(());
    };
    if ended {
        return Ok(());
    }
    let hosts: Vec<(String,)> = sqlx::query_as("select state from run_hosts where run_id = $1")
        .bind(run)
        .fetch_all(&mut *tx)
        .await?;
    let hosts: Vec<HostState> = hosts.iter().filter_map(|h| h.0.parse().ok()).collect();
    if hosts.iter().any(|h| !h.is_terminal()) {
        return Ok(());
    }
    let from: RunState = state.parse().unwrap_or(RunState::Failed);
    let next = if from.is_terminal() {
        core_run::RunStatus {
            state: from,
            reason: reason.as_deref().and_then(|r| r.parse().ok()),
        }
    } else if from == RunState::Running {
        match core_run::transition(from, &RunEvent::HostsFinished(hosts.clone())) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        }
    } else {
        // Hosts finished before the run was dispatched: only a cancel or a failure gets here.
        match core_run::summarize(&hosts) {
            Some(s) => s,
            None => return Ok(()),
        }
    };
    // The tail goes in before the run reads as ended.
    finish_log(app, run).await;
    let summary = summary(&mut tx, run).await?;
    sqlx::query(
        "update runs set state = $2, state_reason = $3, ended_at = now(), summary = $4 \
         where id = $1",
    )
    .bind(run)
    .bind(next.state.as_str())
    .bind(next.reason.map(|r| r.as_str()))
    .bind(json!(summary))
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        Entry {
            actor: &Actor::System,
            action: "run_finished",
            workspace: Some(WorkspaceId(workspace)),
            target: json!({ "run": run }),
            decision: Decision::Allowed,
            reason: next.reason.map(|r| r.as_str()),
            context: json!({ "state": next.state.as_str(), "hosts": summary.hosts }),
        },
    )
    .await?;
    tx.commit().await?;
    push_state(app, workspace, run, next);
    Ok(())
}

/// The run's host counts and summed stats.
pub(crate) async fn summary(
    conn: &mut PgConnection,
    run: Uuid,
) -> Result<keel_protocol::RunSummary, sqlx::Error> {
    let states: Vec<(String,)> = sqlx::query_as("select state from run_hosts where run_id = $1")
        .bind(run)
        .fetch_all(&mut *conn)
        .await?;
    let mut s = keel_protocol::RunSummary::default();
    for (st,) in &states {
        s.hosts += 1;
        match st.as_str() {
            "succeeded" => s.succeeded += 1,
            "failed" => s.failed += 1,
            "refused" => s.refused += 1,
            "lost" => s.lost += 1,
            "cancelled" => s.cancelled += 1,
            _ => {}
        }
    }
    let (ok, changed, failed, unreachable, skipped, rescued, ignored): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
        "select coalesce(sum(ok), 0)::bigint, coalesce(sum(changed), 0)::bigint, \
         coalesce(sum(failed), 0)::bigint, coalesce(sum(unreachable), 0)::bigint, \
         coalesce(sum(skipped), 0)::bigint, coalesce(sum(rescued), 0)::bigint, \
         coalesce(sum(ignored), 0)::bigint from run_host_results where run_id = $1",
    )
    .bind(run)
    .fetch_one(&mut *conn)
    .await?;
    let u = |n: i64| u32::try_from(n).unwrap_or(u32::MAX);
    s.stats = keel_protocol::HostStats {
        ok: u(ok),
        changed: u(changed),
        failed: u(failed),
        unreachable: u(unreachable),
        skipped: u(skipped),
        rescued: u(rescued),
        ignored: u(ignored),
    };
    Ok(s)
}

/// Pushes `RunState` to the workspace's viewers.
pub(crate) fn push_state(app: &AppState, workspace: Uuid, run: Uuid, s: core_run::RunStatus) {
    app.repos.push(
        workspace,
        Event::RunState {
            run,
            state: crate::bindings::enum_from(s.state.as_str())
                .unwrap_or(keel_protocol::RunPhase::Failed),
            reason: s
                .reason
                .and_then(|r| crate::bindings::enum_from(r.as_str())),
        },
    );
}

// ---- dispatch --------------------------------------------------------------------------------

/// Dispatches what is ready, in the background.
pub fn kick(app: &AppState) {
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(e) = dispatch_ready(&app).await {
            tracing::error!(error = %e, "dispatch failed");
        }
    });
}

async fn dispatch_ready(app: &AppState) -> Result<(), sqlx::Error> {
    let connected = app.runs.connected();
    if connected.is_empty() {
        return Ok(());
    }
    let mut tx = app.db.begin().await?;
    let claimed: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "select rh.run_id, rh.host_id from run_hosts rh join runs r on r.id = rh.run_id \
         where rh.state = 'queued' and rh.host_id = any($1) and r.state in ('queued', 'running') \
         order by r.created_at for update of rh skip locked",
    )
    .bind(&connected)
    .fetch_all(&mut *tx)
    .await?;
    if claimed.is_empty() {
        return Ok(());
    }
    let (runs, hosts): (Vec<Uuid>, Vec<Uuid>) = claimed.iter().copied().unzip();
    sqlx::query(
        "update run_hosts rh set state = 'sent' from unnest($1::uuid[], $2::uuid[]) c(run, host) \
         where rh.run_id = c.run and rh.host_id = c.host",
    )
    .bind(&runs)
    .bind(&hosts)
    .execute(&mut *tx)
    .await?;
    let started: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "update runs set state = 'running', started_at = now() \
         where id = any($1) and state = 'queued' returning id, workspace_id",
    )
    .bind(&runs)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    for (run, ws) in started {
        if let Ok(s) = core_run::transition(RunState::Queued, &RunEvent::Dispatched) {
            push_state(app, ws, run, s);
        }
    }
    for (run, host) in claimed {
        let app = app.clone();
        tokio::spawn(async move {
            if let Err(why) = assign(&app, run, host).await {
                fail_host(&app, run, host, why).await;
            }
        });
    }
    Ok(())
}

/// Why a claimed host could not be sent its run. Codes only: never a secret.
#[derive(Debug, Clone, Copy)]
enum AssignError {
    ArtifactMissing,
    CredentialUnavailable,
    NotConnected,
    Internal,
}

impl AssignError {
    const fn code(self) -> &'static str {
        match self {
            Self::ArtifactMissing => "artifact_missing",
            Self::CredentialUnavailable => "credential_unavailable",
            Self::NotConnected => "runner_lost",
            Self::Internal => "internal",
        }
    }
}

async fn assign(app: &AppState, run: Uuid, host: Uuid) -> Result<(), AssignError> {
    type Row = (
        Option<Uuid>,
        String,
        String,
        Option<Value>,
        Uuid,
        bool,
        Option<String>,
        String,
        Option<i32>,
    );
    let row: Option<Row> = sqlx::query_as(
        "select r.binding_id, r.job_name, r.commit, j.definition, r.workspace_id, r.signed_mode, \
                (select hr.commit from host_revisions hr join runs pr on pr.id = hr.run_id \
                  where hr.host_id = rh.host_id and pr.job_name = r.job_name \
                    and hr.authorization_kind <> 'unchecked' \
                  order by hr.applied_at desc, hr.id desc limit 1), \
                rh.host_name, \
                greatest(b.override_quorum, h.override_quorum, 1) \
         from runs r \
         join run_hosts rh on rh.run_id = r.id and rh.host_id = $2 \
         join hosts h on h.id = rh.host_id \
         left join jobs j on j.id = r.job_id \
         left join repo_bindings b on b.id = r.binding_id \
         where r.id = $1",
    )
    .bind(run)
    .bind(host)
    .fetch_optional(&app.db)
    .await
    .map_err(|_| AssignError::Internal)?;
    let Some((Some(binding), job, commit, def, workspace, signed, base, host_name, quorum)) = row
    else {
        return Err(AssignError::ArtifactMissing);
    };
    let def: Option<keel_core::manifest::JobDefinition> =
        def.and_then(|d| serde_json::from_value(d).ok());
    let Some(def) = def else {
        return Err(AssignError::ArtifactMissing);
    };
    let dir = crate::sync::data_dir(app);
    let target = commit.clone();
    let pack = tokio::task::spawn_blocking(move || {
        Mirror::open(&dir, binding)?.build_pack(None, &CommitId(target))
    })
    .await
    .map_err(|_| AssignError::Internal)?
    .map_err(|_| AssignError::ArtifactMissing)?;
    let items = secrets(app, binding, host, &def.credentials).await?;
    let timeout = def.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    let deadline = u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp())
        .unwrap_or(0)
        .saturating_add(timeout);
    let run_id = run.to_string();
    let mut frames = vec![
        Frame::Assign {
            run_id: run_id.clone(),
            workspace: workspace.to_string(),
            binding: binding.to_string(),
            job,
            base,
            target: commit,
            deadline,
            pack_size: pack.bytes.len() as u64,
            quorum: quorum.and_then(|q| u32::try_from(q).ok()),
        },
        Frame::Secrets {
            run_id: run_id.clone(),
            items,
        },
    ];
    // A signed-mode host waits for `Approvals`, empty when every commit is signed.
    if signed {
        frames.push(Frame::Approvals {
            run_id: run_id.clone(),
            items: approvals(app, run, &host_name).await?,
        });
    }
    frames.extend(keel_exec::pack_builder::frames(&run_id, &pack.bytes));
    for f in frames {
        if !app.runs.send(host, f) {
            return Err(AssignError::NotConnected);
        }
    }
    Ok(())
}

/// The approved overrides and baselines of `run` whose review group covers `host_name`, as the
/// host verifies them. The assertion goes out exactly as stored; a row whose assertion cannot be
/// read is an internal error (the run is not sent with a partial set).
async fn approvals(
    app: &AppState,
    run: Uuid,
    host_name: &str,
) -> Result<Vec<frames::Approval>, AssignError> {
    type Row = (
        String,
        String,
        Value,
        time::OffsetDateTime,
        Vec<String>,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "select coalesce(u.email, u.display_name), a.comment, a.assertion, a.expires_at, \
         g.hosts, g.base_commit \
         from approvals a \
         join review_groups g on g.id = a.review_group_id \
         join users u on u.id = a.approver_user_id \
         where a.run_id = $1 and a.kind = 'override' and a.decision = 'approve' \
           and $2 = any(g.hosts) \
         order by a.at",
    )
    .bind(run)
    .bind(host_name)
    .fetch_all(&app.db)
    .await
    .map_err(|_| AssignError::Internal)?;
    let mut out = Vec::with_capacity(rows.len());
    for (email, comment, assertion, expires_at, hosts, base) in rows {
        let field = |name: &str| -> Result<String, AssignError> {
            assertion
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or(AssignError::Internal)
        };
        let mut hosts = hosts;
        hosts.sort();
        hosts.dedup();
        out.push(frames::Approval {
            // The challenge's kind: `baseline` exactly when the group has no base.
            kind: if base.is_none() {
                "baseline"
            } else {
                "override"
            }
            .into(),
            approver: email,
            credential_id: field("credential_id")?,
            authenticator_data: field("authenticator_data")?,
            client_data_json: field("client_data_json")?,
            signature: field("signature")?,
            expires_at: u64::try_from(expires_at.unix_timestamp())
                .map_err(|_| AssignError::Internal)?,
            comment,
            hosts,
        });
    }
    Ok(out)
}

/// The job's credentials, as granted to the binding for one of the host's profiles.
async fn secrets(
    app: &AppState,
    binding: Uuid,
    host: Uuid,
    names: &[String],
) -> Result<Vec<SecretItem>, AssignError> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "select distinct c.id, c.name from credentials c \
         join credential_grants g on g.credential_id = c.id \
         where g.binding_id = $1 and c.name = any($2) and c.kind not like 'git_%' \
           and (cardinality(g.profiles) = 0 or g.profiles && coalesce((select array_agg(p.name) \
                from host_profiles hp join profiles p on p.id = hp.profile_id \
                where hp.host_id = $3), '{}'))",
    )
    .bind(binding)
    .bind(names)
    .bind(host)
    .fetch_all(&app.db)
    .await
    .map_err(|_| AssignError::Internal)?;
    if rows.len() != names.len() {
        return Err(AssignError::CredentialUnavailable);
    }
    let mut items = Vec::with_capacity(rows.len());
    for (id, name) in rows {
        let (kind, value) = crate::credentials::open(app, id)
            .await
            .map_err(|_| AssignError::CredentialUnavailable)?;
        let value = {
            use secrecy::ExposeSecret as _;
            String::from_utf8(value.expose_secret().to_vec())
                .map_err(|_| AssignError::CredentialUnavailable)?
        };
        items.push(SecretItem {
            name,
            kind,
            value: SecretString::new(value),
        });
    }
    Ok(items)
}

async fn fail_host(app: &AppState, run: Uuid, host: Uuid, why: AssignError) {
    let name: Option<(String,)> = sqlx::query_as(
        "update run_hosts set state = $3, ended_at = now() \
         where run_id = $1 and host_id = $2 and state = 'sent' returning host_name",
    )
    .bind(run)
    .bind(host)
    .bind(if matches!(why, AssignError::NotConnected) {
        "lost"
    } else {
        "failed"
    })
    .fetch_optional(&app.db)
    .await
    .ok()
    .flatten();
    tracing::warn!(run = %run, host = %host, code = why.code(), "a host could not be sent its run");
    if let Some((name,)) = name {
        record(
            app,
            run,
            vec![Rec::Event {
                kind: why.code().into(),
                host: Some(name),
                task: None,
                data: json!({}),
            }],
        )
        .await;
    }
    maybe_finish(app, run).await;
}

/// Sends `Cancel` to `host`'s runner (best effort).
pub(crate) fn cancel(app: &AppState, run: Uuid, host: Uuid) {
    app.runs.send(
        host,
        Frame::Cancel {
            run_id: run.to_string(),
        },
    );
}

// ---- history ---------------------------------------------------------------------------------

/// Everything stored for `run` after `after_seq`, in order.
///
/// # Errors
///
/// The database failed.
pub async fn history(app: &AppState, run: Uuid, after_seq: u64) -> Result<Vec<Event>, sqlx::Error> {
    let after = i64::try_from(after_seq).unwrap_or(i64::MAX);
    type Ev = (
        i64,
        time::OffsetDateTime,
        String,
        Option<String>,
        Option<String>,
        Value,
    );
    let evs: Vec<Ev> = sqlx::query_as(
        "select seq, at, kind, host, task, data from run_events \
         where run_id = $1 and seq > $2 order by seq",
    )
    .bind(run)
    .bind(after)
    .fetch_all(&app.db)
    .await?;
    let logs: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "select seq, stream, bytes from run_log_chunks where run_id = $1 and seq > $2 order by seq",
    )
    .bind(run)
    .bind(after)
    .fetch_all(&app.db)
    .await?;
    let mut out: Vec<(i64, Event)> = Vec::with_capacity(evs.len() + logs.len());
    for (seq, at, kind, host, task, data) in evs {
        out.push((
            seq,
            Event::RunEvent {
                run,
                seq: u64::try_from(seq).unwrap_or(0),
                at: crate::bindings::ts(at),
                kind,
                host,
                task,
                data: data.to_string(),
            },
        ));
    }
    for (seq, stream, bytes) in logs {
        out.push((
            seq,
            Event::RunLog {
                run,
                seq: u64::try_from(seq).unwrap_or(0),
                stream: stream_of(&stream),
                text: String::from_utf8_lossy(&bytes).into_owned(),
            },
        ));
    }
    out.sort_by_key(|(s, _)| *s);
    Ok(out.into_iter().map(|(_, e)| e).collect())
}
