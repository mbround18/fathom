//! Hosted server configuration (spec 031). Validated fail-closed; nothing binds before this passes.

use std::path::PathBuf;
use std::time::Duration;

use secrecy::SecretString;
use url::Url;

use crate::session::SessionConfig;

/// Process exit code for invalid configuration.
pub const EXIT_INVALID_CONFIG: i32 = 2;
/// Process exit code for failed OIDC discovery.
pub const EXIT_DISCOVERY_FAILED: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessModel {
    PassThrough,
    Impersonation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KubeTokenKind {
    Id,
    Access,
}

#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: Url,
    /// The configured issuer string, verbatim (`Url` would add a `/` to a bare host).
    pub issuer_raw: String,
    pub client_id: String,
    pub client_secret: Option<SecretString>,
    pub scopes: Vec<String>,
    pub ca_file: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ClaimsConfig {
    pub username: String,
    pub groups: String,
    pub username_prefix: String,
    pub groups_prefix: String,
}

pub use crate::limits::SessionLimits;

#[derive(Debug, Clone)]
pub struct HostedConfig {
    pub external_url: Url,
    pub oidc: OidcConfig,
    pub claims: ClaimsConfig,
    pub access_model: AccessModel,
    pub kube_token: KubeTokenKind,
    pub audit_viewer_groups: Vec<String>,
    /// Entries kept by the hosted audit log (`FATHOM_AUDIT_RETENTION`).
    pub audit_retention: usize,
    pub limits: SessionLimits,
    pub session_absolute: Duration,
    pub session_idle: Duration,
    /// How long an unreachable IdP is tolerated before a session ends (`refreshGraceSeconds`).
    pub refresh_grace: Duration,
    /// Upper bound between token refreshes, doubling as the revocation probe (`revalidateSeconds`).
    pub revalidate: Duration,
    pub state_dir: PathBuf,
    pub ui_dir: PathBuf,
    pub listen: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{name} is invalid: {reason}")]
    Invalid { name: &'static str, reason: String },
}

impl ConfigError {
    pub fn exit_code(&self) -> i32 {
        EXIT_INVALID_CONFIG
    }
}

fn invalid(name: &'static str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        name,
        reason: reason.into(),
    }
}

fn list(v: Option<String>) -> Vec<String> {
    v.map(|s| {
        s.split([',', ' '])
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(String::from)
            .collect()
    })
    .unwrap_or_default()
}

fn parse_secs(
    name: &'static str,
    v: Option<String>,
    default: Duration,
) -> Result<Duration, ConfigError> {
    match v {
        None => Ok(default),
        Some(s) => {
            let n: u64 = s
                .trim()
                .parse()
                .map_err(|_| invalid(name, "expected whole seconds"))?;
            if n == 0 {
                Err(invalid(name, "must be > 0"))
            } else {
                Ok(Duration::from_secs(n))
            }
        }
    }
}

fn parse_num<T>(name: &'static str, v: Option<String>, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr + PartialEq + From<u8>,
{
    match v {
        None => Ok(default),
        Some(s) => {
            let n: T = s
                .trim()
                .parse()
                .map_err(|_| invalid(name, "expected a number"))?;
            if n == T::from(0u8) {
                Err(invalid(name, "must be > 0"))
            } else {
                Ok(n)
            }
        }
    }
}

fn is_loopback(u: &Url) -> bool {
    match u.host() {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

impl HostedConfig {
    /// Read configuration from the process environment (supports `FATHOM_OIDC_CLIENT_SECRET_FILE`).
    pub fn from_env() -> Result<Self, ConfigError> {
        let sa =
            std::path::Path::new("/var/run/secrets/kubernetes.io/serviceaccount/token").exists();
        Self::from_lookup(|k| std::env::var(k).ok().filter(|v| !v.is_empty()), sa)
    }

    /// Parse and validate from an arbitrary key lookup (testable).
    pub fn from_lookup(
        get: impl Fn(&str) -> Option<String>,
        sa_token_present: bool,
    ) -> Result<Self, ConfigError> {
        let allow_insecure = matches!(
            get("FATHOM_ALLOW_INSECURE_LOCALHOST").as_deref(),
            Some("1" | "true")
        );

        let ext = get("FATHOM_EXTERNAL_URL").ok_or(ConfigError::Missing("FATHOM_EXTERNAL_URL"))?;
        let external_url =
            Url::parse(&ext).map_err(|e| invalid("FATHOM_EXTERNAL_URL", e.to_string()))?;
        match external_url.scheme() {
            "https" => {}
            "http" if allow_insecure => {
                if !is_loopback(&external_url) {
                    return Err(invalid(
                        "FATHOM_EXTERNAL_URL",
                        "--allow-insecure-localhost only permits a loopback host",
                    ));
                }
            }
            _ => return Err(invalid("FATHOM_EXTERNAL_URL", "must be an https URL")),
        }
        if external_url.host().is_none() {
            return Err(invalid("FATHOM_EXTERNAL_URL", "missing host"));
        }

        let issuer_raw =
            get("FATHOM_OIDC_ISSUER").ok_or(ConfigError::Missing("FATHOM_OIDC_ISSUER"))?;
        let issuer =
            Url::parse(&issuer_raw).map_err(|e| invalid("FATHOM_OIDC_ISSUER", e.to_string()))?;
        if issuer.scheme() != "https" && !(allow_insecure && is_loopback(&issuer)) {
            return Err(invalid("FATHOM_OIDC_ISSUER", "must be an https URL"));
        }
        let client_id = get("FATHOM_OIDC_CLIENT_ID")
            .filter(|s| !s.trim().is_empty())
            .ok_or(ConfigError::Missing("FATHOM_OIDC_CLIENT_ID"))?;

        let client_secret = match get("FATHOM_OIDC_CLIENT_SECRET") {
            Some(s) => Some(SecretString::from(s)),
            None => match get("FATHOM_OIDC_CLIENT_SECRET_FILE") {
                Some(path) => {
                    let s = std::fs::read_to_string(&path).map_err(|e| {
                        invalid("FATHOM_OIDC_CLIENT_SECRET_FILE", format!("{path}: {e}"))
                    })?;
                    Some(SecretString::from(s.trim().to_string()))
                }
                None => None,
            },
        };

        let scopes = {
            let s = list(get("FATHOM_OIDC_SCOPES"));
            if s.is_empty() {
                ["openid", "profile", "email", "groups"]
                    .map(String::from)
                    .to_vec()
            } else if !s.iter().any(|x| x == "openid") {
                return Err(invalid("FATHOM_OIDC_SCOPES", "must include `openid`"));
            } else {
                s
            }
        };

        let access_model = match get("FATHOM_ACCESS_MODEL").as_deref() {
            None | Some("passthrough") => AccessModel::PassThrough,
            Some("impersonation") => AccessModel::Impersonation,
            Some(o) => {
                return Err(invalid(
                    "FATHOM_ACCESS_MODEL",
                    format!("`{o}`: use passthrough|impersonation"),
                ));
            }
        };
        if access_model == AccessModel::Impersonation && !sa_token_present {
            return Err(invalid(
                "FATHOM_ACCESS_MODEL",
                "impersonation requires a mounted service account token",
            ));
        }
        let kube_token = match get("FATHOM_KUBE_TOKEN").as_deref() {
            None | Some("id") => KubeTokenKind::Id,
            Some("access") => KubeTokenKind::Access,
            Some(o) => {
                return Err(invalid(
                    "FATHOM_KUBE_TOKEN",
                    format!("`{o}`: use id|access"),
                ));
            }
        };

        let d = SessionLimits::default();
        let limits = SessionLimits {
            max_sessions: parse_num(
                "FATHOM_LIMIT_MAX_SESSIONS",
                get("FATHOM_LIMIT_MAX_SESSIONS"),
                d.max_sessions,
            )?,
            max_sessions_per_user: parse_num(
                "FATHOM_LIMIT_MAX_SESSIONS_PER_USER",
                get("FATHOM_LIMIT_MAX_SESSIONS_PER_USER"),
                d.max_sessions_per_user,
            )?,
            max_watches_per_user: parse_num(
                "FATHOM_LIMIT_MAX_WATCHES_PER_USER",
                get("FATHOM_LIMIT_MAX_WATCHES_PER_USER"),
                d.max_watches_per_user,
            )?,
            max_streams_per_user: parse_num(
                "FATHOM_LIMIT_MAX_STREAMS_PER_USER",
                get("FATHOM_LIMIT_MAX_STREAMS_PER_USER"),
                d.max_streams_per_user,
            )?,
            max_tabs_per_user: parse_num(
                "FATHOM_LIMIT_MAX_TABS_PER_USER",
                get("FATHOM_LIMIT_MAX_TABS_PER_USER"),
                d.max_tabs_per_user,
            )?,
            stream_recycle_minutes: parse_num(
                "FATHOM_LIMIT_STREAM_RECYCLE_MINUTES",
                get("FATHOM_LIMIT_STREAM_RECYCLE_MINUTES"),
                d.stream_recycle_minutes,
            )?,
            idle_park_minutes: parse_num(
                "FATHOM_LIMIT_IDLE_PARK_MINUTES",
                get("FATHOM_LIMIT_IDLE_PARK_MINUTES"),
                d.idle_park_minutes,
            )?,
        };

        Ok(Self {
            external_url,
            oidc: OidcConfig {
                issuer,
                issuer_raw: issuer_raw.trim().to_owned(),
                client_id,
                client_secret,
                scopes,
                ca_file: get("FATHOM_OIDC_CA_FILE").map(PathBuf::from),
            },
            claims: ClaimsConfig {
                username: get("FATHOM_CLAIM_USERNAME")
                    .unwrap_or_else(|| "preferred_username".into()),
                groups: get("FATHOM_CLAIM_GROUPS").unwrap_or_else(|| "groups".into()),
                username_prefix: get("FATHOM_USERNAME_PREFIX").unwrap_or_default(),
                groups_prefix: get("FATHOM_GROUPS_PREFIX").unwrap_or_default(),
            },
            access_model,
            kube_token,
            audit_viewer_groups: list(get("FATHOM_AUDIT_VIEWER_GROUPS")),
            audit_retention: parse_num(
                "FATHOM_AUDIT_RETENTION",
                get("FATHOM_AUDIT_RETENTION"),
                crate::audit::DEFAULT_AUDIT_RETENTION,
            )?,
            limits,
            session_absolute: parse_secs(
                "FATHOM_SESSION_ABSOLUTE",
                get("FATHOM_SESSION_ABSOLUTE"),
                Duration::from_secs(8 * 3600),
            )?,
            session_idle: parse_secs(
                "FATHOM_SESSION_IDLE",
                get("FATHOM_SESSION_IDLE"),
                Duration::from_secs(30 * 60),
            )?,
            refresh_grace: parse_secs(
                "FATHOM_REFRESH_GRACE_SECONDS",
                get("FATHOM_REFRESH_GRACE_SECONDS"),
                Duration::from_secs(120),
            )?,
            revalidate: parse_secs(
                "FATHOM_REVALIDATE_SECONDS",
                get("FATHOM_REVALIDATE_SECONDS"),
                Duration::from_secs(60),
            )?,
            state_dir: get("FATHOM_STATE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| "/var/lib/fathom".into()),
            ui_dir: get("FATHOM_UI_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| "/ui".into()),
            listen: get("FATHOM_LISTEN").unwrap_or_else(|| "0.0.0.0:8080".into()),
        })
    }

    pub fn redirect_uri(&self) -> String {
        let mut u = self.external_url.clone();
        u.set_path("/auth/callback");
        u.set_query(None);
        u.to_string()
    }

    pub fn session_config(&self) -> SessionConfig {
        SessionConfig {
            absolute: self.session_absolute,
            idle: self.session_idle,
            max_sessions: self.limits.max_sessions as usize,
            max_sessions_per_user: self.limits.max_sessions_per_user as usize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn base() -> HashMap<&'static str, String> {
        HashMap::from([
            (
                "FATHOM_EXTERNAL_URL",
                "https://fathom.example.com".to_string(),
            ),
            (
                "FATHOM_OIDC_ISSUER",
                "https://idp.example.com/realms/x".to_string(),
            ),
            ("FATHOM_OIDC_CLIENT_ID", "fathom".to_string()),
        ])
    }
    fn parse(m: &HashMap<&'static str, String>, sa: bool) -> Result<HostedConfig, ConfigError> {
        HostedConfig::from_lookup(|k| m.get(k).cloned(), sa)
    }

    #[test]
    fn defaults() {
        let c = parse(&base(), false).unwrap();
        assert_eq!(c.claims.username, "preferred_username");
        assert_eq!(c.claims.groups, "groups");
        assert_eq!(c.access_model, AccessModel::PassThrough);
        assert_eq!(c.kube_token, KubeTokenKind::Id);
        assert_eq!(c.oidc.scopes, ["openid", "profile", "email", "groups"]);
        assert_eq!(c.session_absolute, Duration::from_secs(8 * 3600));
        assert_eq!(c.session_idle, Duration::from_secs(1800));
        assert_eq!(c.limits.max_sessions, 50);
        assert_eq!(c.listen, "0.0.0.0:8080");
        assert_eq!(c.redirect_uri(), "https://fathom.example.com/auth/callback");
    }

    #[test]
    fn issuer_is_not_normalized() {
        // OIDC issuer matching is exact: a trailing slash is significant.
        for issuer in [
            "https://idp.example.com/auth/v1",
            "https://idp.example.com/auth/v1/",
        ] {
            let mut m = base();
            m.insert("FATHOM_OIDC_ISSUER", issuer.into());
            assert_eq!(parse(&m, false).unwrap().oidc.issuer_raw, issuer);
        }
    }

    #[test]
    fn offline_access_is_only_requested_when_configured() {
        assert!(
            !parse(&base(), false)
                .unwrap()
                .oidc
                .scopes
                .contains(&"offline_access".into())
        );
        let mut m = base();
        m.insert("FATHOM_OIDC_SCOPES", "openid,groups,offline_access".into());
        let c = parse(&m, false).unwrap();
        assert_eq!(c.oidc.scopes, ["openid", "groups", "offline_access"]);
    }

    #[test]
    fn refresh_settings_default_and_override() {
        let c = parse(&base(), false).unwrap();
        assert_eq!(c.refresh_grace, Duration::from_secs(120));
        assert_eq!(c.revalidate, Duration::from_secs(60));
        assert_eq!(c.limits.stream_recycle_minutes, 4);
        let mut m = base();
        m.insert("FATHOM_REFRESH_GRACE_SECONDS", "30".into());
        m.insert("FATHOM_REVALIDATE_SECONDS", "15".into());
        m.insert("FATHOM_LIMIT_STREAM_RECYCLE_MINUTES", "2".into());
        let c = parse(&m, false).unwrap();
        assert_eq!(c.refresh_grace, Duration::from_secs(30));
        assert_eq!(c.revalidate, Duration::from_secs(15));
        assert_eq!(c.limits.stream_recycle_minutes, 2);
        m.insert("FATHOM_REFRESH_GRACE_SECONDS", "0".into());
        assert!(parse(&m, false).is_err());
    }

    #[test]
    fn missing_required() {
        for k in [
            "FATHOM_EXTERNAL_URL",
            "FATHOM_OIDC_ISSUER",
            "FATHOM_OIDC_CLIENT_ID",
        ] {
            let mut m = base();
            m.remove(k);
            let e = parse(&m, false).unwrap_err();
            assert_eq!(e, ConfigError::Missing(k));
            assert_eq!(e.exit_code(), 2);
        }
    }

    #[test]
    fn external_url_must_be_https() {
        let mut m = base();
        m.insert("FATHOM_EXTERNAL_URL", "http://fathom.example.com".into());
        assert!(parse(&m, false).is_err());
        m.insert("FATHOM_ALLOW_INSECURE_LOCALHOST", "1".into());
        assert!(parse(&m, false).is_err(), "non-loopback stays refused");
        m.insert("FATHOM_EXTERNAL_URL", "http://localhost:8080".into());
        assert!(parse(&m, false).is_ok());
        m.insert("FATHOM_EXTERNAL_URL", "not a url".into());
        assert!(parse(&m, false).is_err());
    }

    #[test]
    fn impersonation_needs_sa_token() {
        let mut m = base();
        m.insert("FATHOM_ACCESS_MODEL", "impersonation".into());
        assert!(parse(&m, false).is_err());
        assert_eq!(
            parse(&m, true).unwrap().access_model,
            AccessModel::Impersonation
        );
        m.insert("FATHOM_ACCESS_MODEL", "bogus".into());
        assert!(parse(&m, true).is_err());
    }

    #[test]
    fn scopes_and_lists() {
        let mut m = base();
        m.insert("FATHOM_OIDC_SCOPES", "openid, email".into());
        m.insert("FATHOM_AUDIT_VIEWER_GROUPS", "admins,auditors".into());
        let c = parse(&m, false).unwrap();
        assert_eq!(c.oidc.scopes, ["openid", "email"]);
        assert_eq!(c.audit_viewer_groups, ["admins", "auditors"]);
        assert_eq!(c.audit_retention, crate::audit::DEFAULT_AUDIT_RETENTION);
        m.insert("FATHOM_AUDIT_RETENTION", "250".into());
        assert_eq!(parse(&m, false).unwrap().audit_retention, 250);
        m.insert("FATHOM_AUDIT_RETENTION", "0".into());
        assert!(parse(&m, false).is_err());
        m.remove("FATHOM_AUDIT_RETENTION");
        m.insert("FATHOM_OIDC_SCOPES", "email".into());
        assert!(parse(&m, false).is_err());
    }

    #[test]
    fn numeric_overrides_validated() {
        let mut m = base();
        m.insert("FATHOM_LIMIT_MAX_SESSIONS", "7".into());
        m.insert("FATHOM_SESSION_IDLE", "60".into());
        let c = parse(&m, false).unwrap();
        assert_eq!(c.limits.max_sessions, 7);
        assert_eq!(c.session_idle, Duration::from_secs(60));
        m.insert("FATHOM_LIMIT_MAX_SESSIONS", "0".into());
        assert!(parse(&m, false).is_err());
        m.insert("FATHOM_LIMIT_MAX_SESSIONS", "x".into());
        assert!(parse(&m, false).is_err());
    }

    #[test]
    fn secret_not_in_debug() {
        let mut m = base();
        m.insert("FATHOM_OIDC_CLIENT_SECRET", "hunter2".into());
        let c = parse(&m, false).unwrap();
        assert!(!format!("{c:?}").contains("hunter2"));
    }
}
