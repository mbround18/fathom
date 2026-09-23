//! Stdio MCP (Model Context Protocol) server (015-mcp-agent-exposure, Phase 3/T021).
//!
//! Exposes Fathom's live cluster context to external, same-machine MCP clients (Claude Code,
//! Claude Desktop, or any other MCP-speaking agent) over local stdio — there is no network
//! listener and no HTTP surface. This replaces the deleted `src/mcp_server.rs` prototype, whose
//! axum server on port 18288 also shipped an ungated `restart_deployment` tool.
//!
//! **Off by default (FR-002), and off means nothing exists.** [`McpState::default`] holds no task
//! and no cancellation token, and [`McpState::start`] is reachable from exactly one place in the
//! whole backend: the `SetMcpServerEnabled { enabled: true }` arm of
//! `crate::backend::cmd::mcp::handle`. Nothing spawns on startup, on a context switch, or from a
//! status read. `spawn_attempts` records every call that got as far as `tokio::spawn`, so this
//! invariant is asserted by tests here and in `crate::backend::cmd::mcp`, not just documented.
//!
//! Turning the toggle off calls [`McpState::stop`], which cancels the `CancellationToken` handed
//! to `rmcp::service::serve_server_with_ct` and awaits the task, so no listener survives the
//! command that disabled it (FR-013).

pub(super) mod demo_chat;
pub(super) mod rbac_guard;
pub(super) mod tools_mutate;
pub(super) mod tools_read;

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, RwLock};

use crossbeam_channel::Sender;
use tokio_util::sync::CancellationToken;

use crate::protocol::BackendEventEnvelope;

/// The cluster connection the spawned MCP server task reads through.
///
/// The server task cannot borrow `BackendState` (it outlives any one command's lock), so the
/// backend *publishes* its `kube::Client` here whenever it connects or switches context, and the
/// MCP tools read a clone out of it per call. Critical sections only clone the handle — no
/// `.await` ever happens while the lock is held.
#[derive(Clone, Default)]
pub(super) struct McpClusterHandle(Arc<RwLock<McpCluster>>);

#[derive(Default)]
struct McpCluster {
    client: Option<kube::Client>,
    context: Option<String>,
    /// The backend's own event bus (sequence counter + sender), published alongside the client
    /// so a mutation performed by an MCP tool emits the same `BackendEvent` a UI-driven
    /// mutation does and the operator's Fathom window updates live (015 T037, Principle I).
    events: Option<(Arc<AtomicU64>, Sender<BackendEventEnvelope>)>,
}

impl McpClusterHandle {
    /// Publish the backend's current cluster connection to whatever MCP server task is (or
    /// later will be) running. Safe to call whether or not the server is enabled.
    pub(super) fn publish(&self, client: Option<kube::Client>, context: Option<String>) {
        if let Ok(mut inner) = self.0.write() {
            inner.client = client;
            inner.context = context;
        }
    }

    /// The current `kube::Client`, or `None` when Fathom is not connected to a cluster.
    pub(super) fn client(&self) -> Option<kube::Client> {
        self.0.read().ok().and_then(|inner| inner.client.clone())
    }

    /// The active kubeconfig context name, for tool output and denial messages.
    pub(super) fn context(&self) -> Option<String> {
        self.0.read().ok().and_then(|inner| inner.context.clone())
    }

    /// Publish the backend's event bus, so successful mutations from MCP tools reach every
    /// attached Fathom client. Called from the same `SetMcpServerEnabled` reconcile path that
    /// publishes the cluster client.
    pub(super) fn publish_events(&self, seq: Arc<AtomicU64>, sender: Sender<BackendEventEnvelope>) {
        if let Ok(mut inner) = self.0.write() {
            inner.events = Some((seq, sender));
        }
    }

    /// The published event bus, or `None` when the server was started without one (only
    /// possible in tests — the reconcile path always publishes it).
    pub(super) fn events(&self) -> Option<(Arc<AtomicU64>, Sender<BackendEventEnvelope>)> {
        self.0.read().ok().and_then(|inner| inner.events.clone())
    }
}

