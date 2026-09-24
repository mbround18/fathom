//! `DaemonConfig` from environment (data-model.md). Parsing takes a lookup closure so tests
//! need no process environment.

use anyhow::{Context, Result, bail};
use chrono::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    /// mTLS API listener (front-proxy traffic from the API server).
    pub listen: String,
    /// Plain-HTTP kubelet probe listener (`/healthz`, `/readyz` only).
    pub health_listen: String,
    pub max_events: usize,
    pub max_age: Duration,
    /// Empty = cluster-wide.
    pub namespaces: Vec<String>,
    pub coalesce: Duration,
    /// Directory holding `tls.crt` and `tls.key` for the API listener.
    pub tls_dir: String,
    /// Optional file override for the requestheader CA bundle; normally read from the
    /// `extension-apiserver-authentication` ConfigMap in `kube-system`.
    pub requestheader_ca_file: Option<String>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8443".into(),
            health_listen: "0.0.0.0:8081".into(),
            max_events: 10_000,
            max_age: Duration::hours(24),
            namespaces: vec![],
            coalesce: Duration::seconds(5),
            tls_dir: "/tls".into(),
            requestheader_ca_file: None,
        }
    }
}

/// `24h`, `90m`, `30s`, `2d` or plain seconds.
pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    let (num, mult) = match s.chars().last() {
        Some('d') => (&s[..s.len() - 1], 86_400),
        Some('h') => (&s[..s.len() - 1], 3_600),
        Some('m') => (&s[..s.len() - 1], 60),
        Some('s') => (&s[..s.len() - 1], 1),
        Some(c) if c.is_ascii_digit() => (s, 1),
        _ => bail!("invalid duration {s:?}"),
    };
    let n: i64 = num
        .parse()
        .with_context(|| format!("invalid duration {s:?}"))?;
    Ok(Duration::seconds(n * mult))
}

impl DaemonConfig {
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let mut c = Self::default();
        if let Some(v) = get("FATHOM_DAEMON_LISTEN") {
            c.listen = v;
        }
        if let Some(v) = get("FATHOM_DAEMON_HEALTH_LISTEN") {
            c.health_listen = v;
        }
        if let Some(v) = get("FATHOM_DAEMON_MAX_EVENTS") {
            c.max_events = v.parse().context("FATHOM_DAEMON_MAX_EVENTS")?;
        }
        if let Some(v) = get("FATHOM_DAEMON_MAX_AGE") {
            c.max_age = parse_duration(&v).context("FATHOM_DAEMON_MAX_AGE")?;
        }
        if let Some(v) = get("FATHOM_DAEMON_NAMESPACES") {
            c.namespaces = v
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
        if let Some(v) = get("FATHOM_DAEMON_COALESCE_SECS") {
            c.coalesce = Duration::seconds(v.parse().context("FATHOM_DAEMON_COALESCE_SECS")?);
        }
        if let Some(v) = get("FATHOM_DAEMON_TLS_DIR") {
            c.tls_dir = v;
        }
        c.requestheader_ca_file = get("FATHOM_DAEMON_REQUESTHEADER_CA_FILE");
        Ok(c)
    }

    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let d = DaemonConfig::from_lookup(|_| None).unwrap();
        assert_eq!(d.listen, "0.0.0.0:8443");
        assert_eq!(d.max_events, 10_000);
        assert_eq!(d.max_age, Duration::hours(24));
        assert!(d.namespaces.is_empty());
        let c = DaemonConfig::from_lookup(|k| match k {
            "FATHOM_DAEMON_MAX_EVENTS" => Some("5".into()),
            "FATHOM_DAEMON_MAX_AGE" => Some("90m".into()),
            "FATHOM_DAEMON_NAMESPACES" => Some("a, b,,".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(c.max_events, 5);
        assert_eq!(c.max_age, Duration::minutes(90));
        assert_eq!(c.namespaces, vec!["a", "b"]);
        assert!(DaemonConfig::from_lookup(|_| Some("x".into())).is_err());
    }
}
