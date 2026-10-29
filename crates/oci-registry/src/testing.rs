//! `FakeRegistry`: an in-process OCI registry for tests (feature `test-util`).
//!
//! An axum router on `127.0.0.1:0` serving `/token`, HEAD and GET on `/v2/<repo>/manifests/<ref>`
//! and GET on `/v2/<repo>/blobs/<digest>` from in-memory fixtures, with digests computed from the
//! fixture bodies. Every request is logged as `(method, path, had_authorization)` so tests can
//! assert the exact requests a client made. Nothing here reaches the network.
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::Response;
use serde_json::{Value, json};

use crate::reference::Digest;

/// Media types the fixtures use.
pub mod media {
    pub const OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
    pub const DOCKER_LIST: &str = "application/vnd.docker.distribution.manifest.list.v2+json";
    pub const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
    pub const DOCKER_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json";
    pub const OCI_CONFIG: &str = "application/vnd.oci.image.config.v1+json";
}

/// The service name the fake's `Bearer` challenge names.
pub const SERVICE: &str = "fake-registry";

/// One request the fake received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedRequest {
    pub method: String,
    pub path: String,
    pub had_authorization: bool,
}

impl LoggedRequest {
    pub fn new(method: &str, path: &str, had_authorization: bool) -> LoggedRequest {
        LoggedRequest {
            method: method.to_string(),
            path: path.to_string(),
            had_authorization,
        }
    }
}

struct StoredManifest {
    media_type: String,
    body: Vec<u8>,
    /// What `Docker-Content-Digest` says; normally the body's digest.
    digest: String,
}

struct ProgrammedStatus {
    status: u16,
    headers: Vec<(String, String)>,
}

#[derive(Default)]
struct Inner {
    realm: String,
    manifests: HashMap<(String, String), StoredManifest>,
    blobs: HashMap<(String, String), Vec<u8>>,
    omit_digest_header: bool,
    statuses: HashMap<String, ProgrammedStatus>,
    basic: Option<(String, String)>,
    tokens: Vec<String>,
    token_expires_in: Option<u64>,
    log: Vec<LoggedRequest>,
}

/// What a single-manifest image's config holds.
#[derive(Debug, Clone)]
pub struct ImageSpec {
    pub os: String,
    pub architecture: String,
    pub variant: Option<String>,
    /// `None` leaves `config.Labels` out entirely.
    pub labels: Option<BTreeMap<String, String>>,
    pub user: Option<String>,
    pub created: Option<String>,
}

impl ImageSpec {
    pub fn new(os: &str, architecture: &str) -> ImageSpec {
        ImageSpec {
            os: os.to_string(),
            architecture: architecture.to_string(),
            variant: None,
            labels: None,
            user: None,
            created: None,
        }
    }

    pub fn variant(mut self, v: &str) -> ImageSpec {
        self.variant = Some(v.to_string());
        self
    }

    pub fn label(mut self, k: &str, v: &str) -> ImageSpec {
        self.labels
            .get_or_insert_with(BTreeMap::new)
            .insert(k.to_string(), v.to_string());
        self
    }

    pub fn user(mut self, u: &str) -> ImageSpec {
        self.user = Some(u.to_string());
        self
    }

    pub fn created(mut self, c: &str) -> ImageSpec {
        self.created = Some(c.to_string());
        self
    }
}

/// A stored single-platform image.
#[derive(Debug, Clone)]
pub struct Image {
    pub manifest: Digest,
    pub config: Digest,
}

/// A stored multi-platform index.
#[derive(Debug, Clone)]
pub struct MultiArch {
    pub index: Digest,
    pub amd64: Image,
    pub arm64: Image,
    /// The attestation manifests the index lists, which platform selection must skip.
    pub attestations: Vec<Digest>,
}

