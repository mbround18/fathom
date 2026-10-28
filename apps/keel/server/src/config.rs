//! Server configuration (spec 055 T020, contracts/config.md).
//!
//! Every setting comes from a flag, its `KEEL_*` variable, or a default, in that order. Each
//! resolved setting remembers where it came from so start-up can log it; secret values never
//! reach [`Config::sources`] or `Debug`.
//!
//! Sign-in providers use the same settings as Depths, under the `KEEL_` prefix:
//! `KEEL_{DISCORD,GITHUB,OIDC}_CLIENT_ID` (or `--{discord,github,oidc}-client-id`),
//! `KEEL_{..}_CLIENT_SECRET` or `KEEL_{..}_CLIENT_SECRET_FILE` (never a flag: the process list is
//! world-readable), `KEEL_OIDC_ISSUER` (`--oidc-issuer`) and `KEEL_OIDC_SCOPES`. At least one is
//! required; there is no anonymous mode.
//!
//! Anything wrong refuses start with one line naming the setting.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use secrecy::SecretString;
use url::Url;

/// The command line.
#[derive(Debug, Parser)]
#[command(
    name = "keel",
    version,
    about = "Keel: the Ansible control plane bound to source control"
)]
pub struct Cli {
    #[command(flatten)]
    pub flags: Flags,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcommands. With none, `keel` serves.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Write a new 32-byte master key to --master-key-file (mode 0400). Refuses to overwrite.
    Keygen,
    /// Apply database migrations and exit.
    Migrate,
    /// Audit log tools.
    Audit {
        #[command(subcommand)]
        command: AuditCommand,
    },
    /// The SSH certificate authorities runners trust.
    SshCa {
        #[command(subcommand)]
        command: SshCaCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum SshCaCommand {
    /// Print both CAs' fingerprints and public keys, and the `@cert-authority` line for
    /// `keel_runner_host_ca`. Creates the CAs if they do not exist yet.
    Show,
}

#[derive(Debug, Subcommand)]
pub enum AuditCommand {
    /// Check the audit hash chain; exits nonzero at the first broken event.
    Verify,
}

/// Flags. Every one is optional here; env and defaults are applied in [`Config::resolve`].
#[derive(Debug, Default, Args)]
pub struct Flags {
    /// UI, /api and /hooks [env: KEEL_LISTEN] [default: 0.0.0.0:7830]
    #[arg(long, global = true)]
    pub listen: Option<String>,
    /// Embedded SSH server for host runners [env: KEEL_RUNNER_LISTEN] [default: 0.0.0.0:7831]
    #[arg(long, global = true)]
    pub runner_listen: Option<String>,
    /// Lifetime of runner certificates [env: KEEL_RUNNER_CERT_TTL] [default: 24h]
    #[arg(long, global = true)]
    pub runner_cert_ttl: Option<String>,
    /// How long after certificate expiry a host may renew with its bare key
    /// [env: KEEL_RUNNER_MAX_OFFLINE] [default: 30d]
    #[arg(long, global = true)]
    pub runner_max_offline: Option<String>,
    /// Names on the runner listener's host certificate, comma-separated; runners connect to one
    /// of them (`keel_runner_server`) [env: KEEL_RUNNER_HOST_NAMES] [default: --public-url's host]
    #[arg(long, global = true)]
    pub runner_host_names: Option<String>,
    /// Signed mode for new bindings [env: KEEL_SIGNED_MODE_DEFAULT] [default: false]
    #[arg(long, global = true)]
    pub signed_mode_default: Option<String>,
    /// External origin, like https://keel.example.com (required) [env: KEEL_PUBLIC_URL]
    #[arg(long, global = true)]
    pub public_url: Option<String>,
    /// PostgreSQL URL (required) [env: KEEL_DATABASE_URL]
    #[arg(long, global = true)]
    pub database_url: Option<String>,
    /// 32-byte master key, mode 0400 (required) [env: KEEL_MASTER_KEY_FILE]
    #[arg(long, global = true)]
    pub master_key_file: Option<PathBuf>,
    /// Git mirrors and export downloads [env: KEEL_DATA_DIR] [default: /var/lib/keel]
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
    /// Web dist, served from disk [env: KEEL_STATIC_DIR] [default: /usr/share/keel/web]
    #[arg(long, global = true)]
    pub static_dir: Option<PathBuf>,
    /// OIDC issuer URL [env: KEEL_OIDC_ISSUER]
    #[arg(long, global = true)]
    pub oidc_issuer: Option<String>,
    /// OIDC client id [env: KEEL_OIDC_CLIENT_ID]
    #[arg(long, global = true)]
    pub oidc_client_id: Option<String>,
    /// GitHub OAuth client id [env: KEEL_GITHUB_CLIENT_ID]
    #[arg(long, global = true)]
    pub github_client_id: Option<String>,
    /// Discord OAuth client id [env: KEEL_DISCORD_CLIENT_ID]
    #[arg(long, global = true)]
    pub discord_client_id: Option<String>,
    /// How long a passkey check covers step-up actions [env: KEEL_STEP_UP_WINDOW] [default: 5m]
    #[arg(long, global = true)]
    pub step_up_window: Option<String>,
    /// Approval TTL for policies that set none [env: KEEL_APPROVAL_TTL_DEFAULT] [default: 24h]
    #[arg(long, global = true)]
    pub approval_ttl_default: Option<String>,
    /// Largest stored log per run [env: KEEL_RUN_LOG_CAP] [default: 64MiB]
    #[arg(long, global = true)]
    pub run_log_cap: Option<String>,
    /// OTLP endpoint; telemetry is off unless set [env: OTEL_EXPORTER_OTLP_ENDPOINT]
    #[arg(long, global = true)]
    pub otel_endpoint: Option<String>,
    /// Allow an http:// public URL, loopback listen only (development) [env: KEEL_INSECURE_DEV]
    #[arg(long, global = true)]
    pub insecure_dev: bool,
}

/// Why start-up refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

fn refuse<T>(msg: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError(msg.into()))
}

/// A configured sign-in provider. `Debug` never shows the secret.
#[derive(Clone)]
pub enum ProviderConfig {
    Discord {
        client_id: String,
        client_secret: SecretString,
    },
    Github {
        client_id: String,
        client_secret: SecretString,
    },
    Oidc {
        issuer: String,
        client_id: String,
        client_secret: Option<SecretString>,
        scopes: Vec<String>,
    },
}

impl ProviderConfig {
    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            Self::Discord { .. } => "discord",
            Self::Github { .. } => "github",
            Self::Oidc { .. } => "oidc",
        }
    }
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Discord { client_id, .. } | Self::Github { client_id, .. } => f
                .debug_struct(self.id())
                .field("client_id", client_id)
                .field("client_secret", &"[redacted]")
                .finish(),
            Self::Oidc {
                issuer,
                client_id,
                scopes,
                ..
            } => f
                .debug_struct("oidc")
                .field("issuer", issuer)
                .field("client_id", client_id)
                .field("scopes", scopes)
                .finish_non_exhaustive(),
        }
    }
}