/// Lifecycle manager for the stdio MCP server, stored on `BackendState.mcp`.
#[derive(Default)]
pub(super) struct McpState {
    /// The cluster handle the server task reads through; kept even while stopped, so the
    /// backend can publish context switches unconditionally.
    pub(super) cluster: McpClusterHandle,
    /// The running server task and the token that cancels it. Both `None` while off — there
    /// is no "paused server" state.
    running: Option<(tokio::task::JoinHandle<()>, CancellationToken)>,
    /// Set from `Args::disable_mcp`: shells that must never expose an MCP endpoint (e.g.
    /// `fathom-devbridge`) hard-set this and [`Self::start`] refuses outright.
    pub(super) disabled: bool,
    /// How many times [`Self::start`] reached `tokio::spawn`. Instrumentation for the
    /// "off spawns nothing" tests below; never used for control flow.
    spawn_attempts: usize,
    /// 015 US4: the "Launch example chat" subprocess. Lives here rather than on
    /// `BackendState` because it is meaningless without a running server — `demo_chat::start`
    /// reads [`Self::is_running`] before it will spawn anything.
    pub(super) demo_chat: demo_chat::DemoChatState,
}

impl McpState {
    /// Whether a stdio server task exists right now. Never `true` unless
    /// `SetMcpServerEnabled { enabled: true }` has run in this process.
    pub(super) fn is_running(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|(task, _)| !task.is_finished())
    }

    /// Spawn the stdio server, unless it is already running or disabled for this process.
    ///
    /// The `rmcp` handshake itself is awaited *inside* the spawned task, not here: on a stdio
    /// transport `serve_server_with_ct` blocks until the client sends `initialize`, which may be
    /// never in a GUI process. The command handler must not block on that.
    pub(super) fn start(&mut self) -> anyhow::Result<()> {
        if self.disabled {
            anyhow::bail!("The MCP server is disabled in this process.");
        }
        if self.is_running() {
            return Ok(());
        }

        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let cluster = self.cluster.clone();
        self.spawn_attempts += 1;
        let task = tokio::spawn(async move {
            let server = tools_read::FathomMcpServer::new(cluster);
            match rmcp::service::serve_server_with_ct(
                server,
                rmcp::transport::io::stdio(),
                task_cancel,
            )
            .await
            {
                Ok(running) => {
                    if let Ok(reason) = running.waiting().await {
                        tracing::info!("MCP stdio server stopped: {reason:?}");
                    }
                }
                Err(e) => tracing::warn!("MCP stdio server did not start: {e}"),
            }
        });

        self.running = Some((task, cancel));
        tracing::info!("MCP stdio server started");
        Ok(())
    }

    /// Cancel and await the server task, if any. Idempotent, and a no-op while off.
    pub(super) async fn stop(&mut self) {
        // A demo chat outliving the server it attached to would be a session talking to
        // nothing, so it goes down with it.
        self.demo_chat.stop();
        let Some((task, cancel)) = self.running.take() else {
            return;
        };
        cancel.cancel();
        let _ = task.await;
        tracing::info!("MCP stdio server stopped");
    }

    /// Await the running server task, returning as soon as it finishes on its own.
    ///
    /// For the headless `--only-mcp` process, whose whole reason to exist is that one task: it
    /// finishes when the attached client disconnects (stdin EOF), and the process should then
    /// exit rather than linger as an orphan (quickstart Scenario 3 / SC-004). A no-op when
    /// nothing is running.
    pub(super) async fn wait(&mut self) {
        if let Some((task, _)) = self.running.take() {
            let _ = task.await;
        }
    }

    /// Abort the server task without awaiting it, for the synchronous shutdown path.
    pub(super) fn abort(&mut self) {
        self.demo_chat.stop();
        if let Some((task, cancel)) = self.running.take() {
            cancel.cancel();
            task.abort();
        }
    }

    #[cfg(test)]
    fn spawn_attempts(&self) -> usize {
        self.spawn_attempts
    }
}

/// What the backend should do after [`bootstrap`] has run.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Bootstrap {
    /// Carry on into the normal command loop (the GUI and devbridge shells).
    Continue,
    /// This process is finished — `--only-mcp` either refused to run or has served its client
    /// and exited. There is no command loop to enter.
    Done,
}

