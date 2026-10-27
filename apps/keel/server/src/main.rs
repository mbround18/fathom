//! `keel`: the Keel server binary (spec 055). A thin layer over `keel_server`.
//!
//! - `keel` serves.
//! - `keel migrate` applies database migrations.
//! - `keel audit verify` checks the audit hash chain.
//! - `keel keygen` writes a new master key.
//!
//! Exit codes: 0 success, 1 a check failed (audit chain broken), 2 refused (configuration or
//! start-up error).

use std::process::ExitCode;

use clap::Parser;
use keel_server::config::{self, AuditCommand, Cli, Command, Flags};
use keel_server::{audit, db, seal};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return refuse(&format!("cannot start the async runtime: {e}")),
    };
    runtime.block_on(run(cli))
}

async fn run(cli: Cli) -> ExitCode {
    match cli.command {
        Some(Command::Migrate) => migrate(&cli.flags).await,
        Some(Command::Audit {
            command: AuditCommand::Verify,
        }) => audit_verify(&cli.flags).await,
        Some(Command::Keygen) => keygen(&cli.flags),
        None => serve(cli.flags).await,
    }
}

fn refuse(msg: &str) -> ExitCode {
    eprintln!("keel: {msg}");
    ExitCode::from(2)
}

async fn migrate(flags: &Flags) -> ExitCode {
    let opts = match config::database_url(flags, &config::process_env)
        .map_err(|e| e.to_string())
        .and_then(|url| db::options(&url))
    {
        Ok(o) => o,
        Err(e) => return refuse(&e),
    };
    match db::migrate(opts).await {
        Ok(()) => {
            println!("keel: migrations applied");
            ExitCode::SUCCESS
        }
        Err(e) => refuse(&e),
    }
}

async fn audit_verify(flags: &Flags) -> ExitCode {
    let opts = match config::database_url(flags, &config::process_env)
        .map_err(|e| e.to_string())
        .and_then(|url| db::options(&url))
    {
        Ok(o) => o,
        Err(e) => return refuse(&e),
    };
    let pool = match db::connect(opts, 1).await {
        Ok(p) => p,
        Err(e) => return refuse(&e),
    };
    let report = match pool.acquire().await {
        Ok(mut conn) => audit::verify(&mut conn).await,
        Err(e) => Err(e),
    };
    pool.close().await;
    match report {
        Ok(audit::Report {
            checked,
            broken: None,
        }) => {
            println!("keel: audit chain intact ({checked} events)");
            ExitCode::SUCCESS
        }
        Ok(audit::Report {
            checked,
            broken: Some(b),
        }) => {
            println!(
                "keel: audit chain BROKEN at seq {} (event {}): {}; {checked} events before it verified",
                b.seq, b.id, b.why
            );
            ExitCode::from(1)
        }
        Err(e) => refuse(&format!("cannot read the audit log: {e}")),
    }
}

fn keygen(flags: &Flags) -> ExitCode {
    let path = match config::master_key_file(flags, &config::process_env) {
        Ok(p) => p,
        Err(e) => return refuse(&e.to_string()),
    };
    match seal::keygen(&path) {
        Ok(()) => {
            println!("keel: wrote a new master key to {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => refuse(&e.to_string()),
    }
}

async fn serve(_flags: Flags) -> ExitCode {
    refuse("serving is not implemented yet")
}
