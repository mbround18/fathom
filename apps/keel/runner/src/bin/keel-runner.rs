//! `keel-runner`: the network half: holds the SSH channel to the Keel server (spec 055).
//!
//! Phase 1 scaffolding: only `--help` and `--version` work.

use clap::Parser;

/// Keel host runner: the network half: holds the SSH channel to the Keel server.
#[derive(Debug, Parser)]
#[command(name = "keel-runner", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    eprintln!("keel-runner: not implemented yet (spec 055 Phase 1 scaffolding)");
    std::process::exit(2);
}