/// Start the stdio MCP server at process startup if it is enabled, and in `--only-mcp` mode run
/// it as the entire process.
///
/// Called once from [`super::init`], before the command loop. Three outcomes:
///
/// * Not enabled, not headless → [`Bootstrap::Continue`] with nothing spawned (the default).
/// * Headless but not enabled → a stated refusal and [`Bootstrap::Done`]: a process whose only
///   job is an endpoint that is switched off would otherwise block forever, which an attaching
///   client can only observe as an unexplained hang (FR-013: fail cleanly, never silently).
/// * Headless and enabled → connect to the cluster, serve the client, exit when it disconnects.
pub(super) fn bootstrap(
    rt: &tokio::runtime::Runtime,
    state: &mut crate::backend::state::BackendState,
    sender: &Sender<BackendEventEnvelope>,
    args: &crate::config::Args,
) -> Bootstrap {
    match (args.only_mcp, startup_enabled(args)) {
        (true, false) => {
            tracing::error!(
                "Refusing to run --only-mcp: the MCP server is turned off. \
                 Enable it in Fathom's Settings → Available Agents, or pass --mcp-stdio."
            );
            Bootstrap::Done
        }
        (false, false) => Bootstrap::Continue,
        (false, true) => {
            start_at_startup(rt, state, sender, false);
            Bootstrap::Continue
        }
        (true, true) => {
            start_at_startup(rt, state, sender, true);
            // The server task *is* the process here. It ends when the attached client closes
            // stdin; waiting for it is what stops a disconnected client leaving an orphan
            // behind (SC-004).
            rt.block_on(state.mcp.wait());
            tracing::info!("Headless MCP server finished; exiting.");
            Bootstrap::Done
        }
    }
}

/// Publish what the server task needs, then spawn it. `headless` additionally gives it a cluster
/// connection of its own, which only a `--only-mcp` process lacks.
fn start_at_startup(
    rt: &tokio::runtime::Runtime,
    state: &mut crate::backend::state::BackendState,
    sender: &Sender<BackendEventEnvelope>,
    headless: bool,
) {
    state
        .mcp
        .cluster
        .publish_events(state.cluster.seq.clone(), sender.clone());

    if headless {
        rt.block_on(connect_headless(state));
    }

    // `McpState::start` calls `tokio::spawn`, so it needs the runtime in scope.
    let _guard = rt.enter();
    if let Err(e) = state.mcp.start() {
        tracing::warn!("Could not start the MCP server on startup: {e}");
    }
}

/// Give the headless server a cluster connection of its own.
///
/// A `--only-mcp` process has no UI to issue `SwitchContext`, so without this nothing would ever
/// hand the tools a `kube::Client` and every call would answer "no cluster connected". It uses
/// the kubeconfig's own current context — the same one the operator's Fathom window opens on.
///
/// A failure here is not fatal: the server still starts and answers `tools/list`, and each tool
/// call then reports the missing connection honestly instead of inventing data.
async fn connect_headless(state: &mut crate::backend::state::BackendState) {
    use crate::backend::kube;

    state.cluster.current_context = kube::read_kubeconfig_current_context();
    match kube::connect_client_for_current_context(state).await {
        Ok(client) => {
            state.cluster.client = Some(client);
            state.mcp.cluster.publish(
                state.cluster.client.clone(),
                state.cluster.current_context.clone(),
            );
            tracing::info!(
                "Headless MCP server connected to context {:?}",
                state.cluster.current_context
            );
        }
        Err(e) => tracing::warn!("Headless MCP server has no cluster connection: {e:#}"),
    }
}

/// Whether this process should bring the stdio server up at startup, before any command arrives.
///
/// Three inputs, in order of authority:
///
/// 1. `Args::disable_mcp` — a hard no for shells that must never expose an endpoint
///    (`fathom-devbridge`). Beats everything else.
/// 2. `Args::mcp_stdio` / `FATHOM_MCP_STDIO_ENABLED` — a force-on that skips the persisted
///    preference, so a client can launch the server without the operator having visited Settings.
/// 3. The persisted `auto_start.json` `mcp_stdio` preference, written by the Settings toggle.
///    Absent or unreadable means **off** (FR-002), never "assume on".
///
/// A `false` here is why a stock Fathom install spawns nothing: neither flag is set and no
/// preference file says otherwise.
pub(super) fn startup_enabled(args: &crate::config::Args) -> bool {
    if args.disable_mcp {
        return false;
    }
    args.mcp_stdio || crate::backend::persistence::load_mcp_auto_start().unwrap_or(false)
}

