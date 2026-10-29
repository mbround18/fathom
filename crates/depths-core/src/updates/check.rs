//! The registry check of one service (research R1, data-model "Service update state").
//!
//! HEAD first: a tag whose digest the running image already carries costs one HEAD and nothing
//! else. A new digest is resolved once, through the caller's resolution cache, and compared with
//! the running image the way the host's image store names it. Nothing here pulls, and a failure
//! never reports an update (FR-005).

use super::policy::EffectivePolicy;
use super::reference::ImageRef;
use super::registry::{Platform, Registry, RegistryError, Resolution};
use crate::ports::ImageStore;

/// Resolutions by [`cache_key`]. Content addressed, so an entry never goes stale. The server
/// keeps it in `update-state.json`.
pub trait ResolutionCache {
    fn get(&mut self, key: &str) -> Option<Resolution>;
    fn put(&mut self, key: String, resolution: Resolution);
}

/// A plain in-memory cache, for tests and one-off passes.
impl ResolutionCache for std::collections::HashMap<String, Resolution> {
    fn get(&mut self, key: &str) -> Option<Resolution> {
        std::collections::HashMap::get(self, key).cloned()
    }

    fn put(&mut self, key: String, resolution: Resolution) {
        self.insert(key, resolution);
    }
}

/// `docker.io/library/nginx@sha256:…`: the repository and the digest, never the tag.
pub fn cache_key(r: &ImageRef, digest: &str) -> String {
    format!("{}@{digest}", r.name())
}

/// What the service's container runs, from `docker image inspect`.
#[derive(Debug, Clone, Copy)]
pub struct Running<'a> {
    pub image_id: &'a str,
    /// `RepoDigests`, as Docker writes them (`nginx@sha256:…`).
    pub repo_digests: &'a [String],
}

/// One service to check.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    /// The compose `image:`, normalised.
    pub reference: &'a ImageRef,
    /// `None` when the service has no container.
    pub running: Option<Running<'a>>,
    /// Digests this service ran before (its history, research R5).
    pub ran_before: &'a [String],
    /// Digests rejected for this stack and service (a rollback or a reject).
    pub rejected: &'a [String],
}

/// The outcome of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The container runs what the tag points to. `digest` is the tag's digest.
    UpToDate { digest: String },
    /// The tag points to a different image for this host.
    Available {
        digest: String,
        resolution: Resolution,
    },
    /// The tag points back to an image this service ran before.
    MovedBack { digest: String },
    /// The tag points to an image rejected for this service (FR-015).
    Rejected { digest: String },
    /// The service has no container to compare with.
    NotRunning { digest: String },
    /// The registry read failed. The caller keeps the last known running and available fields.
    Failed(RegistryError),
}

/// The image store, detected from one image: under containerd the image id equals one of its
/// `RepoDigests` digests, under the classic store it never does.
pub fn detect_store(image_id: &str, repo_digests: &[String]) -> ImageStore {
    if repo_digests.iter().any(|d| digest_of(d) == Some(image_id)) {
        ImageStore::Containerd
    } else {
        ImageStore::Classic
    }
}

fn digest_of(repo_digest: &str) -> Option<&str> {
    repo_digest.rsplit_once('@').map(|(_, d)| d)
}

/// The running image already carries `<repo>@digest`.
fn carries(reference: &ImageRef, running: &Running<'_>, digest: &str) -> bool {
    if running.image_id == digest {
        return true;
    }
    let name = reference.name();
    running
        .repo_digests
        .iter()
        .any(|rd| match ImageRef::parse(rd) {
            Ok(r) => r.name() == name && r.digest.as_deref() == Some(digest),
            Err(_) => false,
        })
}

/// Checks services against one registry port, for one host platform and image store.
pub struct Checker<'a, C: ResolutionCache + ?Sized> {
    registry: &'a dyn Registry,
    platform: &'a Platform,
    store: ImageStore,
    cache: &'a mut C,
}

