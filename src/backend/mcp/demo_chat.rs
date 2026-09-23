//! The "Launch example chat" affordance (015-mcp-agent-exposure, US4/T047).
//!
//! An operator who has never attached an external MCP client should be able to *see* one work
//! without reading documentation: this module launches a real `claude` CLI subprocess on the
//! operator's own machine, pre-configured to attach to this very Fathom process's stdio MCP
//! server, and relays its output to a terminal view in Settings.
//!
//! Three honesty rules, straight out of FR-009/FR-010/FR-011, are structural here rather than
//! documented:
//!
//! 1. **Nothing is ever fabricated.** Detection is a fresh `PATH` scan on every call — there is
//!    no cache to go stale, and [`detect_claude_cli`] is the *only* thing that decides whether a
//!    launch is possible. When it returns `None` the handler emits
//!    `SystemEvent::McpDemoChatNotDetected` and returns; it never reaches `Command::spawn`.
//! 2. **The server must already be on.** A demo chat against a server that is off would attach
//!    to nothing, so [`start`] checks [`McpState::is_running`] *before* detection or spawning
//!    and refuses with a specific reason (FR-010, and the `refuses_*` test below).
//! 3. **The connection it hands `claude` is the connection Settings shows.** Both come from
//!    [`super::connection_argv`].
//!
//! ## How the MCP server is registered with `claude`
//!
//! Claude Code's CLI takes an *ephemeral*, session-scoped MCP server configuration on the
//! command line:
//!
//! ```text
//! claude --mcp-config '{"mcpServers":{"fathom":{"command":"…","args":["--only-mcp"]}}}' \
//!        --strict-mcp-config
//! ```
//!
//! `--mcp-config` accepts a JSON string (or a file path) in the same `mcpServers` shape as a
//! `.mcp.json`, and `--strict-mcp-config` tells the CLI to use *only* what was passed rather
//! than merging the operator's own user/project MCP servers.
//!
//! This is deliberately **not** `claude mcp add fathom …`: that subcommand writes a persistent
//! entry into the operator's own Claude Code configuration, which a demo launched from a
//! Settings panel has no business doing — the registration would outlive the demo, point at a
//! Fathom process that is no longer running, and be invisible from Fathom afterwards. The
//! ephemeral flag pair leaves nothing behind.
//!
//! Both flags are a best-effort integration point against an external tool this repo does not
//! vendor or version-pin: if a future `claude` release renames them, the subprocess will exit
//! immediately with its own usage error, which is relayed verbatim to the operator's terminal
//! view as a `McpDemoChatEnded` reason. That is the intended failure mode — a visible, honest
//! error from the real CLI, never a simulated session.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use crossbeam_channel::Sender;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStdin};

use crate::backend::events;
use crate::backend::state::BackendState;
use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};

/// The executable looked for on `PATH`. Per spec.md's Assumptions, a `claude` binary is the
/// only "compatible local agent CLI" v1 recognises.
const CLAUDE_BINARY: &str = "claude";

/// The name the MCP server is given inside the `claude` session — what an operator will see
/// its tools prefixed with (`mcp__fathom__get_pod_logs`, …).
const MCP_SERVER_NAME: &str = "fathom";

/// Shown verbatim when no `claude` is on `PATH` (FR-011). Written as setup instructions, not as
/// an error: nothing went wrong, the operator simply does not have the CLI.
pub(in crate::backend) const NOT_DETECTED_INSTRUCTIONS: &str = concat!(
    "No `claude` executable was found on this machine's PATH, so there is nothing to launch. ",
    "Install Claude Code (npm install -g @anthropic-ai/claude-code), make sure `claude` runs ",
    "from a terminal, then reopen this panel. You can also attach any other MCP client by hand ",
    "using the connection command above."
);

/// The refusal when the stdio MCP server is off (FR-010 / US4 acceptance scenario 3).
pub(in crate::backend) const SERVER_OFF_REASON: &str =
    "Turn the MCP server on first — an example chat would have nothing to attach to.";

/// A fallback terminal size, used until the operator's terminal view reports its own.
const DEFAULT_SIZE: (u16, u16) = (100, 30);

