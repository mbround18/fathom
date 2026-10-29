//! `Client`: `head_digest` and `resolve`, with the exact request set of the contract.
use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::redirect::Policy;
use reqwest::{Method, StatusCode, header};
use secrecy::ExposeSecret;
use sha2::{Digest as _, Sha256};

use crate::auth::{
    Challenge, Credentials, TOKEN_BODY_LIMIT, TokenCache, parse_bearer, parse_token,
};
use crate::error::RegistryError;
use crate::manifest::{ManifestBody, Platform, parse_config, parse_manifest, select_platform};
use crate::reference::{Digest, ImageRef, Reference};

/// The requests one call made, success or failure.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RequestCounts {
    /// Every HEAD.
    pub head: u32,
    /// Every GET: token, manifest and blob.
    pub get: u32,
    /// The GETs on `/v2/*/manifests/*`, which registries count as pulls.
    pub manifest_get: u32,
}

/// Every call returns its result together with the requests it made, success or failure.
#[derive(Debug)]
pub struct Counted<T> {
    pub result: Result<T, RegistryError>,
    pub requests: RequestCounts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Set when the digest named an index or manifest list.
    pub index_digest: Option<Digest>,
    /// The platform manifest.
    pub manifest_digest: Digest,
    pub config_digest: Digest,
    /// `config.Labels`, empty when absent.
    pub labels: BTreeMap<String, String>,
    /// `config.User`.
    pub user: Option<String>,
    /// The config's top-level `created`, as given.
    pub created: Option<String>,
}

/// The `Accept` header of every manifest request, HEAD or GET.
pub const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.index.v1+json, \
application/vnd.docker.distribution.manifest.list.v2+json, \
application/vnd.oci.image.manifest.v1+json, \
application/vnd.docker.distribution.manifest.v2+json";

/// Manifests, indexes and config blobs are at most 4 MiB.
const BODY_LIMIT: usize = 4 * 1024 * 1024;
/// Per request (rule 7).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Blob GETs follow at most this many redirects (rule 3).
const MAX_BLOB_REDIRECTS: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Manifest,
    Blob,
}

/// A HEAD-first registry client. Holds two reqwest clients (one that never follows redirects,
/// for manifests and tokens; one that follows up to three redirects, for blobs) and an in-memory
/// token cache.
pub struct Client {
    plain: reqwest::Client,
    blobs: reqwest::Client,
    /// `test-util`: every registry and realm request goes here.
    base: Option<reqwest::Url>,
    tokens: TokenCache,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base.as_ref().map(reqwest::Url::as_str))
            .finish_non_exhaustive()
    }
}

