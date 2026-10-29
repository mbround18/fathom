//! The `Registry` port (contracts/registry.md §2): how the check logic reads a registry.
//!
//! It returns boxed futures so it can be held as `Arc<dyn Registry>` without `async-trait`. It
//! takes no credentials: the server's adapter looks them up per registry.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use super::reference::ImageRef;

/// What every port method returns.
pub type RegistryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, RegistryError>> + Send + 'a>>;

/// Read access to an OCI registry.
pub trait Registry: Send + Sync {
    /// HEAD the tag and return its `Docker-Content-Digest`. Never GETs.
    fn head_digest<'a>(&'a self, r: &'a ImageRef) -> RegistryFuture<'a, String>;
    /// Resolve a digest of the reference's repository for a platform: the platform manifest,
    /// its config, labels, user and creation time. Docker Hub counts this as one pull.
    fn resolve<'a>(
        &'a self,
        r: &'a ImageRef,
        digest: &'a str,
        p: &'a Platform,
    ) -> RegistryFuture<'a, Resolution>;
    /// The `Retry-After` of the last rate-limited answer from `registry`, when it sent one. The
    /// scheduler's backoff honours it; without one the backoff is its own (research R5).
    fn retry_after(&self, _registry: &str) -> Option<std::time::Duration> {
        None
    }
}

/// The host's platform, as `docker info` reports it, normalised to OCI names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Platform {
    pub os: String,
    pub architecture: String,
    pub variant: Option<String>,
}

/// A digest resolved for one platform. Content addressed: never refreshed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The index digest when the tag names an index; `None` for a single manifest.
    pub index_digest: Option<String>,
    pub manifest_digest: String,
    pub config_digest: String,
    pub labels: BTreeMap<String, String>,
    pub user: Option<String>,
    pub created: Option<String>,
}

/// Why a registry read failed. Each becomes `check_failed` with a reason; the service keeps its
/// last known state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    AuthFailed,
    NotFound,
    RateLimited,
    /// Timeout or network failure.
    Unreachable(String),
    BadResponse(String),
    /// The registry sent no `Docker-Content-Digest`: "can't check", never pulled to find out.
    NoDigestHeader,
    NoImageForPlatform,
    /// A credential helper or identity token in `config.json`: "can't check".
    CredentialsUnsupported(String),
    /// Docker reaches the registry over plain http (loopback or `insecure-registries`); update
    /// checks speak https only: "can't check".
    PlainHttp,
}

impl RegistryError {
    /// Only these start the per-registry backoff (research R5).
    pub fn starts_backoff(&self) -> bool {
        matches!(self, Self::RateLimited | Self::Unreachable(_))
    }

