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
    /// The section route the command targets (`None` for global/system commands). Error
    /// notifications raised while it runs carry this route, so the UI can fail exactly the
    /// list a failed `Refresh*` was loading instead of every loading list.
    route: Option<Route>,
    /// Every error-level notification emitted while the command ran, verbatim.
    errors: Mutex<Vec<String>>,
}

tokio::task_local! {
    static CMD_SCOPE: CmdScope;
}

/// Run `fut` (one command's handling) with `request` as the default correlation of every
/// envelope it emits, collecting its error notifications for [`take_cmd_errors`].
pub(super) async fn in_cmd_scope<F: Future>(
    request: Option<RequestId>,
    route: Option<Route>,
    fut: F,
) -> F::Output {
    CMD_SCOPE
        .scope(
            CmdScope {
                request,
                route,
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
    // Inside a section command, attribute the error to that section's route.
    let route = CMD_SCOPE
        .try_with(|scope| scope.route.clone())
        .ok()
        .flatten()
        .unwrap_or_else(|| Route::global(section_keys::system()));
    emit(
        seq,
        sender,
        route,
        Correlation::None,
        BackendEvent::System(SystemEvent::Notification {
            level: NotificationLevel::Error,
            message,
        }),
    );
}

/// Report a failure. An RBAC denial (`forbidden`) inside a section command becomes
/// `AccessDenied` on that section's route, so its pane shows "Not permitted" rather than an
/// error toast (031); a denial outside one (a background poller) is only logged. Anything else
/// is an ordinary error notification. The command still finishes `ok: false` either way.
pub(super) fn report_error(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    message: String,
    forbidden: bool,
) {
    if !forbidden {
        return notify_error(seq, sender, message);
    }
    let route = CMD_SCOPE
        .try_with(|scope| {
            lock(&scope.errors).push(message.clone());
            scope.route.clone()
        })
        .ok()
        .flatten();
    match route {
        Some(route) => emit(
            seq,
            sender,
            route,
            Correlation::None,
            BackendEvent::System(SystemEvent::AccessDenied { message }),
        ),
        None => tracing::info!("not permitted: {message}"),
    }
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
        let errors = in_cmd_scope(Some(RequestId(5)), None, async {
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
    async fn a_denial_in_a_section_scope_is_access_denied_not_a_notification() {
        let seq = AtomicU64::new(0);
        let (tx, rx) = unbounded();
        let route = Route::namespaced(section_keys::config_configmaps(), None, None);
        let errors = in_cmd_scope(Some(RequestId(9)), Some(route.clone()), async {
            report_error(&seq, &tx, "configmaps is forbidden".into(), true);
            take_cmd_errors()
        })
        .await;
        // The command still fails (its `CommandFinished` is `ok: false`) ...
        assert_eq!(errors, vec!["configmaps is forbidden".to_owned()]);
        // ... but the UI hears `AccessDenied` on the section, not an error toast.
        let env = rx.try_recv().unwrap();
        assert_eq!(env.route, route);
        assert!(matches!(
            env.event,
            BackendEvent::System(SystemEvent::AccessDenied { ref message })
                if message == "configmaps is forbidden"
        ));
        assert!(rx.try_recv().is_err());

        // Outside a command (a background poller) a denial is only logged.
        report_error(&seq, &tx, "nodes is forbidden".into(), true);
        assert!(rx.try_recv().is_err());
        // Anything else is an ordinary error notification.
        report_error(&seq, &tx, "boom".into(), false);
        assert!(matches!(
            rx.try_recv().unwrap().event,
            BackendEvent::System(SystemEvent::Notification { .. })
        ));
    }

    #[tokio::test]
    async fn error_in_a_section_scope_carries_the_section_route() {
        let seq = AtomicU64::new(0);
        let (tx, rx) = unbounded();
        let route = Route::namespaced(
            crate::protocol::SectionKey::new("workloads/pods"),
            None,
            Some("default".into()),
        );
        in_cmd_scope(Some(RequestId(9)), Some(route.clone()), async {
            notify_error(&seq, &tx, "forbidden".into());
            notify_info(&seq, &tx, "fine".into());
        })
        .await;
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(got[0].route, route);
        assert_eq!(got[0].correlation, Correlation::Request(RequestId(9)));
        assert_eq!(got[1].route, Route::global(section_keys::system()));
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
