//! `depths`: the Depths server binary (spec 050). A thin execution layer: resolve the config,
//! open the users registry, build the sign-in providers, serve. Every refusal exits nonzero with
//! one line naming what to fix; no port is bound until all of it succeeded.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use depths_server::config::{Config, ProviderConfig};
use depths_server::http::{AppState, Providers, router};
use depths_server::users::Registry;
use web_auth::provider::IdentityProvider;

fn main() -> ExitCode {
    // 054 FR-021: `depths passkey-recovery --user <id>` touches only the data dir, so it runs
    // before telemetry, config and the server runtime.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("passkey-recovery") {
        return passkey_recovery(&args[1..]);
    }
    // Before the runtime: the OTLP exporter's HTTP client is a blocking one.
    let _telemetry = match depths_server::telemetry::init() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("depths: {e}");
            return ExitCode::from(2);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("depths: cannot start the async runtime: {e}");
            return ExitCode::from(2);
        }
    };
    let result = rt.block_on(run());
    // Shut the runtime down before the telemetry guard flushes.
    drop(rt);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("depths: {e}");
            ExitCode::from(2)
        }
    }
}

const RECOVERY_USAGE: &str = "usage: depths passkey-recovery --user <provider:subject> \
     [--data-dir <path>] (or DEPTHS_DATA_DIR; default /var/lib/depths)";

/// Prints a one-time passkey recovery code for `--user`. Exit 2 with one line on any refusal.
fn passkey_recovery(args: &[String]) -> ExitCode {
    let mut user = None;
    let mut data_dir = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) => (f, Some(v.to_owned())),
            None => (a.as_str(), None),
        };
        let slot = match flag {
            "--user" => &mut user,
            "--data-dir" => &mut data_dir,
            "-h" | "--help" => {
                println!("{RECOVERY_USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("depths: unknown argument `{a}`\n{RECOVERY_USAGE}");
                return ExitCode::from(2);
            }
        };
        match inline.or_else(|| it.next().cloned()) {
            Some(v) => *slot = Some(v),
            None => {
                eprintln!("depths: {flag} needs a value\n{RECOVERY_USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(user) = user else {
        eprintln!("depths: --user is required\n{RECOVERY_USAGE}");
        return ExitCode::from(2);
    };
    let data_dir = std::path::PathBuf::from(
        data_dir
            .or_else(|| std::env::var("DEPTHS_DATA_DIR").ok())
            .unwrap_or_else(|| "/var/lib/depths".into()),
    );
    if !data_dir.is_absolute() {
        eprintln!(
            "depths: the data dir `{}` must be an absolute path",
            data_dir.display()
        );
        return ExitCode::from(2);
    }
    let rotate = match std::env::var("DEPTHS_AUDIT_ROTATE_BYTES") {
        Ok(v) => match depths_server::config::parse_bytes("DEPTHS_AUDIT_ROTATE_BYTES", &v) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("depths: {e}");
                return ExitCode::from(2);
            }
        },
        Err(_) => 64 << 20,
    };
    let rt = match tokio::runtime::Builder::new_current_thread().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("depths: cannot start the async runtime: {e}");
            return ExitCode::from(2);
        }
    };
    let now = chrono::Utc::now();
    match rt.block_on(depths_server::passkeys::recovery_command(
        &data_dir, rotate, &user, now,
    )) {
        Ok(issued) => {
            let mins = (issued.record.expires - now.timestamp()).max(0) / 60;
            println!("Recovery code for {user}: {}", issued.code);
            println!(
                "It works once, for {mins} minutes. Sign in as {user} and enter it on the \
                 Sessions page; it removes all of that person's passkeys."
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("depths: {e}");
            ExitCode::from(2)
        }
    }
}

