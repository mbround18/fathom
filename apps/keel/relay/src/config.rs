//! `/etc/keel-relay/relay.toml` (contracts/relay.md "Configuration").
//!
//! Unknown keys are refused, so a typo fails at `check-config` instead of silently using a
//! default.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Where `serve` and `check-config` read the config without `--config`.
pub const DEFAULT_CONFIG_PATH: &str = "/etc/keel-relay/relay.toml";

/// Largest config file read; a real one is a few hundred bytes.
const MAX_CONFIG_LEN: u64 = 64 * 1024;

/// The relay's settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Where Keel dials the SSH link.
    #[serde(default = "default_link_listen")]
    pub link_listen: SocketAddr,
    /// Where runners dial.
    #[serde(default = "default_runner_listen")]
    pub runner_listen: SocketAddr,
    /// The pinned Keel link keys, one OpenSSH public line per key. Required.
    pub hub_keys: PathBuf,
    /// The optional release cache. Unset means off.
    #[serde(default)]
    pub http_listen: Option<SocketAddr>,
    /// Concurrent runner streams.
    #[serde(default = "default_max_streams")]
    pub max_streams: u32,
}

fn default_link_listen() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 7832))
}

fn default_runner_listen() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 7831))
}

const fn default_max_streams() -> u32 {
    256
}

/// Why a config was refused. Every variant maps to exit code 78 (`EX_CONFIG`).
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: larger than {MAX_CONFIG_LEN} bytes")]
    TooLarge { path: PathBuf },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{path}: {message}")]
    Invalid { path: PathBuf, message: String },
}

impl Config {
    /// Parses and checks a config's text. `path` is only used in errors.
    pub fn parse(text: &str, path: &Path) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(text).map_err(|e| ConfigError::Parse {
            path: path.to_owned(),
            message: e.message().to_owned(),
        })?;
        let invalid = |message: &str| ConfigError::Invalid {
            path: path.to_owned(),
            message: message.to_owned(),
        };
        if config.max_streams == 0 {
            return Err(invalid("max_streams must be at least 1"));
        }
        if config.link_listen == config.runner_listen {
            return Err(invalid("link_listen and runner_listen must differ"));
        }
        if config
            .http_listen
            .is_some_and(|h| h == config.link_listen || h == config.runner_listen)
        {
            return Err(invalid("http_listen must differ from the other listeners"));
        }
        if !config.hub_keys.is_absolute() {
            return Err(invalid("hub_keys must be an absolute path"));
        }
        Ok(config)
    }

    /// Reads and checks the config at `path`.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let read = |source| ConfigError::Read {
            path: path.to_owned(),
            source,
        };
        let meta = std::fs::metadata(path).map_err(read)?;
        if meta.len() > MAX_CONFIG_LEN {
            return Err(ConfigError::TooLarge {
                path: path.to_owned(),
            });
        }
        let text = std::fs::read_to_string(path).map_err(read)?;
        Self::parse(&text, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Config, ConfigError> {
        Config::parse(text, Path::new("relay.toml"))
    }

    #[test]
    fn defaults_fill_everything_but_hub_keys() {
        let c = parse(r#"hub_keys = "/etc/keel-relay/hub_keys""#).unwrap();
        assert_eq!(c.link_listen, "0.0.0.0:7832".parse().unwrap());
        assert_eq!(c.runner_listen, "0.0.0.0:7831".parse().unwrap());
        assert_eq!(c.hub_keys, Path::new("/etc/keel-relay/hub_keys"));
        assert_eq!(c.http_listen, None);
        assert_eq!(c.max_streams, 256);
    }

    #[test]
    fn the_contract_example_parses() {
        let c = parse(
            r#"
            link_listen = "10.0.0.5:7832"
            runner_listen = "10.0.0.5:7831"
            hub_keys = "/etc/keel-relay/hub_keys"
            http_listen = "10.0.0.5:7880"
            max_streams = 32
            "#,
        )
        .unwrap();
        assert_eq!(c.http_listen, Some("10.0.0.5:7880".parse().unwrap()));
        assert_eq!(c.max_streams, 32);
    }

    #[test]
    fn unknown_keys_and_missing_hub_keys_fail() {
        assert!(matches!(
            parse("hub_keys = \"/k\"\nmax_stream = 4"),
            Err(ConfigError::Parse { .. })
        ));
        assert!(matches!(parse(""), Err(ConfigError::Parse { .. })));
        assert!(matches!(
            parse("hub_keys = \"/k\"\nlink_listen = \"not an address\""),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn nonsense_values_fail() {
        for text in [
            "hub_keys = \"/k\"\nmax_streams = 0",
            "hub_keys = \"relative/keys\"",
            "hub_keys = \"/k\"\nlink_listen = \"0.0.0.0:7831\"",
            "hub_keys = \"/k\"\nhttp_listen = \"0.0.0.0:7832\"",
        ] {
            assert!(
                matches!(parse(text), Err(ConfigError::Invalid { .. })),
                "{text}"
            );
        }
    }

    #[test]
    fn load_reads_a_file_and_reports_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relay.toml");
        assert!(matches!(Config::load(&path), Err(ConfigError::Read { .. })));
        std::fs::write(&path, "hub_keys = \"/k\"\n").unwrap();
        assert_eq!(Config::load(&path).unwrap().max_streams, 256);
    }
}