    /// Stable wire code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::AuthFailed => "auth_failed",
            Self::NotFound => "not_found",
            Self::RateLimited => "rate_limited",
            Self::Unreachable(_) => "unreachable",
            Self::BadResponse(_) => "bad_response",
            Self::NoDigestHeader => "no_digest_header",
            Self::NoImageForPlatform => "no_image_for_platform",
            Self::CredentialsUnsupported(_) => "credentials_unsupported",
            Self::PlainHttp => "plain_http",
        }
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthFailed => f.write_str("the registry refused the credentials"),
            Self::NotFound => f.write_str("the registry has no such image or tag"),
            Self::RateLimited => f.write_str("the registry is rate limiting requests"),
            Self::Unreachable(why) => write!(f, "the registry is unreachable: {why}"),
            Self::BadResponse(why) => write!(f, "the registry sent a bad response: {why}"),
            Self::NoDigestHeader => {
                f.write_str("can't check: the registry does not report a digest for a tag")
            }
            Self::NoImageForPlatform => f.write_str("the image has no build for this host"),
            Self::CredentialsUnsupported(why) => write!(f, "can't check: {why}"),
            Self::PlainHttp => f.write_str(
                "can't check: the registry is reached over plain http, which update checks don't use",
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

/// A scripted registry for core tests: digests per reference, resolutions per digest, and call
/// counts.
#[cfg(test)]
pub(crate) mod fake {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    pub(crate) struct FakeRegistry {
        heads: Mutex<HashMap<String, Result<String, RegistryError>>>,
        resolutions: Mutex<HashMap<String, Result<Resolution, RegistryError>>>,
        pub(crate) head_calls: AtomicUsize,
        pub(crate) resolve_calls: AtomicUsize,
    }

    impl FakeRegistry {
        /// The tag of `r` now points at `digest` (or fails).
        pub(crate) fn set_head(&self, r: &ImageRef, answer: Result<String, RegistryError>) {
            self.heads.lock().unwrap().insert(r.to_string(), answer);
        }

        /// Resolving `digest` answers this.
        pub(crate) fn set_resolution(
            &self,
            digest: &str,
            answer: Result<Resolution, RegistryError>,
        ) {
            self.resolutions
                .lock()
                .unwrap()
                .insert(digest.to_owned(), answer);
        }

        pub(crate) fn heads(&self) -> usize {
            self.head_calls.load(Ordering::SeqCst)
        }

        pub(crate) fn resolves(&self) -> usize {
            self.resolve_calls.load(Ordering::SeqCst)
        }
    }

    impl Registry for FakeRegistry {
        fn head_digest<'a>(&'a self, r: &'a ImageRef) -> RegistryFuture<'a, String> {
            self.head_calls.fetch_add(1, Ordering::SeqCst);
            let answer = self
                .heads
                .lock()
                .unwrap()
                .get(&r.to_string())
                .cloned()
                .unwrap_or(Err(RegistryError::NotFound));
            Box::pin(async move { answer })
        }

        fn resolve<'a>(
            &'a self,
            _r: &'a ImageRef,
            digest: &'a str,
            _p: &'a Platform,
        ) -> RegistryFuture<'a, Resolution> {
            self.resolve_calls.fetch_add(1, Ordering::SeqCst);
            let answer = self
                .resolutions
                .lock()
                .unwrap()
                .get(digest)
                .cloned()
                .unwrap_or(Err(RegistryError::NotFound));
            Box::pin(async move { answer })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};

    use super::fake::FakeRegistry;
    use super::*;

    /// The fake's futures are ready at once; no runtime needed.
    fn ready<T>(f: RegistryFuture<'_, T>) -> Result<T, RegistryError> {
        let mut f = f;
        match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("fake registry future was pending"),
        }
    }

    fn platform() -> Platform {
        Platform {
            os: "linux".into(),
            architecture: "amd64".into(),
            variant: None,
        }
    }

    #[test]
    fn fake_serves_scripted_digests_and_counts_calls() {
        let reg = Arc::new(FakeRegistry::default());
        let nginx = ImageRef::parse("nginx:1.27").unwrap();
        reg.set_head(&nginx, Ok("sha256:aa".into()));
        let res = Resolution {
            index_digest: Some("sha256:aa".into()),
            manifest_digest: "sha256:bb".into(),
            config_digest: "sha256:cc".into(),
            labels: BTreeMap::new(),
            user: None,
            created: None,
        };
        reg.set_resolution("sha256:aa", Ok(res.clone()));

        let port: Arc<dyn Registry> = reg.clone();
        assert_eq!(ready(port.head_digest(&nginx)), Ok("sha256:aa".into()));
        assert_eq!(
            ready(port.resolve(&nginx, "sha256:aa", &platform())),
            Ok(res)
        );
        let other = ImageRef::parse("redis").unwrap();
        assert_eq!(
            ready(port.head_digest(&other)),
            Err(RegistryError::NotFound)
        );
        assert_eq!((reg.heads(), reg.resolves()), (2, 1));
    }

    #[test]
    fn only_rate_limits_and_network_failures_back_off() {
        let backs_off = [
            RegistryError::RateLimited,
            RegistryError::Unreachable("timeout".into()),
        ];
        let does_not = [
            RegistryError::AuthFailed,
            RegistryError::NotFound,
            RegistryError::BadResponse("x".into()),
            RegistryError::NoDigestHeader,
            RegistryError::NoImageForPlatform,
            RegistryError::CredentialsUnsupported("helper".into()),
            RegistryError::PlainHttp,
        ];
        assert!(backs_off.iter().all(RegistryError::starts_backoff));
        assert!(!does_not.iter().any(RegistryError::starts_backoff));
        assert!(
            RegistryError::NoDigestHeader
                .to_string()
                .starts_with("can't check")
        );
        assert!(
            RegistryError::CredentialsUnsupported("credential helpers".into())
                .to_string()
                .starts_with("can't check")
        );
    }
}
