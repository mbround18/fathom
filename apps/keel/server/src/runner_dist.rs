//! Runner downloads (spec 055 T147; contracts/join.md "Runner downloads"): the runner release
//! this Keel ships, served from `--runner-dist-dir` on disk, never embedded.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET, HEAD | `/runner/{release}/SHA256SUMS` | `sha256sum` lines, keyed by the paths below |
//! | GET, HEAD | `/runner/{release}/{arch}/{binary}` | `arch`: `x86_64`, `aarch64`; `binary`: `keel-runner`, `keel-executor`, `keel-relay` |
//! | GET, HEAD | `/runner/{release}/requirements.txt` | the hash-pinned `ansible-core` requirements |
//! | GET, HEAD | `/runner/{release}/keel_events.tar.gz` | the callback plugin |
//! | GET | `/runner/version` | `{ "version": …, "commit": … }`, `null` when unknown |
//!
//! `release` is `latest` (what this server ships) or this build's version; `latest` stays an
//! alias once releases come from git tags. Any other path, including every traversal, is `404`:
//! a request names one of a fixed set of release paths or nothing. A missing directory makes every
//! `/runner/` path `404`; it never stops the server. Public, no authentication: hosts check each
//! file against their own pinned SHA-256, never against this server.
//!
//! Every file carries `Cache-Control: no-cache` and a strong `ETag`, its SHA-256, so a host (or a
//! relay's release cache, spec 067) revalidates cheaply. The hash is computed on first request
//! and kept until the file's size or modification time changes.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::json;
use sha2::{Digest, Sha256};
use tower_http::services::ServeFile;

use crate::config::Config;
use crate::serve::error;

/// The architectures a release carries, in `uname -m` names (what Ansible's
/// `ansible_facts.architecture` reports).
pub const ARCHES: &[&str] = &["x86_64", "aarch64"];
/// The per-architecture binaries. `keel-relay` is present once its crate builds (spec 067).
pub const BINARIES: &[&str] = &["keel-runner", "keel-executor", "keel-relay"];
/// The architecture-independent files.
pub const COMMON_FILES: &[&str] = &["SHA256SUMS", "requirements.txt", "keel_events.tar.gz"];
/// The alias for "what this server ships".
pub const LATEST: &str = "latest";

/// Whether `path` (relative to `/runner/{release}/`) is one of the release's files. This is the
/// whole of the path check: anything else, `..` included, is not a release path.
#[must_use]
pub fn is_release_path(path: &str) -> bool {
    if COMMON_FILES.contains(&path) {
        return true;
    }
    match path.split_once('/') {
        Some((arch, bin)) => ARCHES.contains(&arch) && BINARIES.contains(&bin),
        None => false,
    }
}

/// A release file found on disk.
#[derive(Debug, Clone)]
pub struct ReleaseFile {
    pub path: PathBuf,
    pub len: u64,
    /// Lowercase hex.
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

/// The runner release on disk and the build it belongs to.
#[derive(Debug, Clone)]
pub struct RunnerDist {
    dir: PathBuf,
    version: Option<String>,
    commit: Option<String>,
    hashes: Arc<Mutex<HashMap<PathBuf, (Stamp, String)>>>,
}

impl RunnerDist {
    /// `version` is the release a `/runner/{version}/` path may name besides `latest`.
    #[must_use]
    pub fn new(dir: PathBuf, version: Option<String>, commit: Option<String>) -> Self {
        Self {
            dir,
            version,
            commit,
            hashes: Arc::default(),
        }
    }

