//! `keel-executor`: runs Ansible as the automation user, verifying what it is given (spec 055).
//!
//! Phase 1 scaffolding: only `--help` and `--version` work.

use clap::Parser;

/// Keel host runner: runs Ansible as the automation user, verifying what it is given.
#[derive(Debug, Parser)]
#[command(name = "keel-executor", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    eprintln!("keel-executor: not implemented yet (spec 055 Phase 1 scaffolding)");
    std::process::exit(2);
}
