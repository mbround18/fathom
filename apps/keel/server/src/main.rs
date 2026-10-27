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
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use keel_core::ports::SystemClock;
use keel_server::config::{self, AuditCommand, Cli, Command, Config, Flags};
use keel_server::{audit, db, seal, serve, sync, telemetry};
use tokio::net::TcpListener;

/// Serving pool size.
const DB_POOL: u32 = 16;
/// How often expired in-memory sessions are swept.
const SESSION_SWEEP: Duration = Duration::from_secs(60);

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.command.is_none() {
        return serve_main(cli.flags);
    }
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
        None => refuse("internal: serve runs outside this runtime"),
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

/// Resolves the configuration and starts telemetry before the runtime (the OTLP client is a
/// blocking one), then serves. The runtime is dropped before the telemetry guard, so the final
/// flush is not cut short.
fn serve_main(flags: Flags) -> ExitCode {
    let cfg = match Config::resolve(flags, &config::process_env) {
        Ok(c) => c,
        Err(e) => return refuse(&e.to_string()),
    };
    let guard = match telemetry::init(cfg.otel_endpoint.as_deref()) {
        Ok(g) => g,
        Err(e) => return refuse(&e),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return refuse(&format!("cannot start the async runtime: {e}")),
    };
    let code = runtime.block_on(serve(cfg, guard.exporting()));
    drop(runtime);
    drop(guard);
    code
}

async fn serve(cfg: Config, exporting: bool) -> ExitCode {
    // Fail closed: no master key, no server (contracts/config.md).
    let master_key = match seal::MasterKey::load(&cfg.master_key_file) {
        Ok(k) => k,
        Err(e) => return refuse(&format!("master key: {e}")),
    };
    let opts = match db::options(&cfg.database_url) {
        Ok(o) => o,
        Err(e) => return refuse(&e),
    };
    if let Err(e) = db::migrate(opts.clone()).await {
        return refuse(&e);
    }
    let pool = match db::connect(opts, DB_POOL).await {
        Ok(p) => p,
        Err(e) => return refuse(&e),
    };
    let providers = match serve::build_providers(&cfg).await {
        Ok(p) => p,
        Err(e) => return refuse(&e),
    };
    let listener = match TcpListener::bind(cfg.listen).await {
        Ok(l) => l,
        Err(e) => return refuse(&format!("cannot listen on {}: {e}", cfg.listen)),
    };
    tracing::info!(
        listen = %cfg.listen,
        public_url = %cfg.public_url,
        providers = ?cfg.provider_ids(),
        static_dir = %cfg.static_dir.display(),
        insecure_dev = cfg.insecure_dev,
        otel = exporting,
        sources = ?cfg.sources,
        "keel serving"
    );
    let app = serve::AppState::new(Arc::new(cfg), providers, pool, Arc::new(SystemClock))
        .with_master_key(master_key);
    let sweeper = app.auth.sessions.spawn_sweeper(SESSION_SWEEP);
    let poller = sync::spawn_poller(app.clone());
    let result = serve::serve(listener, serve::router(app.clone())).await;
    poller.abort();
    sweeper.abort();
    app.db.close().await;
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => refuse(&format!("server failed: {e}")),
    }
}
