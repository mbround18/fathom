// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use fathom::config::Args;

fn main() {
    let args = Args::parse();
    if args.only_mcp {
        fathom_desktop::run_only_mcp(args);
    } else {
        fathom_desktop::run_with_args(args);
    }
}
