//! The crate's contract tests, against `FakeRegistry` only. Each asserts the fake's request log and
//! the returned `RequestCounts` exactly (contracts/registry.md "Requests").
use std::time::Duration;

use oci_registry::testing::{FakeRegistry, ImageSpec, LoggedRequest, media};
use oci_registry::{Client, Credentials, Digest, ImageRef, Platform, RegistryError, RequestCounts};
use secrecy::SecretString;

const UA: &str = "oci-registry-test/0 (test)";
const REPO: &str = "acme/app";

fn amd64() -> Platform {
    Platform {
        os: "linux".into(),
        architecture: "amd64".into(),
        variant: None,
    }
}

fn arm64() -> Platform {
    Platform {
        os: "linux".into(),
        architecture: "arm64".into(),
        variant: Some("v8".into()),
    }
}

fn tag_ref(tag: &str) -> ImageRef {
    ImageRef::parse(&format!("{REPO}:{tag}")).unwrap()
}

fn digest_ref(d: &Digest) -> ImageRef {
    tag_ref("latest").with_digest(d.clone())
}

fn manifest_path(reference: &str) -> String {
    format!("/v2/{REPO}/manifests/{reference}")
}

fn blob_path(d: &Digest) -> String {
    format!("/v2/{REPO}/blobs/{}", d.as_str())
}

fn req(method: &str, path: &str, auth: bool) -> LoggedRequest {
    LoggedRequest::new(method, path, auth)
}

fn counts(head: u32, get: u32, manifest_get: u32) -> RequestCounts {
    RequestCounts {
        head,
        get,
        manifest_get,
    }
}

async fn setup() -> (FakeRegistry, Client) {
    let fake = FakeRegistry::start().await;
    let client = Client::with_base_url(UA, fake.url());
    (fake, client)
}

/// HEAD the tag once so the client holds a token, then clear the log.
async fn warm(fake: &FakeRegistry, client: &Client, tag: &str) {
    let r = client
        .head_digest(&tag_ref(tag), &Credentials::Anonymous)
        .await;
    r.result.expect("warm-up HEAD");
    fake.clear_requests();
}

fn labelled() -> ImageSpec {
    ImageSpec::new("linux", "amd64")
        .label("org.opencontainers.image.version", "0.2.0")
        .label("org.opencontainers.image.revision", "abc1234")
        .user("10001")
        .created("2026-10-01T12:00:00Z")
}

#[tokio::test]
async fn head_digest_anonymous_then_cached_token() {
    let (fake, client) = setup().await;
    let image = fake.image(REPO, "latest", &labelled());

    let out = client
        .head_digest(&tag_ref("latest"), &Credentials::Anonymous)
        .await;
    assert_eq!(out.result, Ok(image.manifest.clone()));
    assert_eq!(out.requests, counts(2, 1, 0));
    assert_eq!(
        fake.requests(),
        vec![
            req("HEAD", &manifest_path("latest"), false),
            req("GET", "/token", false),
            req("HEAD", &manifest_path("latest"), true),
        ]
    );

    fake.clear_requests();
    let again = client
        .head_digest(&tag_ref("latest"), &Credentials::Anonymous)
        .await;
    assert_eq!(again.result, Ok(image.manifest));
    assert_eq!(again.requests, counts(1, 0, 0));
    assert_eq!(
        fake.requests(),
        vec![req("HEAD", &manifest_path("latest"), true)]
    );
}

#[tokio::test]
async fn head_digest_never_gets_a_manifest() {
    let (fake, client) = setup().await;
    fake.image(REPO, "latest", &labelled());
    fake.omit_digest_header(true);

    let out = client
        .head_digest(&tag_ref("latest"), &Credentials::Anonymous)
        .await;
    assert_eq!(out.result, Err(RegistryError::NoDigestHeader));
    assert_eq!(out.requests, counts(2, 1, 0));
    assert!(
        !fake
            .requests()
            .iter()
            .any(|r| r.method == "GET" && r.path.contains("/manifests/")),
        "{:?}",
        fake.requests()
    );
}

#[tokio::test]
async fn basic_credentials_go_only_to_the_token_endpoint() {
    let (fake, client) = setup().await;
    let image = fake.image(REPO, "latest", &labelled());
    fake.require_basic("bob", "hunter2-very-secret");
    let creds = Credentials::Basic {
        username: "bob".into(),
        password: SecretString::from("hunter2-very-secret".to_string()),
    };

    let debug = format!("{creds:?}");
    assert!(!debug.contains("hunter2"), "{debug}");
    assert!(
        debug.contains("bob") && debug.contains("[redacted]"),
        "{debug}"
    );

    let out = client.head_digest(&tag_ref("latest"), &creds).await;
    // The fake only accepts Basic on /token, and only an issued Bearer token on /v2/.
    assert_eq!(out.result, Ok(image.manifest));
    assert_eq!(
        fake.requests(),
        vec![
            req("HEAD", &manifest_path("latest"), false),
            req("GET", "/token", true),
            req("HEAD", &manifest_path("latest"), true),
        ]
    );
    assert!(!format!("{:?}", out.result).contains("hunter2"));
}

