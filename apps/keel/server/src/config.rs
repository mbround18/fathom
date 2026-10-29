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
//! The OIDC provider can also grant instance roles from a claim (spec 060, US5;
//! contracts/config.md): `KEEL_OIDC_ROLE_CLAIM`, `KEEL_OIDC_{OWNER,ADMIN,USER}_VALUES` and
//! `KEEL_OIDC_ROLE_REQUIRED`, each with an `--oidc-…` flag. See [`RoleMapping`].
//!
//! The update check (spec 061; contracts/config.md): `KEEL_UPDATE_CHECK`, `KEEL_UPDATE_INTERVAL`,
//! `KEEL_UPDATE_IMAGE` and `KEEL_UPDATE_REGISTRY_USERNAME`, each with an `--update-…` flag, and
//! `KEEL_UPDATE_REGISTRY_PASSWORD` or `_FILE` (never a flag). `KEEL_IMAGE_DIGEST`
//! (`--image-digest`) and the image's `KEEL_BUILD_*` make up [`BuildInfo`]. See [`UpdateConfig`].
//!
//! Anything wrong refuses start with one line naming the setting.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use secrecy::SecretString;
use url::Url;

use crate::build_info::{BuildInfo, parse_digest};

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
    /// Dotted path to the string-array claim instance roles are read from, like
    /// `realm_access.roles`; unset means no role mapping [env: KEEL_OIDC_ROLE_CLAIM]
    #[arg(long, global = true)]
    pub oidc_role_claim: Option<String>,
    /// Comma-separated claim values that grant owner [env: KEEL_OIDC_OWNER_VALUES]
    #[arg(long, global = true)]
    pub oidc_owner_values: Option<String>,
    /// Comma-separated claim values that grant admin [env: KEEL_OIDC_ADMIN_VALUES]
    #[arg(long, global = true)]
    pub oidc_admin_values: Option<String>,
    /// Comma-separated claim values that grant user [env: KEEL_OIDC_USER_VALUES]
    #[arg(long, global = true)]
    pub oidc_user_values: Option<String>,
    /// Refuse sign-ins that match no role value [env: KEEL_OIDC_ROLE_REQUIRED] [default: false]
    #[arg(long, global = true)]
    pub oidc_role_required: Option<String>,
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
    /// PEM certificate chain: serve --listen over HTTPS; needs --tls-key [env: KEEL_TLS_CERT]
    #[arg(long, global = true)]
    pub tls_cert: Option<PathBuf>,
    /// PEM private key for --tls-cert, read once at start [env: KEEL_TLS_KEY]
    #[arg(long, global = true)]
    pub tls_key: Option<PathBuf>,
    /// Check the registry for a newer Keel image [env: KEEL_UPDATE_CHECK] [default: false]
    #[arg(long, global = true)]
    pub update_check: Option<String>,
    /// How often to check, at least 1h [env: KEEL_UPDATE_INTERVAL] [default: 6h]
    #[arg(long, global = true)]
    pub update_interval: Option<String>,
    /// The watched image, `[registry/]repository[:tag]` [env: KEEL_UPDATE_IMAGE]
    /// [default: docker.io/mbround18/keel:latest]
    #[arg(long, global = true)]
    pub update_image: Option<String>,
    /// Registry user for the update check; the password is KEEL_UPDATE_REGISTRY_PASSWORD (or
    /// _FILE), never a flag [env: KEEL_UPDATE_REGISTRY_USERNAME]
    #[arg(long, global = true)]
    pub update_registry_username: Option<String>,
    /// The running image's digest, as deployed [env: KEEL_IMAGE_DIGEST]
    #[arg(long, global = true)]
    pub image_digest: Option<String>,
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

/// Instance roles granted by the OIDC provider's claims (spec 060, US5). Deployment
/// configuration only: nothing in the UI can change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleMapping {
    /// Dotted path to a string-array claim (`groups`, `realm_access.roles`).
    pub claim: String,
    pub owner: Vec<String>,
    pub admin: Vec<String>,
    pub user: Vec<String>,
    /// Sign-ins matching no value, and sign-ins through other providers, are refused.
    pub required: bool,
}

