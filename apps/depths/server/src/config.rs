//! T035: configuration (contracts/config.md).
//!
//! Every setting comes from a flag, a `DEPTHS_*` variable, an alias (`ORIGIN_URL`,
//! `DOCKGE_STACKS_DIR`, `OWNER_USER_ID`, `OAUTH_*`) or a default, in that order. Each resolved
//! setting remembers the name it came from so start-up can log it; secret values never reach
//! [`Config::sources`] or `Debug`.
//!
//! Anything wrong refuses start with a message naming the setting. There is no anonymous mode:
//! at least one sign-in provider must be configured.

use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Parser;
use secrecy::SecretString;
use url::Url;

use depths_core::compose_lint::ComposePolicy;
use depths_core::security::ShellPolicy;

use crate::client_ip::Cidr;
use depths_core::network::{Grouping, NetworkKey, TrustPolicy};

/// The command line. Every flag is optional here; defaults and env are applied in [`Config::resolve`].
#[derive(Debug, Default, Parser)]
#[command(
    name = "depths",
    version,
    about = "Depths: a Docker Compose stack manager"
)]
pub struct Flags {
    /// Bind address [env: DEPTHS_LISTEN] [default: 0.0.0.0:8080]
    #[arg(long)]
    pub listen: Option<String>,
    /// External URL [env: DEPTHS_PUBLIC_URL, alias ORIGIN_URL]
    #[arg(long)]
    pub public_url: Option<String>,
    /// Stacks root [env: DEPTHS_STACKS_DIR, alias DOCKGE_STACKS_DIR] [default: /opt/stacks]
    #[arg(long)]
    pub stacks_dir: Option<PathBuf>,
    /// Audit log and users.json [env: DEPTHS_DATA_DIR] [default: /var/lib/depths]
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    /// Built web assets [env: STATIC_DIR] [default: /usr/share/depths/web]
    #[arg(long)]
    pub static_dir: Option<PathBuf>,
    /// Comma-separated owners [env: DEPTHS_OWNER_USER_ID, alias OWNER_USER_ID]
    #[arg(long)]
    pub owner_user_id: Option<String>,
    /// Docker CLI to invoke [env: DEPTHS_DOCKER_BIN] [default: docker]
    #[arg(long)]
    pub docker_bin: Option<String>,
    /// Idle session lifetime [env: DEPTHS_SESSION_IDLE] [default: 8h]
    #[arg(long)]
    pub session_idle: Option<String>,
    /// Absolute session lifetime [env: DEPTHS_SESSION_MAX] [default: 24h]
    #[arg(long)]
    pub session_max: Option<String>,
    /// Largest compose or .env file loaded [env: DEPTHS_MAX_FILE_BYTES] [default: 1MiB]
    #[arg(long)]
    pub max_file_bytes: Option<String>,
    /// Audit log rotation size [env: DEPTHS_AUDIT_ROTATE_BYTES] [default: 64MiB]
    #[arg(long)]
    pub audit_rotate_bytes: Option<String>,
    /// Comma-separated CIDRs whose X-Forwarded-For / CF-Connecting-IP are believed
    /// [env: DEPTHS_TRUSTED_PROXIES]
    #[arg(long)]
    pub trusted_proxies: Option<String>,
    /// Restrict untrusted networks (spec 052) [env: DEPTHS_NETWORK_GATE] [default: true]
    #[arg(long)]
    pub network_gate: Option<String>,
    /// Days before a "home" network is trusted; 0 never [env: DEPTHS_NETWORK_MATURITY_DAYS]
    /// [default: 7]
    #[arg(long)]
    pub network_maturity_days: Option<String>,
    /// The host's /proc, mounted read-only, for admin-only host metrics; unset turns them off
    /// [env: DEPTHS_HOST_PROC]
    #[arg(long)]
    pub host_proc: Option<PathBuf>,
    /// Allow an http:// public URL; loopback listen only [env: DEPTHS_INSECURE_DEV]
    #[arg(long)]
    pub insecure_dev: bool,
    /// Who may open a shell by a container's security level: strict or relaxed
    /// [env: DEPTHS_SHELL_POLICY] [default: strict]
    #[arg(long)]
    pub shell_policy: Option<String>,
    /// Whether escape-level compose findings block saves and deploys: warn or enforce
    /// [env: DEPTHS_COMPOSE_POLICY] [default: warn]
    #[arg(long)]
    pub compose_policy: Option<String>,
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

/// One configured owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// `discord:<snowflake>`, `github:<numeric id>` or `oidc:<sub>`: a registry key.
    Id(String),
    /// `github:<login>`, pinned to the numeric id at first sign-in.
    GithubLogin(String),
}

impl Owner {
    pub fn provider(&self) -> &str {
        match self {
            Self::Id(id) => id.split_once(':').map_or("", |(p, _)| p),
            Self::GithubLogin(_) => "github",
        }
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => f.write_str(id),
            Self::GithubLogin(l) => write!(f, "github:{l}"),
        }
    }
}

/// Runtime feature flags (R12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Features {
    pub shell: bool,
    pub edit: bool,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub public_url: Url,
    /// `scheme://host[:port]` of `public_url`: the only accepted `Origin`.
    pub origin: String,
    pub stacks_dir: PathBuf,
    pub data_dir: PathBuf,
    pub static_dir: PathBuf,
    pub docker_bin: String,
    pub owners: Vec<Owner>,
    pub providers: Vec<ProviderConfig>,
    pub session_idle: Duration,
    pub session_max: Duration,
    pub max_file_bytes: u64,
    pub audit_rotate_bytes: u64,
    pub features: Features,
    /// The host's `/proc` (`DEPTHS_HOST_PROC`). `None` turns host metrics off.
    pub host_proc: Option<PathBuf>,
    pub trusted_proxies: Vec<Cidr>,
    /// Network trust (spec 052).
    pub network: NetworkSettings,
    pub insecure_dev: bool,
    pub otlp_endpoint: Option<String>,
    /// 051 FR-007: `DEPTHS_SHELL_POLICY`.
    pub shell_policy: ShellPolicy,
    /// 051 FR-017: `DEPTHS_COMPOSE_POLICY`.
    pub compose_policy: ComposePolicy,
    /// `(setting, where it came from)`, for the start-up log. Never a secret value.
    pub sources: Vec<(&'static str, String)>,
}

