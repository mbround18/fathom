//! 036-incluster-daemon: desktop client for the optional in-cluster daemon.
//!
//! Transport is the user's own kube client issuing raw requests against the aggregated API
//! (`/apis/daemon.fathom.dev/v1`); the daemon never receives tokens or credentials from us, it
//! is told the caller's identity by the API server. With the per-context preference `off` (the
//! default) nothing here issues a single request. No polling: one probe, then a stream that
//! resumes from a persisted cursor; failures back off 5 s to 5 min.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use futures_util::AsyncBufReadExt;

use crate::protocol::{
    DAEMON_PROTOCOL_MAJOR, DaemonCapabilities, DaemonFrame, DaemonMetricSample, DaemonState,
    DaemonStatus, DaemonTimelineEntry, EdgeDelta, EventCursor, MetricKind, MetricSeries, SeriesKey,
    SystemEvent, TopologyDelta, TopologyEdge, TopologyNode, TruncationMarker,
};

pub(super) const API_BASE: &str = "/apis/daemon.fathom.dev/v1";
pub(super) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const BACKOFF_BASE: Duration = Duration::from_secs(5);
pub(super) const BACKOFF_CAP: Duration = Duration::from_secs(300);
pub(super) const BATCH_MAX: usize = 200;
pub(super) const BATCH_WINDOW: Duration = Duration::from_millis(250);
/// Heartbeats arrive every 15 s; three missed means the stream is dead.
pub(super) const STREAM_IDLE_LIMIT: Duration = Duration::from_secs(45);
/// Client-side timeline buffer per context.
pub(super) const TIMELINE_CAP: usize = 2000;
const CURSOR_SAVE_EVERY: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TransportError {
    Http { code: u16, message: String },
    Timeout,
    Other(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http { code, message } => write!(f, "HTTP {code}: {message}"),
            Self::Timeout => write!(f, "timed out"),
            Self::Other(m) => write!(f, "{m}"),
        }
    }
}

pub(super) type LineStream = BoxStream<'static, std::io::Result<String>>;

/// Abstraction over "GET a path on the daemon". Tests supply a scripted implementation.
#[async_trait]
pub(super) trait DaemonTransport: Send + Sync {
    async fn get_text(&self, path: &str) -> Result<String, TransportError>;
    async fn open_stream(&self, path: &str) -> Result<LineStream, TransportError>;
}

pub(super) struct KubeTransport(pub(super) kube::Client);

fn from_kube(e: kube::Error) -> TransportError {
    match e {
        kube::Error::Api(status) => TransportError::Http {
            code: status.code,
            message: status.message,
        },
        other => TransportError::Other(other.to_string()),
    }
}

fn get_request(path: &str) -> Result<http::Request<Vec<u8>>, TransportError> {
    http::Request::get(path)
        .body(Vec::new())
        .map_err(|e| TransportError::Other(e.to_string()))
}

#[async_trait]
impl DaemonTransport for KubeTransport {
    async fn get_text(&self, path: &str) -> Result<String, TransportError> {
        self.0
            .request_text(get_request(path)?)
            .await
            .map_err(from_kube)
    }

    async fn open_stream(&self, path: &str) -> Result<LineStream, TransportError> {
        let reader = self
            .0
            .request_stream(get_request(path)?)
            .await
            .map_err(from_kube)?;
        Ok(reader.lines().boxed())
    }
}

// ---------------------------------------------------------------------------------------
// Pure helpers (classification, backoff, framing, batching)
// ---------------------------------------------------------------------------------------

/// Result of classifying a probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Classified {
    pub state: DaemonState,
    pub detail: Option<String>,
    pub capabilities: Option<DaemonCapabilities>,
}

fn classified(state: DaemonState, detail: Option<String>) -> Classified {
    Classified {
        state,
        detail,
        capabilities: None,
    }
}

/// Map a `/v1/capabilities` result to a daemon state (contract: "Client classification").
pub(super) fn classify_probe(result: Result<String, TransportError>) -> Classified {
    match result {
        Ok(text) => match serde_json::from_str::<DaemonCapabilities>(&text) {
            Ok(caps) if caps.protocol_major != DAEMON_PROTOCOL_MAJOR => classified(
                DaemonState::VersionMismatch,
                Some(format!(
                    "Daemon speaks protocol major {}, this client speaks {}",
                    caps.protocol_major, DAEMON_PROTOCOL_MAJOR
                )),
            ),
            Ok(caps) => Classified {
                state: DaemonState::Available,
                detail: None,
                capabilities: Some(caps),
            },
            Err(e) => classified(
                DaemonState::Unavailable,
                Some(format!("Unreadable capabilities response: {e}")),
            ),
        },
        Err(TransportError::Http { code, message }) => match code {
            404 | 503 => classified(DaemonState::NotInstalled, None),
            401 | 403 => classified(DaemonState::NoAccess, Some(message)),
            _ => classified(
                DaemonState::Unavailable,
                Some(format!("HTTP {code}: {message}")),
            ),
        },
        Err(TransportError::Timeout) => classified(
            DaemonState::Unavailable,
            Some(format!("Timed out after {}s", PROBE_TIMEOUT.as_secs())),
        ),
        Err(TransportError::Other(m)) => classified(DaemonState::Unavailable, Some(m)),
    }
}

