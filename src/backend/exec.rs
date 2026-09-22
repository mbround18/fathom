//! Interactive `kube exec` shell sessions (Pod Logs & Terminal, User Story 2).
//!
//! One active session application-wide (spec FR-012, research.md R2). `start_exec`
//! attempts `/bin/bash` first, falling back once to `/bin/sh` (research.md R1); the
//! stdout-relay task emits `StreamEvent::PodExecOutputChunk` and, on EOF/error,
//! `StreamEvent::StreamClosed` — reusing the same event `pods.rs::start_logs` uses for
//! its own stream lifecycle.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use crossbeam_channel::Sender;
use futures::SinkExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, AttachParams, AttachedProcess, TerminalSize};
use tokio::io::AsyncReadExt;

use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, Route, StreamEvent, section_keys,
};

use super::{events, state::BackendState};

/// The two shells attempted, in order, per research.md R1.
const SHELL_ATTEMPTS: [&str; 2] = ["/bin/bash", "/bin/sh"];

/// How long to wait, after a successful websocket upgrade, for the remote process to
/// report an immediate exit before treating the attach as genuinely live.
///
/// `Api::exec()` only awaits the initial websocket handshake (kube-client's internal
/// `connect()`), not confirmation from the container runtime that the requested binary
/// actually started (verified against kube-client 2.0.1's `subresource.rs`/
/// `remote_command.rs`: `AttachedProcess::new()` spawns its relay task and returns
/// synchronously). A missing shell (e.g. no `/bin/bash` in an alpine/busybox image —
/// the common case for minimal containers) therefore does NOT surface as an `Err` from
/// `exec()`; it only shows up moments later as an early `Status` on the exec session's
/// status channel. Without this grace-window check, the bash → sh fallback (research.md
/// R1) never triggers for that case, since it only reacts to `Err` results.
const EARLY_EXIT_GRACE: Duration = Duration::from_millis(400);

/// True when a `kube::Error` represents an RBAC-denied (`403 Forbidden`) API response.
fn is_forbidden(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(ae) if ae.code == 403)
}

/// Why a single shell attempt (bash or sh) failed to produce a usable session.
#[derive(Debug)]
enum AttachFailure {
    /// `Api::exec()` itself returned an error (e.g. RBAC-denied, pod not found).
    Api(kube::Error),
    /// The websocket upgrade succeeded, but the remote process reported exiting
    /// within `EARLY_EXIT_GRACE` — almost always because the requested binary
    /// doesn't exist in the container. See `EARLY_EXIT_GRACE`'s doc comment.
    EarlyExit,
}

fn is_forbidden_failure(failure: &AttachFailure) -> bool {
    matches!(failure, AttachFailure::Api(err) if is_forbidden(err))
}

fn describe_failure(failure: &AttachFailure) -> String {
    match failure {
        AttachFailure::Api(err) => err.to_string(),
        AttachFailure::EarlyExit => {
            "process exited immediately after attaching (binary likely missing)".to_owned()
        }
    }
}

/// Map the two failed exec attempts (bash, then sh) to a single user-facing reason
/// string, per FR-013: a distinct "permission denied" message when either attempt was
/// RBAC-denied, otherwise a "no usable shell" message carrying the last error verbatim.
fn describe_attach_failure(bash_failure: &AttachFailure, sh_failure: &AttachFailure) -> String {
    if is_forbidden_failure(bash_failure) || is_forbidden_failure(sh_failure) {
        return "Permission denied: exec into this pod is forbidden (check RBAC for pods/exec)."
            .to_owned();
    }
    format!(
        "No usable shell found in container (tried /bin/bash and /bin/sh): {}",
        describe_failure(sh_failure)
    )
}

/// Build the `Route` exec events are emitted on — same section as pod logs, since exec
/// sessions are also a pod-scoped stream.
fn exec_route(state: &BackendState, namespace: &str) -> Route {
    Route::namespaced(
        section_keys::workloads_pods(),
        state.cluster.current_context.clone(),
        Some(namespace.to_owned()),
    )
}

