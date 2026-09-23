//! `fathom-devbridge`: serve the Fathom backend over a loopback WebSocket for dev and
//! live e2e runs. See the crate docs in `lib.rs` for the safety fences.

use anyhow::{Context as _, bail};
use clap::Parser;
use crossbeam_channel::unbounded;
use fathom::bridge::pump::BUFFER_CAP;
use fathom::bridge::{BackendLink, spawn_backend, spawn_pump};
use fathom::config::Args;
use fathom::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, CmdRequest, Correlation, SystemEvent,
};
use fathom::startup::{init_tracing, install_crypto_provider};
use fathom_devbridge::guard::WriteGuard;
use fathom_devbridge::server::{Bridge, WsHub, serve};
use fathom_devbridge::{DEFAULT_PORT, sandbox};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

/// How long startup waits for the backend to connect to `--context`.
const CONTEXT_SWITCH_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Parser, Debug)]
#[command(version, about = "Dev-only WebSocket bridge to the Fathom backend")]
struct Cli {
    /// Port to listen on (always bound to 127.0.0.1).
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// Shared secret clients must pass as `?token=` on the WebSocket upgrade.
    #[arg(long, env = "FATHOM_DEVBRIDGE_TOKEN", hide_env_values = true)]
    token: String,

    /// Kube context the backend is pinned to. Switching to any other context is refused.
    #[arg(long)]
    context: String,

    /// The only namespace writes may target. Must exist in `--context` and be labelled
    /// `fathom.dev/sandbox=true`.
    #[arg(long)]
    write_namespace: String,
}

/// What startup learns from the pump about its initial `SetContext`.
enum Startup {
    ContextChanged(Option<String>),
    Finished { ok: bool, error: Option<String> },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.token.trim().is_empty() {
        bail!("--token / FATHOM_DEVBRIDGE_TOKEN must not be empty");
    }
    install_crypto_provider();
    init_tracing();

    sandbox::verify_sandbox(&cli.context, &cli.write_namespace)
        .await
        .context("sandbox verification failed")?;
    tracing::info!(
        "devbridge: namespace '{}' in context '{}' is the labelled sandbox",
        cli.write_namespace,
        cli.context
    );

    let (gui_tx, gui_rx) = unbounded::<BackendEventEnvelope>();
    let (cmd_tx, cmd_rx) = unbounded::<CmdRequest>();
    let args = Args {
        mcp_stdio: false,
        only_mcp: false,
        disable_mcp: true,
    };
    let _backend_thread = spawn_backend(cmd_rx, gui_tx, args);

    let hub: Arc<WsHub> = Arc::new(WsHub::new(BUFFER_CAP));
    let link = BackendLink::new(cmd_tx);
    let set_context = link
        .send(BackendCmd::SetContext {
            context: cli.context.clone(),
        })
        .context("backend exited before startup")?;

    let (startup_tx, startup_rx) = mpsc::channel::<Startup>();
    let mut startup_tx = Some(startup_tx);
    spawn_pump(gui_rx, Arc::clone(&hub), &link, move |envelope| {
        let Some(tx) = &startup_tx else { return };
        if envelope.correlation != Correlation::Request(set_context) {
            return;
        }
        match &envelope.event {
            BackendEvent::System(SystemEvent::ContextChanged { current }) => {
                let _ = tx.send(Startup::ContextChanged(current.clone()));
            }
            BackendEvent::CommandFinished { ok, error } => {
                let _ = tx.send(Startup::Finished {
                    ok: *ok,
                    error: error.clone(),
                });
                startup_tx = None;
            }
            _ => {}
        }
    });

    let expected = cli.context.clone();
    tokio::task::spawn_blocking(move || wait_for_context(&startup_rx, &expected))
        .await
        .context("startup wait panicked")??;

    let guard = WriteGuard::new(cli.context.clone(), cli.write_namespace.clone());
    let bridge = Arc::new(Bridge {
        link,
        hub,
        guard,
        token: cli.token,
    });
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, cli.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!(
        "devbridge: listening on ws://{addr}/?token=… (context '{}', writes only in '{}')",
        cli.context,
        cli.write_namespace
    );
    serve(listener, bridge).await?;
    Ok(())
}

/// Block until the initial `SetContext` finishes. Fails unless the backend reported the
/// expected context; a non-fatal error after the switch is only logged.
fn wait_for_context(rx: &mpsc::Receiver<Startup>, expected: &str) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + CONTEXT_SWITCH_TIMEOUT;
    let mut switched = false;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(Startup::ContextChanged(current)) => {
                if current.as_deref() != Some(expected) {
                    bail!("backend switched to {current:?}, expected '{expected}'");
                }
                switched = true;
            }
            Ok(Startup::Finished { ok, error }) => {
                let error = error.unwrap_or_default();
                if !switched {
                    bail!("switching to context '{expected}' failed: {error}");
                }
                if !ok {
                    tracing::warn!("devbridge: context switch reported errors: {error}");
                }
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                bail!("timed out switching to context '{expected}'")
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("backend exited while switching to context '{expected}'")
            }
        }
    }
}