/// Network trust settings (spec 052 contracts/config.md).
#[derive(Debug, Clone, Default)]
pub struct NetworkSettings {
    /// The gate, maturity window, grouping and `DEPTHS_TRUSTED_NETWORKS`.
    pub policy: TrustPolicy,
    /// `DEPTHS_GEOIP_DB`: a City `.mmdb`, display only. A missing file is logged, not refused.
    pub geoip_db: Option<PathBuf>,
}

/// Looks an environment variable up. Empty counts as unset.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The process environment, as [`Config::resolve`] reads it.
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

    /// flag > each env name in order > default.
    fn pick(
        &mut self,
        setting: &'static str,
        flag: Option<String>,
        names: &[&str],
    ) -> Option<String> {
        if let Some(v) = flag {
            self.sources
                .push((setting, format!("--{}", setting.replace('_', "-"))));
            return Some(v);
        }
        for name in names {
            if let Some(v) = self.var(name) {
                self.sources.push((setting, (*name).to_owned()));
                return Some(v);
            }
        }
        self.sources.push((setting, "default".to_owned()));
        None
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

/// The 052 settings. Invalid values refuse start, naming the variable.
fn network_settings(
    r: &mut Resolver<'_>,
    (gate_flag, days_flag): &(Option<String>, Option<String>),
) -> Result<NetworkSettings, ConfigError> {
    let gate = match r.pick("network_gate", gate_flag.clone(), &["DEPTHS_NETWORK_GATE"]) {
        Some(v) => parse_bool("DEPTHS_NETWORK_GATE", &v)?,
        None => true,
    };
    let maturity_days = match r.pick(
        "network_maturity_days",
        days_flag.clone(),
        &["DEPTHS_NETWORK_MATURITY_DAYS"],
    ) {
        Some(v) => match v.trim().parse::<u32>() {
            Ok(d) if d <= 365 => d,
            _ => {
                return refuse(format!(
                    "DEPTHS_NETWORK_MATURITY_DAYS: `{v}` is not a number of days from 0 to 365"
                ));
            }
        },
        None => 7,
    };
    let prefix = |r: &mut Resolver<'_>, setting, name: &str, max: u8, default: u8| match r.pick(
        setting,
        None,
        &[name],
    ) {
        Some(v) => match v.trim().parse::<u8>() {
            Ok(p) if (1..=max).contains(&p) => Ok(p),
            _ => refuse(format!(
                "{name}: `{v}` is not a prefix length from 1 to {max}"
            )),
        },
        None => Ok(default),
    };
    let v4_prefix = prefix(r, "network_v4_prefix", "DEPTHS_NETWORK_V4_PREFIX", 32, 32)?;
    let v6_prefix = prefix(r, "network_v6_prefix", "DEPTHS_NETWORK_V6_PREFIX", 128, 64)?;
    let always = match r.pick("trusted_networks", None, &["DEPTHS_TRUSTED_NETWORKS"]) {
        Some(raw) => raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                if !s.contains('/') {
                    return refuse(format!(
                        "DEPTHS_TRUSTED_NETWORKS: `{s}` is not a CIDR (like 100.64.0.0/10)"
                    ));
                }
                NetworkKey::parse(s).ok_or_else(|| {
                    ConfigError(format!(
                        "DEPTHS_TRUSTED_NETWORKS: `{s}` is not a CIDR (like 100.64.0.0/10)"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    let geoip_db = r
        .pick("geoip_db", None, &["DEPTHS_GEOIP_DB"])
        .map(PathBuf::from);
    Ok(NetworkSettings {
        policy: TrustPolicy {
            gate,
            maturity_days,
            grouping: Grouping {
                v4_prefix,
                v6_prefix,
            },
            always,
        },
        geoip_db,
    })
}

fn parse_bool(name: &str, v: &str) -> Result<bool, ConfigError> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => refuse(format!("{name}: `{v}` is not a boolean (true or false)")),
    }
}

/// `90s`, `30m`, `8h`, `1d`.
pub fn parse_duration(name: &str, v: &str) -> Result<Duration, ConfigError> {
    let v = v.trim();
    let split = v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len());
    let (num, unit) = v.split_at(split);
    let n: u64 = num
        .parse()
        .or_else(|_| refuse(format!("{name}: `{v}` is not a duration (like 30m or 8h)")))?;
    let mult = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return refuse(format!("{name}: `{v}` is not a duration (like 30m or 8h)")),
    };
    if n == 0 {
        return refuse(format!("{name}: must be more than zero"));
    }
    Ok(Duration::from_secs(n.saturating_mul(mult)))
}

/// `1048576`, `512KiB`, `1MiB`, `64MiB`, `1GiB`, `10MB`.
pub fn parse_bytes(name: &str, v: &str) -> Result<u64, ConfigError> {
    let v = v.trim();
    let split = v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len());
    let (num, unit) = v.split_at(split);
    let n: u64 = num
        .parse()
        .or_else(|_| refuse(format!("{name}: `{v}` is not a size (like 1MiB)")))?;
    let mult: u64 = match unit.to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "kib" => 1 << 10,
        "mib" => 1 << 20,
        "gib" => 1 << 30,
        "kb" => 1_000,
        "mb" => 1_000_000,
        "gb" => 1_000_000_000,
        _ => return refuse(format!("{name}: `{v}` is not a size (like 1MiB)")),
    };
    if n == 0 {
        return refuse(format!("{name}: must be more than zero"));
    }
    Ok(n.saturating_mul(mult))
}