/// Backoff for the `attempt`-th consecutive failure (0-based): `base` doubling to `cap`.
pub(super) fn backoff_with(attempt: u32, base: Duration, cap: Duration) -> Duration {
    base.saturating_mul(1u32 << attempt.min(10)).min(cap)
}

/// Parse one NDJSON line; unknown frame types / malformed lines are ignored (forward compat).
pub(super) fn parse_frame(line: &str) -> Option<DaemonFrame> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    serde_json::from_str(line).ok()
}

pub(super) fn stream_path(cursor: Option<&EventCursor>) -> String {
    match cursor {
        Some(c) => format!("{API_BASE}/v1/stream?epoch={}&after={}", c.epoch, c.seq),
        None => format!("{API_BASE}/v1/stream"),
    }
}

/// Fold `next` into `acc` (later wins): nodes replace by uid, removals drop earlier adds.
pub(super) fn merge_delta(acc: &mut TopologyDelta, next: TopologyDelta) {
    acc.seq = acc.seq.max(next.seq);
    for uid in &next.removed {
        acc.added.retain(|n| &n.uid != uid);
        if !acc.removed.contains(uid) {
            acc.removed.push(uid.clone());
        }
    }
    for node in next.added {
        acc.removed.retain(|u| u != &node.uid);
        acc.added.retain(|n| n.uid != node.uid);
        acc.added.push(node);
    }
    for e in next.edges.removed {
        if let Some(pos) = acc.edges.added.iter().position(|a| a == &e) {
            acc.edges.added.remove(pos);
        } else if !acc.edges.removed.contains(&e) {
            acc.edges.removed.push(e);
        }
    }
    for e in next.edges.added {
        acc.edges.removed.retain(|r| r != &e);
        if !acc.edges.added.contains(&e) {
            acc.edges.added.push(e);
        }
    }
}

/// Accumulates stream output between flushes (<= 200 entries / 250 ms).
#[derive(Default)]
pub(super) struct Batcher {
    pub entries: Vec<DaemonTimelineEntry>,
    pub truncated: Option<TruncationMarker>,
    pub topology: Option<TopologyDelta>,
}

impl Batcher {
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.truncated.is_none() && self.topology.is_none()
    }
    pub(super) fn is_full(&self) -> bool {
        self.entries.len() >= BATCH_MAX
    }
}

// ---------------------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------------------

pub(super) type Emitter = Arc<dyn Fn(SystemEvent) + Send + Sync>;
pub(super) type CursorSaver = Arc<dyn Fn(EventCursor) + Send + Sync>;
pub(super) type Timeline = Arc<Mutex<VecDeque<DaemonTimelineEntry>>>;

#[derive(Clone, Copy)]
pub(super) struct Timings {
    pub probe_timeout: Duration,
    pub backoff_base: Duration,
    pub backoff_cap: Duration,
    pub batch_window: Duration,
    pub idle_limit: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            probe_timeout: PROBE_TIMEOUT,
            backoff_base: BACKOFF_BASE,
            backoff_cap: BACKOFF_CAP,
            batch_window: BATCH_WINDOW,
            idle_limit: STREAM_IDLE_LIMIT,
        }
    }
}

/// Everything one per-context session task needs.
pub(super) struct Session {
    pub context: String,
    pub id: u64,
    /// Latest session id for this context; anything emitted by an older session is dropped.
    pub current: Arc<AtomicU64>,
    pub transport: Arc<dyn DaemonTransport>,
    pub emit: Emitter,
    pub save_cursor: CursorSaver,
    pub timeline: Timeline,
    pub cursor: Option<EventCursor>,
    pub timings: Timings,
    /// Latest capabilities once available (used by on-demand requests).
    pub caps: Arc<Mutex<Option<DaemonCapabilities>>>,
}

impl Session {
    fn live(&self) -> bool {
        self.current.load(Ordering::SeqCst) == self.id
    }

    fn send(&self, event: SystemEvent) {
        if self.live() {
            (self.emit)(event);
        }
    }

