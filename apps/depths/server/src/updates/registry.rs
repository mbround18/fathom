//! depths-core's `Registry` port over the shared `oci-registry` client (contracts/registry.md §3).
//!
//! Credentials are read from `$DOCKER_CONFIG/config.json` for every call (never cached across
//! passes). Every call's `RequestCounts` go to `depths.updates.registry_requests`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use depths_core::updates::reference::ImageRef;
use depths_core::updates::registry::{
    Platform, Registry, RegistryError, RegistryFuture, Resolution,
};
use oci_registry::{Client, Counted, RequestCounts};

use super::creds::CredsSnapshot;
use crate::telemetry::{self, RegistryRequest};

/// Where request counts go: the metric, or a test's recorder.
pub type CountSink = Box<dyn Fn(&str, RequestCounts) + Send + Sync>;

fn to_metric() -> CountSink {
    Box::new(|registry, c| {
        telemetry::record_registry_requests(registry, RegistryRequest::Head, c.head.into());
        telemetry::record_registry_requests(registry, RegistryRequest::Get, c.get.into());
        telemetry::record_registry_requests(
            registry,
            RegistryRequest::ManifestGet,
            c.manifest_get.into(),
        );
    })
}

/// The user agent of every update-check request.
pub fn user_agent() -> String {
    format!("depths/{} (update-check)", env!("CARGO_PKG_VERSION"))
}

/// The port over one `oci_registry::Client`.
pub struct OciRegistry {
    client: Client,
    docker_config: PathBuf,
    /// Registries Docker reaches over plain http, beyond loopback.
    insecure: Vec<String>,
    counts: CountSink,
    retry_after: Mutex<HashMap<String, Duration>>,
}

impl OciRegistry {
    /// The production adapter: `https` only, counts to the metric.
    pub fn new(docker_config: PathBuf) -> Result<Self, String> {
        let client = Client::new(&user_agent()).map_err(|e| e.to_string())?;
        Ok(Self::with_client(client, docker_config, to_metric()))
    }

    pub fn with_client(client: Client, docker_config: PathBuf, counts: CountSink) -> Self {
        Self {
            client,
            docker_config,
            insecure: Vec::new(),
            counts,
            retry_after: Mutex::new(HashMap::new()),
        }
    }

    /// Hosts Docker reaches over plain http (`insecure-registries`).
    pub fn with_insecure(mut self, hosts: Vec<String>) -> Self {
        self.insecure = hosts;
        self
    }

    fn plain_http(&self, host: &str) -> bool {
        is_loopback(host) || self.insecure.iter().any(|h| h == host)
    }

    /// Credentials and the crate's reference, or why the call can't be made.
    fn prepare(
        &self,
        r: &ImageRef,
    ) -> Result<(oci_registry::ImageRef, oci_registry::Credentials), RegistryError> {
        if self.plain_http(&r.registry) {
            return Err(RegistryError::PlainHttp);
        }
        let creds = CredsSnapshot::load(&self.docker_config).for_host(&r.registry)?;
        Ok((to_oci(r)?, creds))
    }

    fn finish<T>(&self, r: &ImageRef, counted: Counted<T>) -> Result<T, RegistryError> {
        (self.counts)(&r.registry, counted.requests);
        counted.result.map_err(|e| {
            if let oci_registry::RegistryError::RateLimited {
                retry_after: Some(d),
            } = &e
            {
                self.retry_after
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(r.registry.clone(), *d);
            }
            map_error(e)
        })
    }
}

impl Registry for OciRegistry {
    fn head_digest<'a>(&'a self, r: &'a ImageRef) -> RegistryFuture<'a, String> {
        Box::pin(async move {
            let (oci, creds) = self.prepare(r)?;
            let counted = self.client.head_digest(&oci, &creds).await;
            self.finish(r, counted).map(|d| d.as_str().to_owned())
        })
    }

