//! `keel-runner`: the network half. Holds the SSH channel to Keel and relays run frames to the
//! executor socket (spec 055 T084, contracts/runner.md).

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use keel_runner::runner::join::EX_CONFIG;
use keel_runner::runner::{KEY_FILE, Runner, RunnerConfig, load_or_create_key};

/// Keel host runner: the network half. Holds the SSH channel to the Keel server.
#[derive(Debug, Parser)]
#[command(name = "keel-runner", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Keel's runner listener, `host[:port]` (default port 7831). The host part must match the
    /// server's host certificate. Required to run.
    #[arg(long, env = "KEEL_RUNNER_SERVER")]
    server: Option<String>,
    /// Dial this `host:port` instead of `--server` (repeatable; `KEEL_RUNNER_DIAL` is
    /// comma-separated). Tried in order each pass, for example a zone's relays. The host
    /// certificate must still verify for `--server`'s host name.
    #[arg(long, env = "KEEL_RUNNER_DIAL", value_delimiter = ',')]
    dial: Vec<String>,
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
    /// How long a run result Keel has not acknowledged is kept for replay (`30d`, `12h`, `90m`).
    #[arg(
        long,
        env = "KEEL_RUNNER_MAX_OFFLINE",
        default_value = "30d",
        value_parser = keel_runner::runner::spool::parse_duration
    )]
    max_offline: std::time::Duration,
    /// The host's age public key, reported to Keel so it can tell which sops files this host
    /// can decrypt. Optional: a missing or malformed file reports nothing.
    #[arg(
        long,
        env = "KEEL_RUNNER_AGE_RECIPIENT_FILE",
        default_value = keel_runner::runner::DEFAULT_AGE_RECIPIENT_FILE
    )]
    age_recipient_file: PathBuf,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Prints the host key's SHA-256 fingerprint, to compare with the one Keel shows beside a
    /// join request, and exits.
    Fingerprint,
}

/// `keel-runner fingerprint`: reads the key the runner created; never creates one, so running
/// it as another user before the first start leaves no key owned by that user.
fn fingerprint(cli: &Cli) -> ! {
    if !cli.state_dir.join(KEY_FILE).exists() {
        eprintln!("keel-runner: no host key yet; start keel-runner once to create it");
        std::process::exit(1);
    }
    match load_or_create_key(&cli.state_dir) {
        Ok(key) => {
            println!(
                "{}",
                keel_exec::ca::fingerprint(key.public_key().key_data())
            );
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("keel-runner: {e}");
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Command::Fingerprint)) {
        fingerprint(&cli);
    }
    let Some(server) = cli.server else {
        eprintln!("keel-runner: --server (KEEL_RUNNER_SERVER) is required");
        std::process::exit(2);
    };
    let runner = match Runner::load(RunnerConfig {
        server,
        // `KEEL_RUNNER_DIAL=` (empty) or a trailing comma means no entry, not an empty one.
        dial: cli
            .dial
            .into_iter()
            .map(|a| a.trim().to_owned())
            .filter(|a| !a.is_empty())
            .collect(),
        trust_dir: cli.trust_dir,
        state_dir: cli.state_dir,
        executor_socket: cli.executor_socket,
        venv: cli.venv,
    }) {
        Ok(r) => r
            .with_outside_log(cli.outside_log)
            .with_max_offline(cli.max_offline)
            .with_age_recipient_file(cli.age_recipient_file),
        Err(e) => {
            eprintln!("keel-runner: {e}");
            std::process::exit(1);
        }
    };
    if runner.certificate().ok().flatten().is_none() {
        eprintln!(
            "keel-runner: not enrolled; host key {}",
            keel_exec::ca::fingerprint(runner.public_key())
        );
    }
    // A join token waits here until an operator decides; the session loop below is unchanged.
    if let Err(e) = runner.join_if_needed().await {
        eprintln!("keel-runner: {e}");
        std::process::exit(if e.is_config() { EX_CONFIG } else { 1 });
    }
    runner.run_forever().await
}
