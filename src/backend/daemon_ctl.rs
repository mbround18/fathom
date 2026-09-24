//! 036-incluster-daemon: per-context lifecycle and command handlers for the daemon client.
//!
//! The daemon is only ever used for the active context. With the preference `off` (default)
//! nothing here spawns a task or issues a request; connect paths call `sync_active`, which
//! spawns the session task and returns immediately so connecting is never blocked.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, DaemonMode, DaemonState, DaemonStatus,
    EdgeDelta, EventCursor, MetricKind, Route, SystemEvent, TopologyDelta, section_keys,
};

use super::daemon_client::{
    API_BASE, CursorSaver, DaemonTransport, Emitter, KubeTransport, Session, Timeline, Timings,
    fetch_metrics,
};
use super::events;
use super::persistence::{self, DaemonFile};
use super::state::BackendState;

/// Serializes read-modify-write of `daemon.json` between the command thread and session tasks.
static FILE_LOCK: Mutex<()> = Mutex::new(());

struct Handle {
    task: tokio::task::JoinHandle<()>,
    current: Arc<AtomicU64>,
    session: u64,
    transport: Arc<dyn DaemonTransport>,
    timeline: Timeline,
}

#[derive(Default)]
pub(super) struct DaemonRuntime {
    handles: HashMap<String, Handle>,
    next_session: u64,
    /// Override of `~/.fathom/daemon.json` (tests only).
    pub(super) path: Option<PathBuf>,
}

impl DaemonRuntime {
    pub(super) fn abort_all(&mut self) {
        for (_, h) in self.handles.drain() {
            h.current.store(0, Ordering::SeqCst);
            h.task.abort();
        }
    }

    fn stop(&mut self, context: &str) {
        if let Some(h) = self.handles.remove(context) {
            h.current.store(0, Ordering::SeqCst);
            h.task.abort();
        }
    }

    #[cfg(test)]
    pub(super) fn is_running(&self, context: &str) -> bool {
        self.handles.contains_key(context)
    }
}

fn file_path(state: &BackendState) -> Option<PathBuf> {
    state
        .daemon
        .path
        .clone()
        .or_else(|| persistence::daemon_config_path().ok())
}

fn load(path: Option<&PathBuf>) -> DaemonFile {
    let _g = FILE_LOCK.lock();
    path.map(|p| persistence::load_daemon_from(p))
        .unwrap_or_default()
}

fn update_file(path: Option<&PathBuf>, f: impl FnOnce(&mut DaemonFile)) {
    let Some(path) = path else { return };
    let _g = FILE_LOCK.lock();
    let mut file = persistence::load_daemon_from(path);
    f(&mut file);
    if let Err(e) = persistence::save_daemon_to(path, &file) {
        tracing::warn!("Failed to save daemon.json: {e}");
    }
}

fn emitter(state: &BackendState, sender: &Sender<BackendEventEnvelope>) -> Emitter {
    let seq = state.cluster.seq.clone();
    let sender = sender.clone();
    Arc::new(move |ev| {
        events::emit(
            seq.as_ref(),
            &sender,
            Route::global(section_keys::system()),
            Correlation::None,
            BackendEvent::System(ev),
        );
    })
}

fn emit_status(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
    st: DaemonState,
    detail: Option<String>,
) {
    (emitter(state, sender))(SystemEvent::DaemonStatusUpdated {
        context: context.to_owned(),
        status: DaemonStatus {
            context: context.to_owned(),
            state: st,
            detail,
            capabilities: None,
            last_event_at: None,
            next_retry_at: None,
        },
    });
}

/// Called from every connect / context-switch path. Never blocks and never fails the connect.
pub(super) fn sync_active(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    let Some(ctx) = state.cluster.current_context.clone() else {
        return;
    };
    let others: Vec<String> = state
        .daemon
        .handles
        .keys()
        .filter(|k| **k != ctx)
        .cloned()
        .collect();
    for o in others {
        state.daemon.stop(&o);
    }
    let mode = load(file_path(state).as_ref()).mode_for(&ctx);
    match (mode, state.cluster.client.clone()) {
        (DaemonMode::Auto, Some(client)) => {
            start_with(
                state,
                sender,
                &ctx,
                Arc::new(KubeTransport(client)),
                Timings::default(),
            );
        }
        _ => state.daemon.stop(&ctx),
    }
}