/// The single live demo-chat subprocess, plus the id counter that keeps sessions distinguishable.
///
/// Like the pod-exec session this mirrors, the demo chat is a singleton: starting a second one
/// stops the first. `spawn_attempts` is instrumentation for the "refused calls never spawn"
/// tests below and is never read for control flow.
#[derive(Default)]
pub(in crate::backend) struct DemoChatState {
    next_session_id: u32,
    session: Option<DemoChatSession>,
    /// Last size reported by the terminal view, applied as `COLUMNS`/`LINES` at spawn.
    size: Option<(u16, u16)>,
    spawn_attempts: usize,
}

struct DemoChatSession {
    child: Child,
    stdin: Option<ChildStdin>,
    relays: Vec<tokio::task::JoinHandle<()>>,
}

impl DemoChatState {
    /// Kill the live subprocess and drop its relay tasks. Idempotent.
    pub(super) fn stop(&mut self) {
        let Some(mut session) = self.session.take() else {
            return;
        };
        for relay in session.relays.drain(..) {
            relay.abort();
        }
        drop(session.stdin.take());
        // `start_kill` only signals; the child is reaped by tokio once it exits, and we do not
        // block a command handler waiting for that.
        let _ = session.child.start_kill();
    }

    #[cfg(test)]
    fn spawn_attempts(&self) -> usize {
        self.spawn_attempts
    }
}

/// Where demo-chat events are emitted: the same section the "Available Agents" panel lives in.
fn route() -> Route {
    Route::global(section_keys::cluster_settings())
}

fn emit(state: &BackendState, sender: &Sender<BackendEventEnvelope>, event: SystemEvent) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        route(),
        Correlation::None,
        BackendEvent::System(event),
    );
}

/// Is `path` a file this process could execute?
fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Search an explicit `PATH`-shaped value for an executable `claude`.
///
/// Split out from [`detect_claude_cli`] so it is testable without mutating the process's
/// environment (`$PATH` is process-global, and the lib test binary runs its tests in parallel).
pub(in crate::backend) fn detect_claude_cli_in(path_var: Option<&OsString>) -> Option<PathBuf> {
    let path_var = path_var?;
    std::env::split_paths(path_var)
        .map(|dir| dir.join(CLAUDE_BINARY))
        .find(|candidate| is_executable(candidate))
}

/// Whether a `claude` CLI exists on this machine's `PATH`, checked fresh every call.
///
/// Deliberately uncached: FR-009 forbids claiming detection of something Fathom cannot verify
/// *now*, and an operator who installs (or removes) the CLI while Fathom is open must see the
/// truth on their next click, not a cached answer from startup.
pub(in crate::backend) fn detect_claude_cli() -> Option<PathBuf> {
    detect_claude_cli_in(std::env::var_os("PATH").as_ref())
}

/// The `--mcp-config` payload handed to `claude`: this Fathom process, in `.mcp.json` shape.
fn mcp_config_json() -> String {
    let (command, args) = super::connection_argv();
    serde_json::json!({
        "mcpServers": {
            MCP_SERVER_NAME: { "command": command, "args": args }
        }
    })
    .to_string()
}

/// Start the demo chat, or explain — honestly — why it cannot start.
///
/// Emits exactly one of: `McpDemoChatReady` (a real subprocess is attached),
/// `McpDemoChatNotDetected` (no `claude` on `PATH`), or an error notification (the server is
/// off, or the spawn itself failed). Only the first of those ever reaches `Command::spawn`.
pub(in crate::backend) async fn start(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    // Rule 2, checked first: refuse before detection, before spawning.
    if !state.mcp.is_running() {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            SERVER_OFF_REASON.to_owned(),
        );
        return;
    }

    // Rule 1: a fresh PATH scan decides, and a `None` ends the call here.
    let Some(binary) = detect_claude_cli() else {
        emit(
            state,
            sender,
            SystemEvent::McpDemoChatNotDetected {
                instructions: NOT_DETECTED_INSTRUCTIONS.to_owned(),
            },
        );
        return;
    };

    // Singleton: whatever was running is replaced, not stacked.
    state.mcp.demo_chat.stop();

    let (cols, rows) = state.mcp.demo_chat.size.unwrap_or(DEFAULT_SIZE);
    let mut command = tokio::process::Command::new(&binary);
    command
        .arg("--mcp-config")
        .arg(mcp_config_json())
        .arg("--strict-mcp-config")
        .env("COLUMNS", cols.to_string())
        .env("LINES", rows.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    state.mcp.demo_chat.spawn_attempts += 1;
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            events::notify_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Could not launch {}: {e}", binary.display()),
            );
            return;
        }
    };

    state.mcp.demo_chat.next_session_id = state.mcp.demo_chat.next_session_id.saturating_add(1);
    let session_id = state.mcp.demo_chat.next_session_id;

    let stdin = child.stdin.take();
    let seq = state.cluster.seq.clone();
    let mut relays = Vec::new();
    // stdout carries the session, stderr carries the CLI's own errors (including a usage error
    // from a renamed flag) — both are the operator's to see, so both are relayed as output.
    if let Some(stdout) = child.stdout.take() {
        relays.push(tokio::spawn(relay(
            stdout,
            session_id,
            seq.clone(),
            sender.clone(),
            true,
        )));
    }
    if let Some(stderr) = child.stderr.take() {
        relays.push(tokio::spawn(relay(
            stderr,
            session_id,
            seq,
            sender.clone(),
            false,
        )));
    }

    state.mcp.demo_chat.session = Some(DemoChatSession {
        child,
        stdin,
        relays,
    });

    emit(state, sender, SystemEvent::McpDemoChatReady { session_id });
}

