//! Image references: `[registry/]repository[:tag|@digest]`, with Docker Hub's defaults.
use std::fmt;

use crate::error::RegistryError;

/// Docker Hub's registry API host.
pub const DOCKER_HUB: &str = "registry-1.docker.io";

/// `sha256:` followed by 64 lowercase hex characters. Parse-validated.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest(String);

impl Digest {
    pub fn parse(s: &str) -> Result<Digest, RegistryError> {
        let hex = s
            .strip_prefix("sha256:")
            .ok_or_else(|| RegistryError::InvalidReference("digest is not sha256".into()))?;
        if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(RegistryError::InvalidReference(
                "digest is not 64 lowercase hex characters".into(),
            ));
        }
        Ok(Digest(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    Tag(String),
    Digest(Digest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    registry: String,
    repository: String,
    reference: Reference,
}

impl ImageRef {
    /// `[registry/]repository[:tag|@digest]`. Docker Hub defaults: no registry, `docker.io` or
    /// `index.docker.io` mean `registry-1.docker.io`; a single-segment repository on Docker Hub
    /// gets `library/`; no tag means `latest`.
    pub fn parse(s: &str) -> Result<ImageRef, RegistryError> {
        let bad = |why: &str| RegistryError::InvalidReference(format!("{why}: {s:?}"));
        if s.is_empty() {
            return Err(bad("empty reference"));
        }
        if s.contains("://") {
            return Err(bad("a reference has no scheme"));
        }
        let (name, reference) = match s.split_once('@') {
            Some((name, digest)) => (name, Reference::Digest(Digest::parse(digest)?)),
            None => {
                let after_slash = s.rfind('/').map_or(0, |i| i + 1);
                match s[after_slash..].rfind(':') {
                    Some(i) => {
                        let i = after_slash + i;
                        let tag = &s[i + 1..];
                        if !valid_tag(tag) {
                            return Err(bad("invalid tag"));
                        }
                        (&s[..i], Reference::Tag(tag.to_string()))
                    }
                    None => (s, Reference::Tag("latest".to_string())),
                }
            }
        };
        let (registry, repository) = match name.split_once('/') {
            Some((first, rest))
                if first.contains('.') || first.contains(':') || first == "localhost" =>
            {
                (first, rest)
            }
            _ => ("", name),
        };
        let registry = match registry {
            "" | "docker.io" | "index.docker.io" => DOCKER_HUB,
            r => r,
        };
        if !registry
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
        {
            return Err(bad("invalid registry"));
        }
        if !repository.split('/').all(valid_component) {
            return Err(bad("invalid repository"));
        }
        let repository = if registry == DOCKER_HUB && !repository.contains('/') {
            format!("library/{repository}")
        } else {
            repository.to_string()
        };
        Ok(ImageRef {
            registry: registry.to_string(),
            repository,
            reference,
        })
    }

    /// API host[:port], e.g. `registry-1.docker.io`.
    pub fn registry(&self) -> &str {
        &self.registry
    }

    /// e.g. `mbround18/keel`, `library/nginx`.
    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn reference(&self) -> &Reference {
        &self.reference
    }

    /// The same registry and repository, by digest.
    pub fn with_digest(&self, d: Digest) -> ImageRef {
        ImageRef {
            registry: self.registry.clone(),
            repository: self.repository.clone(),
            reference: Reference::Digest(d),
        }
    }

    /// The tag or digest as it goes in a manifest URL.
    pub(crate) fn reference_str(&self) -> &str {
        match &self.reference {
            Reference::Tag(t) => t,
            Reference::Digest(d) => d.as_str(),
        }
    }
}

impl fmt::Display for ImageRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sep = match self.reference {
            Reference::Tag(_) => ':',
            Reference::Digest(_) => '@',
        };
        write!(
            f,
            "{}/{}{sep}{}",
            self.registry,
            self.repository,
            self.reference_str()
        )
    }
}

/// A path component: lowercase alphanumerics, separated by `.`, `_` or `-`.
fn valid_component(c: &str) -> bool {
    let b = c.as_bytes();
    !b.is_empty()
        && b.iter()
            .all(|x| matches!(x, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
        && b[0].is_ascii_alphanumeric()
        && b[b.len() - 1].is_ascii_alphanumeric()
}

/// `[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}`.
fn valid_tag(t: &str) -> bool {
    let b = t.as_bytes();
    !b.is_empty()
        && b.len() <= 128
        && (b[0].is_ascii_alphanumeric() || b[0] == b'_')
        && b.iter()
            .all(|x| x.is_ascii_alphanumeric() || matches!(x, b'_' | b'.' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn tag(s: &str) -> Reference {
        Reference::Tag(s.to_string())
    }

    fn check(input: &str, registry: &str, repository: &str, reference: Reference) {
        let r = ImageRef::parse(input).unwrap_or_else(|e| panic!("{input}: {e}"));
        assert_eq!(r.registry(), registry, "{input}");
        assert_eq!(r.repository(), repository, "{input}");
        assert_eq!(r.reference(), &reference, "{input}");
    }

    #[test]
    fn contract_table() {
        check(
            "mbround18/keel:latest",
            DOCKER_HUB,
            "mbround18/keel",
            tag("latest"),
        );
        check("nginx", DOCKER_HUB, "library/nginx", tag("latest"));
        check(
            "docker.io/library/foo",
            DOCKER_HUB,
            "library/foo",
            tag("latest"),
        );
        check(
            &format!("index.docker.io/mbround18/keel@sha256:{HEX}"),
            DOCKER_HUB,
            "mbround18/keel",
            Reference::Digest(Digest::parse(&format!("sha256:{HEX}")).unwrap()),
        );
        check("ghcr.io/org/keel:edge", "ghcr.io", "org/keel", tag("edge"));
        check(
            "registry.example:5000/keel",
            "registry.example:5000",
            "keel",
            tag("latest"),
        );
    }

    #[test]
    fn docker_hub_aliases() {
        check(
            "docker.io/nginx",
            DOCKER_HUB,
            "library/nginx",
            tag("latest"),
        );
        check(
            "index.docker.io/nginx:1",
            DOCKER_HUB,
            "library/nginx",
            tag("1"),
        );
        check(
            "registry-1.docker.io/nginx",
            DOCKER_HUB,
            "library/nginx",
            tag("latest"),
        );
    }

    #[test]
    fn library_only_on_docker_hub() {
        check("ghcr.io/keel", "ghcr.io", "keel", tag("latest"));
        check("localhost/keel", "localhost", "keel", tag("latest"));
    }

    #[test]
    fn first_segment_is_a_registry_only_with_dot_colon_or_localhost() {
        check("localhost:5000/a/b:t", "localhost:5000", "a/b", tag("t"));
        check("localhost/a", "localhost", "a", tag("latest"));
        // No dot, colon or localhost: a Docker Hub namespace.
        check("myorg/app", DOCKER_HUB, "myorg/app", tag("latest"));
        check(
            "myorg/team/app",
            DOCKER_HUB,
            "myorg/team/app",
            tag("latest"),
        );
    }

    #[test]
    fn port_colon_is_not_a_tag() {
        check("reg.example:5000/a", "reg.example:5000", "a", tag("latest"));
        check("reg.example:5000/a:v1", "reg.example:5000", "a", tag("v1"));
    }

    #[test]
    fn refused() {
        let long = "a".repeat(129);
        for bad in [
            "",
            "Mbround18/keel",
            "mbround18/Keel:latest",
            &format!("keel:{long}"),
            "keel:",
            "keel:-bad",
            "keel@sha256:abc",
            &format!("keel@sha256:{}", HEX.to_uppercase()),
            &format!("keel@sha512:{HEX}"),
            "a//b",
            "/a",
            "a/",
            "http://ghcr.io/a",
            "ghcr.io/",
            "keel:has space",
        ] {
            assert!(
                matches!(
                    ImageRef::parse(bad),
                    Err(RegistryError::InvalidReference(_))
                ),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn longest_tag_is_accepted() {
        let t = "a".repeat(128);
        check(&format!("keel:{t}"), DOCKER_HUB, "library/keel", tag(&t));
    }

    #[test]
    fn digest_parse() {
        let ok = format!("sha256:{HEX}");
        assert_eq!(Digest::parse(&ok).unwrap().as_str(), ok);
        for bad in [
            "",
            "sha256:",
            &format!("sha256:{}", &HEX[..63]),
            &format!("sha256:{HEX}0"),
            &format!("sha256:{}", HEX.to_uppercase()),
            &format!("sha512:{HEX}"),
            &format!("SHA256:{HEX}"),
            &format!(" sha256:{HEX}"),
            &format!("sha256:{}g", &HEX[..63]),
        ] {
            assert!(Digest::parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn with_digest_keeps_registry_and_repository() {
        let r = ImageRef::parse("ghcr.io/org/keel:edge").unwrap();
        let d = Digest::parse(&format!("sha256:{HEX}")).unwrap();
        let by = r.with_digest(d.clone());
        assert_eq!(by.registry(), "ghcr.io");
        assert_eq!(by.repository(), "org/keel");
        assert_eq!(by.reference(), &Reference::Digest(d));
    }

    #[test]
    fn display() {
        assert_eq!(
            ImageRef::parse("nginx").unwrap().to_string(),
            "registry-1.docker.io/library/nginx:latest"
        );
        assert_eq!(
            ImageRef::parse(&format!("ghcr.io/org/keel@sha256:{HEX}"))
                .unwrap()
                .to_string(),
            format!("ghcr.io/org/keel@sha256:{HEX}")
        );
    }
}