pub(super) fn start_with(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
    transport: Arc<dyn DaemonTransport>,
    timings: Timings,
) {
    state.daemon.stop(context);
    state.daemon.next_session += 1;
    let id = state.daemon.next_session;
    let current = Arc::new(AtomicU64::new(id));
    let timeline: Timeline = Arc::new(Mutex::new(VecDeque::new()));
    let path = file_path(state);
    let cursor = load(path.as_ref()).cursors.get(context).cloned();
    let ctx_owned = context.to_owned();
    let save: CursorSaver = {
        let ctx = ctx_owned.clone();
        Arc::new(move |c: EventCursor| {
            update_file(path.as_ref(), |f| {
                f.cursors.insert(ctx.clone(), c);
            });
        })
    };
    let session = Session {
        context: ctx_owned,
        id,
        current: current.clone(),
        transport: transport.clone(),
        emit: emitter(state, sender),
        save_cursor: save,
        timeline: timeline.clone(),
        cursor,
        timings,
        caps: Arc::new(Mutex::new(None)),
    };
    let task = tokio::spawn(session.run());
    state.daemon.handles.insert(
        context.to_owned(),
        Handle {
            task,
            current,
            session: id,
            transport,
            timeline,
        },
    );
}

pub(super) fn set_preference(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
    mode: DaemonMode,
) {
    let path = file_path(state);
    update_file(path.as_ref(), |f| {
        if mode == DaemonMode::Off {
            f.preferences.remove(context);
        } else {
            f.preferences.insert(context.to_owned(), mode);
        }
    });
    match mode {
        DaemonMode::Off => {
            state.daemon.stop(context);
            emit_status(state, sender, context, DaemonState::Off, None);
        }
        DaemonMode::Auto => {
            if state.cluster.current_context.as_deref() == Some(context) {
                sync_active(state, sender);
            } else {
                emit_status(
                    state,
                    sender,
                    context,
                    DaemonState::Off,
                    Some(
                        "Preference saved; the daemon is probed when this context is active".into(),
                    ),
                );
            }
        }
    }
}

/// Manual re-probe: restarts the session (resets backoff).
pub(super) fn refresh(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
) {
    if state.cluster.current_context.as_deref() == Some(context) {
        sync_active(state, sender);
    }
}

/// Re-emit the buffered timeline (no server call).
pub(super) fn request_timeline(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
    limit: u32,
) {
    let Some(h) = state.daemon.handles.get(context) else {
        return;
    };
    let entries: Vec<_> = h
        .timeline
        .lock()
        .map(|t| {
            let skip = t.len().saturating_sub(limit as usize);
            t.iter().skip(skip).cloned().collect()
        })
        .unwrap_or_default();
    (emitter(state, sender))(SystemEvent::DaemonEventsBatch {
        context: context.to_owned(),
        session: h.session,
        events: entries,
        truncated: None,
    });
}

pub(super) fn request_topology(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
) {
    let Some(h) = state.daemon.handles.get(context) else {
        return;
    };
    let (transport, session, current) = (h.transport.clone(), h.session, h.current.clone());
    let emit = emitter(state, sender);
    let context = context.to_owned();
    tokio::spawn(async move {
        let Ok(text) = transport.get_text(&format!("{API_BASE}/v1/topology")).await else {
            return;
        };
        #[derive(serde::Deserialize)]
        struct Snap {
            seq: u64,
            nodes: Vec<crate::protocol::TopologyNode>,
            edges: Vec<crate::protocol::TopologyEdge>,
        }
        if let Ok(s) = serde_json::from_str::<Snap>(&text)
            && current.load(Ordering::SeqCst) == session
        {
            emit(SystemEvent::DaemonTopologyUpdated {
                context,
                session,
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
    });
}

pub(super) fn request_metrics(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
    uid: String,
    metric: MetricKind,
    range_secs: u64,
) {
    let Some(h) = state.daemon.handles.get(context) else {
        return;
    };
    let (transport, session, current) = (h.transport.clone(), h.session, h.current.clone());
    let emit = emitter(state, sender);
    let context = context.to_owned();
    tokio::spawn(async move {
        if let Some(series) = fetch_metrics(transport.as_ref(), &uid, metric, range_secs).await
            && current.load(Ordering::SeqCst) == session
        {
            emit(SystemEvent::DaemonMetricsUpdated {
                context,
                session,
                series: vec![series],
            });
        }
    });
}

#[cfg(test)]
mod tests;