fn build_clients(
    user_agent: &str,
    allow_http: bool,
) -> Result<(reqwest::Client, reqwest::Client), reqwest::Error> {
    let plain = reqwest::Client::builder()
        .user_agent(user_agent)
        .redirect(Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .https_only(!allow_http)
        .build()?;
    // reqwest itself drops `Authorization` when a redirect changes scheme, host or port.
    let policy = Policy::custom(move |attempt| {
        if attempt.previous().len() > MAX_BLOB_REDIRECTS {
            attempt.error("too many redirects")
        } else if !allow_http && attempt.url().scheme() != "https" {
            attempt.error("redirect to a non-https URL")
        } else {
            attempt.follow()
        }
    });
    let blobs = reqwest::Client::builder()
        .user_agent(user_agent)
        .redirect(policy)
        .timeout(REQUEST_TIMEOUT)
        .https_only(!allow_http)
        .build()?;
    Ok((plain, blobs))
}

impl Client {
    /// `https` only. `user_agent` is the caller's, e.g. `keel/0.2.0 (update-check)`.
    pub fn new(user_agent: &str) -> Result<Client, RegistryError> {
        let (plain, blobs) =
            build_clients(user_agent, false).map_err(|_| RegistryError::Network)?;
        Ok(Client {
            plain,
            blobs,
            base: None,
            tokens: TokenCache::default(),
        })
    }

    /// Feature `test-util` only: every registry and realm request goes to `base` (plain `http`
    /// allowed), so tests point the client at the fake registry.
    #[cfg(feature = "test-util")]
    pub fn with_base_url(user_agent: &str, base: reqwest::Url) -> Client {
        let (plain, blobs) = build_clients(user_agent, true).expect("build the test client");
        Client {
            plain,
            blobs,
            base: Some(base),
            tokens: TokenCache::default(),
        }
    }

    /// HEAD the manifest. Never falls back to GET.
    pub async fn head_digest(&self, r: &ImageRef, c: &Credentials) -> Counted<Digest> {
        let mut requests = RequestCounts::default();
        let result = self.head_inner(r, c, &mut requests).await;
        Counted { result, requests }
    }

    /// `r` must be by digest (`NotByDigest` otherwise, with no request made).
    pub async fn resolve(&self, r: &ImageRef, p: &Platform, c: &Credentials) -> Counted<Resolved> {
        let mut requests = RequestCounts::default();
        let result = self.resolve_inner(r, p, c, &mut requests).await;
        Counted { result, requests }
    }

    async fn head_inner(
        &self,
        r: &ImageRef,
        c: &Credentials,
        counts: &mut RequestCounts,
    ) -> Result<Digest, RegistryError> {
        let path = manifest_path(r.repository(), r.reference_str());
        let resp = self
            .send(Method::HEAD, r, &path, Kind::Manifest, c, counts)
            .await?;
        resp.headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| Digest::parse(v.trim()).ok())
            .ok_or(RegistryError::NoDigestHeader)
    }

    async fn resolve_inner(
        &self,
        r: &ImageRef,
        p: &Platform,
        c: &Credentials,
        counts: &mut RequestCounts,
    ) -> Result<Resolved, RegistryError> {
        let Reference::Digest(requested) = r.reference() else {
            return Err(RegistryError::NotByDigest);
        };
        let repo = r.repository();
        let path = manifest_path(repo, requested.as_str());
        let body = self
            .get_verified(r, &path, requested, Kind::Manifest, c, counts)
            .await?;
        let (index_digest, manifest_digest, config_digest) = match parse_manifest(&body)? {
            ManifestBody::Index(entries) => {
                let platform_digest = select_platform(&entries, p)?;
                let path = manifest_path(repo, platform_digest.as_str());
                let body = self
                    .get_verified(r, &path, &platform_digest, Kind::Manifest, c, counts)
                    .await?;
                match parse_manifest(&body)? {
                    ManifestBody::Image { config } => {
                        (Some(requested.clone()), platform_digest, config)
                    }
                    ManifestBody::Index(_) => {
                        return Err(RegistryError::BadResponse("index entry is an index"));
                    }
                }
            }
            ManifestBody::Image { config } => (None, requested.clone(), config),
        };
        let path = format!("/v2/{repo}/blobs/{}", config_digest.as_str());
        let body = self
            .get_verified(r, &path, &config_digest, Kind::Blob, c, counts)
            .await?;
        let config = parse_config(&body)?;
        if index_digest.is_none() && (config.os != p.os || config.architecture != p.architecture) {
            return Err(RegistryError::NoImageForPlatform);
        }
        Ok(Resolved {
            index_digest,
            manifest_digest,
            config_digest,
            labels: config.labels,
            user: config.user,
            created: config.created,
        })
    }

    /// GET, read at most 4 MiB, and check the body hashes to `digest` (rules 4 and 5).
    async fn get_verified(
        &self,
        r: &ImageRef,
        path: &str,
        digest: &Digest,
        kind: Kind,
        c: &Credentials,
        counts: &mut RequestCounts,
    ) -> Result<Vec<u8>, RegistryError> {
        let resp = self.send(Method::GET, r, path, kind, c, counts).await?;
        let body = read_limited(resp, BODY_LIMIT).await?;
        if sha256_digest(&body) != digest.as_str() {
            return Err(RegistryError::BadResponse("digest mismatch"));
        }
        Ok(body)
    }

    /// One registry request, anonymous or with a cached token; on a `Bearer` challenge, take a
    /// token (#2) and repeat it once (rule 2).
    async fn send(
        &self,
        method: Method,
        r: &ImageRef,
        path: &str,
        kind: Kind,
        c: &Credentials,
        counts: &mut RequestCounts,
    ) -> Result<reqwest::Response, RegistryError> {
        let url = self.registry_url(r, path)?;
        let scope = format!("repository:{}:pull", r.repository());
        let cached = self
            .tokens
            .challenge_for(r.registry())
            .and_then(|ch| self.tokens.get(&TokenCache::key(&ch, &scope, c)));
        let resp = self
            .request(&method, &url, kind, cached.as_deref(), counts)
            .await?;
        if resp.status() != StatusCode::UNAUTHORIZED || cached.is_some() {
            return check_status(resp).await;
        }
        let challenge = resp
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_bearer)
            .ok_or(RegistryError::AuthFailed)?;
        let (token, ttl) = self.fetch_token(&challenge, &scope, c, counts).await?;
        self.tokens
            .put(TokenCache::key(&challenge, &scope, c), token.clone(), ttl);
        self.tokens.remember_challenge(r.registry(), challenge);
        let resp = self
            .request(&method, &url, kind, Some(&token), counts)
            .await?;
        check_status(resp).await
    }

    async fn request(
        &self,
        method: &Method,
        url: &reqwest::Url,
        kind: Kind,
        token: Option<&str>,
        counts: &mut RequestCounts,
    ) -> Result<reqwest::Response, RegistryError> {
        let client = match kind {
            Kind::Manifest => &self.plain,
            Kind::Blob => &self.blobs,
        };
        let mut req = client.request(method.clone(), url.clone());
        if kind == Kind::Manifest {
            req = req.header(header::ACCEPT, MANIFEST_ACCEPT);
        }
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if *method == Method::HEAD {
            counts.head += 1;
        } else {
            counts.get += 1;
            if kind == Kind::Manifest {
                counts.manifest_get += 1;
            }
        }
        let result = req.send().await;
        log_request(method, url, result.as_ref().ok().map(|r| r.status()));
        result.map_err(map_reqwest)
    }

    /// The token request (#2): Basic credentials go here and nowhere else.
    async fn fetch_token(
        &self,
        challenge: &Challenge,
        scope: &str,
        c: &Credentials,
        counts: &mut RequestCounts,
    ) -> Result<(String, Duration), RegistryError> {
        let mut url = self.realm_url(&challenge.realm)?;
        {
            let mut q = url.query_pairs_mut();
            if let Some(service) = &challenge.service {
                q.append_pair("service", service);
            }
            q.append_pair("scope", scope);
        }
        let mut req = self.plain.get(url.clone());
        if let Credentials::Basic { username, password } = c {
            req = req.basic_auth(username, Some(password.expose_secret()));
        }
        counts.get += 1;
        let result = req.send().await;
        log_request(&Method::GET, &url, result.as_ref().ok().map(|r| r.status()));
        let resp = result.map_err(map_reqwest)?;
        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(RegistryError::AuthFailed);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(RegistryError::RateLimited {
                retry_after: retry_after(resp.headers()),
            });
        }
        if !status.is_success() {
            return Err(RegistryError::BadResponse("token endpoint status"));
        }
        let body = read_limited(resp, TOKEN_BODY_LIMIT).await?;
        parse_token(&body)
    }

    fn registry_url(&self, r: &ImageRef, path: &str) -> Result<reqwest::Url, RegistryError> {
        let url = match &self.base {
            Some(base) => base.join(path.trim_start_matches('/')),
            None => reqwest::Url::parse(&format!("https://{}{path}", r.registry())),
        };
        url.map_err(|_| RegistryError::InvalidReference(format!("no URL for {r}")))
    }

    /// The realm, `https` only; under `test-util` its path is served from the base.
    fn realm_url(&self, realm: &str) -> Result<reqwest::Url, RegistryError> {
        let mut url = reqwest::Url::parse(realm)
            .map_err(|_| RegistryError::BadResponse("malformed token realm"))?;
        match &self.base {
            Some(base) => {
                let mut rebased = base.clone();
                rebased.set_path(url.path());
                url = rebased;
            }
            None if url.scheme() != "https" => {
                return Err(RegistryError::BadResponse("token realm is not https"));
            }
            None => {}
        }
        url.set_query(None);
        Ok(url)
    }
}