/// The resolved configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub runner_listen: SocketAddr,
    pub runner_cert_ttl: Duration,
    pub runner_max_offline: Duration,
    /// Principals of the listener's host certificate.
    pub runner_host_names: Vec<String>,
    pub signed_mode_default: bool,
    /// An origin only (no path).
    pub public_url: Url,
    /// May carry a password: never logged.
    pub database_url: SecretString,
    pub master_key_file: PathBuf,
    pub data_dir: PathBuf,
    pub static_dir: PathBuf,
    pub providers: Vec<ProviderConfig>,
    pub step_up_window: Duration,
    pub approval_ttl_default: Duration,
    pub run_log_cap: u64,
    pub otel_endpoint: Option<String>,
    pub insecure_dev: bool,
    /// `(setting, where it came from)`, for the start-up log. Never values.
    pub sources: Vec<(&'static str, String)>,
}

/// How the environment is read: the process env in production, a map in tests.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The process environment.
#[must_use]
pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

struct Resolver<'a> {
    env: Env<'a>,
    sources: Vec<(&'static str, String)>,
}

impl Resolver<'_> {
    fn var(&self, name: &str) -> Option<String> {
        (self.env)(name).filter(|v| !v.trim().is_empty())
    }

    /// flag > env > default (`None`).
    fn pick(&mut self, setting: &'static str, flag: Option<String>, env: &str) -> Option<String> {
        if let Some(v) = flag.filter(|v| !v.trim().is_empty()) {
            self.sources
                .push((setting, format!("--{}", setting.replace('_', "-"))));
            return Some(v);
        }
        if let Some(v) = self.var(env) {
            self.sources.push((setting, env.to_owned()));
            return Some(v);
        }
        self.sources.push((setting, "default".to_owned()));
        None
    }

    fn required(
        &mut self,
        setting: &'static str,
        flag: Option<String>,
        env: &str,
    ) -> Result<String, ConfigError> {
        match self.pick(setting, flag, env) {
            Some(v) => Ok(v),
            None => refuse(format!(
                "--{} ({env}) is required",
                setting.replace('_', "-")
            )),
        }
    }

    /// A secret from `NAME` or the file `NAME_FILE`; both set refuses.
    fn secret(
        &mut self,
        setting: &'static str,
        name: &str,
    ) -> Result<Option<SecretString>, ConfigError> {
        let file_var = format!("{name}_FILE");
        match (self.var(name), self.var(&file_var)) {
            (Some(_), Some(_)) => refuse(format!("{name} and {file_var} are both set; set one")),
            (Some(v), None) => {
                self.sources.push((setting, name.to_owned()));
                Ok(Some(SecretString::from(v)))
            }
            (None, Some(path)) => {
                let raw = std::fs::read_to_string(&path)
                    .or_else(|e| refuse(format!("{file_var}: cannot read {path}: {e}")))?;
                let v = raw.trim().to_owned();
                if v.is_empty() {
                    return refuse(format!("{file_var}: {path} is empty"));
                }
                self.sources.push((setting, file_var));
                Ok(Some(SecretString::from(v)))
            }
            (None, None) => Ok(None),
        }
    }
}

