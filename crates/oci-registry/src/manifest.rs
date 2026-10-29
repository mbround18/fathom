//! Index, manifest and config parsing, and platform selection (contract rule 6).
use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::RegistryError;
use crate::reference::Digest;

/// An image platform, as an index entry or config names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: String,
    pub architecture: String,
    pub variant: Option<String>,
}

impl Platform {
    /// From `std::env::consts`: `x86_64` → `amd64`, `aarch64` → `arm64` (variant `v8`).
    pub fn current() -> Platform {
        Platform::from_consts(std::env::consts::OS, std::env::consts::ARCH)
    }

    fn from_consts(os: &str, arch: &str) -> Platform {
        let (architecture, variant) = match arch {
            "x86_64" => ("amd64", None),
            "aarch64" => ("arm64", Some("v8")),
            "x86" => ("386", None),
            other => (other, None),
        };
        Platform {
            os: os.to_string(),
            architecture: architecture.to_string(),
            variant: variant.map(str::to_string),
        }
    }
}

#[derive(Deserialize)]
struct RawDescriptor {
    digest: String,
    platform: Option<RawPlatform>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct RawPlatform {
    os: String,
    architecture: String,
    variant: Option<String>,
}

#[derive(Deserialize)]
struct RawManifest {
    manifests: Option<Vec<RawDescriptor>>,
    config: Option<RawDescriptor>,
}

/// A manifest-endpoint body: an index (or manifest list), or a single image manifest.
#[derive(Debug)]
pub(crate) enum ManifestBody {
    Index(Vec<IndexEntry>),
    Image { config: Digest },
}

#[derive(Debug)]
pub(crate) struct IndexEntry {
    digest: Digest,
    platform: Option<Platform>,
    attestation: bool,
}

/// Parse an OCI index, Docker manifest list, OCI manifest or Docker v2 manifest.
pub(crate) fn parse_manifest(body: &[u8]) -> Result<ManifestBody, RegistryError> {
    let raw: RawManifest = serde_json::from_slice(body)
        .map_err(|_| RegistryError::BadResponse("malformed manifest"))?;
    let digest =
        |s: &str| Digest::parse(s).map_err(|_| RegistryError::BadResponse("malformed digest"));
    if let Some(entries) = raw.manifests {
        let entries = entries
            .into_iter()
            .map(|e| {
                Ok(IndexEntry {
                    digest: digest(&e.digest)?,
                    attestation: e.annotations.contains_key("vnd.docker.reference.type"),
                    platform: e.platform.map(|p| Platform {
                        os: p.os,
                        architecture: p.architecture,
                        variant: p.variant.filter(|v| !v.is_empty()),
                    }),
                })
            })
            .collect::<Result<Vec<_>, RegistryError>>()?;
        return Ok(ManifestBody::Index(entries));
    }
    match raw.config {
        Some(c) => Ok(ManifestBody::Image {
            config: digest(&c.digest)?,
        }),
        None => Err(RegistryError::BadResponse("manifest without config")),
    }
}

/// Rule 6: the first entry whose os and architecture match, and whose variant matches when it has
/// one. Attestations (`unknown/unknown`, or annotated `vnd.docker.reference.type`) are skipped.
pub(crate) fn select_platform(
    entries: &[IndexEntry],
    want: &Platform,
) -> Result<Digest, RegistryError> {
    entries
        .iter()
        .filter(|e| !e.attestation)
        .find(|e| {
            e.platform.as_ref().is_some_and(|p| {
                !(p.os == "unknown" && p.architecture == "unknown")
                    && p.os == want.os
                    && p.architecture == want.architecture
                    && p.variant
                        .as_ref()
                        .is_none_or(|v| want.variant.as_ref() == Some(v))
            })
        })
        .map(|e| e.digest.clone())
        .ok_or(RegistryError::NoImageForPlatform)
}

#[derive(Deserialize)]
struct RawConfig {
    os: Option<String>,
    architecture: Option<String>,
    created: Option<String>,
    config: Option<RawConfigInner>,
}

#[derive(Deserialize)]
struct RawConfigInner {
    #[serde(rename = "Labels")]
    labels: Option<BTreeMap<String, String>>,
    #[serde(rename = "User")]
    user: Option<String>,
}

/// What `resolve` reads from an image config.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ImageConfig {
    pub os: String,
    pub architecture: String,
    pub labels: BTreeMap<String, String>,
    pub user: Option<String>,
    pub created: Option<String>,
}