/// `ORIGIN_URL` with no scheme is read as `https://`. Only an origin (an optional `/` path) is
/// accepted: redirect URIs and the Origin check are built from it.
fn parse_public_url(source: &str, raw: &str, insecure_dev: bool) -> Result<Url, ConfigError> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&with_scheme)
        .or_else(|e| refuse(format!("{source}: `{raw}` is not a URL: {e}")))?;
    match url.scheme() {
        "https" => {}
        "http" if insecure_dev => {}
        "http" => {
            return refuse(format!(
                "{source}: must be https:// (http:// only with --insecure-dev on a loopback listen)"
            ));
        }
        other => return refuse(format!("{source}: scheme `{other}` is not https")),
    }
    if url.host_str().is_none_or(str::is_empty) {
        return refuse(format!("{source}: `{raw}` has no host"));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return refuse(format!(
            "{source}: must be an origin only (like https://depths.example.com), without a path"
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return refuse(format!("{source}: must not carry credentials"));
    }
    Ok(url)
}

fn parse_owners(
    source: &str,
    raw: &str,
    oauth_provider: Option<&str>,
) -> Result<Vec<Owner>, ConfigError> {
    let mut out = Vec::new();
    for item in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let qualified = if item.contains(':') {
            item.to_owned()
        } else {
            let Some(p) = oauth_provider else {
                return refuse(format!(
                    "{source}: `{item}` has no provider; write `discord:{item}` (or set OAUTH_PROVIDER)"
                ));
            };
            format!("{p}:{item}")
        };
        let owner = match depths_protocol::UserId::new(qualified.clone()) {
            Ok(id) => Owner::Id(id.as_str().to_owned()),
            Err(_) => match qualified.split_once(':') {
                Some(("github", login)) if valid_github_login(login) => {
                    Owner::GithubLogin(login.to_ascii_lowercase())
                }
                _ => {
                    return refuse(format!(
                        "{source}: `{item}` is not discord:<id>, github:<login|id> or oidc:<sub>"
                    ));
                }
            },
        };
        if !out.contains(&owner) {
            out.push(owner);
        }
    }
    Ok(out)
}