fn parse_addr(name: &str, v: &str) -> Result<SocketAddr, ConfigError> {
    v.trim().parse().or_else(|_| {
        refuse(format!(
            "{name}: `{v}` is not an address (like 0.0.0.0:7830)"
        ))
    })
}

fn parse_bool(name: &str, v: &str) -> Result<bool, ConfigError> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => refuse(format!("{name}: `{v}` is not a boolean (true or false)")),
    }
}

/// `90s`, `5m`, `24h`, `30d`.
pub fn parse_duration(name: &str, v: &str) -> Result<Duration, ConfigError> {
    let v = v.trim();
    let split = v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len());
    let (num, unit) = v.split_at(split);
    let bad = || {
        refuse(format!(
            "{name}: `{v}` is not a duration (like 5m, 24h or 30d)"
        ))
    };
    let Ok(n) = num.parse::<u64>() else {
        return bad();
    };
    let mult = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return bad(),
    };
    if n == 0 {
        return refuse(format!("{name}: must be more than zero"));
    }
    Ok(Duration::from_secs(n.saturating_mul(mult)))
}

/// `1048576`, `512KiB`, `64MiB`, `1GiB`, `10MB`.
pub fn parse_bytes(name: &str, v: &str) -> Result<u64, ConfigError> {
    let v = v.trim();
    let split = v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len());
    let (num, unit) = v.split_at(split);
    let bad = || refuse(format!("{name}: `{v}` is not a size (like 64MiB)"));
    let Ok(n) = num.parse::<u64>() else {
        return bad();
    };
    let mult: u64 = match unit.to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "kib" => 1 << 10,
        "mib" => 1 << 20,
        "gib" => 1 << 30,
        "kb" => 1_000,
        "mb" => 1_000_000,
        "gb" => 1_000_000_000,
        _ => return bad(),
    };
    if n == 0 {
        return refuse(format!("{name}: must be more than zero"));
    }
    Ok(n.saturating_mul(mult))
}