    fn status(
        &self,
        state: DaemonState,
        detail: Option<String>,
        capabilities: Option<DaemonCapabilities>,
        next_retry: Option<Duration>,
    ) {
        let now = chrono::Utc::now();
        self.send(SystemEvent::DaemonStatusUpdated {
            context: self.context.clone(),
            status: DaemonStatus {
                context: self.context.clone(),
                state,
                detail,
                capabilities,
                last_event_at: None,
                next_retry_at: next_retry.and_then(|d| {
                    chrono::Duration::from_std(d)
                        .ok()
                        .map(|d| (now + d).to_rfc3339())
                }),
            },
        });
    }

    fn backoff(&self, attempt: u32) -> Duration {
        backoff_with(attempt, self.timings.backoff_base, self.timings.backoff_cap)
    }

    fn flush(&self, batch: &mut Batcher) {
        if batch.is_empty() {
            return;
        }
        let b = std::mem::take(batch);
        if let Some(delta) = b.topology {
            self.send(SystemEvent::DaemonTopologyUpdated {
                context: self.context.clone(),
                session: self.id,
                delta,
            });
        }
        if !b.entries.is_empty() || b.truncated.is_some() {
            if let Ok(mut t) = self.timeline.lock() {
                if b.truncated.is_some() {
                    t.clear();
                }
                t.extend(b.entries.iter().cloned());
                while t.len() > TIMELINE_CAP {
                    t.pop_front();
                }
            }
            self.send(SystemEvent::DaemonEventsBatch {
                context: self.context.clone(),
                session: self.id,
                events: b.entries,
                truncated: b.truncated,
            });
        }
    }

    /// Probe, stream, back off, repeat, until aborted or superseded.
    pub(super) async fn run(mut self) {
        let mut attempt: u32 = 0;
        while self.live() {
            self.status(DaemonState::Probing, None, None, None);
            let probe = match tokio::time::timeout(
                self.timings.probe_timeout,
                self.transport
                    .get_text(&format!("{API_BASE}/v1/capabilities")),
            )
            .await
            {
                Ok(r) => r,
                Err(_) => Err(TransportError::Timeout),
            };
            let c = classify_probe(probe);
            if c.state != DaemonState::Available {
                let wait = self.backoff(attempt);
                attempt = attempt.saturating_add(1);
                self.status(c.state, c.detail, None, Some(wait));
                tokio::time::sleep(wait).await;
                continue;
            }
            attempt = 0;
            let caps = c.capabilities.expect("available carries capabilities");
            if let Ok(mut g) = self.caps.lock() {
                *g = Some(caps.clone());
            }
            self.status(DaemonState::Available, None, Some(caps.clone()), None);
            if caps.topology {
                self.load_topology().await;
            }
            let reason = self.stream(&caps).await;
            if !self.live() {
                return;
            }
            let wait = self.backoff(attempt);
            attempt = attempt.saturating_add(1);
            self.status(DaemonState::Unavailable, Some(reason), None, Some(wait));
            tokio::time::sleep(wait).await;
        }
    }

    async fn load_topology(&self) {
        let Ok(text) = self
            .transport
            .get_text(&format!("{API_BASE}/v1/topology"))
            .await
        else {
            return;
        };
        #[derive(serde::Deserialize)]
        struct Snap {
            seq: u64,
            nodes: Vec<TopologyNode>,
            edges: Vec<TopologyEdge>,
        }
        if let Ok(s) = serde_json::from_str::<Snap>(&text) {
            self.send(SystemEvent::DaemonTopologyUpdated {
                context: self.context.clone(),
                session: self.id,
                delta: TopologyDelta {
                    seq: s.seq,
                    added: s.nodes,
                    removed: Vec::new(),
                    edges: EdgeDelta {
                        added: s.edges,
                        removed: Vec::new(),
                    },
                },
            });
        }
    }