#[tokio::test]
async fn wrong_basic_credentials_are_auth_failed() {
    let (fake, client) = setup().await;
    fake.image(REPO, "latest", &labelled());
    fake.require_basic("bob", "right");
    let creds = Credentials::Basic {
        username: "bob".into(),
        password: SecretString::from("wrong".to_string()),
    };
    let out = client.head_digest(&tag_ref("latest"), &creds).await;
    assert_eq!(out.result, Err(RegistryError::AuthFailed));
    assert_eq!(out.requests, counts(1, 1, 0));
}

#[tokio::test]
async fn resolve_index_picks_the_platform_and_skips_attestations() {
    let (fake, client) = setup().await;
    let ma = fake.multi_arch(
        REPO,
        "latest",
        &[("org.opencontainers.image.version", "0.2.0")],
    );
    warm(&fake, &client, "latest").await;

    let out = client
        .resolve(&digest_ref(&ma.index), &amd64(), &Credentials::Anonymous)
        .await;
    assert_eq!(out.requests, counts(0, 3, 2));
    assert_eq!(
        fake.requests(),
        vec![
            req("GET", &manifest_path(ma.index.as_str()), true),
            req("GET", &manifest_path(ma.amd64.manifest.as_str()), true),
            req("GET", &blob_path(&ma.amd64.config), true),
        ]
    );
    let r = out.result.expect("resolved");
    assert_eq!(r.index_digest, Some(ma.index.clone()));
    assert_eq!(r.manifest_digest, ma.amd64.manifest);
    assert_eq!(r.config_digest, ma.amd64.config);
    assert_eq!(
        r.labels
            .get("org.opencontainers.image.version")
            .map(String::as_str),
        Some("0.2.0")
    );
    assert_eq!(r.user.as_deref(), Some("10001"));
    assert_eq!(r.created.as_deref(), Some("2026-10-01T12:00:00Z"));
    for att in &ma.attestations {
        assert!(
            !fake
                .requests()
                .iter()
                .any(|q| q.path.contains(att.as_str()))
        );
    }

    let arm = client
        .resolve(&digest_ref(&ma.index), &arm64(), &Credentials::Anonymous)
        .await
        .result
        .expect("arm64");
    assert_eq!(arm.manifest_digest, ma.arm64.manifest);
    assert_eq!(arm.config_digest, ma.arm64.config);
}

#[tokio::test]
async fn resolve_cold_takes_a_token_like_head() {
    let (fake, client) = setup().await;
    let image = fake.image(REPO, "latest", &labelled());
    let out = client
        .resolve(
            &digest_ref(&image.manifest),
            &amd64(),
            &Credentials::Anonymous,
        )
        .await;
    assert!(out.result.is_ok(), "{:?}", out.result);
    let m = manifest_path(image.manifest.as_str());
    assert_eq!(
        fake.requests(),
        vec![
            req("GET", &m, false),
            req("GET", "/token", false),
            req("GET", &m, true),
            req("GET", &blob_path(&image.config), true),
        ]
    );
    // Every GET on /manifests/ is counted, the challenged one included.
    assert_eq!(out.requests, counts(0, 4, 2));
}

#[tokio::test]
async fn resolve_single_manifest() {
    let (fake, client) = setup().await;
    let image = fake.image(REPO, "latest", &labelled());
    warm(&fake, &client, "latest").await;

    let out = client
        .resolve(
            &digest_ref(&image.manifest),
            &amd64(),
            &Credentials::Anonymous,
        )
        .await;
    assert_eq!(out.requests, counts(0, 2, 1));
    let r = out.result.expect("resolved");
    assert_eq!(r.index_digest, None);
    assert_eq!(r.manifest_digest, image.manifest);
    assert_eq!(r.config_digest, image.config);
    assert_eq!(r.labels.len(), 2);

    let other = client
        .resolve(
            &digest_ref(&image.manifest),
            &arm64(),
            &Credentials::Anonymous,
        )
        .await;
    assert_eq!(other.result, Err(RegistryError::NoImageForPlatform));
    assert_eq!(other.requests, counts(0, 2, 1));
}