impl RoleMapping {
    /// The highest role any of `groups` grants (owner, then admin, then user).
    #[must_use]
    pub fn role_for(&self, groups: &[String]) -> Option<keel_core::InstanceRole> {
        use keel_core::InstanceRole::{Admin, Owner, User};
        let any = |values: &[String]| groups.iter().any(|g| values.contains(g));
        if any(&self.owner) {
            Some(Owner)
        } else if any(&self.admin) {
            Some(Admin)
        } else if any(&self.user) {
            Some(User)
        } else {
            None
        }
    }

    /// The read-only view System Settings shows.
    #[must_use]
    pub fn info(&self) -> keel_protocol::RoleMappingInfo {
        keel_protocol::RoleMappingInfo {
            provider: "oidc".to_owned(),
            claim: self.claim.clone(),
            owner: self.owner.clone(),
            admin: self.admin.clone(),
            user: self.user.clone(),
            required: self.required,
        }
    }
}

/// `a, b` into `["a", "b"]`; an empty item (`a,,b`) refuses, naming `name`.
fn parse_values(name: &str, raw: Option<&str>) -> Result<Vec<String>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    raw.split(',')
        .map(str::trim)
        .map(|v| {
            if v.is_empty() {
                refuse(format!("{name}: `{raw}` has an empty value"))
            } else {
                Ok(v.to_owned())
            }
        })
        .collect()
}

/// The update check's settings (spec 061, contracts/config.md). Validated even when checks are
/// off, so a typo doesn't wait for the day someone turns them on.
#[derive(Debug, Clone)]
pub struct UpdateConfig {
    /// `false`: Keel makes no registry request at all (FR-004).
    pub check: bool,
    /// At least an hour.
    pub interval: Duration,
    pub image: WatchedImage,
    pub credentials: RegistryCredentials,
}

/// The watched reference, `registry/repository:tag`, never with credentials. Parsed here per
/// contracts/registry.md until `oci_registry::ImageRef` replaces it (T020).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedImage {
    /// As written, except that `index.docker.io` and no registry are `docker.io`.
    pub registry: String,
    /// With `library/` for a single-segment Docker Hub name.
    pub repository: String,
    pub tag: String,
}

impl WatchedImage {
    /// `[registry/]repository[:tag]`: no registry means `docker.io`, no tag means `latest`. The
    /// first segment is a registry only when it has a `.` or a `:`, or is `localhost`. Refuses a
    /// digest, a scheme, an upper-case repository and a tag over 128 characters.
    ///
    /// # Errors
    ///
    /// Why `s` isn't a reference to watch.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.starts_with("http://") {
            return Err("names an http:// registry; registries are reached over https".into());
        }
        if s.contains("://") {
            return Err("is a URL; write it as [registry/]repository[:tag]".into());
        }
        if s.contains('@') {
            return Err("carries a digest; a digest never changes, so watch a tag".into());
        }
        let (registry, rest) = match s.split_once('/') {
            Some((first, rest))
                if first.contains('.') || first.contains(':') || first == "localhost" =>
            {
                (first, rest)
            }
            _ => ("docker.io", s),
        };
        let registry = if registry == "index.docker.io" {
            "docker.io"
        } else {
            registry
        };
        let (repository, tag) = match rest.rsplit_once(':') {
            Some((repo, tag)) => (repo, tag),
            None => (rest, "latest"),
        };
        if repository.is_empty() {
            return Err("has no repository".into());
        }
        if repository.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err("has an upper-case repository".into());
        }
        let segment_ok = |seg: &str| {
            !seg.is_empty()
                && seg.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
                })
                && seg.as_bytes()[0].is_ascii_alphanumeric()
        };
        if !repository.split('/').all(segment_ok) {
            return Err(format!("`{repository}` is not a repository name"));
        }
        let tag_ok = (1..=128).contains(&tag.len())
            && tag.as_bytes()[0] != b'.'
            && tag.as_bytes()[0] != b'-'
            && tag
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !tag_ok {
            return Err(format!("`{tag}` is not a tag (at most 128 characters)"));
        }
        let repository = if registry == "docker.io" && !repository.contains('/') {
            format!("library/{repository}")
        } else {
            repository.to_owned()
        };
        Ok(Self {
            registry: registry.to_owned(),
            repository,
            tag: tag.to_owned(),
        })
    }
}