/// Relay one output pipe to `McpDemoChatOutputChunk` events until EOF.
///
/// `ends_session`: the stdout relay owns the session's lifetime — its EOF means the CLI exited,
/// and emits the single `McpDemoChatEnded`. The stderr relay just stops, so a process that
/// closes stderr early is not reported as finished.
async fn relay(
    mut pipe: impl tokio::io::AsyncRead + Unpin,
    session_id: u32,
    seq: Arc<AtomicU64>,
    sender: Sender<BackendEventEnvelope>,
    ends_session: bool,
) {
    let mut buf = [0u8; 4096];
    let reason = loop {
        match pipe.read(&mut buf).await {
            Ok(0) => break "exit".to_owned(),
            Ok(n) => {
                events::emit(
                    seq.as_ref(),
                    &sender,
                    route(),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::McpDemoChatOutputChunk {
                        session_id,
                        data: BASE64.encode(&buf[..n]),
                    }),
                );
            }
            Err(e) => break format!("demo chat stream error: {e}"),
        }
    };

    if ends_session {
        events::emit(
            seq.as_ref(),
            &sender,
            route(),
            Correlation::None,
            BackendEvent::System(SystemEvent::McpDemoChatEnded { session_id, reason }),
        );
    }
}

/// Write base64-decoded keystrokes to the live session's stdin. A no-op when none is live,
/// matching the pod-exec session's "implicit target" commands.
pub(in crate::backend) async fn send_input(
    state: &mut BackendState,
    data: String,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;

    let Some(session) = state.mcp.demo_chat.session.as_mut() else {
        return Ok(());
    };
    let Some(stdin) = session.stdin.as_mut() else {
        return Ok(());
    };
    let bytes = BASE64.decode(data.as_bytes())?;
    stdin.write_all(&bytes).await?;
    stdin.flush().await?;
    Ok(())
}

/// Record the terminal view's size.
///
/// The subprocess is attached through pipes rather than a pty, so there is no `TIOCSWINSZ` to
/// send: the size is applied as `COLUMNS`/`LINES` at the *next* spawn. Recording it (instead of
/// dropping it) is what makes a chat launched after a window resize wrap correctly.
pub(in crate::backend) fn resize(state: &mut BackendState, cols: u16, rows: u16) {
    state.mcp.demo_chat.size = Some((cols, rows));
}