/// An origin only: redirect URIs, receiver URLs and the Origin check are built from it.
/// `KEEL_RUNNER_HOST_NAMES`: DNS names (letters, digits, `.`, `-`), at least one.
fn parse_host_names(raw: &str) -> Result<Vec<String>, ConfigError> {
    let names: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .collect();
    let valid = |n: &String| {
        n.len() <= 253
            && n.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
    };
    if names.is_empty() || !names.iter().all(valid) {
        return refuse("KEEL_RUNNER_HOST_NAMES must be comma-separated DNS names");
    }
    Ok(names)
}

fn parse_public_url(raw: &str, insecure_dev: bool) -> Result<Url, ConfigError> {
    const NAME: &str = "--public-url (KEEL_PUBLIC_URL)";
    let raw = raw.trim();
    let url = Url::parse(raw).or_else(|e| refuse(format!("{NAME}: `{raw}` is not a URL: {e}")))?;
    match url.scheme() {
        "https" => {}
        "http" if insecure_dev => {}
        "http" => {
            return refuse(format!(
                "{NAME}: must be https:// (http:// only with --insecure-dev)"
            ));
        }
        other => return refuse(format!("{NAME}: scheme `{other}` is not https")),
    }
    if url.host_str().is_none_or(str::is_empty) {
        return refuse(format!("{NAME}: `{raw}` has no host"));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return refuse(format!(
            "{NAME}: must be an origin only (like https://keel.example.com)"
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return refuse(format!("{NAME}: must not carry credentials"));
    }
    Ok(url)
}

/// `--database-url` alone, for `keel migrate` and `keel audit verify`.
pub fn database_url(flags: &Flags, env: Env<'_>) -> Result<SecretString, ConfigError> {
    let mut r = Resolver {
        env,
        sources: Vec::new(),
    };
    r.required(
        "database_url",
        flags.database_url.clone(),
        "KEEL_DATABASE_URL",
    )
    .map(SecretString::from)
}

/// `--master-key-file` alone, for `keel keygen`.
pub fn master_key_file(flags: &Flags, env: Env<'_>) -> Result<PathBuf, ConfigError> {
    let mut r = Resolver {
        env,
        sources: Vec::new(),
    };
    r.required(
        "master_key_file",
        flags
            .master_key_file
            .as_ref()
            .map(|p| p.display().to_string()),
        "KEEL_MASTER_KEY_FILE",
    )
    .map(PathBuf::from)
}

impl Config {
    /// Resolves the serving configuration.
    pub fn resolve(flags: Flags, env: Env<'_>) -> Result<Self, ConfigError> {
        let mut r = Resolver {
            env,
            sources: Vec::new(),
        };

        let insecure_dev = if flags.insecure_dev {
            r.sources.push(("insecure_dev", "--insecure-dev".into()));
            true
        } else {
            match r.var("KEEL_INSECURE_DEV") {
                Some(v) => {
                    r.sources.push(("insecure_dev", "KEEL_INSECURE_DEV".into()));
                    parse_bool("KEEL_INSECURE_DEV", &v)?
                }
                None => false,
            }
        };

        let listen = r.pick("listen", flags.listen, "KEEL_LISTEN");
        let listen = parse_addr("KEEL_LISTEN", listen.as_deref().unwrap_or("0.0.0.0:7830"))?;
        if insecure_dev && !listen.ip().is_loopback() {
            return refuse("--insecure-dev needs a loopback --listen (like 127.0.0.1:7830)");
        }
        let runner_listen = r.pick("runner_listen", flags.runner_listen, "KEEL_RUNNER_LISTEN");
        let runner_listen = parse_addr(
            "KEEL_RUNNER_LISTEN",
            runner_listen.as_deref().unwrap_or("0.0.0.0:7831"),
        )?;

        let dur = |r: &mut Resolver<'_>,
                   setting,
                   flag,
                   env: &str,
                   default: &str|
         -> Result<Duration, ConfigError> {
            let v = r.pick(setting, flag, env);
            parse_duration(env, v.as_deref().unwrap_or(default))
        };
        let runner_cert_ttl = dur(
            &mut r,
            "runner_cert_ttl",
            flags.runner_cert_ttl,
            "KEEL_RUNNER_CERT_TTL",
            "24h",
        )?;
        let runner_max_offline = dur(
            &mut r,
            "runner_max_offline",
            flags.runner_max_offline,
            "KEEL_RUNNER_MAX_OFFLINE",
            "30d",
        )?;
        let step_up_window = dur(
            &mut r,
            "step_up_window",
            flags.step_up_window,
            "KEEL_STEP_UP_WINDOW",
            "5m",
        )?;
        let approval_ttl_default = dur(
            &mut r,
            "approval_ttl_default",
            flags.approval_ttl_default,
            "KEEL_APPROVAL_TTL_DEFAULT",
            "24h",
        )?;

        let signed = r.pick(
            "signed_mode_default",
            flags.signed_mode_default,
            "KEEL_SIGNED_MODE_DEFAULT",
        );
        let signed_mode_default = parse_bool(
            "KEEL_SIGNED_MODE_DEFAULT",
            signed.as_deref().unwrap_or("false"),
        )?;

        let cap = r.pick("run_log_cap", flags.run_log_cap, "KEEL_RUN_LOG_CAP");
        let run_log_cap = parse_bytes("KEEL_RUN_LOG_CAP", cap.as_deref().unwrap_or("64MiB"))?;

        let public_url = r.required("public_url", flags.public_url, "KEEL_PUBLIC_URL")?;
        let public_url = parse_public_url(&public_url, insecure_dev)?;
        let runner_host_names = r.pick(
            "runner_host_names",
            flags.runner_host_names,
            "KEEL_RUNNER_HOST_NAMES",
        );
        let runner_host_names = parse_host_names(
            runner_host_names
                .as_deref()
                .or(public_url.host_str())
                .unwrap_or_default(),
        )?;
        let database_url = SecretString::from(r.required(
            "database_url",
            flags.database_url,
            "KEEL_DATABASE_URL",
        )?);
        let master_key_file = PathBuf::from(r.required(
            "master_key_file",
            flags.master_key_file.map(|p| p.display().to_string()),
            "KEEL_MASTER_KEY_FILE",
        )?);

        let data_dir = r.pick(
            "data_dir",
            flags.data_dir.map(|p| p.display().to_string()),
            "KEEL_DATA_DIR",
        );
        let data_dir = PathBuf::from(data_dir.unwrap_or_else(|| "/var/lib/keel".into()));
        let static_dir = r.pick(
            "static_dir",
            flags.static_dir.map(|p| p.display().to_string()),
            "KEEL_STATIC_DIR",
        );
        let static_dir = PathBuf::from(static_dir.unwrap_or_else(|| "/usr/share/keel/web".into()));

        let otel_endpoint = r.pick(
            "otel_endpoint",
            flags.otel_endpoint,
            "OTEL_EXPORTER_OTLP_ENDPOINT",
        );

        let mut providers = Vec::new();
        if let Some(client_id) = r.pick_quiet(
            "discord_client_id",
            flags.discord_client_id,
            "KEEL_DISCORD_CLIENT_ID",
        ) {
            let Some(client_secret) =
                r.secret("discord_client_secret", "KEEL_DISCORD_CLIENT_SECRET")?
            else {
                return refuse("discord: KEEL_DISCORD_CLIENT_SECRET (or _FILE) is required");
            };
            providers.push(ProviderConfig::Discord {
                client_id,
                client_secret,
            });
        }
        if let Some(client_id) = r.pick_quiet(
            "github_client_id",
            flags.github_client_id,
            "KEEL_GITHUB_CLIENT_ID",
        ) {
            let Some(client_secret) =
                r.secret("github_client_secret", "KEEL_GITHUB_CLIENT_SECRET")?
            else {
                return refuse("github: KEEL_GITHUB_CLIENT_SECRET (or _FILE) is required");
            };
            providers.push(ProviderConfig::Github {
                client_id,
                client_secret,
            });
        }
        let oidc_issuer = r.pick_quiet("oidc_issuer", flags.oidc_issuer, "KEEL_OIDC_ISSUER");
        let oidc_client = r.pick_quiet(
            "oidc_client_id",
            flags.oidc_client_id,
            "KEEL_OIDC_CLIENT_ID",
        );
        match (oidc_issuer, oidc_client) {
            (Some(issuer), Some(client_id)) => {
                let client_secret = r.secret("oidc_client_secret", "KEEL_OIDC_CLIENT_SECRET")?;
                let scopes = r
                    .var("KEEL_OIDC_SCOPES")
                    .map(|s| {
                        s.split([',', ' '])
                            .filter(|x| !x.is_empty())
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_else(|| vec!["openid".into(), "profile".into(), "email".into()]);
                providers.push(ProviderConfig::Oidc {
                    issuer,
                    client_id,
                    client_secret,
                    scopes,
                });
            }
            (Some(_), None) => {
                return refuse("oidc: --oidc-client-id (KEEL_OIDC_CLIENT_ID) is required");
            }
            (None, Some(_)) => return refuse("oidc: --oidc-issuer (KEEL_OIDC_ISSUER) is required"),
            (None, None) => {}
        }
        if providers.is_empty() {
            return refuse(
                "no sign-in provider is configured: set KEEL_OIDC_ISSUER and KEEL_OIDC_CLIENT_ID, \
                 or KEEL_GITHUB_CLIENT_ID, or KEEL_DISCORD_CLIENT_ID, with its secret",
            );
        }

        Ok(Config {
            listen,
            runner_listen,
            runner_cert_ttl,
            runner_max_offline,
            runner_host_names,
            signed_mode_default,
            public_url,
            database_url,
            master_key_file,
            data_dir,
            static_dir,
            providers,
            step_up_window,
            approval_ttl_default,
            run_log_cap,
            otel_endpoint,
            insecure_dev,
            sources: r.sources,
        })
    }

    #[must_use]
    pub fn provider_ids(&self) -> Vec<&'static str> {
        self.providers.iter().map(ProviderConfig::id).collect()
    }

    /// The origin browsers send in `Origin` (no trailing slash).
    #[must_use]
    pub fn origin(&self) -> String {
        self.public_url.origin().ascii_serialization()
    }
}

impl Resolver<'_> {
    /// Like [`Resolver::pick`], but records a source only when set.
    fn pick_quiet(
        &mut self,
        setting: &'static str,
        flag: Option<String>,
        env: &str,
    ) -> Option<String> {
        if let Some(v) = flag.filter(|v| !v.trim().is_empty()) {
            self.sources
                .push((setting, format!("--{}", setting.replace('_', "-"))));
            return Some(v);
        }
        let v = self.var(env)?;
        self.sources.push((setting, env.to_owned()));
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect();
        move |k| map.get(k).cloned()
    }

    const BASE: &[(&str, &str)] = &[
        ("KEEL_PUBLIC_URL", "https://keel.example.com"),
        ("KEEL_DATABASE_URL", "postgres://keel:hunter2@db/keel"),
        ("KEEL_MASTER_KEY_FILE", "/etc/keel/master.key"),
        ("KEEL_GITHUB_CLIENT_ID", "gh-id"),
        ("KEEL_GITHUB_CLIENT_SECRET", "gh-secret"),
    ];

    fn resolve(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let env = env_of(pairs);
        Config::resolve(Flags::default(), &env)
    }

    fn without(skip: &str) -> Vec<(&'static str, &'static str)> {
        BASE.iter().copied().filter(|(k, _)| *k != skip).collect()
    }

    #[test]
    fn defaults_per_the_contract() {
        let c = resolve(BASE).unwrap();
        assert_eq!(c.listen, "0.0.0.0:7830".parse().unwrap());
        assert_eq!(c.runner_listen, "0.0.0.0:7831".parse().unwrap());
        assert_eq!(c.runner_cert_ttl, Duration::from_secs(24 * 3600));
        assert_eq!(c.runner_max_offline, Duration::from_secs(30 * 86_400));
        assert!(!c.signed_mode_default);
        assert_eq!(c.data_dir, PathBuf::from("/var/lib/keel"));
        assert_eq!(c.static_dir, PathBuf::from("/usr/share/keel/web"));
        assert_eq!(c.step_up_window, Duration::from_secs(300));
        assert_eq!(c.approval_ttl_default, Duration::from_secs(24 * 3600));
        assert_eq!(c.run_log_cap, 64 << 20);
        assert_eq!(c.otel_endpoint, None);
        assert_eq!(c.provider_ids(), vec!["github"]);
        assert_eq!(c.origin(), "https://keel.example.com");
        assert_eq!(c.master_key_file, PathBuf::from("/etc/keel/master.key"));
    }

    #[test]
    fn each_required_key_refuses_by_name() {
        for (key, flag) in [
            ("KEEL_PUBLIC_URL", "--public-url"),
            ("KEEL_DATABASE_URL", "--database-url"),
            ("KEEL_MASTER_KEY_FILE", "--master-key-file"),
        ] {
            let err = resolve(&without(key)).unwrap_err().0;
            assert!(err.contains(flag) && err.contains(key), "{err}");
        }
    }

    #[test]
    fn refuses_with_no_provider() {
        let mut pairs = without("KEEL_GITHUB_CLIENT_ID");
        pairs.retain(|(k, _)| *k != "KEEL_GITHUB_CLIENT_SECRET");
        assert!(
            resolve(&pairs)
                .unwrap_err()
                .0
                .contains("no sign-in provider")
        );
    }

    #[test]
    fn a_provider_needs_its_secret() {
        let err = resolve(&without("KEEL_GITHUB_CLIENT_SECRET"))
            .unwrap_err()
            .0;
        assert!(err.contains("KEEL_GITHUB_CLIENT_SECRET"), "{err}");
        let mut pairs = BASE.to_vec();
        pairs.push(("KEEL_OIDC_ISSUER", "https://idp.example.com"));
        assert!(
            resolve(&pairs)
                .unwrap_err()
                .0
                .contains("KEEL_OIDC_CLIENT_ID")
        );
    }

    #[test]
    fn oidc_and_discord() {
        let mut pairs = BASE.to_vec();
        pairs.extend([
            ("KEEL_OIDC_ISSUER", "https://idp.example.com"),
            ("KEEL_OIDC_CLIENT_ID", "keel"),
            ("KEEL_OIDC_SCOPES", "openid,groups"),
            ("KEEL_DISCORD_CLIENT_ID", "d"),
            ("KEEL_DISCORD_CLIENT_SECRET", "ds"),
        ]);
        let c = resolve(&pairs).unwrap();
        assert_eq!(c.provider_ids(), vec!["discord", "github", "oidc"]);
        match &c.providers[2] {
            ProviderConfig::Oidc {
                scopes,
                client_secret,
                ..
            } => {
                assert_eq!(scopes, &["openid", "groups"]);
                assert!(client_secret.is_none());
            }
            p => panic!("{p:?}"),
        }
    }

    #[test]
    fn secrets_never_reach_debug_or_sources() {
        let c = resolve(BASE).unwrap();
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("gh-secret"), "{dbg}");
        assert!(!dbg.contains("hunter2"), "{dbg}");
        for (_, from) in &c.sources {
            assert!(!from.contains("hunter2") && !from.contains("gh-secret"));
        }
        assert_eq!(
            c.database_url.expose_secret(),
            "postgres://keel:hunter2@db/keel"
        );
    }

    #[test]
    fn flags_beat_env() {
        let env = env_of(BASE);
        let flags = Flags {
            listen: Some("127.0.0.1:9000".into()),
            step_up_window: Some("2m".into()),
            run_log_cap: Some("1MiB".into()),
            signed_mode_default: Some("true".into()),
            otel_endpoint: Some("http://otel:4318".into()),
            ..Flags::default()
        };
        let c = Config::resolve(flags, &env).unwrap();
        assert_eq!(c.listen, "127.0.0.1:9000".parse().unwrap());
        assert_eq!(c.step_up_window, Duration::from_secs(120));
        assert_eq!(c.run_log_cap, 1 << 20);
        assert!(c.signed_mode_default);
        assert_eq!(c.otel_endpoint.as_deref(), Some("http://otel:4318"));
        assert!(c.sources.contains(&("listen", "--listen".into())));
        assert!(
            c.sources
                .contains(&("public_url", "KEEL_PUBLIC_URL".into()))
        );
    }

    #[test]
    fn otel_endpoint_from_its_standard_env() {
        let mut pairs = BASE.to_vec();
        pairs.push(("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4318"));
        assert_eq!(
            resolve(&pairs).unwrap().otel_endpoint.as_deref(),
            Some("http://collector:4318")
        );
    }

    #[test]
    fn public_url_must_be_an_https_origin() {
        for bad in [
            "http://keel.example.com",
            "https://keel.example.com/app",
            "keel.example.com",
            "https://u:p@k.example",
        ] {
            let mut pairs = without("KEEL_PUBLIC_URL");
            pairs.push(("KEEL_PUBLIC_URL", bad));
            assert!(resolve(&pairs).is_err(), "{bad}");
        }
    }

    #[test]
    fn insecure_dev_allows_http_on_loopback_only() {
        let mut pairs = without("KEEL_PUBLIC_URL");
        pairs.extend([
            ("KEEL_PUBLIC_URL", "http://localhost:7830"),
            ("KEEL_INSECURE_DEV", "true"),
        ]);
        assert!(resolve(&pairs).unwrap_err().0.contains("loopback"));
        pairs.push(("KEEL_LISTEN", "127.0.0.1:7830"));
        assert!(resolve(&pairs).unwrap().insecure_dev);
    }

    #[test]
    fn invalid_values_refuse_naming_the_variable() {
        for (k, v) in [
            ("KEEL_LISTEN", "nope"),
            ("KEEL_RUNNER_CERT_TTL", "1y"),
            ("KEEL_STEP_UP_WINDOW", "0m"),
            ("KEEL_RUN_LOG_CAP", "lots"),
            ("KEEL_SIGNED_MODE_DEFAULT", "maybe"),
        ] {
            let mut pairs = BASE.to_vec();
            pairs.push((k, v));
            let err = resolve(&pairs).unwrap_err().0;
            assert!(err.contains(k), "{k}: {err}");
        }
    }

    #[test]
    fn subcommand_helpers() {
        let env = env_of(BASE);
        assert!(database_url(&Flags::default(), &env).is_ok());
        assert_eq!(
            master_key_file(&Flags::default(), &env).unwrap(),
            PathBuf::from("/etc/keel/master.key")
        );
        let empty = env_of(&[]);
        assert!(
            database_url(&Flags::default(), &empty)
                .unwrap_err()
                .0
                .contains("KEEL_DATABASE_URL")
        );
        assert!(
            master_key_file(&Flags::default(), &empty)
                .unwrap_err()
                .0
                .contains("KEEL_MASTER_KEY_FILE")
        );
    }

    #[test]
    fn durations_and_sizes() {
        assert_eq!(parse_duration("x", "90s").unwrap(), Duration::from_secs(90));
        assert_eq!(
            parse_duration("x", "30d").unwrap(),
            Duration::from_secs(30 * 86_400)
        );
        assert!(parse_duration("x", "5").is_err());
        assert_eq!(parse_bytes("x", "64MiB").unwrap(), 64 << 20);
        assert_eq!(parse_bytes("x", "10MB").unwrap(), 10_000_000);
        assert!(parse_bytes("x", "0").is_err());
    }

    #[test]
    fn cli_parses_subcommands() {
        let cli =
            Cli::try_parse_from(["keel", "audit", "verify", "--database-url", "postgres://x"])
                .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Audit {
                command: AuditCommand::Verify
            })
        ));
        assert_eq!(cli.flags.database_url.as_deref(), Some("postgres://x"));
        let cli = Cli::try_parse_from(["keel", "keygen", "--master-key-file", "/k"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Keygen)));
        let cli = Cli::try_parse_from(["keel"]).unwrap();
        assert!(cli.command.is_none());
    }
}