#[tokio::test]
async fn index_without_the_platform() {
    let (fake, client) = setup().await;
    let index = fake.index_without_amd64(REPO, "latest");
    warm(&fake, &client, "latest").await;
    let out = client
        .resolve(&digest_ref(&index), &amd64(), &Credentials::Anonymous)
        .await;
    assert_eq!(out.result, Err(RegistryError::NoImageForPlatform));
    assert_eq!(out.requests, counts(0, 1, 1));
}

#[tokio::test]
async fn config_without_labels() {
    let (fake, client) = setup().await;
    let image = fake.image(REPO, "latest", &ImageSpec::new("linux", "amd64"));
    warm(&fake, &client, "latest").await;
    let r = client
        .resolve(
            &digest_ref(&image.manifest),
            &amd64(),
            &Credentials::Anonymous,
        )
        .await
        .result
        .expect("resolved");
    assert!(r.labels.is_empty());
    assert_eq!(r.user, None);
    assert_eq!(r.created, None);
}

#[tokio::test]
async fn rate_limited_is_counted() {
    let (fake, client) = setup().await;
    fake.image(REPO, "latest", &labelled());
    fake.set_status(&manifest_path("latest"), 429);
    let out = client
        .head_digest(&tag_ref("latest"), &Credentials::Anonymous)
        .await;
    assert_eq!(
        out.result,
        Err(RegistryError::RateLimited { retry_after: None })
    );
    assert_eq!(out.result.as_ref().unwrap_err().code(), "rate_limited");
    assert_eq!(out.requests, counts(1, 0, 0));
    assert_eq!(
        fake.requests(),
        vec![req("HEAD", &manifest_path("latest"), false)]
    );
}

#[tokio::test]
async fn rate_limited_carries_retry_after() {
    let (fake, client) = setup().await;
    fake.image(REPO, "latest", &labelled());
    fake.set_status_with_headers(&manifest_path("latest"), 429, &[("Retry-After", "120")]);
    let out = client
        .head_digest(&tag_ref("latest"), &Credentials::Anonymous)
        .await;
    assert_eq!(
        out.result,
        Err(RegistryError::RateLimited {
            retry_after: Some(Duration::from_secs(120))
        })
    );
    assert_eq!(out.requests, counts(1, 0, 0));

    // On a GET too, after a token.
    fake.clear_status(&manifest_path("latest"));
    let image = fake.image(REPO, "v2", &ImageSpec::new("linux", "amd64"));
    warm(&fake, &client, "v2").await;
    fake.set_status_with_headers(
        &manifest_path(image.manifest.as_str()),
        429,
        &[("Retry-After", "7")],
    );
    let out = client
        .resolve(
            &digest_ref(&image.manifest),
            &amd64(),
            &Credentials::Anonymous,
        )
        .await;
    assert_eq!(
        out.result,
        Err(RegistryError::RateLimited {
            retry_after: Some(Duration::from_secs(7))
        })
    );
    assert_eq!(out.requests, counts(0, 1, 1));
}

#[tokio::test]
async fn mismatched_digest_is_bad_response() {
    let (fake, client) = setup().await;
    let claimed = fake.mismatched_manifest(REPO, "latest");
    warm(&fake, &client, "latest").await;
    let out = client
        .resolve(&digest_ref(&claimed), &amd64(), &Credentials::Anonymous)
        .await;
    assert!(
        matches!(out.result, Err(RegistryError::BadResponse(_))),
        "{:?}",
        out.result
    );
    assert_eq!(out.requests, counts(0, 1, 1));
}

#[tokio::test]
async fn oversized_body_is_bad_response() {
    let (fake, client) = setup().await;
    let big = fake.put_manifest(REPO, media::OCI_MANIFEST, vec![b' '; 4 * 1024 * 1024 + 1]);
    fake.tag(REPO, "big", &big);
    warm(&fake, &client, "big").await;
    let out = client
        .resolve(&digest_ref(&big), &amd64(), &Credentials::Anonymous)
        .await;
    assert!(
        matches!(out.result, Err(RegistryError::BadResponse(_))),
        "{:?}",
        out.result
    );
    assert_eq!(out.requests, counts(0, 1, 1));
}

#[tokio::test]
async fn resolve_on_a_tag_makes_no_request() {
    let (fake, client) = setup().await;
    fake.image(REPO, "latest", &labelled());
    let out = client
        .resolve(&tag_ref("latest"), &amd64(), &Credentials::Anonymous)
        .await;
    assert_eq!(out.result, Err(RegistryError::NotByDigest));
    assert_eq!(out.requests, RequestCounts::default());
    assert!(fake.requests().is_empty());
}

#[tokio::test]
async fn missing_manifest_is_not_found() {
    let (fake, client) = setup().await;
    let out = client
        .head_digest(&tag_ref("nope"), &Credentials::Anonymous)
        .await;
    assert_eq!(out.result, Err(RegistryError::NotFound));
    assert_eq!(out.requests, counts(2, 1, 0));
    assert_eq!(fake.requests().len(), 3);
}

