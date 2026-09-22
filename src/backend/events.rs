use crate::protocol::RequestId;
use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, NotificationLevel, Route, SystemEvent,
    section_keys,
};
use crossbeam_channel::Sender;
use std::future::Future;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-command context, installed as a tokio task-local for the duration of one
/// `run_cmd` (see [`in_cmd_scope`]).
///
/// Task-locals are not inherited by `tokio::spawn`, so long-lived work a handler spawns
/// (watchers, log streams, metrics polling, port-forward tunnels) keeps its own
/// `Stream`/`None` correlation and its errors are not attributed to the command.
struct CmdScope {
    request: Option<RequestId>,
    /// Every error-level notification emitted while the command ran, verbatim.
    errors: Mutex<Vec<String>>,
}

tokio::task_local! {
    static CMD_SCOPE: CmdScope;
}

/// Run `fut` (one command's handling) with `request` as the default correlation of every
/// envelope it emits, collecting its error notifications for [`take_cmd_errors`].
pub(super) async fn in_cmd_scope<F: Future>(request: Option<RequestId>, fut: F) -> F::Output {
    CMD_SCOPE
        .scope(
            CmdScope {
                request,
                errors: Mutex::new(Vec::new()),
            },
            fut,
        )
        .await
}

/// Drain the error notifications recorded in the current command scope (empty outside one).
pub(super) fn take_cmd_errors() -> Vec<String> {
    CMD_SCOPE
        .try_with(|scope| std::mem::take(&mut *lock(&scope.errors)))
        .unwrap_or_default()
}

/// The current command's request id, stringified, for stream events (e.g.
/// `StreamEvent::ResourceApplied`) whose payload carries `request_id` as data rather than
/// only as the envelope's correlation. Empty outside a command scope or for an
/// uncorrelated command.
pub(super) fn current_request_id_string() -> String {
    CMD_SCOPE
        .try_with(|scope| scope.request.map(|id| id.0.to_string()))
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Default an uncorrelated envelope to the current command's request, and record error
/// notifications against it.
fn apply_cmd_scope(correlation: Correlation, event: &BackendEvent) -> Correlation {
    CMD_SCOPE
        .try_with(|scope| {
            if let BackendEvent::System(SystemEvent::Notification {
                level: NotificationLevel::Error,
                message,
            }) = event
            {
                lock(&scope.errors).push(message.clone());
            }
            match (correlation, scope.request) {
                (Correlation::None, Some(id)) => Correlation::Request(id),
                (other, _) => other,
            }
        })
        .unwrap_or(correlation)
}

pub(super) fn emit(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    route: Route,
    correlation: Correlation,
    event: BackendEvent,
) {
    let correlation = apply_cmd_scope(correlation, &event);
    let next = seq.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    let _ = sender.send(BackendEventEnvelope::new(next, route, correlation, event));
}

pub(super) fn notify_info(seq: &AtomicU64, sender: &Sender<BackendEventEnvelope>, message: String) {
    emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::Notification {
            level: NotificationLevel::Info,
            message,
        }),
    );
}

pub(super) fn notify_error(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    message: String,
) {
    emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::Notification {
            level: NotificationLevel::Error,
            message,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::StreamId;
    use crossbeam_channel::unbounded;

    fn started() -> BackendEvent {
        BackendEvent::System(SystemEvent::BackendStarted)
    }

    #[tokio::test]
    async fn scope_sets_request_correlation_and_collects_errors() {
        let seq = AtomicU64::new(0);
        let (tx, rx) = unbounded();
        let errors = in_cmd_scope(Some(RequestId(5)), async {
            emit(
                &seq,
                &tx,
                Route::global(section_keys::system()),
                Correlation::None,
                started(),
            );
            emit(
                &seq,
                &tx,
                Route::global(section_keys::system()),
                Correlation::Stream(StreamId(2)),
                started(),
            );
            notify_info(&seq, &tx, "fine".into());
            notify_error(&seq, &tx, "boom: 403 Forbidden".into());
            // Spawned work does not inherit the scope.
            let tx2 = tx.clone();
            tokio::spawn(async move {
                notify_error(&AtomicU64::new(0), &tx2, "background".into());
            })
            .await
            .unwrap();
            take_cmd_errors()
        })
        .await;
        assert_eq!(errors, vec!["boom: 403 Forbidden".to_owned()]);

        let got: Vec<Correlation> = rx.try_iter().map(|e| e.correlation).collect();
        assert_eq!(
            got,
            vec![
                Correlation::Request(RequestId(5)),
                Correlation::Stream(StreamId(2)),
                Correlation::Request(RequestId(5)),
                Correlation::Request(RequestId(5)),
                Correlation::None,
            ]
        );
    }

    #[tokio::test]
    async fn outside_a_scope_nothing_changes() {
        let seq = AtomicU64::new(0);
        let (tx, rx) = unbounded();
        notify_error(&seq, &tx, "x".into());
        assert!(take_cmd_errors().is_empty());
        assert_eq!(rx.recv().unwrap().correlation, Correlation::None);
    }
}
