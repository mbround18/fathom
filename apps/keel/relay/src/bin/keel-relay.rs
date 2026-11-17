//! `keel-relay serve | key | check-config` (contracts/relay.md "CLI").
//!
//! stdout carries only command output (`key` prints exactly two lines); logs go to stderr. An
//! invalid config or `hub_keys` exits 78 (`EX_CONFIG`), which the systemd unit lists in
//! `RestartPreventExitStatus`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use keel_relay::Relay;
use keel_relay::config::{Config, DEFAULT_CONFIG_PATH};
use keel_relay::key::{self, DEFAULT_KEY_PATH};
use russh::keys::ssh_key::public::KeyData;

/// sysexits.h `EX_CONFIG`.
const EX_CONFIG: u8 = 78;

#[derive(Debug, Parser)]
#[command(
    name = "keel-relay",
    version,
    about = "Keel relay: carries runner connections from a spoke network to Keel"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the relay (started by systemd).
    Serve {
        #[arg(long, env = "KEEL_RELAY_CONFIG", default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        #[arg(long, env = "KEEL_RELAY_KEY", default_value = DEFAULT_KEY_PATH)]
        key: PathBuf,
    },
    /// Print the host key's public line and SHA256 fingerprint, generating the key if missing.
    Key {
        #[arg(long, env = "KEEL_RELAY_KEY", default_value = DEFAULT_KEY_PATH)]
        key: PathBuf,
    },
    /// Check relay.toml and the hub_keys file it names; exit 78 if either is invalid.
    CheckConfig {
        #[arg(long, env = "KEEL_RELAY_CONFIG", default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
    },
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    match Cli::parse().command {
        Command::Key { key } => print_key(&key),
        Command::CheckConfig { config } => match check(&config) {
            Ok((config, _)) => {
                eprintln!(
                    "config ok: hub_keys {}, link {}, runners {}, max_streams {}",
                    config.hub_keys.display(),
                    config.link_listen,
                    config.runner_listen,
                    config.max_streams
                );
                ExitCode::SUCCESS
            }
            Err(code) => code,
        },
        Command::Serve { config, key } => serve(&config, &key),
    }
}

/// `keel-relay serve`: binds both listeners and serves until SIGTERM or SIGINT, then closes the
/// link and every runner stream.
fn serve(config: &Path, key: &Path) -> ExitCode {
    let (config, hub_keys) = match check(config) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    let host_key = match key::load_or_create_host_key(key) {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async move {
        let relay = match Relay::bind(&config, host_key, hub_keys).await {
            Ok(relay) => relay,
            Err(e) => {
                tracing::error!(
                    link_listen = %config.link_listen,
                    runner_listen = %config.runner_listen,
                    "bind: {e}"
                );
                return ExitCode::FAILURE;
            }
        };
        match relay.run_until(shutdown_signal()).await {
            Ok(()) => {
                tracing::info!("keel-relay stopped");
                ExitCode::SUCCESS
            }
            Err(e) => {
                tracing::error!("listener failed: {e}");
                ExitCode::FAILURE
            }
        }
    })
}

/// Resolves on SIGTERM (systemd stop) or SIGINT.
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        }
        Err(e) => {
            tracing::warn!("SIGTERM handler: {e}; stopping on SIGINT only");
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

/// Loads the config and its `hub_keys`; either failing is `EX_CONFIG`.
fn check(path: &Path) -> Result<(Config, Vec<KeyData>), ExitCode> {
    let config = Config::load(path).map_err(|e| {
        tracing::error!("{e}");
        ExitCode::from(EX_CONFIG)
    })?;
    let hub_keys = key::load_hub_keys(&config.hub_keys).map_err(|e| {
        tracing::error!("{e}");
        ExitCode::from(EX_CONFIG)
    })?;
    Ok((config, hub_keys))
}

fn print_key(path: &Path) -> ExitCode {
    match key::load_or_create_host_key(path) {
        Ok(k) => {
            let [line, fingerprint] = key::public_lines(&k);
            println!("{line}");
            println!("{fingerprint}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}