/// The in-process fake registry. Dropping it stops the server.
pub struct FakeRegistry {
    url: reqwest::Url,
    inner: Arc<Mutex<Inner>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeRegistry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// `sha256:<hex>` of `body`.
pub fn digest_of(body: &[u8]) -> Digest {
    Digest::parse(&crate::client::sha256_digest(body)).expect("sha256 digest is well-formed")
}

impl FakeRegistry {
    pub async fn start() -> FakeRegistry {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the fake registry");
        let addr = listener.local_addr().expect("fake registry address");
        let url = reqwest::Url::parse(&format!("http://{addr}/")).expect("fake registry url");
        let inner = Arc::new(Mutex::new(Inner {
            realm: format!("{url}token"),
            token_expires_in: Some(300),
            ..Inner::default()
        }));
        let app = Router::new().fallback(handle).with_state(inner.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        FakeRegistry { url, inner, task }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The base URL, for `Client::with_base_url`.
    pub fn url(&self) -> reqwest::Url {
        self.url.clone()
    }

    /// Every request so far, in order.
    pub fn requests(&self) -> Vec<LoggedRequest> {
        self.lock().log.clone()
    }

    pub fn clear_requests(&self) {
        self.lock().log.clear();
    }

    /// Require `Basic` with these credentials on `/token`.
    pub fn require_basic(&self, username: &str, password: &str) {
        self.lock().basic = Some((username.to_string(), password.to_string()));
    }

    /// Omit `Docker-Content-Digest` on HEAD responses.
    pub fn omit_digest_header(&self, omit: bool) {
        self.lock().omit_digest_header = omit;
    }

    /// `expires_in` in token responses; `None` leaves it out.
    pub fn token_expires_in(&self, secs: Option<u64>) {
        self.lock().token_expires_in = secs;
    }

    /// Answer every request on `path` with `status` (a 401 carries the `Bearer` challenge).
    pub fn set_status(&self, path: &str, status: u16) {
        self.set_status_with_headers(path, status, &[]);
    }

    pub fn set_status_with_headers(&self, path: &str, status: u16, headers: &[(&str, &str)]) {
        self.lock().statuses.insert(
            path.to_string(),
            ProgrammedStatus {
                status,
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            },
        );
    }

    pub fn clear_status(&self, path: &str) {
        self.lock().statuses.remove(path);
    }

    pub fn put_blob(&self, repo: &str, body: Vec<u8>) -> Digest {
        let d = digest_of(&body);
        self.lock()
            .blobs
            .insert((repo.to_string(), d.as_str().to_string()), body);
        d
    }

    pub fn put_manifest(&self, repo: &str, media_type: &str, body: Vec<u8>) -> Digest {
        let d = digest_of(&body);
        self.put_manifest_as(repo, &d, media_type, body);
        d
    }

    /// Store `body` under `digest` whether or not it hashes to it.
    pub fn put_manifest_as(&self, repo: &str, digest: &Digest, media_type: &str, body: Vec<u8>) {
        self.lock().manifests.insert(
            (repo.to_string(), digest.as_str().to_string()),
            StoredManifest {
                media_type: media_type.to_string(),
                body,
                digest: digest.as_str().to_string(),
            },
        );
    }

    /// Point `tag` at the manifest stored under `digest`.
    pub fn tag(&self, repo: &str, tag: &str, digest: &Digest) {
        let mut g = self.lock();
        let m = g
            .manifests
            .get(&(repo.to_string(), digest.as_str().to_string()))
            .expect("tag an existing manifest");
        let copy = StoredManifest {
            media_type: m.media_type.clone(),
            body: m.body.clone(),
            digest: m.digest.clone(),
        };
        g.manifests
            .insert((repo.to_string(), tag.to_string()), copy);
    }

    /// A single-manifest image (OCI manifest plus config), tagged `tag`.
    pub fn image(&self, repo: &str, tag: &str, spec: &ImageSpec) -> Image {
        let image = self.store_image(repo, spec);
        self.tag(repo, tag, &image.manifest);
        image
    }

    fn store_image(&self, repo: &str, spec: &ImageSpec) -> Image {
        let config = self.put_blob(repo, config_body(spec));
        let size = self.blob_len(repo, &config);
        let manifest = self.put_manifest(
            repo,
            media::OCI_MANIFEST,
            manifest_body(media::OCI_MANIFEST, &config, size),
        );
        Image { manifest, config }
    }

    fn blob_len(&self, repo: &str, d: &Digest) -> usize {
        self.lock()
            .blobs
            .get(&(repo.to_string(), d.as_str().to_string()))
            .map(Vec::len)
            .unwrap_or(0)
    }

    fn manifest_len(&self, repo: &str, d: &Digest) -> usize {
        self.lock()
            .manifests
            .get(&(repo.to_string(), d.as_str().to_string()))
            .map(|m| m.body.len())
            .unwrap_or(0)
    }

    /// An OCI index with amd64 and arm64 images (both with `labels`) and attestation entries:
    /// first an attestation that claims `linux/amd64` but carries `vnd.docker.reference.type`,
    /// then the usual `unknown/unknown` one. Tagged `tag`.
    pub fn multi_arch(&self, repo: &str, tag: &str, labels: &[(&str, &str)]) -> MultiArch {
        let mut amd = ImageSpec::new("linux", "amd64")
            .user("10001")
            .created("2026-10-01T12:00:00Z");
        let mut arm = ImageSpec::new("linux", "arm64")
            .variant("v8")
            .user("10001")
            .created("2026-10-01T12:00:00Z");
        for (k, v) in labels {
            amd = amd.label(k, v);
            arm = arm.label(k, v);
        }
        let amd64 = self.store_image(repo, &amd);
        let arm64 = self.store_image(repo, &arm);
        let att_a = self.store_image(
            repo,
            &ImageSpec::new("unknown", "unknown").label("att", "a"),
        );
        let att_b = self.store_image(
            repo,
            &ImageSpec::new("unknown", "unknown").label("att", "b"),
        );
        let attestation = |d: &Digest, os: &str, arch: &str| {
            json!({
                "mediaType": media::OCI_MANIFEST,
                "digest": d.as_str(),
                "size": self.manifest_len(repo, d),
                "platform": {"os": os, "architecture": arch},
                "annotations": {
                    "vnd.docker.reference.digest": amd64.manifest.as_str(),
                    "vnd.docker.reference.type": "attestation-manifest"
                }
            })
        };
        let entries = vec![
            attestation(&att_a.manifest, "linux", "amd64"),
            attestation(&att_b.manifest, "unknown", "unknown"),
            self.entry(repo, &amd64.manifest, "linux", "amd64", None),
            self.entry(repo, &arm64.manifest, "linux", "arm64", Some("v8")),
        ];
        let index = self.put_manifest(
            repo,
            media::OCI_INDEX,
            index_body(media::OCI_INDEX, entries),
        );
        self.tag(repo, tag, &index);
        MultiArch {
            index,
            amd64,
            arm64,
            attestations: vec![att_a.manifest, att_b.manifest],
        }
    }

    /// A Docker manifest list holding only `linux/arm64` and an attestation, tagged `tag`.
    pub fn index_without_amd64(&self, repo: &str, tag: &str) -> Digest {
        let arm = self.store_image(repo, &ImageSpec::new("linux", "arm64").variant("v8"));
        let entries = vec![self.entry(repo, &arm.manifest, "linux", "arm64", Some("v8"))];
        let index = self.put_manifest(
            repo,
            media::DOCKER_LIST,
            index_body(media::DOCKER_LIST, entries),
        );
        self.tag(repo, tag, &index);
        index
    }

    /// A manifest stored under a digest its body does not hash to. Returns the claimed digest.
    pub fn mismatched_manifest(&self, repo: &str, tag: &str) -> Digest {
        let image = self.store_image(repo, &ImageSpec::new("linux", "amd64"));
        let claimed = digest_of(b"some other content");
        let body = self
            .lock()
            .manifests
            .get(&(repo.to_string(), image.manifest.as_str().to_string()))
            .map(|m| m.body.clone())
            .expect("stored manifest");
        self.put_manifest_as(repo, &claimed, media::OCI_MANIFEST, body);
        self.tag(repo, tag, &claimed);
        claimed
    }

    fn entry(&self, repo: &str, d: &Digest, os: &str, arch: &str, variant: Option<&str>) -> Value {
        let mut platform = json!({"os": os, "architecture": arch});
        if let Some(v) = variant {
            platform["variant"] = json!(v);
        }
        json!({
            "mediaType": media::OCI_MANIFEST,
            "digest": d.as_str(),
            "size": self.manifest_len(repo, d),
            "platform": platform,
        })
    }
}

/// An image config body.
pub fn config_body(spec: &ImageSpec) -> Vec<u8> {
    let mut config = json!({});
    if let Some(labels) = &spec.labels {
        config["Labels"] = json!(labels);
    }
    if let Some(user) = &spec.user {
        config["User"] = json!(user);
    }
    let mut body = json!({
        "os": spec.os,
        "architecture": spec.architecture,
        "config": config,
        "rootfs": {"type": "layers", "diff_ids": []},
    });
    if let Some(v) = &spec.variant {
        body["variant"] = json!(v);
    }
    if let Some(c) = &spec.created {
        body["created"] = json!(c);
    }
    serde_json::to_vec(&body).expect("serialize config")
}

/// A manifest body pointing at `config`.
pub fn manifest_body(media_type: &str, config: &Digest, config_size: usize) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": media_type,
        "config": {
            "mediaType": media::OCI_CONFIG,
            "digest": config.as_str(),
            "size": config_size,
        },
        "layers": [],
    }))
    .expect("serialize manifest")
}