impl<'a, C: ResolutionCache + ?Sized> Checker<'a, C> {
    pub fn new(
        registry: &'a dyn Registry,
        platform: &'a Platform,
        store: ImageStore,
        cache: &'a mut C,
    ) -> Self {
        Self {
            registry,
            platform,
            store,
            cache,
        }
    }

    /// `None` when the policy excludes the service or turns checks off (FR-004): no request.
    pub async fn check_if(
        &mut self,
        policy: &EffectivePolicy,
        target: &Target<'_>,
    ) -> Option<Verdict> {
        if policy.is_checked() {
            Some(self.check(target).await)
        } else {
            None
        }
    }

    /// One HEAD, then at most one resolve per digest never seen before.
    pub async fn check(&mut self, target: &Target<'_>) -> Verdict {
        let r = target.reference;
        let digest = match self.registry.head_digest(r).await {
            Ok(d) => d,
            Err(e) => return Verdict::Failed(e),
        };
        let Some(running) = target.running else {
            return Verdict::NotRunning { digest };
        };
        if carries(r, &running, &digest) {
            return Verdict::UpToDate { digest };
        }
        if target.rejected.contains(&digest) {
            return Verdict::Rejected { digest };
        }
        if target.ran_before.contains(&digest) {
            return Verdict::MovedBack { digest };
        }
        let resolution = match self.resolve(r, &digest).await {
            Ok(res) => res,
            Err(e) => return Verdict::Failed(e),
        };
        match self.same_image(r, &running, &resolution).await {
            Ok(true) => Verdict::UpToDate { digest },
            Ok(false) => Verdict::Available { digest, resolution },
            Err(e) => Verdict::Failed(e),
        }
    }

    async fn resolve(&mut self, r: &ImageRef, digest: &str) -> Result<Resolution, RegistryError> {
        let key = cache_key(r, digest);
        if let Some(hit) = self.cache.get(&key) {
            return Ok(hit);
        }
        let res = self.registry.resolve(r, digest, self.platform).await?;
        self.cache.put(key, res.clone());
        Ok(res)
    }

