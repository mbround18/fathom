//! The stdio MCP server's operator toggle (015-mcp-agent-exposure, T024).
//!
//! Two commands, both answered with `SystemEvent::McpServerStatusChanged`:
//!
//! * `LoadMcpServerStatus` — a pure read of in-process state plus the connection snippet. Reads
//!   nothing from the cluster and, critically, **never starts anything**.
//! * `SetMcpServerEnabled { enabled }` — persists the preference (`~/.fathom/auto_start.json`'s
//!   `mcp_stdio`) *and* starts or stops the server task in the same handler.
//!
//! This handler is the only place in the backend that calls
//! [`crate::backend::mcp::McpState::start`], which is what makes "off means no listener exists"
//! (FR-002/FR-013) a structural property rather than a promise: there is no startup path, no
//! context-switch path and no status-read path that can spawn one.

use crossbeam_channel::Sender;

use crate::backend::events;
use crate::backend::mcp;
use crate::backend::persistence;
use crate::backend::state::BackendState;
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};

/// Handles every 015 MCP-server command; returns the command back if it belongs to another
/// family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::LoadMcpServerStatus => {
            let enabled = persistence::load_mcp_auto_start().unwrap_or(false);
            emit_status(state, sender, enabled);
            // 015 US4/FR-009: the panel must not offer a "Launch example chat" button it cannot
            // honour, so the same fresh PATH scan `StartMcpDemoChat` would do runs here too and
            // tells the UI up front when there is nothing to launch. Silence means "a `claude`
            // was on PATH a moment ago" — the launch re-checks rather than trusting this.
            if mcp::demo_chat::detect_claude_cli().is_none() {
                events::emit(
                    state.cluster.seq.as_ref(),
                    sender,
                    route(),
                    Correlation::None,
                    BackendEvent::System(SystemEvent::McpDemoChatNotDetected {
                        instructions: mcp::demo_chat::NOT_DETECTED_INSTRUCTIONS.to_owned(),
                    }),
                );
            }
        }
        BackendCmd::SetMcpServerEnabled { enabled } => {
            set_enabled(state, sender, enabled).await;
        }
        // 015 US4: the "Launch example chat" subprocess. `mcp::demo_chat` owns every honesty
        // rule (server-off refusal, fresh PATH detection); this arm is only dispatch.
        BackendCmd::StartMcpDemoChat => mcp::demo_chat::start(state, sender).await,
        BackendCmd::StopMcpDemoChat => mcp::demo_chat::stop(state, sender),
        BackendCmd::SendMcpDemoChatInput { data } => {
            mcp::demo_chat::send_input(state, data).await?;
        }
        BackendCmd::ResizeMcpDemoChat { cols, rows } => mcp::demo_chat::resize(state, cols, rows),
        other => return Ok(Some(other)),
    }
    Ok(None)
}

fn route() -> Route {
    Route::global(section_keys::cluster_settings())
}

/// Broadcast the current status. `running` is read from the manager itself, never assumed from
/// `enabled`, so a failed start is visible as `enabled: true, running: false` instead of being
/// silently reported as healthy.
fn emit_status(state: &BackendState, sender: &Sender<BackendEventEnvelope>, enabled: bool) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        route(),
        Correlation::None,
        BackendEvent::System(SystemEvent::McpServerStatusChanged {
            enabled,
            running: state.mcp.is_running(),
            connection_command: mcp::connection_command(),
        }),
    );
}

async fn set_enabled(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    enabled: bool,
) {
    if let Err(e) = persistence::save_mcp_auto_start(enabled) {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Could not save the MCP server preference: {e}"),
        );
        // Still reconcile the live server with what the operator asked for: the toggle they
        // just flipped must take effect now even if the preference did not survive to disk.
    }
    reconcile(state, sender, enabled).await;
}

/// Bring the live server in line with `enabled`, then broadcast. Split from [`set_enabled`] so
/// the reconcile/broadcast behaviour is testable without writing to `~/.fathom/`.
async fn reconcile(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>, enabled: bool) {
    if enabled {
        // Publish whatever cluster connection exists right now, so a server started while
        // already connected does not have to wait for the next context switch.
        state.mcp.cluster.publish(
            state.cluster.client.clone(),
            state.cluster.current_context.clone(),
        );
        // 015 T037: give the server task the backend's event bus, so a mutation an external
        // agent performs emits the same `BackendEvent` a UI-driven one does.
        state
            .mcp
            .cluster
            .publish_events(state.cluster.seq.clone(), sender.clone());
        if let Err(e) = state.mcp.start() {
            events::notify_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Could not start the MCP server: {e}"),
            );
        }
    } else {
        state.mcp.stop().await;
    }

    emit_status(state, sender, enabled);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FR-002, at the command boundary: a status read must never bring a server into being.
    /// `LoadMcpServerStatus` is the one command the UI issues on every panel mount, so if it
    /// could start the server, "off by default" would be unobservable in practice.
    #[tokio::test]
    async fn loading_status_never_starts_the_server() {
        let mut state = BackendState::new();
        let (tx, rx) = crossbeam_channel::unbounded();

        let leftover = handle(BackendCmd::LoadMcpServerStatus, &mut state, &tx)
            .await
            .expect("status read should succeed");
        assert!(leftover.is_none(), "this family owns LoadMcpServerStatus");
        assert!(
            !state.mcp.is_running(),
            "a status read must not spawn the stdio server"
        );

        let envelope = rx.try_recv().expect("status event should be emitted");
        match envelope.event {
            BackendEvent::System(SystemEvent::McpServerStatusChanged {
                running,
                connection_command,
                ..
            }) => {
                assert!(!running);
                assert!(connection_command.ends_with(" --only-mcp"));
            }
            other => panic!("expected McpServerStatusChanged, got {other:?}"),
        }
    }

    /// Turning it off is a no-op when it was never on, and still reports status — the UI needs
    /// an answer to every command, and the manager must stay empty.
    #[tokio::test]
    async fn disabling_an_already_off_server_stops_nothing_and_still_reports() {
        let mut state = BackendState::new();
        let (tx, rx) = crossbeam_channel::unbounded();

        // `reconcile` rather than `handle`, so the test never writes to the operator's real
        // `~/.fathom/auto_start.json` (`persistence` resolves the home directory directly).
        reconcile(&mut state, &tx, false).await;

        assert!(!state.mcp.is_running());
        let events: Vec<_> = rx.try_iter().collect();
        assert!(
            events.iter().any(|e| matches!(
                &e.event,
                BackendEvent::System(SystemEvent::McpServerStatusChanged {
                    enabled: false,
                    running: false,
                    ..
                })
            )),
            "every change must broadcast a status: {events:?}"
        );
    }

    /// Commands from other families pass straight through.
    #[tokio::test]
    async fn unrelated_commands_are_handed_back() {
        let mut state = BackendState::new();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let leftover = handle(BackendCmd::LoadUiConfig, &mut state, &tx)
            .await
            .expect("pass-through should succeed");
        assert!(matches!(leftover, Some(BackendCmd::LoadUiConfig)));
    }
}
