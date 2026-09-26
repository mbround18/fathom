//! Headless live-validation driver: the real backend (exactly what the Tauri shell spawns),
//! driven by JSON `BackendCmd` lines on stdin, with every event envelope printed as one JSON
//! line on stdout. No write guard: point it only at clusters you may mutate.
//!
//! Used for the live validation tasks that need backend paths the devbridge refuses (kind
//! provisioning, node shell, multi-context federation) on hosts without WebKitWebDriver:
//!
//! ```sh
//! mkfifo /tmp/cmds
//! cargo run --example live_driver < /tmp/cmds > events.jsonl &
//! exec 3>/tmp/cmds
//! echo '{"type":"SetContext","data":{"context":"kind-x"}}' >&3
//! ```
//!
//! Isolate state with `HOME=<scratch>` (config, `~/.fathom`) and `KUBECONFIG=<scratch file>`.
//! A line `#quit` (or EOF) sends `Shutdown` and exits, like closing the app window.

use crossbeam_channel::unbounded;
use fathom::bridge::BackendLink;
use fathom::bridge::spawn_backend;
use fathom::config::Args;
use fathom::protocol::{BackendCmd, BackendEventEnvelope, CmdRequest};
use fathom::startup::{init_tracing, install_crypto_provider};
use std::io::{BufRead, Write};

fn main() -> anyhow::Result<()> {
    install_crypto_provider();
    init_tracing();
    let (gui_tx, gui_rx) = unbounded::<BackendEventEnvelope>();
    let (cmd_tx, cmd_rx) = unbounded::<CmdRequest>();
    let backend = spawn_backend(
        cmd_rx,
        gui_tx,
        Args {
            mcp_stdio: false,
            only_mcp: false,
            disable_mcp: true,
        },
    );
    let quiet = std::env::var_os("LIVE_DRIVER_QUIET").is_some();
    std::thread::spawn(move || {
        let stdout = std::io::stdout();
        for env in gui_rx {
            if quiet {
                continue;
            }
            let mut out = stdout.lock();
            if let Ok(json) = serde_json::to_string(&env) {
                let _ = writeln!(out, "{json}");
                let _ = out.flush();
            }
        }
    });
    let link = BackendLink::new(cmd_tx);
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "#quit" {
            break;
        }
        match serde_json::from_str::<BackendCmd>(line) {
            Ok(cmd) => {
                let id = link.send(cmd);
                eprintln!("live_driver: sent {id:?}: {line}");
            }
            Err(e) => eprintln!("live_driver: bad command {line}: {e}"),
        }
    }
    // Like the desktop shell on exit: `Shutdown`, then a bounded wait for the backend.
    let _ = link.send(BackendCmd::Shutdown);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !backend.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(())
}
