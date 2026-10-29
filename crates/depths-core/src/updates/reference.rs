//! Image references as compose writes them, with Docker's normalisation (data-model.md "Image
//! reference"). Pure: it decides exclusions and keep-tag names without I/O. The server converts
//! it to the registry adapter's own type for requests.

use std::fmt;

/// Docker Hub's canonical registry name. `index.docker.io` is read as this.
pub const DOCKER_HUB: &str = "docker.io";

/// A compose `image:` reference, normalised.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageRef {
    /// `docker.io`, `ghcr.io`, `registry.example:5000`.
    pub registry: String,
    /// `library/nginx`.
    pub repository: String,
    /// `1.27`; `None` only when the reference names a digest and no tag.
    pub tag: Option<String>,
    /// `sha256:…` when the reference pins one.
    pub digest: Option<String>,
}

/// The reference is not one Docker would accept (excluded `bad_reference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadReference {
    pub reason: &'static str,
}

impl fmt::Display for BadReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a valid image reference: {}", self.reason)
    }
}

impl std::error::Error for BadReference {}

fn bad(reason: &'static str) -> BadReference {
    BadReference { reason }
}

impl ImageRef {
    /// Parse a compose `image:` value.
    pub fn parse(s: &str) -> Result<Self, BadReference> {
        if s.is_empty() {
            return Err(bad("empty"));
        }
        if s.len() > 4096 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(bad("whitespace or control character"));
        }
        let (name_tag, digest) = match s.split_once('@') {
            Some((n, d)) => {
                if !valid_digest(d) {
                    return Err(bad("digest"));
                }
                (n, Some(d.to_owned()))
            }
            None => (s, None),
        };
        // A tag is a `:` after the last `/`; a `:` before it belongs to a registry port.
        let last_slash = name_tag.rfind('/').map_or(0, |i| i + 1);
        let (name, tag) = match name_tag[last_slash..].rfind(':') {
            Some(i) => {
                let at = last_slash + i;
                (&name_tag[..at], Some(&name_tag[at + 1..]))
            }
            None => (name_tag, None),
        };
        if let Some(t) = tag
            && !valid_tag(t)
        {
            return Err(bad("tag"));
        }
        let (registry, path) = match name.split_once('/') {
            Some((first, rest)) if is_registry(first) => (first, rest),
            _ => (DOCKER_HUB, name),
        };
        if !valid_registry(registry) {
            return Err(bad("registry"));
        }
        let registry = if registry == "index.docker.io" {
            DOCKER_HUB
        } else {
            registry
        };
        if path.is_empty() || !path.split('/').all(valid_component) {
            return Err(bad("repository"));
        }
        let repository = if registry == DOCKER_HUB && !path.contains('/') {
            format!("library/{path}")
        } else {
            path.to_owned()
        };
        if repository.len() > 255 {
            return Err(bad("repository too long"));
        }
        let tag = match (tag, &digest) {
            (Some(t), _) => Some(t.to_owned()),
            (None, Some(_)) => None,
            (None, None) => Some("latest".to_owned()),
        };
        Ok(Self {
            registry: registry.to_owned(),
            repository,
            tag,
            digest,
        })
    }

    /// The reference pins a digest: excluded `pinned_by_digest` (FR-004).
    pub fn is_pinned(&self) -> bool {
        self.digest.is_some()
    }

    /// `registry/repository`, without tag or digest.
    pub fn name(&self) -> String {
        format!("{}/{}", self.registry, self.repository)
    }
}

impl fmt::Display for ImageRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.registry, self.repository)?;
        if let Some(t) = &self.tag {
            write!(f, ":{t}")?;
        }
        if let Some(d) = &self.digest {
            write!(f, "@{d}")?;
        }
        Ok(())
    }
}

/// Docker's rule: the first component is a registry when it has a `.` or a `:`, or is
/// `localhost`.
fn is_registry(first: &str) -> bool {
    first.contains('.') || first.contains(':') || first == "localhost"
}

