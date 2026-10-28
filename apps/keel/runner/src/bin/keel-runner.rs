//! `keel-runner`: the network half. Holds the SSH channel to Keel and relays run frames to the
//! executor socket (spec 055 T084, contracts/runner.md).

use std::path::PathBuf;

use clap::Parser;
use keel_runner::runner::{Runner, RunnerConfig};

/// Keel host runner: the network half. Holds the SSH channel to the Keel server.
#[derive(Debug, Parser)]
#[command(name = "keel-runner", version, about)]
struct Cli {
    /// Keel's runner listener, `host[:port]` (default port 7831). The host part must match the
    /// server's host certificate.
    #[arg(long, env = "KEEL_RUNNER_SERVER")]
    server: String,
    /// `policy.toml` and `host_ca.pub`.
    #[arg(
        long,
        env = "KEEL_RUNNER_TRUST_DIR",
        default_value = "/etc/keel-runner/trust"
    )]
    trust_dir: PathBuf,
    /// The host key, certificate, host id, KRL and enrollment token.
    #[arg(
        long,
        env = "KEEL_RUNNER_STATE_DIR",
        default_value = "/var/lib/keel-runner"
    )]
    state_dir: PathBuf,
    /// The executor's socket.
    #[arg(
        long,
        env = "KEEL_RUNNER_EXECUTOR_SOCKET",
        default_value = "/run/keel-runner/executor.sock"
    )]
    executor_socket: PathBuf,
    /// The Ansible venv, for the versions reported to Keel.
    #[arg(
        long,
        env = "KEEL_RUNNER_VENV",
        default_value = "/opt/keel-runner/venv"
    )]
    venv: PathBuf,
    /// The record of `ansible-playbook` runs made outside Keel, which the runner reports.
    #[arg(
        long,
        env = "KEEL_RUNNER_OUTSIDE_LOG",
        default_value = keel_runner::runner::outside::DEFAULT_OUTSIDE_LOG
    )]
    outside_log: PathBuf,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let runner = match Runner::load(RunnerConfig {
        server: cli.server,
        trust_dir: cli.trust_dir,
        state_dir: cli.state_dir,
        executor_socket: cli.executor_socket,
        venv: cli.venv,
    }) {
        Ok(r) => r.with_outside_log(cli.outside_log),
        Err(e) => {
            eprintln!("keel-runner: {e}");
            std::process::exit(1);
        }
    };
    runner.run_forever().await
}
