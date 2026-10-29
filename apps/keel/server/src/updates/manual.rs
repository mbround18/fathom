//! The manual update path (spec 061 US5, FR-021): when a prerequisite for the update run is
//! missing, owners get the exact image to deploy and the steps to deploy it, rendered here so
//! every client shows the same text.

use keel_protocol::UpdatePath;
use oci_registry::ImageRef;

/// The registry Docker Hub references resolve to; it is left out of `digest_ref`.
const DOCKER_HUB: &str = "registry-1.docker.io";

/// `<repository>@<digest>`: the watched repository by digest, as compose and `docker pull` take
/// it. Docker Hub repositories drop the registry (`mbround18/keel@sha256:…`); others keep it.
/// Without a digest, just the repository.
#[must_use]
pub fn digest_ref(watched: &ImageRef, digest: Option<&str>) -> String {
    let repo = if watched.registry() == DOCKER_HUB {
        watched
            .repository()
            .strip_prefix("library/")
            .unwrap_or(watched.repository())
            .to_owned()
    } else {
        format!("{}/{}", watched.registry(), watched.repository())
    };
    match digest {
        Some(d) => format!("{repo}@{d}"),
        None => repo,
    }
}

/// The steps to deploy `digest_ref` by hand: the deployment repository's one command, then the
/// plain compose ones.
#[must_use]
pub fn steps(digest_ref: &str, digest: Option<&str>) -> Vec<String> {
    let digest = digest.unwrap_or("sha256:…");
    vec![
        format!("With the Ansible repository: make keel-server KEEL_DIGEST={digest}"),
        format!("Or with plain compose: set the keel service's image: to {digest_ref}"),
        "docker compose pull".to_owned(),
        "docker compose up -d".to_owned(),
    ]
}

/// `UpdatePath::Manual` for `missing` (`no_system_workspace`, `no_keel_host`,
/// `profile_not_allowed` or `job_missing_param`).
#[must_use]
pub fn path(missing: &str, watched: &ImageRef, digest: Option<&str>) -> UpdatePath {
    let digest_ref = digest_ref(watched, digest);
    UpdatePath::Manual {
        missing: missing.to_owned(),
        steps: steps(&digest_ref, digest),
        digest_ref,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";

    #[test]
    fn docker_hub_drops_the_registry() {
        let r = ImageRef::parse("mbround18/keel:latest").unwrap();
        assert_eq!(digest_ref(&r, Some(D)), format!("mbround18/keel@{D}"));
        let r = ImageRef::parse("nginx:1").unwrap();
        assert_eq!(digest_ref(&r, Some(D)), format!("nginx@{D}"));
    }

    #[test]
    fn other_registries_keep_it() {
        let r = ImageRef::parse("ghcr.io/mbround18/keel:latest").unwrap();
        assert_eq!(
            digest_ref(&r, Some(D)),
            format!("ghcr.io/mbround18/keel@{D}")
        );
    }

    #[test]
    fn steps_name_the_digest_and_the_reference() {
        let r = ImageRef::parse("mbround18/keel:latest").unwrap();
        let UpdatePath::Manual {
            missing,
            digest_ref,
            steps,
        } = path("no_keel_host", &r, Some(D))
        else {
            panic!("manual");
        };
        assert_eq!(missing, "no_keel_host");
        assert!(steps[0].contains(&format!("make keel-server KEEL_DIGEST={D}")));
        assert!(steps[1].contains(&digest_ref));
        assert_eq!(steps[2], "docker compose pull");
        assert_eq!(steps[3], "docker compose up -d");
    }
}