    /// Fold one frame into `batch`/cursor; `Some(reason)` when the stream should end.
    fn apply_frame(
        &mut self,
        frame: DaemonFrame,
        caps: &DaemonCapabilities,
        batch: &mut Batcher,
    ) -> Option<String> {
        match frame {
            DaemonFrame::Hello { .. } => {}
            DaemonFrame::Event { seq, time, event } => {
                self.cursor = Some(EventCursor {
                    epoch: caps.epoch.clone(),
                    seq,
                });
                batch.entries.push(DaemonTimelineEntry { seq, time, event });
            }
            DaemonFrame::Truncated {
                oldest_seq,
                oldest_time,
                dropped,
                reason,
            } => {
                self.cursor = Some(EventCursor {
                    epoch: caps.epoch.clone(),
                    seq: oldest_seq.saturating_sub(1),
                });
                batch.truncated = Some(TruncationMarker {
                    oldest_seq,
                    oldest_time,
                    dropped,
                    reason,
                });
            }
            DaemonFrame::Heartbeat { seq, .. } => {
                if self
                    .cursor
                    .as_ref()
                    .is_none_or(|c| c.epoch != caps.epoch || c.seq < seq)
                {
                    self.cursor = Some(EventCursor {
                        epoch: caps.epoch.clone(),
                        seq,
                    });
                }
            }
            DaemonFrame::Bye { reason } => return Some(format!("Daemon closed stream: {reason}")),
            DaemonFrame::ScopeChanged { .. } => {}
            DaemonFrame::TopologyDelta {
                seq,
                added,
                removed,
                edges,
            } => {
                let d = TopologyDelta {
                    seq,
                    added,
                    removed,
                    edges,
                };
                match batch.topology.as_mut() {
                    Some(acc) => merge_delta(acc, d),
                    None => batch.topology = Some(d),
                }
            }
        }
        None
    }

    /// Consume one stream connection; returns why it ended.
    async fn stream(&mut self, caps: &DaemonCapabilities) -> String {
        let path = stream_path(self.cursor.as_ref());
        let mut lines = match self.transport.open_stream(&path).await {
            Ok(s) => s,
            Err(e) => return format!("Could not open stream: {e}"),
        };
        let mut batch = Batcher::default();
        let mut deadline: Option<tokio::time::Instant> = None;
        let mut last_save = tokio::time::Instant::now();
        let reason = loop {
            let wait = match deadline {
                Some(d) => d.saturating_duration_since(tokio::time::Instant::now()),
                None => self.timings.idle_limit,
            };
            let next = tokio::time::timeout(wait, lines.next()).await;
            let line = match next {
                Err(_) if deadline.is_some() => {
                    self.flush(&mut batch);
                    deadline = None;
                    continue;
                }
                Err(_) => break "Stream idle: no heartbeat received".to_owned(),
                Ok(None) => break "Stream closed".to_owned(),
                Ok(Some(Err(e))) => break format!("Stream error: {e}"),
                Ok(Some(Ok(l))) => l,
            };
            let Some(frame) = parse_frame(&line) else {
                continue;
            };
            if let Some(reason) = self.apply_frame(frame, caps, &mut batch) {
                break reason;
            }
            if batch.is_full() {
                self.flush(&mut batch);
                deadline = None;
            } else if !batch.is_empty() && deadline.is_none() {
                deadline = Some(tokio::time::Instant::now() + self.timings.batch_window);
            }
            if last_save.elapsed() >= CURSOR_SAVE_EVERY
                && let Some(c) = &self.cursor
            {
                (self.save_cursor)(c.clone());
                last_save = tokio::time::Instant::now();
            }
        };
        self.flush(&mut batch);
        if let Some(c) = &self.cursor {
            (self.save_cursor)(c.clone());
        }
        reason
    }
}

// ---------------------------------------------------------------------------------------
// On-demand requests
// ---------------------------------------------------------------------------------------

/// Build the metrics query path.
pub(super) fn metrics_path(
    uid: &str,
    metric: MetricKind,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> String {
    let m = match metric {
        MetricKind::Cpu => "cpu",
        MetricKind::Memory => "memory",
    };
    format!(
        "{API_BASE}/v1/metrics?uid={uid}&metric={m}&from={}&to={}",
        from.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        to.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )
}

/// Parse `/v1/metrics` into a series. `truncated` may be an ISO time or `true`.
pub(super) fn parse_metrics(uid: &str, metric: MetricKind, text: &str) -> Option<MetricSeries> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let samples: Vec<DaemonMetricSample> =
        serde_json::from_value(v.get("samples")?.clone()).ok()?;
    let truncated_oldest_time = match v.get("truncated") {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Bool(true)) => samples.first().map(|s| s.time.clone()),
        _ => None,
    };
    Some(MetricSeries {
        key: SeriesKey {
            object_uid: uid.to_owned(),
            metric,
        },
        samples,
        truncated_oldest_time,
    })
}

/// Fetch one series and emit it tagged with `session`. Errors are silent (chart shows empty).
pub(super) async fn fetch_metrics(
    transport: &dyn DaemonTransport,
    uid: &str,
    metric: MetricKind,
    range_secs: u64,
) -> Option<MetricSeries> {
    let to = chrono::Utc::now();
    let from = to - chrono::Duration::seconds(i64::try_from(range_secs).unwrap_or(3600));
    let text = transport
        .get_text(&metrics_path(uid, metric, from, to))
        .await
        .ok()?;
    parse_metrics(uid, metric, &text)
}

#[cfg(test)]
pub(crate) mod tests;