/// The literal command an external MCP client should launch to attach to this Fathom
/// installation over stdio (FR-007, data-model.md `McpServerStatus.connectionCommand`).
///
/// `--only-mcp` (`src/config.rs`'s `Args::only_mcp`, env `FATHOM_ONLY_MCP`) runs the backend
/// headless so the process's own stdin/stdout *is* the MCP channel.
pub(super) fn connection_command() -> String {
    let (program, args) = connection_argv();
    std::iter::once(program)
        .chain(args)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The same invocation as [`connection_command`], already split into program and arguments.
///
/// One source of truth for two consumers: the copyable snippet an operator pastes into their
/// own MCP client's config, and the machine-readable server entry `demo_chat` hands the
/// `claude` CLI. Keeping them derived from one function is what makes "the snippet in Settings
/// is exactly what the demo chat uses" true rather than aspirational.
pub(super) fn connection_argv() -> (String, Vec<String>) {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "fathom".to_owned());
    (exe, vec!["--only-mcp".to_owned()])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FR-002's invariant, structurally: a freshly built manager holds no task and no token,
    /// and has never reached `tokio::spawn`. Nothing in `BackendState::new()` or the backend's
    /// startup path calls `start()`, so this is the state of every Fathom process until an
    /// operator flips the toggle on.
    #[test]
    fn a_disabled_manager_holds_no_task_and_has_never_spawned() {
        let mcp = McpState::default();
        assert!(mcp.running.is_none(), "no task or cancellation token");
        assert!(!mcp.is_running());
        assert_eq!(mcp.spawn_attempts(), 0, "start() was never called");
    }

    #[tokio::test]
    async fn stopping_a_never_started_manager_spawns_nothing() {
        let mut mcp = McpState::default();
        mcp.stop().await;
        mcp.stop().await;
        assert!(!mcp.is_running());
        assert_eq!(mcp.spawn_attempts(), 0);
    }

    /// A process with `Args::disable_mcp` (the devbridge shell) can never spawn the server even
    /// if a `SetMcpServerEnabled { enabled: true }` somehow reached it: `start()` bails before
    /// `tokio::spawn`.
    #[test]
    fn start_is_refused_before_spawning_when_the_process_disables_mcp() {
        let mut mcp = McpState {
            disabled: true,
            ..Default::default()
        };
        assert!(mcp.start().is_err());
        assert!(!mcp.is_running());
        assert_eq!(mcp.spawn_attempts(), 0);
    }

    fn args(argv: &[&str]) -> crate::config::Args {
        use clap::Parser;
        crate::config::Args::parse_from(std::iter::once("fathom").chain(argv.iter().copied()))
    }

    /// `--disable-mcp` shells win over every other input, including the force-on flag, so a
    /// devbridge process can never be talked into exposing an endpoint.
    #[test]
    fn a_process_that_disables_mcp_never_starts_it_at_startup() {
        let mut a = args(&["--mcp-stdio"]);
        a.disable_mcp = true;
        assert!(!startup_enabled(&a));
    }

    /// The force-on flag does not consult the persisted preference — this is the path an external
    /// MCP client uses, and it must not depend on the operator's `~/.fathom` contents.
    #[test]
    fn the_force_on_flag_enables_startup_without_reading_the_preference() {
        assert!(startup_enabled(&args(&["--mcp-stdio"])));
        assert!(startup_enabled(&args(&["--only-mcp", "--mcp-stdio"])));
    }

    /// `--only-mcp` with the server switched off must terminate the backend rather than enter a
    /// command loop nothing sends commands to: the client that launched it needs an answer, and a
    /// silent hang is not one.
    #[test]
    fn headless_with_the_server_off_is_done_before_spawning_anything() {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mut state = crate::backend::state::BackendState::new();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut a = args(&["--only-mcp"]);
        // Forced rather than relying on the absence of the developer's own `~/.fathom`.
        a.disable_mcp = true;

        assert_eq!(bootstrap(&rt, &mut state, &tx, &a), Bootstrap::Done);
        assert!(!state.mcp.is_running());
        assert_eq!(state.mcp.spawn_attempts(), 0);
    }

    /// The windowed shells must fall through to their command loop untouched when MCP is off —
    /// the default for every stock install.
    #[test]
    fn a_windowed_shell_with_the_server_off_continues_without_spawning() {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let mut state = crate::backend::state::BackendState::new();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut a = args(&[]);
        a.disable_mcp = true;

        assert_eq!(bootstrap(&rt, &mut state, &tx, &a), Bootstrap::Continue);
        assert!(!state.mcp.is_running());
        assert_eq!(state.mcp.spawn_attempts(), 0);
    }

    #[test]
    fn connection_command_names_the_headless_stdio_flag() {
        let cmd = connection_command();
        assert!(
            cmd.ends_with(" --only-mcp"),
            "connection command must invoke the headless stdio mode: {cmd}"
        );
    }

    #[test]
    fn the_cluster_handle_reports_no_client_until_the_backend_publishes_one() {
        let handle = McpClusterHandle::default();
        assert!(handle.client().is_none());
        assert!(handle.context().is_none());
        handle.publish(None, Some("kind-a".to_owned()));
        assert_eq!(handle.context().as_deref(), Some("kind-a"));
        assert!(handle.client().is_none());
    }
}
