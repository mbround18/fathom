//! `keel-relay serve | key | check-config` (contracts/relay.md "CLI").
//!
//! stdout carries only command output (`key` prints exactly two lines); logs go to stderr. An
//! invalid config or `hub_keys` exits 78 (`EX_CONFIG`), which the systemd unit lists in
//! `RestartPreventExitStatus`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use keel_relay::config::{Config, DEFAULT_CONFIG_PATH};
use keel_relay::key::{self, DEFAULT_KEY_PATH};

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
            Ok(config) => {
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
        Command::Serve { config, key } => {
            let config = match check(&config) {
                Ok(config) => config,
                Err(code) => return code,
            };
            if let Err(e) = key::load_or_create_host_key(&key) {
                tracing::error!("{e}");
                return ExitCode::FAILURE;
            }
            // The link and runner listeners land with spec 067 US1.
            tracing::error!(
                link_listen = %config.link_listen,
                runner_listen = %config.runner_listen,
                "serving is not implemented yet"
            );
            ExitCode::FAILURE
        }
    }
}

/// Loads the config and its `hub_keys`; either failing is `EX_CONFIG`.
fn check(path: &Path) -> Result<Config, ExitCode> {
    let config = Config::load(path).map_err(|e| {
        tracing::error!("{e}");
        ExitCode::from(EX_CONFIG)
    })?;
    key::load_hub_keys(&config.hub_keys).map_err(|e| {
        tracing::error!("{e}");
        ExitCode::from(EX_CONFIG)
    })?;
    Ok(config)
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
