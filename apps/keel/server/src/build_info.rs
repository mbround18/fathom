//! What this Keel is: the running build, read once at start (spec 061 T018; data-model.md
//! "Running build", research R2, R3).
//!
//! The image's runtime stage sets `KEEL_BUILD_VERSION`, `KEEL_BUILD_REVISION`, `KEEL_BUILD_DATE`
//! and `KEEL_BUILD_SOURCE`; the deployment sets `KEEL_IMAGE_DIGEST` (or `--image-digest`, which
//! [`crate::config::Config::resolve`] applies with [`BuildInfo::with_digest`]). A missing or empty
//! value is `None`, shown as "unknown". A malformed one refuses start, naming the variable: a
//! malformed build is a broken image, not a runtime choice.

use oci_registry::Platform;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use url::Url;

use crate::config::{ConfigError, Env, process_env};

/// The running build, fixed for the process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    /// Semver, without the `keel-v` prefix.
    pub version: Option<String>,
    /// 7–40 hex characters, optionally ending `-dirty`.
    pub revision: Option<String>,
    pub built_at: Option<OffsetDateTime>,
    /// `https` only.
    pub source: Option<Url>,
    /// `sha256:` and 64 lowercase hex characters.
    pub digest: Option<String>,
    /// The platform an index entry is picked for (research R1), in OCI's names.
    pub platform: Platform,
}