    fn resolve<'a>(
        &'a self,
        r: &'a ImageRef,
        digest: &'a str,
        p: &'a Platform,
    ) -> RegistryFuture<'a, Resolution> {
        Box::pin(async move {
            let (oci, creds) = self.prepare(r)?;
            let digest = oci_registry::Digest::parse(digest).map_err(map_error)?;
            let counted = self
                .client
                .resolve(&oci.with_digest(digest), &to_oci_platform(p), &creds)
                .await;
            self.finish(r, counted).map(|res| Resolution {
                index_digest: res.index_digest.map(|d| d.as_str().to_owned()),
                manifest_digest: res.manifest_digest.as_str().to_owned(),
                config_digest: res.config_digest.as_str().to_owned(),
                labels: res.labels,
                user: res.user,
                created: res.created,
            })
        })
    }

    fn retry_after(&self, registry: &str) -> Option<Duration> {
        self.retry_after
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(registry)
    }
}

fn is_loopback(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host.rsplit_once(':').map_or(host, |(h, _)| h),
    };
    name == "localhost" || name == "::1" || name.starts_with("127.")
}

/// depths-core's reference as the crate reads it.
pub fn to_oci(r: &ImageRef) -> Result<oci_registry::ImageRef, RegistryError> {
    oci_registry::ImageRef::parse(&r.to_string()).map_err(map_error)
}

pub fn to_oci_platform(p: &Platform) -> oci_registry::Platform {
    oci_registry::Platform {
        os: p.os.clone(),
        architecture: p.architecture.clone(),
        variant: p.variant.clone(),
    }
}

/// contracts/registry.md §3 "Error mapping".
pub fn map_error(e: oci_registry::RegistryError) -> RegistryError {
    use oci_registry::RegistryError as O;
    match e {
        O::AuthFailed => RegistryError::AuthFailed,
        O::NotFound => RegistryError::NotFound,
        O::RateLimited { .. } => RegistryError::RateLimited,
        O::NoDigestHeader => RegistryError::NoDigestHeader,
        O::NoImageForPlatform => RegistryError::NoImageForPlatform,
        O::Timeout => RegistryError::Unreachable("the request timed out".into()),
        O::Network => RegistryError::Unreachable("could not connect".into()),
        O::BadResponse(why) => RegistryError::BadResponse(why.to_owned()),
        e @ (O::InvalidReference(_) | O::NotByDigest) => {
            tracing::error!(code = e.code(), error = %e, "update check built a bad registry request");
            RegistryError::BadResponse(e.to_string())
        }
    }
}

/// A port that always answers `Unreachable`, for when the client cannot be built.
pub struct Unavailable(pub String);

impl Registry for Unavailable {
    fn head_digest<'a>(&'a self, _: &'a ImageRef) -> RegistryFuture<'a, String> {
        let why = self.0.clone();
        Box::pin(async move { Err(RegistryError::Unreachable(why)) })
    }