    /// The release the image ships: `--runner-dist-dir` and the running build.
    #[must_use]
    pub fn from_config(cfg: &Config) -> Self {
        Self::new(
            cfg.runner_dist_dir.clone(),
            cfg.build.version.clone(),
            cfg.build.revision.clone(),
        )
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether `release` names what this server ships.
    #[must_use]
    pub fn is_release(&self, release: &str) -> bool {
        release == LATEST || self.version.as_deref() == Some(release)
    }

    /// The file at `path` under `release`, with its SHA-256, or `None` when the release, the
    /// path or the file is not there. Shared with the relay release cache (spec 067 T044).
    pub async fn file(&self, release: &str, path: &str) -> Option<ReleaseFile> {
        if !self.is_release(release) || !is_release_path(path) {
            return None;
        }
        let full = self.dir.join(path);
        let hashes = self.hashes.clone();
        tokio::task::spawn_blocking(move || hash_file(&full, &hashes))
            .await
            .ok()
            .flatten()
    }
}

/// Hashes `path` unless the cached hash still matches its size and modification time.
fn hash_file(
    path: &Path,
    hashes: &Mutex<HashMap<PathBuf, (Stamp, String)>>,
) -> Option<ReleaseFile> {
    let meta = std::fs::metadata(path)
        .ok()
        .filter(std::fs::Metadata::is_file)?;
    let stamp = Stamp {
        len: meta.len(),
        modified: meta.modified().ok(),
    };
    let cached = hashes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(path)
        .filter(|(s, _)| *s == stamp)
        .map(|(_, h)| h.clone());
    let sha256 = match cached {
        Some(h) => h,
        None => {
            let mut file = std::fs::File::open(path).ok()?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = file.read(&mut buf).ok()?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            let h: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            hashes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(path.to_path_buf(), (stamp, h.clone()));
            h
        }
    };
    Some(ReleaseFile {
        path: path.to_path_buf(),
        len: stamp.len,
        sha256,
    })
}

/// The `/runner` routes. Generic over the outer router's state; they carry their own.
pub fn router<S: Clone + Send + Sync + 'static>(dist: RunnerDist) -> Router<S> {
    Router::new()
        .route("/runner", get(not_found))
        .route("/runner/", get(not_found))
        .route("/runner/{*rest}", get(download))
        .with_state(dist)
}

async fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "not_found")
}

async fn download(State(dist): State<RunnerDist>, req: Request) -> Response {
    // The raw path, not the decoded one: no release path needs percent-encoding, so any `%` (an
    // encoded `/`, `.` or NUL) is refused before anything is matched.
    let Some(rest) = req.uri().path().strip_prefix("/runner/") else {
        return not_found().await;
    };
    if rest.contains('%') {
        return not_found().await;
    }
    let rest = rest.to_owned();
    if rest == "version" {
        return version(&dist).await;
    }
    let Some((release, path)) = rest.split_once('/') else {
        return not_found().await;
    };
    let Some(file) = dist.file(release, path).await else {
        return not_found().await;
    };
    let etag = format!("\"{}\"", file.sha256);
    let Ok(etag_value) = HeaderValue::from_str(&etag) else {
        return not_found().await;
    };
    if none_match(req.headers(), &etag) {
        let mut resp = StatusCode::NOT_MODIFIED.into_response();
        set_cache_headers(resp.headers_mut(), etag_value);
        return resp;
    }
    match ServeFile::new(&file.path).try_call(req).await {
        Ok(resp) if resp.status().is_success() => {
            let mut resp = resp.map(Body::new);
            set_cache_headers(resp.headers_mut(), etag_value);
            resp
        }
        Ok(resp) if resp.status() == StatusCode::NOT_FOUND => not_found().await,
        Ok(resp) => resp.map(Body::new),
        Err(e) => {
            tracing::warn!(error = %e, path = %file.path.display(), "runner download failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

async fn version(dist: &RunnerDist) -> Response {
    let dir = dist.dir.clone();
    let present = tokio::task::spawn_blocking(move || dir.is_dir())
        .await
        .unwrap_or(false);
    if !present {
        return not_found().await;
    }
    let mut resp = Json(json!({ "version": dist.version, "commit": dist.commit })).into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    resp
}

fn set_cache_headers(headers: &mut HeaderMap, etag: HeaderValue) {
    headers.insert(header::ETAG, etag);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
}

/// `If-None-Match` per RFC 9110 13.1.2: weak comparison, `*` matches any current file.
fn none_match(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .any(|tag| tag == "*" || tag.trim_start_matches("W/") == etag)
}