#[tokio::test]
async fn unauthorized_after_a_token_is_auth_failed() {
    let (fake, client) = setup().await;
    fake.image(REPO, "latest", &labelled());
    fake.set_status(&manifest_path("latest"), 401);
    let out = client
        .head_digest(&tag_ref("latest"), &Credentials::Anonymous)
        .await;
    assert_eq!(out.result, Err(RegistryError::AuthFailed));
    assert_eq!(out.requests, counts(2, 1, 0));
    assert_eq!(
        fake.requests(),
        vec![
            req("HEAD", &manifest_path("latest"), false),
            req("GET", "/token", false),
            req("HEAD", &manifest_path("latest"), true),
        ]
    );
}

#[tokio::test]
async fn unauthorized_without_bearer_is_auth_failed() {
    let (fake, client) = setup().await;
    fake.set_status_with_headers(
        &manifest_path("latest"),
        401,
        &[("WWW-Authenticate", "Basic realm=\"registry\"")],
    );
    let out = client
        .head_digest(&tag_ref("latest"), &Credentials::Anonymous)
        .await;
    assert_eq!(out.result, Err(RegistryError::AuthFailed));
    assert_eq!(out.requests, counts(1, 0, 0));
}

#[tokio::test]
async fn manifest_redirects_are_not_followed() {
    let (fake, client) = setup().await;
    let image = fake.image(REPO, "latest", &labelled());
    let target = format!("{}v2/{REPO}/manifests/{}", fake.url(), image.manifest);
    fake.set_status_with_headers(&manifest_path("latest"), 307, &[("Location", &target)]);
    let out = client
        .head_digest(&tag_ref("latest"), &Credentials::Anonymous)
        .await;
    assert!(
        matches!(out.result, Err(RegistryError::BadResponse(_))),
        "{:?}",
        out.result
    );
    assert_eq!(fake.requests().len(), 1);
}

#[tokio::test]
async fn blob_redirects_are_followed_and_bounded() {
    let (fake, client) = setup().await;
    let image = fake.image(REPO, "latest", &labelled());
    // The same blob, also stored under a mirror repository on the same host.
    fake.image("mirror/app", "latest", &labelled());
    warm(&fake, &client, "latest").await;

    let mirror = format!("{}v2/mirror/app/blobs/{}", fake.url(), image.config);
    fake.set_status_with_headers(&blob_path(&image.config), 307, &[("Location", &mirror)]);
    let out = client
        .resolve(
            &digest_ref(&image.manifest),
            &amd64(),
            &Credentials::Anonymous,
        )
        .await;
    assert!(out.result.is_ok(), "{:?}", out.result);
    let log = fake.requests();
    assert_eq!(
        log.last(),
        Some(&req(
            "GET",
            &format!("/v2/mirror/app/blobs/{}", image.config),
            true
        ))
    );

    // A loop is cut off after three redirects.
    fake.clear_requests();
    let own = format!("{}{}", fake.url(), &blob_path(&image.config)[1..]);
    fake.set_status_with_headers(&blob_path(&image.config), 307, &[("Location", &own)]);
    let out = client
        .resolve(
            &digest_ref(&image.manifest),
            &amd64(),
            &Credentials::Anonymous,
        )
        .await;
    assert!(
        matches!(out.result, Err(RegistryError::BadResponse(_))),
        "{:?}",
        out.result
    );
    let blob_gets = fake
        .requests()
        .iter()
        .filter(|r| r.path.contains("/blobs/"))
        .count();
    assert_eq!(blob_gets, 4, "the first request plus three redirects");
}

#[tokio::test]
async fn cross_host_blob_redirect_drops_authorization() {
    let (fake, client) = setup().await;
    let image = fake.image(REPO, "latest", &labelled());
    fake.image("mirror/app", "latest", &labelled());
    warm(&fake, &client, "latest").await;

    // `localhost` is another host than `127.0.0.1` as far as the redirect is concerned.
    let port = fake.url().port().expect("port");
    let other = format!(
        "http://localhost:{port}/v2/mirror/app/blobs/{}",
        image.config
    );
    fake.set_status_with_headers(&blob_path(&image.config), 307, &[("Location", &other)]);
    let _ = client
        .resolve(
            &digest_ref(&image.manifest),
            &amd64(),
            &Credentials::Anonymous,
        )
        .await;
    assert_eq!(
        fake.requests().last(),
        Some(&req(
            "GET",
            &format!("/v2/mirror/app/blobs/{}", image.config),
            false
        ))
    );
}

#[test]
fn client_new_builds() {
    assert!(Client::new(UA).is_ok());
}