/// GitHub's own rule: alphanumerics and single inner hyphens, at most 39.
fn valid_github_login(l: &str) -> bool {
    !l.is_empty()
        && l.len() <= 39
        && !l.starts_with('-')
        && !l.ends_with('-')
        && !l.contains("--")
        && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn is_loopback(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback()
}

impl Config {
    /// Parses the process's flags and environment.
    pub fn from_process() -> Result<Self, ConfigError> {
        Self::resolve(Flags::parse(), &process_env)
    }

    /// Applies the precedence rules and every refusal (contracts/config.md) except the ones that
    /// need `users.json` (checked by [`crate::users::Registry::open`]).
    #[allow(clippy::too_many_lines)]
    pub fn resolve(flags: Flags, env: Env<'_>) -> Result<Self, ConfigError> {
        let mut r = Resolver {
            env,
            sources: Vec::new(),
        };

        let insecure_dev = if flags.insecure_dev {
            r.sources.push(("insecure_dev", "--insecure-dev".into()));
            true
        } else if let Some(v) = r.var("DEPTHS_INSECURE_DEV") {
            r.sources
                .push(("insecure_dev", "DEPTHS_INSECURE_DEV".into()));
            parse_bool("DEPTHS_INSECURE_DEV", &v)?
        } else {
            false
        };

        let listen_raw = r
            .pick("listen", flags.listen, &["DEPTHS_LISTEN"])
            .unwrap_or_else(|| "0.0.0.0:8080".into());
        let listen: SocketAddr = listen_raw.parse().or_else(|_| {
            refuse(format!(
                "DEPTHS_LISTEN: `{listen_raw}` is not an address:port (like 0.0.0.0:8080)"
            ))
        })?;
        if insecure_dev && !is_loopback(&listen) {
            return refuse(format!(
                "DEPTHS_INSECURE_DEV is refused on a non-loopback listen address ({listen})"
            ));
        }

        let public_raw = r
            .pick(
                "public_url",
                flags.public_url,
                &["DEPTHS_PUBLIC_URL", "ORIGIN_URL"],
            )
            .ok_or_else(|| {
                ConfigError(
                    "DEPTHS_PUBLIC_URL (or ORIGIN_URL) is required: the external URL people open"
                        .into(),
                )
            })?;
        let public_source = r.sources.last().map(|(_, s)| s.clone()).unwrap_or_default();
        let public_url = parse_public_url(&public_source, &public_raw, insecure_dev)?;
        let origin = public_url.origin().ascii_serialization();

        let stacks_dir = r
            .pick(
                "stacks_dir",
                flags.stacks_dir.map(path_string),
                &["DEPTHS_STACKS_DIR", "DOCKGE_STACKS_DIR"],
            )
            .map_or_else(|| PathBuf::from("/opt/stacks"), PathBuf::from);
        if !stacks_dir.is_absolute() {
            return refuse(format!(
                "DEPTHS_STACKS_DIR: `{}` must be an absolute path",
                stacks_dir.display()
            ));
        }
        if !stacks_dir.is_dir() {
            return refuse(format!(
                "DEPTHS_STACKS_DIR: `{}` does not exist or is not a directory",
                stacks_dir.display()
            ));
        }

        let data_dir = r
            .pick(
                "data_dir",
                flags.data_dir.map(path_string),
                &["DEPTHS_DATA_DIR"],
            )
            .map_or_else(|| PathBuf::from("/var/lib/depths"), PathBuf::from);
        if !data_dir.is_absolute() {
            return refuse(format!(
                "DEPTHS_DATA_DIR: `{}` must be an absolute path",
                data_dir.display()
            ));
        }

        let static_dir = r
            .pick(
                "static_dir",
                flags.static_dir.map(path_string),
                &["STATIC_DIR"],
            )
            .map_or_else(|| PathBuf::from("/usr/share/depths/web"), PathBuf::from);

        let docker_bin = r
            .pick("docker_bin", flags.docker_bin, &["DEPTHS_DOCKER_BIN"])
            .unwrap_or_else(|| "docker".into());

        // Providers. OAUTH_* is a generic alias for one provider's credentials.
        let oauth_provider = match r.var("OAUTH_PROVIDER") {
            Some(p) => {
                let p = p.trim().to_ascii_lowercase();
                if !matches!(p.as_str(), "discord" | "github" | "oidc") {
                    return refuse(format!(
                        "OAUTH_PROVIDER: `{p}` is not one of discord, github, oidc"
                    ));
                }
                r.sources.push(("oauth_provider", "OAUTH_PROVIDER".into()));
                Some(p)
            }
            None => None,
        };
        let generic_id = r.var("OAUTH_CLIENT_ID");
        let generic_secret = r.secret("oauth_client_secret", "OAUTH_CLIENT_SECRET")?;
        if oauth_provider.is_none() && (generic_id.is_some() || generic_secret.is_some()) {
            return refuse("OAUTH_CLIENT_ID/OAUTH_CLIENT_SECRET are set without OAUTH_PROVIDER");
        }

        let mut providers = Vec::new();
        for p in ["discord", "github", "oidc"] {
            let upper = p.to_ascii_uppercase();
            let id_var = format!("DEPTHS_{upper}_CLIENT_ID");
            let secret_var = format!("DEPTHS_{upper}_CLIENT_SECRET");
            let generic = oauth_provider.as_deref() == Some(p);
            let client_id = match r.var(&id_var) {
                Some(v) => {
                    r.sources
                        .push((leak_setting(p, "client_id"), id_var.clone()));
                    Some(v)
                }
                None if generic && generic_id.is_some() => {
                    r.sources
                        .push((leak_setting(p, "client_id"), "OAUTH_CLIENT_ID".into()));
                    generic_id.clone()
                }
                None => None,
            };
            let secret = match r.secret(leak_setting(p, "client_secret"), &secret_var)? {
                Some(s) => Some(s),
                None if generic => generic_secret.clone(),
                None => None,
            };
            let issuer = (p == "oidc").then(|| r.var("DEPTHS_OIDC_ISSUER")).flatten();
            if client_id.is_none() && secret.is_none() && issuer.is_none() {
                if generic {
                    return refuse(format!(
                        "OAUTH_PROVIDER={p} but no client id is set (OAUTH_CLIENT_ID or {id_var})"
                    ));
                }
                continue;
            }
            let Some(client_id) = client_id else {
                return refuse(format!("{id_var} (or OAUTH_CLIENT_ID) is required for {p}"));
            };
            providers.push(match p {
                "discord" | "github" => {
                    let Some(client_secret) = secret else {
                        return refuse(format!(
                            "{secret_var} (or OAUTH_CLIENT_SECRET, or a _FILE) is required for {p}"
                        ));
                    };
                    if p == "discord" {
                        ProviderConfig::Discord {
                            client_id,
                            client_secret,
                        }
                    } else {
                        ProviderConfig::Github {
                            client_id,
                            client_secret,
                        }
                    }
                }
                _ => {
                    let Some(issuer) = issuer else {
                        return refuse("DEPTHS_OIDC_ISSUER is required for oidc");
                    };
                    let scopes = r
                        .var("DEPTHS_OIDC_SCOPES")
                        .unwrap_or_else(|| "openid profile email".into())
                        .split(|c: char| c == ',' || c.is_whitespace())
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect();
                    ProviderConfig::Oidc {
                        issuer,
                        client_id,
                        client_secret: secret,
                        scopes,
                    }
                }
            });
        }
        if providers.is_empty() {
            return refuse(
                "no sign-in provider is configured: set OAUTH_PROVIDER with OAUTH_CLIENT_ID and \
                 OAUTH_CLIENT_SECRET, or DEPTHS_DISCORD_*/DEPTHS_GITHUB_*/DEPTHS_OIDC_*",
            );
        }

        let owners = match r.pick(
            "owner_user_id",
            flags.owner_user_id,
            &["DEPTHS_OWNER_USER_ID", "OWNER_USER_ID"],
        ) {
            Some(raw) => {
                let src = r.sources.last().map(|(_, s)| s.clone()).unwrap_or_default();
                parse_owners(&src, &raw, oauth_provider.as_deref())?
            }
            None => Vec::new(),
        };
        for o in &owners {
            if !providers.iter().any(|p| p.id() == o.provider()) {
                return refuse(format!(
                    "OWNER_USER_ID: owner `{o}` names provider `{}`, which is not configured",
                    o.provider()
                ));
            }
        }

        let session_idle =
            match r.pick("session_idle", flags.session_idle, &["DEPTHS_SESSION_IDLE"]) {
                Some(v) => parse_duration("DEPTHS_SESSION_IDLE", &v)?,
                None => Duration::from_secs(8 * 3600),
            };
        let session_max = match r.pick("session_max", flags.session_max, &["DEPTHS_SESSION_MAX"]) {
            Some(v) => parse_duration("DEPTHS_SESSION_MAX", &v)?,
            None => Duration::from_secs(24 * 3600),
        };
        if session_idle > session_max {
            return refuse("DEPTHS_SESSION_IDLE must not be longer than DEPTHS_SESSION_MAX");
        }
        let max_file_bytes = match r.pick(
            "max_file_bytes",
            flags.max_file_bytes,
            &["DEPTHS_MAX_FILE_BYTES"],
        ) {
            Some(v) => parse_bytes("DEPTHS_MAX_FILE_BYTES", &v)?,
            None => 1 << 20,
        };
        let audit_rotate_bytes = match r.pick(
            "audit_rotate_bytes",
            flags.audit_rotate_bytes,
            &["DEPTHS_AUDIT_ROTATE_BYTES"],
        ) {
            Some(v) => parse_bytes("DEPTHS_AUDIT_ROTATE_BYTES", &v)?,
            None => 64 << 20,
        };

        let mut feature = |setting: &'static str, name: &str, default: bool| match r.var(name) {
            Some(v) => {
                r.sources.push((setting, name.to_owned()));
                parse_bool(name, &v)
            }
            None => Ok::<_, ConfigError>(default),
        };
        // A shell into a container is the most a session can do, so it is off unless asked for.
        let features = Features {
            shell: feature("feature_shell", "DEPTHS_FEATURE_SHELL", false)?,
            edit: feature("feature_edit", "DEPTHS_FEATURE_EDIT", true)?,
        };

        // Off unless asked for: the host's /proc also shows every process and its command line.
        let host_proc = r
            .pick(
                "host_proc",
                flags.host_proc.map(path_string),
                &["DEPTHS_HOST_PROC"],
            )
            .map(PathBuf::from);
        if let Some(p) = &host_proc {
            if !p.is_absolute() {
                return refuse(format!(
                    "DEPTHS_HOST_PROC: `{}` must be an absolute path",
                    p.display()
                ));
            }
            if !p.is_dir() {
                return refuse(format!(
                    "DEPTHS_HOST_PROC: `{}` does not exist or is not a directory (mount /proc:/host/proc:ro)",
                    p.display()
                ));
            }
            if let Err(e) = std::fs::File::open(p.join("stat")) {
                return refuse(format!(
                    "DEPTHS_HOST_PROC: cannot read `{}`: {e}",
                    p.join("stat").display()
                ));
            }
        }

        let flags_network = (flags.network_gate, flags.network_maturity_days);
        let trusted_proxies = match r.pick(
            "trusted_proxies",
            flags.trusted_proxies,
            &["DEPTHS_TRUSTED_PROXIES"],
        ) {
            Some(raw) => raw
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    Cidr::parse(s).ok_or_else(|| {
                        ConfigError(format!(
                            "DEPTHS_TRUSTED_PROXIES: `{s}` is not a CIDR (like 172.18.0.0/16)"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        };

        let network = network_settings(&mut r, &flags_network)?;

        let otlp_endpoint = r.var("OTEL_EXPORTER_OTLP_ENDPOINT");

        let shell_policy =
            match r.pick("shell_policy", flags.shell_policy, &["DEPTHS_SHELL_POLICY"]) {
                Some(v) => ShellPolicy::parse(&v).ok_or_else(|| {
                    ConfigError(format!(
                        "DEPTHS_SHELL_POLICY: `{v}` is not a shell policy (strict or relaxed)"
                    ))
                })?,
                None => ShellPolicy::default(),
            };
        let compose_policy = match r.pick(
            "compose_policy",
            flags.compose_policy,
            &["DEPTHS_COMPOSE_POLICY"],
        ) {
            Some(v) => ComposePolicy::parse(&v).ok_or_else(|| {
                ConfigError(format!(
                    "DEPTHS_COMPOSE_POLICY: `{v}` is not a compose policy (warn or enforce)"
                ))
            })?,
            None => ComposePolicy::default(),
        };

        Ok(Self {
            listen,
            public_url,
            origin,
            stacks_dir,
            data_dir,
            static_dir,
            docker_bin,
            owners,
            providers,
            session_idle,
            session_max,
            max_file_bytes,
            audit_rotate_bytes,
            features,
            host_proc,
            trusted_proxies,
            network,
            insecure_dev,
            otlp_endpoint,
            shell_policy,
            compose_policy,
            sources: r.sources,
        })
    }

    /// Where `users.json` lives.
    pub fn users_path(&self) -> PathBuf {
        self.data_dir.join("users.json")
    }

    pub fn provider_ids(&self) -> Vec<&'static str> {
        self.providers.iter().map(ProviderConfig::id).collect()
    }

    /// Creates the data directory (0700) if it is missing; refuses when it cannot be written.
    pub fn prepare_data_dir(&self) -> Result<(), ConfigError> {
        ensure_private_dir(&self.data_dir)
            .or_else(|e| refuse(format!("DEPTHS_DATA_DIR: {}: {e}", self.data_dir.display())))
    }
}

fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let meta = std::fs::metadata(dir)?;
    if !meta.is_dir() {
        return Err(std::io::Error::other("not a directory"));
    }
    // Tighten it if we own it; a mounted volume we do not own is left to its owner.
    if meta.permissions().mode() & 0o077 != 0 {
        drop(std::fs::set_permissions(
            dir,
            std::fs::Permissions::from_mode(0o700),
        ));
    }
    // Probe writability now rather than at the first sign-in.
    let probe = dir.join(".depths-write-probe");
    std::fs::write(&probe, b"")?;
    std::fs::remove_file(&probe)
}

fn path_string(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

/// Static names for the per-provider source lines.
fn leak_setting(provider: &str, what: &str) -> &'static str {
    match (provider, what) {
        ("discord", "client_id") => "discord_client_id",
        ("discord", _) => "discord_client_secret",
        ("github", "client_id") => "github_client_id",
        ("github", _) => "github_client_secret",
        (_, "client_id") => "oidc_client_id",
        _ => "oidc_client_secret",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret as _;
    use std::collections::HashMap;

    struct Fixture {
        dir: tempfile::TempDir,
        env: HashMap<String, String>,
    }

    impl Fixture {
        /// The deploy's env: Discord via OAUTH_*, stacks via DOCKGE_STACKS_DIR.
        fn deploy() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let stacks = dir.path().join("stacks");
            std::fs::create_dir(&stacks).unwrap();
            let mut env = HashMap::new();
            for (k, v) in [
                ("DEPTHS_LISTEN", "0.0.0.0:8080"),
                ("ORIGIN_URL", "https://depth.boop.ninja"),
                ("OAUTH_PROVIDER", "discord"),
                ("OAUTH_CLIENT_ID", "1234"),
                ("OAUTH_CLIENT_SECRET", "s3cret-value"),
                ("OWNER_USER_ID", "140642000969007104"),
            ] {
                env.insert(k.to_owned(), v.to_owned());
            }
            env.insert(
                "DOCKGE_STACKS_DIR".into(),
                stacks.to_string_lossy().into_owned(),
            );
            env.insert(
                "DEPTHS_DATA_DIR".into(),
                dir.path().join("data").to_string_lossy().into_owned(),
            );
            Self { dir, env }
        }

        fn set(mut self, k: &str, v: &str) -> Self {
            self.env.insert(k.into(), v.into());
            self
        }

        fn unset(mut self, k: &str) -> Self {
            self.env.remove(k);
            self
        }

        fn resolve_with(&self, flags: Flags) -> Result<Config, ConfigError> {
            let env = self.env.clone();
            Config::resolve(flags, &move |k| env.get(k).cloned())
        }

        fn resolve(&self) -> Result<Config, ConfigError> {
            self.resolve_with(Flags::default())
        }

        fn refusal(&self) -> String {
            self.resolve().expect_err("must refuse").0
        }
    }

    #[test]
    fn the_deploy_env_loads() {
        let f = Fixture::deploy();
        let c = f.resolve().unwrap();
        assert_eq!(c.listen, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(c.origin, "https://depth.boop.ninja");
        assert_eq!(c.stacks_dir, f.dir.path().join("stacks"));
        assert_eq!(c.provider_ids(), vec!["discord"]);
        assert_eq!(
            c.owners,
            vec![Owner::Id("discord:140642000969007104".into())]
        );
        assert_eq!(c.static_dir, PathBuf::from("/usr/share/depths/web"));
        assert_eq!(c.session_idle, Duration::from_secs(8 * 3600));
        assert_eq!(c.session_max, Duration::from_secs(24 * 3600));
        assert_eq!(
            c.features,
            Features {
                shell: false,
                edit: true
            },
            "the shell is off unless DEPTHS_FEATURE_SHELL turns it on"
        );
        match &c.providers[0] {
            ProviderConfig::Discord {
                client_id,
                client_secret,
            } => {
                assert_eq!(client_id, "1234");
                assert_eq!(client_secret.expose_secret(), "s3cret-value");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn secrets_never_reach_debug_or_sources() {
        let c = Fixture::deploy().resolve().unwrap();
        let shown = format!("{c:?}");
        assert!(!shown.contains("s3cret-value"), "{shown}");
        assert!(
            c.sources.iter().all(|(_, s)| !s.contains("s3cret")),
            "{:?}",
            c.sources
        );
    }

    #[test]
    fn sources_name_the_alias_used() {
        let c = Fixture::deploy().resolve().unwrap();
        let src = |k: &str| {
            c.sources
                .iter()
                .find(|(s, _)| *s == k)
                .map(|(_, v)| v.as_str())
                .unwrap()
        };
        assert_eq!(src("public_url"), "ORIGIN_URL");
        assert_eq!(src("stacks_dir"), "DOCKGE_STACKS_DIR");
        assert_eq!(src("owner_user_id"), "OWNER_USER_ID");
        assert_eq!(src("discord_client_id"), "OAUTH_CLIENT_ID");
        assert_eq!(src("static_dir"), "default");
    }

    #[test]
    fn depths_names_beat_aliases_and_flags_beat_both() {
        let f = Fixture::deploy().set("DEPTHS_PUBLIC_URL", "https://a.example");
        assert_eq!(f.resolve().unwrap().origin, "https://a.example");
        let flags = Flags {
            public_url: Some("https://b.example".into()),
            ..Flags::default()
        };
        assert_eq!(f.resolve_with(flags).unwrap().origin, "https://b.example");

        let other = f.dir.path().join("other");
        std::fs::create_dir(&other).unwrap();
        let f = f.set("DEPTHS_STACKS_DIR", other.to_str().unwrap());
        assert_eq!(f.resolve().unwrap().stacks_dir, other);
    }

    #[test]
    fn a_scheme_less_origin_url_is_https() {
        let c = Fixture::deploy()
            .set("ORIGIN_URL", "depth.boop.ninja")
            .resolve()
            .unwrap();
        assert_eq!(c.public_url.as_str(), "https://depth.boop.ninja/");
    }

    #[test]
    fn refuses_without_a_public_url() {
        let r = Fixture::deploy().unset("ORIGIN_URL").refusal();
        assert!(r.contains("DEPTHS_PUBLIC_URL"), "{r}");
    }

    #[test]
    fn refuses_http_public_url_without_insecure_dev() {
        let r = Fixture::deploy()
            .set("ORIGIN_URL", "http://x.example")
            .refusal();
        assert!(r.contains("https"), "{r}");
    }

    #[test]
    fn refuses_a_public_url_with_a_path() {
        let r = Fixture::deploy()
            .set("ORIGIN_URL", "https://x.example/depths")
            .refusal();
        assert!(r.contains("origin only"), "{r}");
    }

    #[test]
    fn insecure_dev_needs_a_loopback_listen() {
        let f = Fixture::deploy()
            .set("DEPTHS_INSECURE_DEV", "true")
            .set("ORIGIN_URL", "http://localhost:8080");
        let r = f.refusal();
        assert!(r.contains("non-loopback"), "{r}");
        let c = f.set("DEPTHS_LISTEN", "127.0.0.1:8080").resolve().unwrap();
        assert!(c.insecure_dev);
        assert_eq!(c.origin, "http://localhost:8080");
    }

    #[test]
    fn refuses_a_missing_stacks_dir() {
        let r = Fixture::deploy()
            .set("DOCKGE_STACKS_DIR", "/nonexistent/depths-test-stacks")
            .refusal();
        assert!(r.contains("DEPTHS_STACKS_DIR"), "{r}");
    }

    #[test]
    fn refuses_with_no_provider() {
        let r = Fixture::deploy()
            .unset("OAUTH_PROVIDER")
            .unset("OAUTH_CLIENT_ID")
            .unset("OAUTH_CLIENT_SECRET")
            .unset("OWNER_USER_ID")
            .refusal();
        assert!(r.contains("no sign-in provider"), "{r}");
    }

    #[test]
    fn refuses_oauth_client_without_oauth_provider() {
        let r = Fixture::deploy().unset("OAUTH_PROVIDER").refusal();
        assert!(r.contains("without OAUTH_PROVIDER"), "{r}");
    }

    #[test]
    fn refuses_an_unknown_oauth_provider() {
        let r = Fixture::deploy().set("OAUTH_PROVIDER", "myspace").refusal();
        assert!(r.contains("OAUTH_PROVIDER"), "{r}");
    }

    #[test]
    fn refuses_a_bare_owner_without_oauth_provider() {
        let r = Fixture::deploy()
            .unset("OAUTH_PROVIDER")
            .unset("OAUTH_CLIENT_ID")
            .unset("OAUTH_CLIENT_SECRET")
            .set("DEPTHS_DISCORD_CLIENT_ID", "1")
            .set("DEPTHS_DISCORD_CLIENT_SECRET", "x")
            .refusal();
        assert!(r.contains("has no provider"), "{r}");
    }

    #[test]
    fn refuses_an_owner_of_an_unconfigured_provider() {
        let r = Fixture::deploy()
            .set("OWNER_USER_ID", "github:mbround18")
            .refusal();
        assert!(r.contains("not configured"), "{r}");
    }

    #[test]
    fn refuses_a_malformed_owner() {
        let r = Fixture::deploy()
            .set("OWNER_USER_ID", "discord:not-a-number")
            .refusal();
        assert!(r.contains("OWNER_USER_ID"), "{r}");
    }

    #[test]
    fn owners_parse_qualified_lists_and_github_logins() {
        let c = Fixture::deploy()
            .set("DEPTHS_GITHUB_CLIENT_ID", "g")
            .set("DEPTHS_GITHUB_CLIENT_SECRET", "gs")
            .set(
                "OWNER_USER_ID",
                "discord:1, github:MBRound18 ,github:42,discord:1",
            )
            .resolve()
            .unwrap();
        assert_eq!(
            c.owners,
            vec![
                Owner::Id("discord:1".into()),
                Owner::GithubLogin("mbround18".into()),
                Owner::Id("github:42".into()),
            ]
        );
        assert_eq!(c.provider_ids(), vec!["discord", "github"]);
    }

    #[test]
    fn a_secret_file_is_read_and_both_forms_refuse() {
        let f = Fixture::deploy().unset("OAUTH_CLIENT_SECRET");
        let path = f.dir.path().join("secret");
        std::fs::write(&path, "from-file\n").unwrap();
        let f = f.set("OAUTH_CLIENT_SECRET_FILE", path.to_str().unwrap());
        let c = f.resolve().unwrap();
        let ProviderConfig::Discord { client_secret, .. } = &c.providers[0] else {
            panic!()
        };
        assert_eq!(client_secret.expose_secret(), "from-file");
        let r = f.set("OAUTH_CLIENT_SECRET", "x").refusal();
        assert!(r.contains("both set"), "{r}");
    }

    #[test]
    fn refuses_discord_without_a_secret() {
        let r = Fixture::deploy().unset("OAUTH_CLIENT_SECRET").refusal();
        assert!(r.contains("OAUTH_CLIENT_SECRET"), "{r}");
    }

    #[test]
    fn network_settings_default_and_parse() {
        let c = Fixture::deploy().resolve().unwrap();
        assert!(c.network.policy.gate);
        assert_eq!(c.network.policy.maturity_days, 7);
        assert_eq!(c.network.policy.grouping.v4_prefix, 32);
        assert_eq!(c.network.policy.grouping.v6_prefix, 64);
        assert!(c.network.policy.always.is_empty());
        assert!(c.network.geoip_db.is_none());
        let c = Fixture::deploy()
            .set("DEPTHS_NETWORK_GATE", "false")
            .set("DEPTHS_NETWORK_MATURITY_DAYS", "0")
            .set("DEPTHS_NETWORK_V4_PREFIX", "24")
            .set("DEPTHS_NETWORK_V6_PREFIX", "56")
            .set(
                "DEPTHS_TRUSTED_NETWORKS",
                "192.168.1.0/24, 100.64.0.0/10,fd00::/8",
            )
            .set("DEPTHS_GEOIP_DB", "/data/city.mmdb")
            .resolve()
            .unwrap();
        assert!(!c.network.policy.gate);
        assert_eq!(c.network.policy.maturity_days, 0);
        assert_eq!(c.network.policy.grouping.v4_prefix, 24);
        assert_eq!(c.network.policy.grouping.v6_prefix, 56);
        assert_eq!(c.network.policy.always.len(), 3);
        assert_eq!(
            c.network.geoip_db.as_deref(),
            Some(Path::new("/data/city.mmdb"))
        );
    }

    #[test]
    fn invalid_network_settings_refuse_naming_the_variable() {
        for (name, value) in [
            ("DEPTHS_NETWORK_GATE", "sometimes"),
            ("DEPTHS_NETWORK_MATURITY_DAYS", "-1"),
            ("DEPTHS_NETWORK_MATURITY_DAYS", "week"),
            ("DEPTHS_NETWORK_V4_PREFIX", "33"),
            ("DEPTHS_NETWORK_V4_PREFIX", "0"),
            ("DEPTHS_NETWORK_V6_PREFIX", "129"),
            ("DEPTHS_TRUSTED_NETWORKS", "10.0.0.0/33"),
            ("DEPTHS_TRUSTED_NETWORKS", "10.0.0.1"),
            ("DEPTHS_TRUSTED_NETWORKS", "home"),
        ] {
            let r = Fixture::deploy().set(name, value).refusal();
            assert!(r.contains(name), "{name}={value}: {r}");
        }
    }

    #[test]
    fn feature_flags_and_proxies_parse() {
        let c = Fixture::deploy()
            .set("DEPTHS_FEATURE_SHELL", "true")
            .set("DEPTHS_FEATURE_EDIT", "false")
            .set(
                "DEPTHS_TRUSTED_PROXIES",
                "172.18.0.0/16, 100.64.0.0/10,::1/128",
            )
            .resolve()
            .unwrap();
        assert!(c.features.shell);
        assert!(!c.features.edit);
        assert_eq!(c.trusted_proxies.len(), 3);
        let r = Fixture::deploy()
            .set("DEPTHS_FEATURE_SHELL", "maybe")
            .refusal();
        assert!(r.contains("DEPTHS_FEATURE_SHELL"), "{r}");
        let r = Fixture::deploy()
            .set("DEPTHS_TRUSTED_PROXIES", "10.0.0.0/33")
            .refusal();
        assert!(r.contains("DEPTHS_TRUSTED_PROXIES"), "{r}");
    }

    #[test]
    fn security_policies_default_strict_and_warn_and_refuse_anything_else() {
        // 051 FR-007, FR-017.
        let c = Fixture::deploy().resolve().unwrap();
        assert_eq!(c.shell_policy, ShellPolicy::Strict);
        assert_eq!(c.compose_policy, ComposePolicy::Warn);
        let c = Fixture::deploy()
            .set("DEPTHS_SHELL_POLICY", "Relaxed")
            .set("DEPTHS_COMPOSE_POLICY", "enforce")
            .resolve()
            .unwrap();
        assert_eq!(c.shell_policy, ShellPolicy::Relaxed);
        assert_eq!(c.compose_policy, ComposePolicy::Enforce);
        let r = Fixture::deploy()
            .set("DEPTHS_SHELL_POLICY", "off")
            .refusal();
        assert!(r.contains("DEPTHS_SHELL_POLICY"), "{r}");
        let r = Fixture::deploy()
            .set("DEPTHS_COMPOSE_POLICY", "block")
            .refusal();
        assert!(r.contains("DEPTHS_COMPOSE_POLICY"), "{r}");
    }

    #[test]
    fn host_proc_is_off_unless_set_and_must_be_a_readable_proc() {
        let f = Fixture::deploy();
        assert_eq!(f.resolve().unwrap().host_proc, None);

        let proc_dir = f.dir.path().join("proc");
        std::fs::create_dir(&proc_dir).unwrap();
        let proc_str = proc_dir.to_string_lossy().into_owned();
        let r = Fixture {
            env: f.env.clone(),
            dir: tempfile::tempdir().unwrap(),
        }
        .set("DEPTHS_HOST_PROC", &proc_str)
        .refusal();
        assert!(
            r.contains("DEPTHS_HOST_PROC") && r.contains("stat"),
            "no stat: {r}"
        );

        std::fs::write(proc_dir.join("stat"), "cpu 1 1 1 1\n").unwrap();
        let c = Fixture {
            env: f.env.clone(),
            dir: tempfile::tempdir().unwrap(),
        }
        .set("DEPTHS_HOST_PROC", &proc_str)
        .resolve()
        .unwrap();
        assert_eq!(c.host_proc, Some(proc_dir.clone()));
        assert!(
            c.sources
                .contains(&("host_proc", "DEPTHS_HOST_PROC".to_owned()))
        );

        let c = f
            .resolve_with(Flags {
                host_proc: Some(proc_dir.clone()),
                ..Flags::default()
            })
            .unwrap();
        assert_eq!(c.host_proc, Some(proc_dir));
        assert!(c.sources.contains(&("host_proc", "--host-proc".to_owned())));

        let r = Fixture::deploy()
            .set("DEPTHS_HOST_PROC", "host/proc")
            .refusal();
        assert!(r.contains("absolute"), "{r}");
        let r = Fixture::deploy()
            .set("DEPTHS_HOST_PROC", "/nonexistent/depths-host-proc")
            .refusal();
        assert!(r.contains("DEPTHS_HOST_PROC"), "{r}");
    }

    #[test]
    fn durations_and_sizes_parse() {
        assert_eq!(
            parse_duration("x", "30m").unwrap(),
            Duration::from_secs(1800)
        );
        assert_eq!(
            parse_duration("x", "1d").unwrap(),
            Duration::from_secs(86_400)
        );
        assert!(parse_duration("x", "8").is_err());
        assert!(parse_duration("x", "0h").is_err());
        assert_eq!(parse_bytes("x", "1MiB").unwrap(), 1 << 20);
        assert_eq!(parse_bytes("x", "4096").unwrap(), 4096);
        assert!(parse_bytes("x", "lots").is_err());
        let r = Fixture::deploy().set("DEPTHS_SESSION_IDLE", "2d").refusal();
        assert!(r.contains("DEPTHS_SESSION_IDLE"), "{r}");
    }

    #[test]
    fn the_data_dir_is_created_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let c = Fixture::deploy().resolve().unwrap();
        c.prepare_data_dir().unwrap();
        let mode = std::fs::metadata(&c.data_dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}