    fn resolve<'a>(
        &'a self,
        _: &'a ImageRef,
        _: &'a str,
        _: &'a Platform,
    ) -> RegistryFuture<'a, Resolution> {
        let why = self.0.clone();
        Box::pin(async move { Err(RegistryError::Unreachable(why)) })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base64::Engine as _;
    use oci_registry::testing::{FakeRegistry, ImageSpec};

    use super::*;

    type Log = Arc<Mutex<Vec<(String, RequestCounts)>>>;

    fn adapter(fake: &FakeRegistry, dir: &std::path::Path) -> (OciRegistry, Log) {
        let log: Log = Arc::default();
        let sink = log.clone();
        let client = Client::with_base_url(&user_agent(), fake.url());
        let a = OciRegistry::with_client(
            client,
            dir.to_path_buf(),
            Box::new(move |reg, c| sink.lock().unwrap().push((reg.to_owned(), c))),
        );
        (a, log)
    }

    fn amd64() -> Platform {
        Platform {
            os: "linux".into(),
            architecture: "amd64".into(),
            variant: None,
        }
    }

    fn r(s: &str) -> ImageRef {
        ImageRef::parse(s).unwrap()
    }

    #[tokio::test]
    async fn head_and_resolve_against_the_fake_registry_count_requests() {
        let fake = FakeRegistry::start().await;
        let dir = tempfile::tempdir().unwrap();
        let multi = fake.multi_arch(
            "acme/app",
            "1",
            &[("org.opencontainers.image.version", "1.2.3")],
        );
        let (a, log) = adapter(&fake, dir.path());
        let reference = r("registry.example/acme/app:1");

        let digest = a.head_digest(&reference).await.unwrap();
        assert_eq!(digest, multi.index.as_str());
        let res = a.resolve(&reference, &digest, &amd64()).await.unwrap();
        assert_eq!(res.index_digest.as_deref(), Some(digest.as_str()));
        assert_eq!(res.manifest_digest, multi.amd64.manifest.as_str());
        assert_eq!(res.config_digest, multi.amd64.config.as_str());
        assert_eq!(
            res.labels
                .get("org.opencontainers.image.version")
                .map(String::as_str),
            Some("1.2.3")
        );
        assert_eq!(res.user.as_deref(), Some("10001"));

        let log = log.lock().unwrap();
        assert_eq!(log.len(), 2);
        assert!(log.iter().all(|(reg, _)| reg == "registry.example"));
        assert!(log[0].1.head >= 1, "{:?}", log[0].1);
        assert_eq!(log[0].1.manifest_get, 0);
        assert_eq!(log[1].1.head, 0);
        assert_eq!(log[1].1.manifest_get, 2, "index then platform manifest");
    }

    #[tokio::test]
    async fn error_mapping_through_the_fake_registry() {
        let fake = FakeRegistry::start().await;
        let dir = tempfile::tempdir().unwrap();
        fake.image("acme/app", "1", &ImageSpec::new("linux", "amd64"));
        let (a, log) = adapter(&fake, dir.path());
        let reference = r("registry.example/acme/app:1");
        let path = "/v2/acme/app/manifests/1";

        assert_eq!(
            a.head_digest(&r("registry.example/acme/missing:1"))
                .await
                .unwrap_err(),
            RegistryError::NotFound
        );

        fake.set_status_with_headers(path, 429, &[("Retry-After", "120")]);
        assert_eq!(
            a.head_digest(&reference).await.unwrap_err(),
            RegistryError::RateLimited
        );
        assert_eq!(
            a.retry_after("registry.example"),
            Some(Duration::from_secs(120))
        );
        assert_eq!(a.retry_after("registry.example"), None, "taken once");

        fake.set_status(path, 503);
        assert!(matches!(
            a.head_digest(&reference).await.unwrap_err(),
            RegistryError::Unreachable(_) | RegistryError::BadResponse(_)
        ));
        fake.clear_status(path);

        fake.omit_digest_header(true);
        let e = a.head_digest(&reference).await.unwrap_err();
        assert_eq!(e, RegistryError::NoDigestHeader);
        assert!(e.to_string().starts_with("can't check"));
        fake.omit_digest_header(false);

        // Failures are counted too.
        assert!(log.lock().unwrap().len() >= 4);
    }

    #[tokio::test]
    async fn credentials_from_config_json_reach_the_registry() {
        let fake = FakeRegistry::start().await;
        let dir = tempfile::tempdir().unwrap();
        fake.image("acme/app", "1", &ImageSpec::new("linux", "amd64"));
        fake.require_basic("alice", "s3cret");
        let (a, _) = adapter(&fake, dir.path());
        let reference = r("registry.example/acme/app:1");

        assert_eq!(
            a.head_digest(&reference).await.unwrap_err(),
            RegistryError::AuthFailed
        );
        let auth = base64::engine::general_purpose::STANDARD.encode("alice:s3cret");
        std::fs::write(
            dir.path().join("config.json"),
            format!(r#"{{"auths": {{"registry.example": {{"auth": "{auth}"}}}}}}"#),
        )
        .unwrap();
        assert!(a.head_digest(&reference).await.is_ok());

        std::fs::write(
            dir.path().join("config.json"),
            r#"{"credHelpers": {"registry.example": "pass"}}"#,
        )
        .unwrap();
        let before = fake.requests().len();
        assert!(matches!(
            a.head_digest(&reference).await.unwrap_err(),
            RegistryError::CredentialsUnsupported(_)
        ));
        assert_eq!(fake.requests().len(), before, "no request when can't check");
    }

    #[tokio::test]
    async fn plain_http_registries_cant_be_checked() {
        let fake = FakeRegistry::start().await;
        let dir = tempfile::tempdir().unwrap();
        let (a, log) = adapter(&fake, dir.path());
        let a = a.with_insecure(vec!["insecure.lan:5000".into()]);
        for s in [
            "localhost:5000/x:1",
            "127.0.0.1:5000/x:1",
            "insecure.lan:5000/x:1",
        ] {
            let e = a.head_digest(&r(s)).await.unwrap_err();
            assert_eq!(e, RegistryError::PlainHttp, "{s}");
            assert!(e.to_string().starts_with("can't check"));
        }
        assert!(fake.requests().is_empty());
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn error_mapping_is_the_contract_table() {
        use oci_registry::RegistryError as O;
        assert_eq!(map_error(O::AuthFailed), RegistryError::AuthFailed);
        assert_eq!(map_error(O::NotFound), RegistryError::NotFound);
        assert_eq!(
            map_error(O::RateLimited { retry_after: None }),
            RegistryError::RateLimited
        );
        assert_eq!(map_error(O::NoDigestHeader), RegistryError::NoDigestHeader);
        assert_eq!(
            map_error(O::NoImageForPlatform),
            RegistryError::NoImageForPlatform
        );
        assert!(matches!(
            map_error(O::Timeout),
            RegistryError::Unreachable(_)
        ));
        assert!(matches!(
            map_error(O::Network),
            RegistryError::Unreachable(_)
        ));
        for e in [
            O::BadResponse("x"),
            O::InvalidReference("y".into()),
            O::NotByDigest,
        ] {
            assert!(matches!(map_error(e), RegistryError::BadResponse(_)));
        }
    }

    /// data-model.md "Image reference": both parsers agree.
    #[test]
    fn both_parsers_agree_on_the_reference_table() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        let table = [
            "nginx".to_owned(),
            "nginx:1.27".to_owned(),
            "library/nginx".to_owned(),
            "docker.io/nginx".to_owned(),
            "index.docker.io/library/nginx".to_owned(),
            "ghcr.io/a/b:c".to_owned(),
            "localhost:5000/x".to_owned(),
            format!("host:5000/a/b@{digest}"),
        ];
        for s in &table {
            let core = ImageRef::parse(s).unwrap();
            let oci = to_oci(&core).unwrap();
            let api_host = if core.registry == "docker.io" {
                oci_registry::reference::DOCKER_HUB
            } else {
                core.registry.as_str()
            };
            assert_eq!(oci.registry(), api_host, "{s}");
            assert_eq!(oci.repository(), core.repository, "{s}");
            let core_reference = core
                .digest
                .clone()
                .or(core.tag.clone())
                .expect("a tag or digest");
            let oci_reference = match oci.reference() {
                oci_registry::Reference::Tag(t) => t.clone(),
                oci_registry::Reference::Digest(d) => d.as_str().to_owned(),
            };
            assert_eq!(oci_reference, core_reference, "{s}");
        }
        assert!(ImageRef::parse("Nginx").is_err());
    }

    #[test]
    fn loopback_hosts() {
        for h in ["localhost:5000", "127.0.0.1", "[::1]:5000", "localhost"] {
            assert!(is_loopback(h), "{h}");
        }
        for h in ["ghcr.io", "docker.io", "registry.example:5000"] {
            assert!(!is_loopback(h), "{h}");
        }
    }

    #[test]
    fn the_host_platform_reaches_the_client_unchanged() {
        let p = Platform {
            os: "linux".into(),
            architecture: "arm64".into(),
            variant: Some("v8".into()),
        };
        let o = to_oci_platform(&p);
        assert_eq!(
            (o.os.as_str(), o.architecture.as_str(), o.variant.as_deref()),
            ("linux", "arm64", Some("v8"))
        );
    }
}