pub(crate) fn parse_config(body: &[u8]) -> Result<ImageConfig, RegistryError> {
    let raw: RawConfig = serde_json::from_slice(body)
        .map_err(|_| RegistryError::BadResponse("malformed image config"))?;
    let (labels, user) = match raw.config {
        Some(c) => (
            c.labels.unwrap_or_default(),
            c.user.filter(|u| !u.is_empty()),
        ),
        None => (BTreeMap::new(), None),
    };
    Ok(ImageConfig {
        os: raw.os.unwrap_or_default(),
        architecture: raw.architecture.unwrap_or_default(),
        labels,
        user,
        created: raw.created,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(c: char) -> String {
        format!("sha256:{}", c.to_string().repeat(64))
    }

    fn plat(os: &str, arch: &str, variant: Option<&str>) -> Platform {
        Platform {
            os: os.into(),
            architecture: arch.into(),
            variant: variant.map(Into::into),
        }
    }

    #[test]
    fn current_platform_mapping() {
        assert_eq!(
            Platform::from_consts("linux", "x86_64"),
            plat("linux", "amd64", None)
        );
        assert_eq!(
            Platform::from_consts("linux", "aarch64"),
            plat("linux", "arm64", Some("v8"))
        );
        assert_eq!(Platform::current().os, std::env::consts::OS);
    }

    fn index() -> Vec<u8> {
        format!(
            r#"{{"schemaVersion":2,"manifests":[
              {{"digest":"{a}","platform":{{"os":"linux","architecture":"amd64"}},
                "annotations":{{"vnd.docker.reference.type":"attestation-manifest"}}}},
              {{"digest":"{b}","platform":{{"os":"unknown","architecture":"unknown"}}}},
              {{"digest":"{c}","platform":{{"os":"linux","architecture":"arm","variant":"v7"}}}},
              {{"digest":"{e}","platform":{{"os":"linux","architecture":"amd64"}}}},
              {{"digest":"{f}","platform":{{"os":"linux","architecture":"arm64","variant":"v8"}}}}
            ]}}"#,
            a = d('a'),
            b = d('b'),
            c = d('c'),
            e = d('e'),
            f = d('f')
        )
        .into_bytes()
    }

    #[test]
    fn selects_from_an_index() {
        let ManifestBody::Index(entries) = parse_manifest(&index()).unwrap() else {
            panic!("an index");
        };
        let pick = |p: Platform| select_platform(&entries, &p).map(|x| x.as_str().to_string());
        assert_eq!(pick(plat("linux", "amd64", None)), Ok(d('e')));
        assert_eq!(pick(plat("linux", "arm64", Some("v8"))), Ok(d('f')));
        assert_eq!(pick(plat("linux", "arm", Some("v7"))), Ok(d('c')));
        assert_eq!(
            pick(plat("linux", "arm", Some("v6"))),
            Err(RegistryError::NoImageForPlatform)
        );
        assert_eq!(
            pick(plat("unknown", "unknown", None)),
            Err(RegistryError::NoImageForPlatform)
        );
        assert_eq!(
            pick(plat("windows", "amd64", None)),
            Err(RegistryError::NoImageForPlatform)
        );
    }

    #[test]
    fn entry_without_variant_matches_any_variant() {
        let body = format!(
            r#"{{"manifests":[{{"digest":"{}","platform":{{"os":"linux","architecture":"arm64"}}}}]}}"#,
            d('1')
        );
        let ManifestBody::Index(entries) = parse_manifest(body.as_bytes()).unwrap() else {
            panic!("an index");
        };
        assert!(select_platform(&entries, &plat("linux", "arm64", Some("v8"))).is_ok());
    }

    #[test]
    fn single_manifest() {
        let body = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.docker.distribution.manifest.v2+json","config":{{"digest":"{}","size":1}},"layers":[]}}"#,
            d('9')
        );
        match parse_manifest(body.as_bytes()).unwrap() {
            ManifestBody::Image { config } => assert_eq!(config.as_str(), d('9')),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn malformed_manifests() {
        assert!(parse_manifest(b"nope").is_err());
        assert!(parse_manifest(b"{}").is_err());
        assert!(parse_manifest(br#"{"config":{"digest":"sha256:short"}}"#).is_err());
    }

    #[test]
    fn config_fields() {
        let c = parse_config(
            br#"{"os":"linux","architecture":"amd64","created":"2026-10-01T12:00:00Z",
                 "config":{"Labels":{"a":"b"},"User":"10001"}}"#,
        )
        .unwrap();
        assert_eq!(c.os, "linux");
        assert_eq!(c.architecture, "amd64");
        assert_eq!(c.labels.get("a").map(String::as_str), Some("b"));
        assert_eq!(c.user.as_deref(), Some("10001"));
        assert_eq!(c.created.as_deref(), Some("2026-10-01T12:00:00Z"));

        let bare =
            parse_config(br#"{"os":"linux","architecture":"amd64","config":{"Labels":null}}"#)
                .unwrap();
        assert!(bare.labels.is_empty());
        assert_eq!(bare.user, None);
        assert_eq!(bare.created, None);
    }
}