impl BuildInfo {
    /// Reads the process environment.
    ///
    /// # Errors
    ///
    /// A malformed value, naming its variable.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(&process_env)
    }

    /// Reads `env`: the process environment in production, a map in tests.
    ///
    /// # Errors
    ///
    /// A malformed value, naming its variable.
    pub fn from_vars(env: Env<'_>) -> Result<Self, ConfigError> {
        let var = |name: &str| {
            env(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let version = var("KEEL_BUILD_VERSION")
            .map(|v| parse_version("KEEL_BUILD_VERSION", v))
            .transpose()?;
        let revision = var("KEEL_BUILD_REVISION")
            .map(|v| parse_revision("KEEL_BUILD_REVISION", v))
            .transpose()?;
        let built_at = var("KEEL_BUILD_DATE")
            .map(|v| {
                OffsetDateTime::parse(&v, &Rfc3339).map_err(|_| {
                    ConfigError(format!(
                        "KEEL_BUILD_DATE: `{v}` is not an RFC 3339 time (like 2026-10-28T12:00:00Z)"
                    ))
                })
            })
            .transpose()?;
        let source = var("KEEL_BUILD_SOURCE")
            .map(|v| match Url::parse(&v) {
                Ok(u) if u.scheme() == "https" => Ok(u),
                _ => Err(ConfigError(format!(
                    "KEEL_BUILD_SOURCE: `{v}` is not an https URL"
                ))),
            })
            .transpose()?;
        let digest = var("KEEL_IMAGE_DIGEST")
            .map(|v| parse_digest("KEEL_IMAGE_DIGEST", v))
            .transpose()?;
        Ok(Self {
            version,
            revision,
            built_at,
            source,
            digest,
            platform: Platform::current(),
        })
    }

    /// `digest` replaces the one from the environment when set (`--image-digest` beats
    /// `KEEL_IMAGE_DIGEST`).
    #[must_use]
    pub fn with_digest(mut self, digest: Option<String>) -> Self {
        if digest.is_some() {
            self.digest = digest;
        }
        self
    }

    /// What the UI calls this build (research R2): the version, else the short
    /// revision (`abc1234`), else `development build`.
    #[must_use]
    pub fn display(&self) -> String {
        match (&self.version, &self.revision) {
            (Some(v), _) => v.clone(),
            (None, Some(r)) => {
                let (sha, dirty) = match r.strip_suffix("-dirty") {
                    Some(sha) => (sha, "-dirty"),
                    None => (r.as_str(), ""),
                };
                format!("{}{dirty}", &sha[..7])
            }
            (None, None) => "development build".to_owned(),
        }
    }
}

/// `sha256:` and 64 lowercase hex characters.
#[must_use]
pub fn is_digest(v: &str) -> bool {
    v.strip_prefix("sha256:")
        .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// [`is_digest`], refusing with `name`.
///
/// # Errors
///
/// `v` is not a digest.
pub fn parse_digest(name: &str, v: String) -> Result<String, ConfigError> {
    if is_digest(&v) {
        Ok(v)
    } else {
        Err(ConfigError(format!(
            "{name}: `{v}` is not a digest (sha256: and 64 lowercase hex characters)"
        )))
    }
}

fn parse_revision(name: &str, v: String) -> Result<String, ConfigError> {
    let sha = v.strip_suffix("-dirty").unwrap_or(&v);
    if (7..=40).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(v)
    } else {
        Err(ConfigError(format!(
            "{name}: `{v}` is not a revision (7 to 40 hex characters, optionally ending -dirty)"
        )))
    }
}

/// `MAJOR.MINOR.PATCH`, with an optional `-pre` and `+build` (semver 2.0, without the leading-zero
/// rule).
fn parse_version(name: &str, v: String) -> Result<String, ConfigError> {
    let ident = |s: &str| {
        !s.is_empty()
            && s.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    };
    let (rest, build) = match v.split_once('+') {
        Some((r, b)) => (r, Some(b)),
        None => (v.as_str(), None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (rest, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let ok = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && pre.is_none_or(ident)
        && build.is_none_or(ident);
    if ok {
        Ok(v)
    } else {
        Err(ConfigError(format!(
            "{name}: `{v}` is not a semver version (like 0.2.0, without the keel-v prefix)"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn build(pairs: &[(&str, &str)]) -> Result<BuildInfo, ConfigError> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect();
        BuildInfo::from_vars(&move |k: &str| map.get(k).cloned())
    }

    #[test]
    fn nothing_set_is_a_development_build() {
        let b = build(&[]).unwrap();
        assert_eq!(
            (&b.version, &b.revision, b.built_at, &b.source, &b.digest),
            (&None, &None, None, &None, &None)
        );
        assert_eq!(b.display(), "development build");
        // Empty is the same as missing: an untagged image carries `KEEL_BUILD_VERSION=`.
        let b = build(&[("KEEL_BUILD_VERSION", ""), ("KEEL_BUILD_DATE", " ")]).unwrap();
        assert_eq!(b.display(), "development build");
    }

    #[test]
    fn a_full_release_build() {
        let b = build(&[
            ("KEEL_BUILD_VERSION", "0.2.0"),
            ("KEEL_BUILD_REVISION", "cf7fdd6"),
            ("KEEL_BUILD_DATE", "2026-10-28T12:00:00Z"),
            ("KEEL_BUILD_SOURCE", "https://github.com/mbround18/fathom"),
            ("KEEL_IMAGE_DIGEST", DIGEST),
        ])
        .unwrap();
        assert_eq!(b.version.as_deref(), Some("0.2.0"));
        assert_eq!(b.revision.as_deref(), Some("cf7fdd6"));
        assert_eq!(b.built_at.unwrap().unix_timestamp(), 1_793_188_800);
        assert_eq!(
            b.source.unwrap().as_str(),
            "https://github.com/mbround18/fathom"
        );
        assert_eq!(b.digest.as_deref(), Some(DIGEST));
    }

    #[test]
    fn display_forms() {
        let b = |pairs: &[(&str, &str)]| build(pairs).unwrap().display();
        assert_eq!(
            b(&[
                ("KEEL_BUILD_VERSION", "0.2.0"),
                ("KEEL_BUILD_REVISION", "abc1234def")
            ]),
            "0.2.0"
        );
        assert_eq!(
            b(&[("KEEL_BUILD_REVISION", "abc1234def5678")]),
            "abc1234"
        );
        assert_eq!(
            b(&[("KEEL_BUILD_REVISION", "abc1234def-dirty")]),
            "abc1234-dirty"
        );
        assert_eq!(b(&[]), "development build");
    }

    #[test]
    fn versions() {
        for ok in [
            "0.2.0",
            "1.10.3",
            "0.3.0-rc.1",
            "1.0.0+build.5",
            "1.0.0-alpha-1+x",
        ] {
            assert!(build(&[("KEEL_BUILD_VERSION", ok)]).is_ok(), "{ok}");
        }
        for bad in [
            "keel-v0.2.0",
            "v0.2.0",
            "0.0",
            "1.2.3.4",
            "1.2.x",
            "0.2.0-",
            "1.0.0+a..b",
        ] {
            let err = build(&[("KEEL_BUILD_VERSION", bad)]).unwrap_err().0;
            assert!(err.contains("KEEL_BUILD_VERSION"), "{bad}: {err}");
        }
    }

    #[test]
    fn revisions() {
        let forty = "a".repeat(40);
        for ok in ["abc1234", "ABCDEF0", forty.as_str(), "abc1234-dirty"] {
            assert!(build(&[("KEEL_BUILD_REVISION", ok)]).is_ok(), "{ok}");
        }
        let forty_one = "a".repeat(41);
        for bad in [
            "abc123",
            forty_one.as_str(),
            "abc123g",
            "abc1234-clean",
            "-dirty",
        ] {
            let err = build(&[("KEEL_BUILD_REVISION", bad)]).unwrap_err().0;
            assert!(err.contains("KEEL_BUILD_REVISION"), "{bad}: {err}");
        }
    }

    #[test]
    fn dates_sources_and_digests_refuse_by_name() {
        for (k, v) in [
            ("KEEL_BUILD_DATE", "2026-10-28"),
            ("KEEL_BUILD_DATE", "yesterday"),
            ("KEEL_BUILD_SOURCE", "http://github.com/mbround18/fathom"),
            ("KEEL_BUILD_SOURCE", "github.com/mbround18/fathom"),
            ("KEEL_IMAGE_DIGEST", "sha256:abc"),
            ("KEEL_IMAGE_DIGEST", &DIGEST.to_uppercase()),
            ("KEEL_IMAGE_DIGEST", "sha512:0123"),
        ] {
            let err = build(&[(k, v)]).unwrap_err().0;
            assert!(err.contains(k), "{k}={v}: {err}");
        }
    }

    #[test]
    fn a_flag_digest_wins() {
        let other = format!("sha256:{}", "f".repeat(64));
        let b = build(&[("KEEL_IMAGE_DIGEST", DIGEST)]).unwrap();
        assert_eq!(
            b.clone().with_digest(Some(other.clone())).digest,
            Some(other)
        );
        assert_eq!(b.with_digest(None).digest.as_deref(), Some(DIGEST));
    }

    #[test]
    fn the_platform_is_the_running_one() {
        // The `std::env::consts` to OCI mapping is the crate's, tested there.
        let b = build(&[]).unwrap();
        assert_eq!(b.platform, Platform::current());
        assert_eq!(b.platform.os, std::env::consts::OS);
    }
}