fn manifest_path(repo: &str, reference: &str) -> String {
    format!("/v2/{repo}/manifests/{reference}")
}

/// Method, host, path and status at `debug`. Never headers, query, tokens or credentials.
fn log_request(method: &Method, url: &reqwest::Url, status: Option<StatusCode>) {
    tracing::debug!(
        method = %method,
        host = url.host_str().unwrap_or(""),
        path = url.path(),
        status = status.map(|s| s.as_u16()),
        "registry request"
    );
}

fn map_reqwest(e: reqwest::Error) -> RegistryError {
    if e.is_timeout() {
        RegistryError::Timeout
    } else if e.is_redirect() {
        RegistryError::BadResponse("redirect refused")
    } else if e.is_builder() {
        RegistryError::BadResponse("request could not be built")
    } else {
        RegistryError::Network
    }
}

/// Map a registry response's status (rule 8). The 401 retry is `send`'s job.
async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, RegistryError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let retry = retry_after(resp.headers());
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(RegistryError::AuthFailed),
        StatusCode::NOT_FOUND => Err(RegistryError::NotFound),
        StatusCode::TOO_MANY_REQUESTS => Err(RegistryError::RateLimited { retry_after: retry }),
        _ => {
            // Docker Hub can say TOOMANYREQUESTS under another status.
            let body = read_limited(resp, TOKEN_BODY_LIMIT)
                .await
                .unwrap_or_default();
            if String::from_utf8_lossy(&body).contains("TOOMANYREQUESTS") {
                Err(RegistryError::RateLimited { retry_after: retry })
            } else {
                Err(RegistryError::BadResponse("unexpected status"))
            }
        }
    }
}