/// Start (or restart) the single application-wide exec session (FR-012: any existing
/// session is stopped first).
pub(super) async fn start_exec(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
    name: String,
    container: Option<String>,
) -> anyhow::Result<()> {
    state.exec_session.stop();

    let stream_id = state.next_stream_id();
    let route = exec_route(state, &namespace);
    let client = state.cluster.client_or_err()?;
    let api: Api<Pod> = Api::namespaced(client, &namespace);

    let mut ap = AttachParams::interactive_tty();
    if let Some(container) = &container {
        ap = ap.container(container.clone());
    }

    let attached = match attach_with_fallback(&api, &name, &ap).await {
        Ok(attached) => attached,
        Err(reason) => {
            events::emit(
                state.cluster.seq.as_ref(),
                sender,
                route,
                Correlation::Stream(stream_id),
                BackendEvent::Stream(StreamEvent::StreamClosed { stream_id, reason }),
            );
            return Ok(());
        }
    };

    install_session(state, sender, route, stream_id, attached);

    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        exec_route(state, &namespace),
        Correlation::Stream(stream_id),
        BackendEvent::Stream(StreamEvent::PodExecReady { stream_id }),
    );
    Ok(())
}

/// Try `/bin/bash`, then `/bin/sh` once, per research.md R1/R2.
async fn attach_with_fallback(
    api: &Api<Pod>,
    name: &str,
    ap: &AttachParams,
) -> Result<AttachedProcess, String> {
    let bash_failure = match try_shell(api, name, SHELL_ATTEMPTS[0], ap).await {
        Ok(attached) => return Ok(attached),
        Err(failure) => failure,
    };
    match try_shell(api, name, SHELL_ATTEMPTS[1], ap).await {
        Ok(attached) => Ok(attached),
        Err(sh_failure) => Err(describe_attach_failure(&bash_failure, &sh_failure)),
    }
}

/// Exec `shell` in the container, then confirm — within `EARLY_EXIT_GRACE` — that the
/// process is still alive rather than having exited immediately (see `EARLY_EXIT_GRACE`).
async fn try_shell(
    api: &Api<Pod>,
    name: &str,
    shell: &str,
    ap: &AttachParams,
) -> Result<AttachedProcess, AttachFailure> {
    let mut attached = api
        .exec(name, [shell], ap)
        .await
        .map_err(AttachFailure::Api)?;
    if confirmed_alive(&mut attached).await {
        return Ok(attached);
    }
    attached.abort();
    Err(AttachFailure::EarlyExit)
}

/// Race the exec session's status future against `EARLY_EXIT_GRACE`: if a `Status`
/// arrives first, the remote process already exited (see `EARLY_EXIT_GRACE`'s doc
/// comment) and the attempt should be treated as failed, not as a live session.
async fn confirmed_alive(attached: &mut AttachedProcess) -> bool {
    let Some(status_fut) = attached.take_status() else {
        // No status channel available; nothing to race against, so assume alive.
        return true;
    };
    tokio::time::timeout(EARLY_EXIT_GRACE, status_fut)
        .await
        .is_err()
}

/// Extract stdin/stdout/resize handles from a freshly-attached process, store them on
/// `state.exec_session`, and spawn the stdout-relay task.
fn install_session(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    route: Route,
    stream_id: crate::protocol::StreamId,
    mut attached: AttachedProcess,
) {
    let stdin_writer = attached
        .stdin()
        .map(|w| Box::new(w) as Box<dyn tokio::io::AsyncWrite + Unpin + Send + Sync>);
    let resize_tx = attached.terminal_size();
    let stdout = attached.stdout();

    let sender_cloned = sender.clone();
    let seq = state.cluster.seq.clone();
    let relay_task = stdout
        .map(|stdout| tokio::spawn(relay_stdout(stdout, route, stream_id, seq, sender_cloned)));

    state.exec_session.stdin_writer = stdin_writer;
    state.exec_session.resize_tx = resize_tx;
    state.exec_session.relay_task = relay_task;
    state.exec_session.attached = Some(attached);
}