impl fmt::Display for WatchedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}:{}", self.registry, self.repository, self.tag)
    }
}

/// How the update check signs in to the registry. Becomes `oci_registry::Credentials` (T020).
/// `Debug` never shows the password.
#[derive(Clone)]
pub enum RegistryCredentials {
    Anonymous,
    Basic {
        username: String,
        password: SecretString,
    },
}

impl fmt::Debug for RegistryCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Anonymous => f.write_str("Anonymous"),
            Self::Basic { username, .. } => f
                .debug_struct("Basic")
                .field("username", username)
                .field("password", &"[redacted]")
                .finish(),
        }
    }
}

/// `--tls-cert` and `--tls-key`: paths only; [`crate::tls::load`] reads them at start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
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
    /// Serve `listen` over HTTPS. `None`: plain HTTP, for a TLS proxy in front or --insecure-dev.
    pub tls: Option<TlsFiles>,
    /// Instance roles from the OIDC provider's claims; `None` keeps first-user-is-owner.
    pub role_mapping: Option<RoleMapping>,
    /// The update check (spec 061).
    pub update: UpdateConfig,
    /// The running build: the image's `KEEL_BUILD_*` and `--image-digest` (spec 061).
    pub build: BuildInfo,
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
        let role_flags = RoleFlags {
            claim: flags.oidc_role_claim,
            owner: flags.oidc_owner_values,
            admin: flags.oidc_admin_values,
            user: flags.oidc_user_values,
            required: flags.oidc_role_required,
        };
        let update_flags = UpdateFlags {
            check: flags.update_check,
            interval: flags.update_interval,
            image: flags.update_image,
            username: flags.update_registry_username,
            image_digest: flags.image_digest,
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

        let path = |p: Option<PathBuf>| p.map(|p| p.display().to_string());
        let tls_cert = r.pick_quiet("tls_cert", path(flags.tls_cert), "KEEL_TLS_CERT");
        let tls_key = r.pick_quiet("tls_key", path(flags.tls_key), "KEEL_TLS_KEY");
        let tls = match (tls_cert, tls_key) {
            (Some(cert), Some(key)) => Some(TlsFiles {
                cert: PathBuf::from(cert),
                key: PathBuf::from(key),
            }),
            (Some(_), None) => {
                return refuse("--tls-cert (KEEL_TLS_CERT) needs --tls-key (KEEL_TLS_KEY)");
            }
            (None, Some(_)) => {
                return refuse("--tls-key (KEEL_TLS_KEY) needs --tls-cert (KEEL_TLS_CERT)");
            }
            (None, None) => None,
        };

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
        let has_oidc = providers
            .iter()
            .any(|p| matches!(p, ProviderConfig::Oidc { .. }));
        let role_mapping = Self::role_mapping(&mut r, role_flags, has_oidc)?;
        let (update, build) = Self::update(&mut r, update_flags)?;

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
            tls,
            role_mapping,
            update,
            build,
            sources: r.sources,
        })
    }

    /// The update check settings and the running build, validated per contracts/config.md
    /// (FR-009), whether or not checks are on.
    fn update(
        r: &mut Resolver<'_>,
        f: UpdateFlags,
    ) -> Result<(UpdateConfig, BuildInfo), ConfigError> {
        const MIN_INTERVAL: Duration = Duration::from_secs(3600);
        let check = r.pick("update_check", f.check, "KEEL_UPDATE_CHECK");
        let check = parse_bool("KEEL_UPDATE_CHECK", check.as_deref().unwrap_or("false"))?;
        let interval = r.pick("update_interval", f.interval, "KEEL_UPDATE_INTERVAL");
        let interval = parse_duration("KEEL_UPDATE_INTERVAL", interval.as_deref().unwrap_or("6h"))?;
        if interval < MIN_INTERVAL {
            return refuse("KEEL_UPDATE_INTERVAL: must be at least 1h");
        }
        let image = r.pick("update_image", f.image, "KEEL_UPDATE_IMAGE");
        let image = image
            .as_deref()
            .unwrap_or("docker.io/mbround18/keel:latest");
        let image = WatchedImage::parse(image)
            .or_else(|why| refuse(format!("KEEL_UPDATE_IMAGE: `{image}` {why}")))?;

        let username = r.pick_quiet(
            "update_registry_username",
            f.username,
            "KEEL_UPDATE_REGISTRY_USERNAME",
        );
        let password = r.secret("update_registry_password", "KEEL_UPDATE_REGISTRY_PASSWORD")?;
        let credentials = match (username, password) {
            (Some(username), Some(password)) => RegistryCredentials::Basic { username, password },
            (None, None) => RegistryCredentials::Anonymous,
            (Some(_), None) => {
                return refuse(
                    "KEEL_UPDATE_REGISTRY_USERNAME is set but KEEL_UPDATE_REGISTRY_PASSWORD \
                     (or _FILE) is not",
                );
            }
            (None, Some(_)) => {
                return refuse(
                    "KEEL_UPDATE_REGISTRY_PASSWORD is set but KEEL_UPDATE_REGISTRY_USERNAME \
                     (--update-registry-username) is not",
                );
            }
        };

        let digest = r
            .pick_quiet("image_digest", f.image_digest, "KEEL_IMAGE_DIGEST")
            .map(|d| parse_digest("KEEL_IMAGE_DIGEST", d.trim().to_owned()))
            .transpose()?;
        // The flag's digest wins; `from_vars` validates the env's either way.
        let build = BuildInfo::from_vars(r.env)?.with_digest(digest);
        Ok((
            UpdateConfig {
                check,
                interval,
                image,
                credentials,
            },
            build,
        ))
    }

    /// The OIDC role mapping as System Settings shows it, or `None` without one.
    #[must_use]
    pub fn role_mapping_info(&self) -> Option<keel_protocol::RoleMappingInfo> {
        self.role_mapping.as_ref().map(RoleMapping::info)
    }

    /// The role mapping settings, validated per contracts/config.md (FR-030).
    fn role_mapping(
        r: &mut Resolver<'_>,
        f: RoleFlags,
        has_oidc: bool,
    ) -> Result<Option<RoleMapping>, ConfigError> {
        const CLAIM: &str = "KEEL_OIDC_ROLE_CLAIM (--oidc-role-claim)";
        let claim = r.pick_quiet("oidc_role_claim", f.claim, "KEEL_OIDC_ROLE_CLAIM");
        let lists = [
            ("oidc_owner_values", f.owner, "KEEL_OIDC_OWNER_VALUES"),
            ("oidc_admin_values", f.admin, "KEEL_OIDC_ADMIN_VALUES"),
            ("oidc_user_values", f.user, "KEEL_OIDC_USER_VALUES"),
        ]
        .map(|(setting, flag, env)| (env, r.pick_quiet(setting, flag, env)));
        let required = r.pick_quiet("oidc_role_required", f.required, "KEEL_OIDC_ROLE_REQUIRED");

        let Some(claim) = claim.map(|c| c.trim().to_owned()) else {
            let set = lists
                .iter()
                .find(|(_, v)| v.is_some())
                .map(|(env, _)| *env)
                .or(required.as_ref().map(|_| "KEEL_OIDC_ROLE_REQUIRED"));
            return match set {
                Some(env) => refuse(format!("{env} is set but {CLAIM} is not")),
                None => Ok(None),
            };
        };
        if !has_oidc {
            return refuse(format!(
                "{CLAIM} needs the OIDC provider: set KEEL_OIDC_ISSUER and KEEL_OIDC_CLIENT_ID"
            ));
        }
        if claim.split('.').any(str::is_empty) {
            return refuse(format!("{CLAIM}: `{claim}` has an empty path segment"));
        }
        let [owner, admin, user] = lists.map(|(env, v)| parse_values(env, v.as_deref()));
        let (owner, admin, user) = (owner?, admin?, user?);
        if owner.is_empty() && admin.is_empty() && user.is_empty() {
            return refuse(format!(
                "{CLAIM} is set but KEEL_OIDC_OWNER_VALUES, KEEL_OIDC_ADMIN_VALUES and \
                 KEEL_OIDC_USER_VALUES are all empty"
            ));
        }
        let required = match required {
            Some(v) => parse_bool("KEEL_OIDC_ROLE_REQUIRED", &v)?,
            None => false,
        };
        Ok(Some(RoleMapping {
            claim,
            owner,
            admin,
            user,
            required,
        }))
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

/// The update check's flags, taken out of [`Flags`] before it is consumed.
struct UpdateFlags {
    check: Option<String>,
    interval: Option<String>,
    image: Option<String>,
    username: Option<String>,
    image_digest: Option<String>,
}

/// The role mapping's flags, taken out of [`Flags`] before it is consumed.
struct RoleFlags {
    claim: Option<String>,
    owner: Option<String>,
    admin: Option<String>,
    user: Option<String>,
    required: Option<String>,
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
    fn tls_needs_both_cert_and_key() {
        assert_eq!(resolve(BASE).unwrap().tls, None);
        let mut pairs = BASE.to_vec();
        pairs.extend([
            ("KEEL_TLS_CERT", "/run/keel/tls.crt"),
            ("KEEL_TLS_KEY", "/run/keel/tls.key"),
        ]);
        let c = resolve(&pairs).unwrap();
        assert_eq!(
            c.tls,
            Some(TlsFiles {
                cert: "/run/keel/tls.crt".into(),
                key: "/run/keel/tls.key".into(),
            })
        );
        assert!(c.sources.contains(&("tls_key", "KEEL_TLS_KEY".into())));
        for only in ["KEEL_TLS_CERT", "KEEL_TLS_KEY"] {
            let mut pairs = BASE.to_vec();
            pairs.push((only, "/run/keel/x.pem"));
            let err = resolve(&pairs).unwrap_err().0;
            assert!(
                err.contains("KEEL_TLS_CERT") && err.contains("KEEL_TLS_KEY"),
                "{err}"
            );
        }
        // A flag counts like its variable.
        let env = env_of(BASE);
        let flags = Flags {
            tls_key: Some("/k.pem".into()),
            ..Flags::default()
        };
        assert!(Config::resolve(flags, &env).is_err());
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

    fn with_oidc(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut pairs = BASE.to_vec();
        pairs.extend([
            ("KEEL_OIDC_ISSUER", "https://idp.example.com/realms/home"),
            ("KEEL_OIDC_CLIENT_ID", "keel"),
        ]);
        pairs.extend_from_slice(extra);
        pairs
    }

    #[test]
    fn no_role_mapping_by_default() {
        let c = resolve(&with_oidc(&[])).unwrap();
        assert_eq!(c.role_mapping, None);
        assert_eq!(c.role_mapping_info(), None);
    }

    #[test]
    fn role_mapping_resolves_and_flags_win() {
        let c = resolve(&with_oidc(&[
            ("KEEL_OIDC_ROLE_CLAIM", "realm_access.roles"),
            ("KEEL_OIDC_OWNER_VALUES", "keel-owners"),
            ("KEEL_OIDC_ADMIN_VALUES", " keel-admins , ops "),
            ("KEEL_OIDC_ROLE_REQUIRED", "true"),
        ]))
        .unwrap();
        let m = c.role_mapping.clone().unwrap();
        assert_eq!(m.claim, "realm_access.roles");
        assert_eq!(m.owner, ["keel-owners"]);
        assert_eq!(m.admin, ["keel-admins", "ops"]);
        assert!(m.user.is_empty());
        assert!(m.required);
        let info = c.role_mapping_info().unwrap();
        assert_eq!(
            (info.provider.as_str(), info.claim.as_str()),
            ("oidc", "realm_access.roles")
        );

        let pairs = with_oidc(&[
            ("KEEL_OIDC_ROLE_CLAIM", "realm_access.roles"),
            ("KEEL_OIDC_USER_VALUES", "everyone"),
        ]);
        let env = env_of(&pairs);
        let flags = Flags {
            oidc_role_claim: Some("groups".into()),
            oidc_user_values: Some("keel-users".into()),
            oidc_role_required: Some("false".into()),
            ..Flags::default()
        };
        let m = Config::resolve(flags, &env).unwrap().role_mapping.unwrap();
        assert_eq!(m.claim, "groups");
        assert_eq!(m.user, ["keel-users"]);
        assert!(!m.required);
    }

    #[test]
    fn highest_mapped_role_wins() {
        use keel_core::InstanceRole::{Admin, Owner, User};
        let m = RoleMapping {
            claim: "groups".into(),
            owner: vec!["o".into()],
            admin: vec!["a".into()],
            user: vec!["u".into()],
            required: false,
        };
        let g = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(m.role_for(&g(&["u", "a"])), Some(Admin));
        assert_eq!(m.role_for(&g(&["u", "o", "a"])), Some(Owner));
        assert_eq!(m.role_for(&g(&["x", "u"])), Some(User));
        assert_eq!(m.role_for(&g(&["x"])), None);
        assert_eq!(m.role_for(&[]), None);
    }

    #[test]
    fn role_mapping_refusals_name_the_setting() {
        // a value list or ROLE_REQUIRED without the claim
        for (key, value) in [
            ("KEEL_OIDC_OWNER_VALUES", "o"),
            ("KEEL_OIDC_ADMIN_VALUES", "a"),
            ("KEEL_OIDC_USER_VALUES", "u"),
            ("KEEL_OIDC_ROLE_REQUIRED", "true"),
        ] {
            let err = resolve(&with_oidc(&[(key, value)])).unwrap_err().0;
            assert!(
                err.contains(key) && err.contains("KEEL_OIDC_ROLE_CLAIM"),
                "{err}"
            );
        }
        // the claim without OIDC configured
        let mut pairs = BASE.to_vec();
        pairs.extend([
            ("KEEL_OIDC_ROLE_CLAIM", "groups"),
            ("KEEL_OIDC_USER_VALUES", "u"),
        ]);
        let err = resolve(&pairs).unwrap_err().0;
        assert!(
            err.contains("KEEL_OIDC_ROLE_CLAIM") && err.contains("KEEL_OIDC_ISSUER"),
            "{err}"
        );
        // the claim with every value list empty
        let err = resolve(&with_oidc(&[("KEEL_OIDC_ROLE_CLAIM", "groups")]))
            .unwrap_err()
            .0;
        assert!(
            err.contains("KEEL_OIDC_ROLE_CLAIM") && err.contains("all empty"),
            "{err}"
        );
        // an empty claim segment
        for claim in ["a..b", ".a", "a."] {
            let err = resolve(&with_oidc(&[
                ("KEEL_OIDC_ROLE_CLAIM", claim),
                ("KEEL_OIDC_USER_VALUES", "u"),
            ]))
            .unwrap_err()
            .0;
            assert!(
                err.contains("KEEL_OIDC_ROLE_CLAIM") && err.contains("segment"),
                "{err}"
            );
        }
        // an empty value after trimming
        for (key, value) in [
            ("KEEL_OIDC_OWNER_VALUES", "a,,b"),
            ("KEEL_OIDC_ADMIN_VALUES", "a, ,b"),
            ("KEEL_OIDC_USER_VALUES", "a,"),
        ] {
            let err = resolve(&with_oidc(&[
                ("KEEL_OIDC_ROLE_CLAIM", "groups"),
                (key, value),
            ]))
            .unwrap_err()
            .0;
            assert!(err.contains(key) && err.contains("empty value"), "{err}");
        }
        // a ROLE_REQUIRED that is not a boolean
        let err = resolve(&with_oidc(&[
            ("KEEL_OIDC_ROLE_CLAIM", "groups"),
            ("KEEL_OIDC_USER_VALUES", "u"),
            ("KEEL_OIDC_ROLE_REQUIRED", "maybe"),
        ]))
        .unwrap_err()
        .0;
        assert!(err.contains("KEEL_OIDC_ROLE_REQUIRED"), "{err}");
    }

    // Spec 061 T019: the update check and the running build (contracts/config.md).

    fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut pairs = BASE.to_vec();
        pairs.extend_from_slice(extra);
        pairs
    }

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn update_defaults_per_the_contract() {
        let c = resolve(BASE).unwrap();
        assert!(!c.update.check);
        assert_eq!(c.update.interval, Duration::from_secs(6 * 3600));
        assert_eq!(
            c.update.image.to_string(),
            "docker.io/mbround18/keel:latest"
        );
        assert!(matches!(
            c.update.credentials,
            RegistryCredentials::Anonymous
        ));
        assert_eq!(c.build.digest, None);
        assert_eq!(c.build.display(), "development build");
        for setting in ["update_check", "update_interval", "update_image"] {
            assert!(
                c.sources.contains(&(setting, "default".into())),
                "{setting}"
            );
        }
        assert!(
            !c.sources
                .iter()
                .any(|(s, _)| s.starts_with("update_registry"))
        );
    }

    #[test]
    fn update_settings_resolve_from_env_and_flags() {
        let c = resolve(&with(&[
            ("KEEL_UPDATE_CHECK", "true"),
            ("KEEL_UPDATE_INTERVAL", "2h"),
            ("KEEL_UPDATE_IMAGE", "ghcr.io/mbround18/keel:edge"),
            ("KEEL_IMAGE_DIGEST", DIGEST),
            ("KEEL_BUILD_VERSION", "0.2.0"),
        ]))
        .unwrap();
        assert!(c.update.check);
        assert_eq!(c.update.interval, Duration::from_secs(7200));
        assert_eq!(c.update.image.to_string(), "ghcr.io/mbround18/keel:edge");
        assert_eq!(c.build.digest.as_deref(), Some(DIGEST));
        assert_eq!(c.build.display(), "0.2.0");

        let other = format!("sha256:{}", "f".repeat(64));
        let pairs = with(&[("KEEL_IMAGE_DIGEST", DIGEST)]);
        let env = env_of(&pairs);
        let flags = Flags {
            update_check: Some("yes".into()),
            update_image: Some("localhost:5000/keel".into()),
            image_digest: Some(other.clone()),
            ..Flags::default()
        };
        let c = Config::resolve(flags, &env).unwrap();
        assert!(c.update.check);
        assert_eq!(c.update.image.to_string(), "localhost:5000/keel:latest");
        assert_eq!(c.build.digest, Some(other));
        assert!(
            c.sources
                .contains(&("update_image", "--update-image".into()))
        );
        assert!(
            c.sources
                .contains(&("image_digest", "--image-digest".into()))
        );
    }

    #[test]
    fn watched_image_follows_the_registry_rules() {
        for (input, want) in [
            ("mbround18/keel", "docker.io/mbround18/keel:latest"),
            ("keel", "docker.io/library/keel:latest"),
            (
                "index.docker.io/mbround18/keel:1",
                "docker.io/mbround18/keel:1",
            ),
            ("docker.io/keel:2", "docker.io/library/keel:2"),
            ("ghcr.io/a/b/c:v1.2_3-x", "ghcr.io/a/b/c:v1.2_3-x"),
            ("localhost/keel", "localhost/keel:latest"),
            ("registry:5000/keel:1", "registry:5000/keel:1"),
        ] {
            assert_eq!(
                WatchedImage::parse(input).unwrap().to_string(),
                want,
                "{input}"
            );
        }
        let long_tag = format!("keel:{}", "a".repeat(129));
        for bad in [
            "mbround18/Keel",
            &format!("mbround18/keel@{DIGEST}"),
            "http://registry.example/keel",
            "https://registry.example/keel",
            "ghcr.io/",
            "keel:",
            long_tag.as_str(),
            "keel:-x",
        ] {
            assert!(WatchedImage::parse(bad).is_err(), "{bad}");
        }
        assert!(WatchedImage::parse(&format!("keel:{}", "a".repeat(128))).is_ok());
    }

    #[test]
    fn invalid_update_settings_refuse_naming_the_variable() {
        let long_tag = format!("keel:{}", "a".repeat(129));
        let image_digest = format!("mbround18/keel@{DIGEST}");
        let cases: Vec<(&str, &str)> = vec![
            ("KEEL_UPDATE_CHECK", "maybe"),
            ("KEEL_UPDATE_INTERVAL", "soon"),
            ("KEEL_UPDATE_INTERVAL", "59m"),
            ("KEEL_UPDATE_IMAGE", "mbround18/Keel"),
            ("KEEL_UPDATE_IMAGE", &image_digest),
            ("KEEL_UPDATE_IMAGE", "http://registry.example/keel"),
            ("KEEL_UPDATE_IMAGE", &long_tag),
            ("KEEL_IMAGE_DIGEST", "sha256:abc"),
            ("KEEL_IMAGE_DIGEST", "latest"),
            ("KEEL_BUILD_DATE", "yesterday"),
        ];
        for (key, value) in cases {
            let pairs: Vec<_> = BASE.iter().copied().chain([(key, value)]).collect();
            let err = resolve(&pairs).unwrap_err().0;
            assert!(err.contains(key), "{key}={value}: {err}");
        }
        // Checks off still validates (FR-009).
        let err = resolve(&with(&[
            ("KEEL_UPDATE_CHECK", "false"),
            ("KEEL_UPDATE_INTERVAL", "10m"),
        ]))
        .unwrap_err()
        .0;
        assert!(err.contains("KEEL_UPDATE_INTERVAL"), "{err}");
    }

    #[test]
    fn registry_credentials_come_in_pairs() {
        let err = resolve(&with(&[("KEEL_UPDATE_REGISTRY_USERNAME", "robot")]))
            .unwrap_err()
            .0;
        assert!(err.contains("KEEL_UPDATE_REGISTRY_PASSWORD"), "{err}");
        let err = resolve(&with(&[("KEEL_UPDATE_REGISTRY_PASSWORD", "pa55word")]))
            .unwrap_err()
            .0;
        assert!(err.contains("KEEL_UPDATE_REGISTRY_USERNAME"), "{err}");
        let err = resolve(&with(&[
            ("KEEL_UPDATE_REGISTRY_USERNAME", "robot"),
            ("KEEL_UPDATE_REGISTRY_PASSWORD", "pa55word"),
            (
                "KEEL_UPDATE_REGISTRY_PASSWORD_FILE",
                "/run/secrets/registry",
            ),
        ]))
        .unwrap_err()
        .0;
        assert!(err.contains("KEEL_UPDATE_REGISTRY_PASSWORD_FILE"), "{err}");
    }

    #[test]
    fn registry_password_never_reaches_debug_or_sources() {
        let c = resolve(&with(&[
            ("KEEL_UPDATE_REGISTRY_USERNAME", "robot"),
            ("KEEL_UPDATE_REGISTRY_PASSWORD", "pa55word"),
        ]))
        .unwrap();
        let RegistryCredentials::Basic { username, password } = &c.update.credentials else {
            panic!("{:?}", c.update.credentials);
        };
        assert_eq!(username, "robot");
        assert_eq!(password.expose_secret(), "pa55word");
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("pa55word"), "{dbg}");
        assert!(dbg.contains("robot"), "{dbg}");
        for (_, from) in &c.sources {
            assert!(!from.contains("pa55word"));
        }
        assert!(c.sources.contains(&(
            "update_registry_password",
            "KEEL_UPDATE_REGISTRY_PASSWORD".into()
        )));
    }

    #[test]
    fn registry_password_reads_from_a_file() {
        let dir = std::env::temp_dir().join(format!("keel-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("registry-password");
        std::fs::write(&path, "fr0m-file\n").unwrap();
        let path_s = path.to_str().unwrap().to_owned();
        let mut pairs: Vec<(&str, &str)> = BASE.to_vec();
        pairs.push(("KEEL_UPDATE_REGISTRY_USERNAME", "robot"));
        pairs.push(("KEEL_UPDATE_REGISTRY_PASSWORD_FILE", &path_s));
        let c = resolve(&pairs).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let RegistryCredentials::Basic { password, .. } = &c.update.credentials else {
            panic!("{:?}", c.update.credentials);
        };
        assert_eq!(password.expose_secret(), "fr0m-file");
    }
}