fn valid_registry(r: &str) -> bool {
    let (host, port) = match r.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (r, None),
    };
    if let Some(p) = port
        && (p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    !host.is_empty()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

/// A path component: lower-case alphanumerics separated by `.`, `_`, `__` or runs of `-`.
fn valid_component(c: &str) -> bool {
    let b = c.as_bytes();
    if b.is_empty() || !is_lower_alnum(b[0]) || !is_lower_alnum(b[b.len() - 1]) {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        if is_lower_alnum(b[i]) {
            i += 1;
            continue;
        }
        let sep_start = i;
        while i < b.len() && !is_lower_alnum(b[i]) {
            i += 1;
        }
        let sep = &c[sep_start..i];
        let ok = sep == "." || sep == "_" || sep == "__" || sep.bytes().all(|x| x == b'-');
        if !ok {
            return false;
        }
    }
    true
}

fn is_lower_alnum(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit()
}

fn valid_tag(t: &str) -> bool {
    let b = t.as_bytes();
    !b.is_empty()
        && b.len() <= 128
        && (b[0].is_ascii_alphanumeric() || b[0] == b'_')
        && b.iter()
            .all(|&x| x.is_ascii_alphanumeric() || matches!(x, b'_' | b'.' | b'-'))
}

fn valid_digest(d: &str) -> bool {
    let Some((alg, hex)) = d.split_once(':') else {
        return false;
    };
    let len = match alg {
        "sha256" => 64,
        "sha512" => 128,
        _ => return false,
    };
    hex.len() == len
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn r(registry: &str, repository: &str, tag: Option<&str>, digest: Option<&str>) -> ImageRef {
        ImageRef {
            registry: registry.into(),
            repository: repository.into(),
            tag: tag.map(Into::into),
            digest: digest.map(Into::into),
        }
    }

    #[test]
    fn normalisation_table() {
        let host_digest = format!("host:5000/a/b@{D}");
        let both = format!("x:1@{D}");
        let cases: Vec<(&str, ImageRef)> = vec![
            (
                "nginx",
                r("docker.io", "library/nginx", Some("latest"), None),
            ),
            (
                "nginx:1.27",
                r("docker.io", "library/nginx", Some("1.27"), None),
            ),
            (
                "library/nginx",
                r("docker.io", "library/nginx", Some("latest"), None),
            ),
            (
                "docker.io/nginx",
                r("docker.io", "library/nginx", Some("latest"), None),
            ),
            (
                "index.docker.io/library/nginx",
                r("docker.io", "library/nginx", Some("latest"), None),
            ),
            ("ghcr.io/a/b:c", r("ghcr.io", "a/b", Some("c"), None)),
            (
                "localhost:5000/x",
                r("localhost:5000", "x", Some("latest"), None),
            ),
            ("localhost/x", r("localhost", "x", Some("latest"), None)),
            (&host_digest, r("host:5000", "a/b", None, Some(D))),
            (&both, r("docker.io", "library/x", Some("1"), Some(D))),
            (
                "someone/app:v2",
                r("docker.io", "someone/app", Some("v2"), None),
            ),
        ];
        for (input, want) in cases {
            assert_eq!(ImageRef::parse(input).as_ref(), Ok(&want), "{input}");
        }
    }

    #[test]
    fn digest_means_pinned() {
        assert!(
            ImageRef::parse(&format!("host:5000/a/b@{D}"))
                .unwrap()
                .is_pinned()
        );
        // Both a tag and a digest: the digest wins, and the service is excluded.
        assert!(ImageRef::parse(&format!("x:1@{D}")).unwrap().is_pinned());
        assert!(!ImageRef::parse("nginx:1.27").unwrap().is_pinned());
    }

    #[test]
    fn invalid_references_are_refused() {
        for input in [
            "Nginx",
            "ghcr.io/A/b",
            "nginx:",
            "",
            "nginx@sha256:abc",
            "nginx@sha256:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef",
            "a//b",
            "-x",
            "x y",
            "host:port/x",
            "nginx:-bad",
        ] {
            assert!(
                ImageRef::parse(input).is_err(),
                "{input:?} should be refused"
            );
        }
    }

    #[test]
    fn display_round_trips() {
        for input in ["nginx", "ghcr.io/a/b:c", "localhost:5000/x"] {
            let parsed = ImageRef::parse(input).unwrap();
            assert_eq!(ImageRef::parse(&parsed.to_string()).unwrap(), parsed);
        }
        assert_eq!(
            ImageRef::parse("nginx").unwrap().to_string(),
            "docker.io/library/nginx:latest"
        );
        assert_eq!(
            ImageRef::parse(&format!("x:1@{D}")).unwrap().to_string(),
            format!("docker.io/library/x:1@{D}")
        );
        assert_eq!(
            ImageRef::parse("ghcr.io/a/b:c").unwrap().name(),
            "ghcr.io/a/b"
        );
    }
}