    /// The host would run the same image (edge case "same image, new digest").
    async fn same_image(
        &mut self,
        r: &ImageRef,
        running: &Running<'_>,
        new: &Resolution,
    ) -> Result<bool, RegistryError> {
        match self.store {
            // The image id is the config digest.
            ImageStore::Classic => Ok(running.image_id == new.config_digest),
            // The image id is the index or manifest it was pulled by: compare platform manifests,
            // the running side resolved once from its own digest.
            ImageStore::Containerd => {
                if running.image_id == new.manifest_digest {
                    return Ok(true);
                }
                match self.resolve(r, running.image_id).await {
                    Ok(old) => Ok(old.manifest_digest == new.manifest_digest),
                    // The old image is gone from the registry: it can't be the new one.
                    Err(RegistryError::NotFound) => Ok(false),
                    Err(e) => Err(e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use super::*;
    use crate::updates::policy::{Exclusion, PolicySource, UpdatePolicy};
    use crate::updates::registry::fake::FakeRegistry;

    fn ready<T>(f: impl Future<Output = T>) -> T {
        let mut f = pin!(f);
        match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("check was pending on the fake registry"),
        }
    }

    fn d(n: u8) -> String {
        format!("sha256:{}", format!("{n:02x}").repeat(32))
    }

    fn amd64() -> Platform {
        Platform {
            os: "linux".into(),
            architecture: "amd64".into(),
            variant: None,
        }
    }

    fn resolution(index: Option<String>, manifest: String, config: String) -> Resolution {
        Resolution {
            index_digest: index,
            manifest_digest: manifest,
            config_digest: config,
            labels: BTreeMap::new(),
            user: None,
            created: None,
        }
    }

    fn nginx() -> ImageRef {
        ImageRef::parse("nginx:1.27").unwrap()
    }

    fn target<'a>(
        r: &'a ImageRef,
        running: Option<Running<'a>>,
        ran_before: &'a [String],
        rejected: &'a [String],
    ) -> Target<'a> {
        Target {
            reference: r,
            running,
            ran_before,
            rejected,
        }
    }

    fn check(
        reg: &FakeRegistry,
        store: ImageStore,
        cache: &mut HashMap<String, Resolution>,
        t: &Target<'_>,
    ) -> Verdict {
        let p = amd64();
        ready(Checker::new(reg, &p, store, cache).check(t))
    }

    #[test]
    fn unchanged_tag_is_one_head_and_no_resolve() {
        let reg = FakeRegistry::default();
        let r = nginx();
        reg.set_head(&r, Ok(d(1)));
        let digests = vec![format!("nginx@{}", d(1))];
        let running = Running {
            image_id: &d(9),
            repo_digests: &digests,
        };
        let v = check(
            &reg,
            ImageStore::Classic,
            &mut HashMap::new(),
            &target(&r, Some(running), &[], &[]),
        );
        assert_eq!(v, Verdict::UpToDate { digest: d(1) });
        assert_eq!((reg.heads(), reg.resolves()), (1, 0));
    }

    #[test]
    fn moved_tag_resolves_each_new_digest_once_for_every_service() {
        let reg = FakeRegistry::default();
        let r = nginx();
        reg.set_head(&r, Ok(d(2)));
        reg.set_resolution(&d(2), Ok(resolution(Some(d(2)), d(3), d(4))));
        let old = vec![format!("nginx@{}", d(1))];
        let mut cache = HashMap::new();
        for _ in 0..5 {
            let running = Running {
                image_id: &d(9),
                repo_digests: &old,
            };
            let v = check(
                &reg,
                ImageStore::Classic,
                &mut cache,
                &target(&r, Some(running), &[], &[]),
            );
            assert!(matches!(v, Verdict::Available { ref digest, .. } if *digest == d(2)));
        }
        assert_eq!((reg.heads(), reg.resolves()), (5, 1));
        // A second pass re-uses the cache.
        let running = Running {
            image_id: &d(9),
            repo_digests: &old,
        };
        check(
            &reg,
            ImageStore::Classic,
            &mut cache,
            &target(&r, Some(running), &[], &[]),
        );
        assert_eq!(reg.resolves(), 1);
    }

    #[test]
    fn multi_arch_repush_with_the_same_host_manifest_is_no_update() {
        // Classic store: the running image id is the config digest, unchanged by the re-push.
        let reg = FakeRegistry::default();
        let r = nginx();
        reg.set_head(&r, Ok(d(2)));
        reg.set_resolution(&d(2), Ok(resolution(Some(d(2)), d(3), d(4))));
        let old = vec![format!("nginx@{}", d(1))];
        let running = Running {
            image_id: &d(4),
            repo_digests: &old,
        };
        let mut cache = HashMap::new();
        let t = target(&r, Some(running), &[], &[]);
        assert_eq!(
            check(&reg, ImageStore::Classic, &mut cache, &t),
            Verdict::UpToDate { digest: d(2) }
        );
        // The next HEAD short-circuits through the cached resolution.
        assert_eq!(
            check(&reg, ImageStore::Classic, &mut cache, &t),
            Verdict::UpToDate { digest: d(2) }
        );
        assert_eq!(reg.resolves(), 1);
    }

    #[test]
    fn containerd_store_compares_platform_manifests() {
        let reg = FakeRegistry::default();
        let r = nginx();
        // Re-pushed index d2 carries the same amd64 manifest d3 as the running index d1.
        reg.set_head(&r, Ok(d(2)));
        reg.set_resolution(&d(2), Ok(resolution(Some(d(2)), d(3), d(4))));
        reg.set_resolution(&d(1), Ok(resolution(Some(d(1)), d(3), d(4))));
        let digests = vec![format!("nginx@{}", d(1))];
        let running = Running {
            image_id: &d(1),
            repo_digests: &digests,
        };
        assert_eq!(detect_store(&d(1), &digests), ImageStore::Containerd);
        let mut cache = HashMap::new();
        let t = target(&r, Some(running), &[], &[]);
        assert_eq!(
            check(&reg, ImageStore::Containerd, &mut cache, &t),
            Verdict::UpToDate { digest: d(2) }
        );
        assert_eq!(
            reg.resolves(),
            2,
            "new index and the running one, once each"
        );
        check(&reg, ImageStore::Containerd, &mut cache, &t);
        assert_eq!(reg.resolves(), 2);

        // A new amd64 manifest is an update.
        reg.set_head(&r, Ok(d(5)));
        reg.set_resolution(&d(5), Ok(resolution(Some(d(5)), d(6), d(7))));
        assert!(matches!(
            check(&reg, ImageStore::Containerd, &mut cache, &t),
            Verdict::Available { .. }
        ));
    }

    #[test]
    fn classic_store_is_detected_when_no_repo_digest_is_the_id() {
        let digests = vec![format!("nginx@{}", d(1))];
        assert_eq!(detect_store(&d(4), &digests), ImageStore::Classic);
    }

    #[test]
    fn moved_back_and_rejected() {
        let reg = FakeRegistry::default();
        let r = nginx();
        let now = vec![format!("nginx@{}", d(2))];
        let running = Running {
            image_id: &d(9),
            repo_digests: &now,
        };
        let history = vec![d(1)];
        let rejected = vec![d(3)];
        let mut cache = HashMap::new();

        reg.set_head(&r, Ok(d(1)));
        let t = target(&r, Some(running), &history, &rejected);
        assert_eq!(
            check(&reg, ImageStore::Classic, &mut cache, &t),
            Verdict::MovedBack { digest: d(1) }
        );
        reg.set_head(&r, Ok(d(3)));
        assert_eq!(
            check(&reg, ImageStore::Classic, &mut cache, &t),
            Verdict::Rejected { digest: d(3) }
        );
        assert_eq!(reg.resolves(), 0);

        // A newer digest for the tag is available again (FR-015).
        reg.set_head(&r, Ok(d(4)));
        reg.set_resolution(&d(4), Ok(resolution(None, d(4), d(5))));
        assert!(matches!(
            check(&reg, ImageStore::Classic, &mut cache, &t),
            Verdict::Available { ref digest, .. } if *digest == d(4)
        ));
    }

    #[test]
    fn excluded_services_are_never_checked() {
        let reg = FakeRegistry::default();
        let r = nginx();
        let p = amd64();
        let mut cache = HashMap::new();
        let mut checker = Checker::new(&reg, &p, ImageStore::Classic, &mut cache);
        let t = target(&r, None, &[], &[]);
        for policy in [
            EffectivePolicy {
                policy: UpdatePolicy::Off,
                source: PolicySource::Default,
                excluded: Some(Exclusion::PinnedByDigest),
                labels: vec![],
            },
            EffectivePolicy {
                policy: UpdatePolicy::Off,
                source: PolicySource::StackPolicy,
                excluded: None,
                labels: vec![],
            },
        ] {
            assert_eq!(ready(checker.check_if(&policy, &t)), None);
        }
        assert_eq!(reg.heads(), 0);
    }

    #[test]
    fn cant_check_errors_fail_without_resolving() {
        let reg = FakeRegistry::default();
        let r = nginx();
        let mut cache = HashMap::new();
        for err in [
            RegistryError::NoDigestHeader,
            RegistryError::CredentialsUnsupported(
                "credential helper `pass` is not supported in the Depths container".into(),
            ),
        ] {
            reg.set_head(&r, Err(err.clone()));
            let v = check(
                &reg,
                ImageStore::Classic,
                &mut cache,
                &target(&r, None, &[], &[]),
            );
            assert_eq!(v, Verdict::Failed(err.clone()));
            assert!(err.to_string().starts_with("can't check: "));
            assert!(!err.starts_backoff());
        }
        assert_eq!(reg.resolves(), 0);
    }

    #[test]
    fn rate_limit_and_unreachable_fail_and_never_report_available() {
        let reg = FakeRegistry::default();
        let r = nginx();
        let old = vec![format!("nginx@{}", d(1))];
        let running = Running {
            image_id: &d(9),
            repo_digests: &old,
        };
        let mut cache = HashMap::new();
        for err in [
            RegistryError::RateLimited,
            RegistryError::Unreachable("timed out".into()),
        ] {
            reg.set_head(&r, Err(err.clone()));
            let v = check(
                &reg,
                ImageStore::Classic,
                &mut cache,
                &target(&r, Some(running), &[], &[]),
            );
            assert_eq!(v, Verdict::Failed(err.clone()));
            assert!(err.starts_backoff());
        }
        // A failed resolve after a moved HEAD is a failure too, not an update.
        reg.set_head(&r, Ok(d(2)));
        reg.set_resolution(&d(2), Err(RegistryError::RateLimited));
        assert_eq!(
            check(
                &reg,
                ImageStore::Classic,
                &mut cache,
                &target(&r, Some(running), &[], &[])
            ),
            Verdict::Failed(RegistryError::RateLimited)
        );
    }

    #[test]
    fn no_container_is_not_running() {
        let reg = FakeRegistry::default();
        let r = nginx();
        reg.set_head(&r, Ok(d(1)));
        assert_eq!(
            check(
                &reg,
                ImageStore::Classic,
                &mut HashMap::new(),
                &target(&r, None, &[], &[])
            ),
            Verdict::NotRunning { digest: d(1) }
        );
        assert_eq!(reg.resolves(), 0);
    }

    /// contracts/registry.md "Request budget": 30 stacks, one service each, every 6 h, 7 days.
    #[test]
    fn a_week_of_checks_stays_inside_the_request_budget() {
        const PASSES: usize = 7 * 24 / 6;
        let refs: Vec<ImageRef> = (0..30)
            .map(|i| ImageRef::parse(&format!("ghcr.io/acme/app{i}:stable")).unwrap())
            .collect();
        let shared = ImageRef::parse("redis:7").unwrap();
        let digests: Vec<Vec<String>> = refs
            .iter()
            .map(|r| vec![format!("{}@{}", r.name(), d(1))])
            .collect();

        // No tag moves: HEADs only.
        let reg = FakeRegistry::default();
        for r in &refs {
            reg.set_head(r, Ok(d(1)));
        }
        let mut cache = HashMap::new();
        for _ in 0..PASSES {
            for (r, rd) in refs.iter().zip(&digests) {
                let running = Running {
                    image_id: &d(9),
                    repo_digests: rd,
                };
                check(
                    &reg,
                    ImageStore::Classic,
                    &mut cache,
                    &target(r, Some(running), &[], &[]),
                );
            }
        }
        assert_eq!((reg.heads(), reg.resolves()), (840, 0));

        // Ten stacks share an image whose tag moves once: still one resolve.
        let reg = FakeRegistry::default();
        for r in &refs[10..] {
            reg.set_head(r, Ok(d(1)));
        }
        reg.set_head(&shared, Ok(d(1)));
        let shared_old = vec![format!("{}@{}", shared.name(), d(1))];
        let mut cache = HashMap::new();
        for pass in 0..PASSES {
            if pass == 3 {
                reg.set_head(&shared, Ok(d(2)));
                reg.set_resolution(&d(2), Ok(resolution(Some(d(2)), d(3), d(4))));
            }
            for (i, (r, rd)) in refs.iter().zip(&digests).enumerate() {
                let (r, rd) = if i < 10 {
                    (&shared, &shared_old)
                } else {
                    (r, rd)
                };
                let running = Running {
                    image_id: &d(9),
                    repo_digests: rd,
                };
                check(
                    &reg,
                    ImageStore::Classic,
                    &mut cache,
                    &target(r, Some(running), &[], &[]),
                );
            }
        }
        assert_eq!((reg.heads(), reg.resolves()), (840, 1));

        // No digest on HEAD: HEADs only, never a resolve.
        let reg = FakeRegistry::default();
        for r in &refs {
            reg.set_head(r, Err(RegistryError::NoDigestHeader));
        }
        for _ in 0..PASSES {
            for r in &refs {
                check(
                    &reg,
                    ImageStore::Classic,
                    &mut cache,
                    &target(r, None, &[], &[]),
                );
            }
        }
        assert_eq!((reg.heads(), reg.resolves()), (840, 0));
    }
}
