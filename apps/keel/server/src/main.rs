//! `keel`: the Keel server binary (spec 055). A thin layer that stitches the Keel crates together.
//!
//! Phase 1 scaffolding: only `--help` and `--version` work; later phases fill the modules.

mod api;
mod audit;
mod config;
mod db;
mod dispatch;
mod hooks;
mod queue;
mod runner_listener;
mod seal;
mod serve;
mod sync;
mod telemetry;
mod ws;

use clap::Parser;

/// Keel: the Ansible control plane bound to source control.
#[derive(Debug, Parser)]
#[command(name = "keel", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    eprintln!("keel: not implemented yet (spec 055 Phase 1 scaffolding)");
    std::process::exit(2);
}