/// Stop the live session and report it ended, so the terminal view leaves its "running" state
/// even though no EOF has been observed yet.
pub(in crate::backend) fn stop(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    if state.mcp.demo_chat.session.is_none() {
        return;
    }
    let session_id = state.mcp.demo_chat.next_session_id;
    state.mcp.demo_chat.stop();
    emit(
        state,
        sender,
        SystemEvent::McpDemoChatEnded {
            session_id,
            reason: "stopped".to_owned(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a fake `claude` into `dir` and mark it executable (no-op on non-unix, where
    /// `is_executable` only checks that the path is a file).
    fn install_fake_claude(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"#!/bin/sh\nexit 0\n").expect("write fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod fixture");
        }
        path
    }

    fn path_var(dirs: &[&Path]) -> OsString {
        std::env::join_paths(dirs).expect("join PATH")
    }

    /// T043: detection is a real `PATH` scan — it finds a `claude` that exists and reports
    /// nothing when one does not. No subprocess is ever started here.
    #[test]
    fn detection_finds_a_claude_on_the_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let expected = install_fake_claude(dir.path(), "claude");

        let found = detect_claude_cli_in(Some(&path_var(&[dir.path()])));
        assert_eq!(found.as_deref(), Some(expected.as_path()));
    }

    #[test]
    fn detection_reports_nothing_when_no_claude_is_on_the_path() {
        let empty = tempfile::tempdir().expect("tempdir");
        // A directory holding *some* other binary, to prove the scan matches by name and does
        // not just report the first executable it sees.
        let decoy = tempfile::tempdir().expect("tempdir");
        install_fake_claude(decoy.path(), "not-claude");

        assert!(detect_claude_cli_in(Some(&path_var(&[empty.path(), decoy.path()]))).is_none());
        assert!(detect_claude_cli_in(None).is_none(), "no PATH at all");
    }

    /// A non-executable file named `claude` is not a CLI — reporting it as one would be exactly
    /// the fabricated detection FR-009 forbids.
    #[cfg(unix)]
    #[test]
    fn detection_ignores_a_non_executable_file_named_claude() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = install_fake_claude(dir.path(), "claude");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");

        assert!(detect_claude_cli_in(Some(&path_var(&[dir.path()]))).is_none());
    }

    /// T044 / US4 acceptance scenario 3: with the MCP server off, `StartMcpDemoChat` refuses
    /// and never reaches `Command::spawn` — proven by the spawn counter, not by the absence of
    /// an observable side effect. The refusal is a specific reason, not a generic failure.
    #[tokio::test]
    async fn starting_a_demo_chat_is_refused_while_the_server_is_off() {
        let mut state = BackendState::new();
        let (tx, rx) = crossbeam_channel::unbounded();
        assert!(!state.mcp.is_running(), "precondition: the server is off");

        start(&mut state, &tx).await;

        assert_eq!(
            state.mcp.demo_chat.spawn_attempts(),
            0,
            "a refused launch must not reach Command::spawn"
        );
        assert!(state.mcp.demo_chat.session.is_none());

        let events: Vec<_> = rx.try_iter().collect();
        let explained = events.iter().any(|e| {
            matches!(&e.event, BackendEvent::System(SystemEvent::Notification { message, .. })
                if message == SERVER_OFF_REASON)
        });
        assert!(
            explained,
            "the operator must be told the server has to be on first: {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(
                &e.event,
                BackendEvent::System(SystemEvent::McpDemoChatReady { .. })
            )),
            "no session may be reported: {events:?}"
        );
    }

    /// Stopping a chat that was never started is a no-op — no event, no panic.
    #[test]
    fn stopping_a_chat_that_never_started_reports_nothing() {
        let mut state = BackendState::new();
        let (tx, rx) = crossbeam_channel::unbounded();
        stop(&mut state, &tx);
        assert!(rx.try_recv().is_err());
        assert_eq!(state.mcp.demo_chat.spawn_attempts(), 0);
    }

    /// A resize before any launch is remembered, not dropped, and still spawns nothing.
    #[test]
    fn a_resize_is_remembered_for_the_next_launch() {
        let mut state = BackendState::new();
        resize(&mut state, 120, 40);
        assert_eq!(state.mcp.demo_chat.size, Some((120, 40)));
        assert_eq!(state.mcp.demo_chat.spawn_attempts(), 0);
    }

    /// The config handed to `claude` must name the very same invocation Settings shows the
    /// operator (`connection_argv`), under a stable server name.
    #[test]
    fn the_mcp_config_matches_the_connection_command_settings_shows() {
        let (program, args) = super::super::connection_argv();
        let value: serde_json::Value = serde_json::from_str(&mcp_config_json()).expect("json");
        let entry = &value["mcpServers"][MCP_SERVER_NAME];
        assert_eq!(entry["command"], serde_json::json!(program));
        assert_eq!(entry["args"], serde_json::json!(args));
        assert!(super::super::connection_command().starts_with(&program));
    }

    /// Input sent with no live session is dropped, not an error (the "implicit target"
    /// convention `SendPodExecInput` established).
    #[tokio::test]
    async fn input_without_a_session_is_a_no_op() {
        let mut state = BackendState::new();
        send_input(&mut state, BASE64.encode("hello"))
            .await
            .expect("no session is not an error");
    }
}
