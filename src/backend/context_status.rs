//! Live per-context connection status (012-incluster-daemon-federation P1, FR-002/FR-004).
//!
//! The active context's status is driven explicitly by `cmd/cluster.rs`'s connect/switch code
//! (it owns the one live `kube::Client` the app maintains). Every *other* context gets a
//! one-shot, bounded background reachability probe whenever the context list is (re)loaded
//! (`Initialize`, `RefreshContexts`, `SetContext`) — never a periodic poller, per
//! `research.md` R2's Constitution V idle-CPU rationale.

use crossbeam_channel::Sender;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, ContextConnectionStatus, Correlation, Route, SystemEvent,
    section_keys,
};

use super::events;
use super::state::BackendState;

/// How long a single reachability probe may take before it's treated as unreachable.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The subset of `contexts` that should be background-probed: every entry other than `active`
/// (research.md R3 — the active context's status is never written by a probe, only by the real
/// connect attempt, to avoid a race between the two).
fn contexts_to_probe(contexts: &[String], active: Option<&str>) -> Vec<String> {
    contexts
        .iter()
        .filter(|c| Some(c.as_str()) != active)
        .cloned()
        .collect()
}

/// Fire-and-forget: spawns one bounded probe task per context in `contexts` other than `active`.
/// Never touches `state.cluster.client` — P1 keeps exactly one live client (the active
/// context's); this only ever builds short-lived, probe-only clients that are dropped once the
/// probe settles.
pub(super) fn spawn_status_probes(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    contexts: &[String],
    active: Option<&str>,
) {
    // 035: detect exec plugins (no spawn) whenever the context list is (re)loaded.
    super::cloud_auth::emit_detection(&state.cluster.seq, sender);
    for name in contexts_to_probe(contexts, active) {
        let seq = state.cluster.seq.clone();
        let sender = sender.clone();
        tokio::spawn(async move {
            emit_status(
                &seq,
                &sender,
                &name,
                ContextConnectionStatus::Connecting,
                None,
            );
            match probe_context(&name).await {
                Ok(()) => emit_status(
                    &seq,
                    &sender,
                    &name,
                    ContextConnectionStatus::NotConnected,
                    None,
                ),
                Err(e) => emit_status_err(&seq, &sender, &name, &e),
            }
        });
    }
}

/// Build a short-lived client for `context_name` and attempt one cheap authenticated round trip
/// (`GET /version`), bounded by `PROBE_TIMEOUT`. Exercises the same kubeconfig-resolution and
/// auth (including any `exec` credential plugin) path a real connection would use, so a failure
/// here is a true reachability/auth signal (research.md R1).
async fn probe_context(context_name: &str) -> anyhow::Result<()> {
    // 035: credentials come through the shared cloud-auth cache (typed plugin errors).
    let client = super::cloud_auth::build_client(context_name).await?;
    tokio::time::timeout(PROBE_TIMEOUT, client.apiserver_version())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Timed out after {}s probing context",
                PROBE_TIMEOUT.as_secs()
            )
        })??;
    Ok(())
}

/// Emit a `ContextStatusUpdated` event. `error` MUST be `None` unless `status ==
/// ContextConnectionStatus::Error` (data-model.md invariant); this is the sole construction
/// site besides `cmd/cluster.rs`'s active-context bracketing, both of which honor it by
/// construction.
pub(super) fn emit_status(
    seq: &Arc<AtomicU64>,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
    status: ContextConnectionStatus,
    error: Option<String>,
) {
    emit_status_full(seq, sender, context, status, error, None);
}

/// Emit an `Error` status for `err`, carrying the typed cloud-auth plugin error (035) when that
/// is what failed.
pub(super) fn emit_status_err(
    seq: &Arc<AtomicU64>,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
    err: &anyhow::Error,
) {
    emit_status_full(
        seq,
        sender,
        context,
        ContextConnectionStatus::Error,
        Some(format!("{err:#}")),
        super::cloud_auth::plugin_error_of(err).cloned(),
    );
}

fn emit_status_full(
    seq: &Arc<AtomicU64>,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
    status: ContextConnectionStatus,
    error: Option<String>,
    plugin_error: Option<crate::protocol::PluginError>,
) {
    debug_assert!(
        error.is_none() || matches!(status, ContextConnectionStatus::Error),
        "ContextStatusUpdated.error must only be set when status == Error"
    );
    events::emit(
        seq.as_ref(),
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::ContextStatusUpdated {
            context: context.to_owned(),
            status,
            error,
            plugin_error,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_context_is_excluded_from_probing() {
        let contexts = vec!["dev".to_owned(), "staging".to_owned(), "prod".to_owned()];
        let probed = contexts_to_probe(&contexts, Some("staging"));
        assert_eq!(probed, vec!["dev".to_owned(), "prod".to_owned()]);
    }

    #[test]
    fn no_active_context_probes_every_context() {
        let contexts = vec!["dev".to_owned(), "staging".to_owned()];
        let probed = contexts_to_probe(&contexts, None);
        assert_eq!(probed, contexts);
    }

    #[test]
    fn empty_contexts_probe_nothing() {
        assert!(contexts_to_probe(&[], Some("dev")).is_empty());
    }

    #[test]
    fn emit_status_carries_error_only_for_error_status() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let seq = Arc::new(AtomicU64::new(0));

        emit_status(
            &seq,
            &tx,
            "prod",
            ContextConnectionStatus::NotConnected,
            None,
        );
        let env = rx.try_recv().expect("an envelope");
        match env.event {
            BackendEvent::System(SystemEvent::ContextStatusUpdated {
                context,
                status,
                error,
                ..
            }) => {
                assert_eq!(context, "prod");
                assert_eq!(status, ContextConnectionStatus::NotConnected);
                assert_eq!(error, None);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        emit_status(
            &seq,
            &tx,
            "prod",
            ContextConnectionStatus::Error,
            Some("connection refused".to_owned()),
        );
        let env = rx.try_recv().expect("an envelope");
        match env.event {
            BackendEvent::System(SystemEvent::ContextStatusUpdated { status, error, .. }) => {
                assert_eq!(status, ContextConnectionStatus::Error);
                assert_eq!(error.as_deref(), Some("connection refused"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