/// Read at most `limit` bytes; more is `BadResponse` (rule 4).
async fn read_limited(mut resp: reqwest::Response, limit: usize) -> Result<Vec<u8>, RegistryError> {
    if resp.content_length().is_some_and(|n| n > limit as u64) {
        return Err(RegistryError::BadResponse("body too large"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(map_reqwest)? {
        if body.len() + chunk.len() > limit {
            return Err(RegistryError::BadResponse("body too large"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn retry_after(headers: &header::HeaderMap) -> Option<Duration> {
    let v = headers.get(header::RETRY_AFTER)?.to_str().ok()?;
    parse_retry_after(v, SystemTime::now())
}

/// `Retry-After`: delta-seconds, or an IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`) measured
/// from `now`, zero when already past. Anything else is `None`.
fn parse_retry_after(v: &str, now: SystemTime) -> Option<Duration> {
    let v = v.trim();
    if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) {
        return v.parse::<u64>().ok().map(Duration::from_secs);
    }
    let at = parse_imf_fixdate(v)?;
    Some(at.duration_since(now).unwrap_or(Duration::ZERO))
}

fn parse_imf_fixdate(v: &str) -> Option<SystemTime> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let parts: Vec<&str> = v.split_whitespace().collect();
    let [weekday, day, month, year, time, "GMT"] = parts.as_slice() else {
        return None;
    };
    if weekday.len() != 4 || !weekday.ends_with(',') || day.len() != 2 || year.len() != 4 {
        return None;
    }
    let day: i64 = day.parse().ok().filter(|d| (1..=31).contains(d))?;
    let month = MONTHS.iter().position(|m| m == month)? as i64 + 1;
    let year: i64 = year.parse().ok().filter(|y| *y >= 1970)?;
    let hms: Vec<i64> = time
        .split(':')
        .map(|p| if p.len() == 2 { p.parse().ok() } else { None })
        .collect::<Option<_>>()?;
    let [h, m, s] = hms.as_slice() else {
        return None;
    };
    if *h > 23 || *m > 59 || *s > 60 {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = u64::try_from(days * 86_400 + h * 3_600 + m * 60 + s).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(secs))
}

/// `sha256:<64 lowercase hex>` of `body`.
pub(crate) fn sha256_digest(body: &[u8]) -> String {
    let mut out = String::with_capacity(71);
    out.push_str("sha256:");
    for b in Sha256::digest(body).iter() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // `Sun, 06 Nov 1994 08:49:37 GMT` is 784111777 seconds after the epoch.
    const RFC_EXAMPLE: u64 = 784_111_777;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn retry_after_delta_seconds() {
        let now = at(1_000);
        assert_eq!(
            parse_retry_after("120", now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(parse_retry_after(" 0 ", now), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("-5", now), None);
        assert_eq!(parse_retry_after("1.5", now), None);
        assert_eq!(parse_retry_after("", now), None);
    }

    #[test]
    fn retry_after_http_date() {
        let date = "Sun, 06 Nov 1994 08:49:37 GMT";
        assert_eq!(
            parse_retry_after(date, at(RFC_EXAMPLE - 30)),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            parse_retry_after(date, at(RFC_EXAMPLE + 5)),
            Some(Duration::ZERO)
        );
        assert_eq!(
            parse_imf_fixdate("Thu, 01 Jan 1970 00:00:00 GMT"),
            Some(UNIX_EPOCH)
        );
        assert_eq!(
            parse_imf_fixdate("Tue, 29 Feb 2028 12:00:00 GMT"),
            Some(at(1_835_438_400))
        );
    }

    #[test]
    fn retry_after_refuses_other_dates() {
        let now = at(0);
        for bad in [
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
            "Sun, 06 Nov 1994 08:49:37 UTC",
            "Sun, 6 Nov 1994 08:49:37 GMT",
            "Sun, 06 Foo 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 25:49:37 GMT",
            "soon",
        ] {
            assert_eq!(parse_retry_after(bad, now), None, "{bad}");
        }
    }

    #[test]
    fn sha256_of_empty() {
        assert_eq!(
            sha256_digest(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn realm_must_be_https_outside_tests() {
        let c = Client::new("t").unwrap();
        assert!(c.realm_url("https://auth.docker.io/token").is_ok());
        assert_eq!(
            c.realm_url("http://auth.example/token"),
            Err(RegistryError::BadResponse("token realm is not https"))
        );
    }

    #[test]
    fn registry_urls_are_https() {
        let c = Client::new("t").unwrap();
        let r = ImageRef::parse("registry.example:5000/keel").unwrap();
        assert_eq!(
            c.registry_url(&r, "/v2/keel/manifests/latest")
                .unwrap()
                .as_str(),
            "https://registry.example:5000/v2/keel/manifests/latest"
        );
    }
}