/// Read stdout chunks until EOF/error, emitting `PodExecOutputChunk` per read, then
/// `StreamClosed` once (normal end or error, distinguished by `reason`).
async fn relay_stdout(
    mut stdout: impl tokio::io::AsyncRead + Unpin,
    route: Route,
    stream_id: crate::protocol::StreamId,
    seq: std::sync::Arc<std::sync::atomic::AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
) {
    let mut buf = [0u8; 4096];
    let reason = loop {
        match stdout.read(&mut buf).await {
            // "exit" (not "ended", unlike the log stream) matches contracts/protocol.md's
            // documented reason for a normal session end, which the frontend's generic
            // `kind !== 'logs'` StreamClosed branch already keys off of.
            Ok(0) => break "exit".to_owned(),
            Ok(n) => {
                events::emit(
                    seq.as_ref(),
                    &sender,
                    route.clone(),
                    Correlation::Stream(stream_id),
                    BackendEvent::Stream(StreamEvent::PodExecOutputChunk {
                        stream_id,
                        data: BASE64.encode(&buf[..n]),
                    }),
                );
            }
            Err(e) => break format!("exec stream error: {e}"),
        }
    };

    events::emit(
        seq.as_ref(),
        &sender,
        route,
        Correlation::Stream(stream_id),
        BackendEvent::Stream(StreamEvent::StreamClosed { stream_id, reason }),
    );
}

/// Write base64-decoded input to the active session's stdin (no-op if none active, per
/// contracts/protocol.md).
pub(super) async fn send_input(state: &mut BackendState, data: String) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;

    let Some(writer) = state.exec_session.stdin_writer.as_mut() else {
        return Ok(());
    };
    let bytes = BASE64.decode(data.as_bytes())?;
    writer.write_all(&bytes).await?;
    Ok(())
}

/// Send a resize to the active session's remote pty (no-op if none active).
pub(super) async fn resize(state: &mut BackendState, cols: u16, rows: u16) -> anyhow::Result<()> {
    let Some(tx) = state.exec_session.resize_tx.as_mut() else {
        return Ok(());
    };
    tx.send(TerminalSize {
        width: cols,
        height: rows,
    })
    .await?;
    Ok(())
}

/// Stop the active exec session, if any (no-op otherwise).
pub(super) fn stop(state: &mut BackendState) {
    state.exec_session.stop();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forbidden_error() -> kube::Error {
        kube::Error::Api(kube::core::ErrorResponse {
            status: "Failure".to_owned(),
            message: "pods/exec is forbidden".to_owned(),
            reason: "Forbidden".to_owned(),
            code: 403,
        })
    }

    fn not_found_error() -> kube::Error {
        kube::Error::Api(kube::core::ErrorResponse {
            status: "Failure".to_owned(),
            message: "exec: \"/bin/bash\": stat /bin/bash: no such file or directory".to_owned(),
            reason: "BadRequest".to_owned(),
            code: 400,
        })
    }

    #[test]
    fn forbidden_bash_attempt_maps_to_permission_denied() {
        let reason = describe_attach_failure(
            &AttachFailure::Api(forbidden_error()),
            &AttachFailure::Api(not_found_error()),
        );
        assert!(
            reason.to_lowercase().contains("permission denied"),
            "expected a permission-denied reason, got: {reason}"
        );
    }

    #[test]
    fn forbidden_sh_attempt_maps_to_permission_denied() {
        let reason = describe_attach_failure(
            &AttachFailure::Api(not_found_error()),
            &AttachFailure::Api(forbidden_error()),
        );
        assert!(
            reason.to_lowercase().contains("permission denied"),
            "expected a permission-denied reason, got: {reason}"
        );
    }

    #[test]
    fn no_usable_shell_maps_to_distinct_reason() {
        let reason = describe_attach_failure(
            &AttachFailure::Api(not_found_error()),
            &AttachFailure::Api(not_found_error()),
        );
        assert!(
            !reason.to_lowercase().contains("permission denied"),
            "no-shell case must not be conflated with permission-denied: {reason}"
        );
        assert!(
            reason.contains("/bin/bash") && reason.contains("/bin/sh"),
            "expected the fallback message to name both attempted shells: {reason}"
        );
    }

    #[test]
    fn early_exit_is_not_conflated_with_permission_denied() {
        // A missing shell binary manifests as an early exit on both attempts (see
        // EARLY_EXIT_GRACE's doc comment), not as an `Err` from `Api::exec()`.
        let reason = describe_attach_failure(&AttachFailure::EarlyExit, &AttachFailure::EarlyExit);
        assert!(
            !reason.to_lowercase().contains("permission denied"),
            "early-exit case must not be conflated with permission-denied: {reason}"
        );
        assert!(
            reason.contains("/bin/bash") && reason.contains("/bin/sh"),
            "expected the fallback message to name both attempted shells: {reason}"
        );
    }

    #[test]
    fn is_forbidden_only_matches_403() {
        assert!(is_forbidden(&forbidden_error()));
        assert!(!is_forbidden(&not_found_error()));
    }
}
