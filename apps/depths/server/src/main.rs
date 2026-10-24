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

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("DEPTHS_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        // Plain text into `docker logs`; colour only on a terminal.
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
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
    let state = AppState::new(cfg.clone(), providers, Arc::new(users), runtime)?;
    let _sweeper = state.auth.sessions.spawn_sweeper(Duration::from_secs(60));

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