/// An index or manifest-list body.
pub fn index_body(media_type: &str, entries: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": media_type,
        "manifests": entries,
    }))
    .expect("serialize index")
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn respond(status: u16, headers: &[(&str, &str)], body: Vec<u8>) -> Response {
    let mut b = Response::builder().status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK));
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    b.body(Body::from(body)).expect("build response")
}

fn errors(code: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"errors": [{"code": code, "message": code}]})).expect("json")
}

fn challenge(g: &Inner, repo: Option<&str>) -> String {
    match repo {
        Some(r) => format!(
            "Bearer realm=\"{}\",service=\"{SERVICE}\",scope=\"repository:{r}:pull\"",
            g.realm
        ),
        None => format!("Bearer realm=\"{}\",service=\"{SERVICE}\"", g.realm),
    }
}

/// Split `/v2/<repo>/<kind>/<ref>`.
fn split_v2<'a>(path: &'a str, kind: &str) -> Option<(&'a str, &'a str)> {
    let rest = path.strip_prefix("/v2/")?;
    let marker = format!("/{kind}/");
    let i = rest.rfind(&marker)?;
    Some((&rest[..i], &rest[i + marker.len()..]))
}

async fn handle(
    State(inner): State<Arc<Mutex<Inner>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path().to_string();
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let mut g = inner.lock().unwrap_or_else(|p| p.into_inner());
    g.log.push(LoggedRequest {
        method: method.as_str().to_string(),
        path: path.clone(),
        had_authorization: authorization.is_some(),
    });
    let manifest = split_v2(&path, "manifests");
    let blob = split_v2(&path, "blobs");
    let repo = manifest.or(blob).map(|(r, _)| r);

    if let Some(p) = g.statuses.get(&path) {
        let mut hs: Vec<(String, String)> = p.headers.clone();
        if p.status == 401
            && !hs
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("www-authenticate"))
        {
            hs.push(("WWW-Authenticate".into(), challenge(&g, repo)));
        }
        let code = match p.status {
            401 => "UNAUTHORIZED",
            404 => "MANIFEST_UNKNOWN",
            429 => "TOOMANYREQUESTS",
            _ => "UNKNOWN",
        };
        let status = p.status;
        let hs: Vec<(&str, &str)> = hs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        return respond(status, &hs, errors(code));
    }

    if path == "/token" {
        if method != Method::GET {
            return respond(405, &[], Vec::new());
        }
        if let Some((u, p)) = &g.basic {
            let want = format!("Basic {}", base64(format!("{u}:{p}").as_bytes()));
            if authorization.as_deref() != Some(want.as_str()) {
                return respond(401, &[], errors("UNAUTHORIZED"));
            }
        }
        let token = format!("fake-token-{}", g.tokens.len() + 1);
        g.tokens.push(token.clone());
        let mut body = json!({"token": token});
        if let Some(e) = g.token_expires_in {
            body["expires_in"] = json!(e);
        }
        return respond(
            200,
            &[("Content-Type", "application/json")],
            serde_json::to_vec(&body).expect("json"),
        );
    }

    let Some(repo) = repo else {
        return respond(404, &[], errors("NAME_UNKNOWN"));
    };
    let bearer_ok = authorization
        .as_deref()
        .and_then(|a| a.strip_prefix("Bearer "))
        .is_some_and(|t| g.tokens.iter().any(|k| k == t));
    if !bearer_ok {
        let c = challenge(&g, Some(repo));
        return respond(
            401,
            &[("WWW-Authenticate", c.as_str())],
            errors("UNAUTHORIZED"),
        );
    }

    if let Some((repo, reference)) = manifest {
        if method != Method::GET && method != Method::HEAD {
            return respond(405, &[], Vec::new());
        }
        let Some(m) = g.manifests.get(&(repo.to_string(), reference.to_string())) else {
            return respond(404, &[], errors("MANIFEST_UNKNOWN"));
        };
        let len = m.body.len().to_string();
        let mut hs = vec![
            ("Content-Type", m.media_type.as_str()),
            ("Content-Length", len.as_str()),
        ];
        let is_head = method == Method::HEAD;
        if !(is_head && g.omit_digest_header) {
            hs.push(("Docker-Content-Digest", m.digest.as_str()));
        }
        let body = if is_head { Vec::new() } else { m.body.clone() };
        return respond(200, &hs, body);
    }

    if let Some((repo, digest)) = blob {
        if method != Method::GET {
            return respond(405, &[], Vec::new());
        }
        return match g.blobs.get(&(repo.to_string(), digest.to_string())) {
            Some(b) => respond(200, &[("Content-Type", media::OCI_CONFIG)], b.clone()),
            None => respond(404, &[], errors("BLOB_UNKNOWN")),
        };
    }
    respond(404, &[], errors("NAME_UNKNOWN"))
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }
}