async fn run() -> Result<(), String> {
    let cfg = Config::from_process().map_err(|e| e.to_string())?;
    for (name, from) in &cfg.sources {
        tracing::info!(setting = name, from = %from, "config");
    }
    if let Some(w) = &cfg.step_up_warning {
        tracing::warn!("{w}");
    }
    cfg.prepare_data_dir().map_err(|e| e.to_string())?;
    let oidc_issuer = cfg.providers.iter().find_map(|p| match p {
        ProviderConfig::Oidc { issuer, .. } => Some(issuer.clone()),
        _ => None,
    });
    let users = Registry::open(cfg.users_path(), cfg.owners.clone(), oidc_issuer)
        .map_err(|e| e.to_string())?;
    let providers = build_providers(&cfg).await?;
    let cfg = Arc::new(cfg);
    let runtime = Arc::new(depths_runtime_cli::CliRuntime::new(cfg.docker_bin.clone()));
    let cli = (*runtime).clone();
    let state = AppState::new(cfg.clone(), providers, Arc::new(users), runtime)?;
    // Spec 062: with DEPTHS_UPDATES=off nothing is opened and no registry client exists.
    let updates = depths_server::updates::Updates::open(
        &cfg,
        Arc::new(depths_server::updates::SystemClock),
        || match depths_server::updates::registry::OciRegistry::new(
            cfg.updates.docker_config.clone(),
        ) {
            Ok(r) => Arc::new(r),
            Err(e) => {
                tracing::warn!(error = %e, "updates: no registry client; checks report unreachable");
                Arc::new(depths_server::updates::registry::Unavailable(e))
            }
        },
    )?;
    let state = state.with_updates(updates);
    tracing::info!(
        "{}",
        depths_server::updates::startup_line(&cfg.updates, state.updates.as_deref())
    );
    // 053 R2: the helper image is found (and old helpers swept) before the first request.
    let (files_state, hostname) = (state.clone(), std::env::var("HOSTNAME").ok());
    tokio::task::spawn_blocking(move || {
        depths_server::files::start(&files_state, cli, hostname.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?;
    let _reaper = depths_server::files::spawn_reaper(state.clone());
    let _sweeper = state.auth.sessions.spawn_sweeper(Duration::from_secs(60));
    let _refresher = Arc::new(depths_server::refresh::Refresher::default()).spawn(state.clone());
    let _update_tasks = state.updates.clone().map(|u| {
        use depths_server::updates::{auto, inputs, run, scheduler};
        // A run open at start was cut short: closed as interrupted, never resumed.
        let closed = run::interrupt_open_runs(&u, &state.ops);
        if closed > 0 {
            tracing::warn!(
                runs = closed,
                "update runs found open at start, marked interrupted"
            );
        }
        let inputs: Arc<dyn inputs::Inputs> = Arc::new(inputs::RuntimeInputs {
            store: state.store.clone(),
            runtime: state.runtime.clone(),
            hostname: std::env::var("HOSTNAME").ok(),
        });
        let deps = Arc::new(auto::Deps {
            ops: state.ops.clone(),
            inputs: inputs.clone(),
            zone: Arc::new(auto::HostZone),
            is_admin: auto::admins(state.users.clone()),
            sleep: Arc::new(std::thread::sleep),
        });
        (
            tokio::spawn(scheduler::run(u.clone(), inputs)),
            tokio::spawn(auto::run(u, deps)),
        )
    });

    let listener = tokio::net::TcpListener::bind(cfg.listen)
        .await
        .map_err(|e| format!("cannot listen on {} (DEPTHS_LISTEN): {e}", cfg.listen))?;
    tracing::info!(
        listen = %cfg.listen,
        public_url = %cfg.public_url,
        providers = ?cfg.provider_ids(),
        static_dir = %cfg.static_dir.display(),
        "depths listening"
    );
    depths_server::serve::serve(listener, router(state))
        .await
        .map_err(|e| e.to_string())
}

async fn build_providers(cfg: &Config) -> Result<Providers, String> {
    let mut out: Providers = Vec::new();
    for p in &cfg.providers {
        let provider: Arc<dyn IdentityProvider> = match p {
            ProviderConfig::Discord {
                client_id,
                client_secret,
            } => Arc::new(
                web_auth::discord::DiscordProvider::new(web_auth::discord::DiscordConfig {
                    client_id: client_id.clone(),
                    client_secret: client_secret.clone(),
                    public_url: cfg.public_url.clone(),
                    request_email: false,
                })
                .map_err(|e| format!("discord provider: {e}"))?,
            ),
            ProviderConfig::Github {
                client_id,
                client_secret,
            } => Arc::new(
                web_auth::github::GithubProvider::new(web_auth::github::GithubConfig {
                    client_id: client_id.clone(),
                    client_secret: client_secret.clone(),
                    redirect_url: web_auth::oauth::callback_url(&cfg.public_url, "github")
                        .map_err(|e| format!("github provider: {e}"))?,
                })
                .map_err(|e| format!("github provider: {e}"))?,
            ),
            ProviderConfig::Oidc {
                issuer,
                client_id,
                client_secret,
                scopes,
            } => {
                let http = web_auth::oidc::build_http_client(None)
                    .map_err(|e| format!("oidc provider: {e}"))?;
                let provider = web_auth::oidc::OidcProvider::discover(
                    web_auth::oidc::OidcConfig {
                        issuer: issuer.clone(),
                        client_id: client_id.clone(),
                        client_secret: client_secret.clone(),
                        redirect_url: web_auth::oauth::callback_url(&cfg.public_url, "oidc")
                            .map_err(|e| format!("oidc provider: {e}"))?,
                        scopes: scopes.clone(),
                        post_logout_url: cfg.public_url.clone(),
                    },
                    web_auth::oidc::ClaimsConfig {
                        username: "preferred_username".into(),
                        groups: "groups".into(),
                        username_prefix: String::new(),
                        groups_prefix: String::new(),
                        groups_required: false,
                    },
                    http,
                )
                .await
                .map_err(|e| format!("oidc provider {issuer}: {e}"))?;
                Arc::new(provider)
            }
        };
        out.push((p.id(), provider));
    }
    Ok(out)
}
