//! Runner downloads (spec 055 T140; contracts/join.md "Runner downloads"): `/runner/…` served
//! from `--runner-dist-dir`. No database: these run without `KEEL_TEST_DATABASE_URL`.

use std::collections::HashMap;
use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use keel_server::config::{Config, Flags};
use keel_server::runner_dist::{self, RunnerDist};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

/// Every file the contract names, keyed by its path under `/runner/<release>/`.
const FILES: &[(&str, &[u8])] = &[
    ("x86_64/keel-runner", b"runner amd64"),
    ("x86_64/keel-executor", b"executor amd64"),
    ("x86_64/keel-relay", b"relay amd64"),
    ("aarch64/keel-runner", b"runner arm64"),
    ("aarch64/keel-executor", b"executor arm64"),
    ("aarch64/keel-relay", b"relay arm64"),
    (
        "requirements.txt",
        b"ansible-core==2.20.0 --hash=sha256:00\n",
    ),
    ("keel_events.tar.gz", b"\x1f\x8b not really gzip"),
];

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A dist directory laid out as the image's `runner-dist` stage writes it, at `<root>/dist`, with a
/// file beside it (`<root>/keel-runner-dist-secret`) that traversal must never reach.
struct Dist {
    root: tempfile::TempDir,
}

impl Dist {
    fn path(&self) -> std::path::PathBuf {
        self.root.path().join("dist")
    }
}

fn dist_dir() -> Dist {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::write(root.path().join("keel-runner-dist-secret"), b"secret").expect("secret");
    let dist = Dist { root };
    let dir = dist.path();
    std::fs::create_dir(&dir).expect("dist");
    let mut sums = String::new();
    for (path, body) in FILES {
        let full = dir.join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        std::fs::write(&full, body).expect("write");
        sums.push_str(&format!("{}  {path}\n", sha256_hex(body)));
    }
    std::fs::write(dir.join("SHA256SUMS"), sums).expect("sums");
    dist
}

fn app(dir: &Path, version: Option<&str>) -> Router {
    runner_dist::router(RunnerDist::new(
        dir.to_path_buf(),
        version.map(str::to_owned),
        Some("0123456789abcdef".to_owned()),
    ))
}

async fn get(app: &Router, uri: &str) -> axum::response::Response {
    send(app, Request::get(uri).body(Body::empty()).expect("request")).await
}

async fn send(app: &Router, req: Request<Body>) -> axum::response::Response {
    app.clone().oneshot(req).await.expect("infallible")
}

async fn body(resp: axum::response::Response) -> Vec<u8> {
    resp.into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec()
}

#[tokio::test]
async fn every_file_under_latest_is_served_with_its_sha256_as_etag() {
    let dir = dist_dir();
    let app = app(&dir.path(), None);
    let sums = std::fs::read(dir.path().join("SHA256SUMS")).expect("sums");
    let all: Vec<(&str, Vec<u8>)> = FILES
        .iter()
        .map(|(p, b)| (*p, b.to_vec()))
        .chain([("SHA256SUMS", sums)])
        .collect();
    for (path, expected) in all {
        let resp = get(&app, &format!("/runner/latest/{path}")).await;
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        let etag = resp.headers()[header::ETAG]
            .to_str()
            .expect("etag")
            .to_owned();
        assert_eq!(etag, format!("\"{}\"", sha256_hex(&expected)), "{path}");
        assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-cache", "{path}");
        assert_eq!(body(resp).await, expected, "{path}");
    }
}

#[tokio::test]
async fn sha256sums_lists_paths_as_the_ansible_role_keys_them() {
    let dir = dist_dir();
    let app = app(&dir.path(), None);
    let sums =
        String::from_utf8(body(get(&app, "/runner/latest/SHA256SUMS").await).await).expect("utf8");
    for key in [
        "x86_64/keel-runner",
        "x86_64/keel-executor",
        "aarch64/keel-runner",
        "aarch64/keel-executor",
        "requirements.txt",
        "keel_events.tar.gz",
    ] {
        assert!(
            sums.lines().any(|l| l.ends_with(&format!("  {key}"))),
            "{key} in {sums}"
        );
    }
}

#[tokio::test]
async fn a_matching_if_none_match_is_not_modified() {
    let dir = dist_dir();
    let app = app(&dir.path(), None);
    let etag = format!("\"{}\"", sha256_hex(b"runner amd64"));
    let req = Request::get("/runner/latest/x86_64/keel-runner")
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .expect("request");
    let resp = send(&app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(resp.headers()[header::ETAG], etag.as_str());
    assert!(body(resp).await.is_empty());
}

#[tokio::test]
async fn a_changed_file_gets_a_new_etag() {
    let dir = dist_dir();
    let app = app(&dir.path(), None);
    let first = get(&app, "/runner/latest/requirements.txt").await;
    let first = first.headers()[header::ETAG].clone();
    // Replaced by rename, as an image upgrade or a re-run of the release target would.
    let tmp = dir.path().join("requirements.txt.new");
    std::fs::write(&tmp, b"ansible-core==2.21.0 and a longer line\n").expect("write");
    std::fs::rename(&tmp, dir.path().join("requirements.txt")).expect("rename");
    let second = get(&app, "/runner/latest/requirements.txt").await;
    assert_ne!(second.headers()[header::ETAG], first);
    assert_eq!(
        second.headers()[header::ETAG].to_str().unwrap(),
        format!(
            "\"{}\"",
            sha256_hex(b"ansible-core==2.21.0 and a longer line\n")
        )
    );
}

#[tokio::test]
async fn unknown_arch_unknown_file_and_traversal_are_404() {
    let dir = dist_dir();
    // Files that exist on disk but are not part of the release: still 404.
    std::fs::create_dir_all(dir.path().join("armv7")).expect("mkdir");
    std::fs::write(dir.path().join("armv7/keel-runner"), b"x").expect("write");
    std::fs::write(dir.path().join("notes.txt"), b"x").expect("write");
    let app = app(&dir.path(), None);
    for uri in [
        "/runner",
        "/runner/",
        "/runner/latest",
        "/runner/latest/",
        "/runner/latest/armv7/keel-runner",
        "/runner/latest/riscv64/keel-runner",
        "/runner/latest/x86_64/",
        "/runner/latest/x86_64/keel-runner/",
        "/runner/latest/x86_64/sh",
        "/runner/latest/notes.txt",
        "/runner/latest/x86_64",
        "/runner/latest/../latest/x86_64/keel-runner",
        "/runner/latest/x86_64/../SHA256SUMS",
        "/runner/latest/x86_64/../../keel-runner-dist-secret",
        "/runner/latest/%2e%2e/keel-runner-dist-secret",
        "/runner/latest/%2E%2E%2Fkeel-runner-dist-secret",
        "/runner/latest/x86_64%2fkeel-runner",
        "/runner/latest/x86_64/..%2f..%2fkeel-runner-dist-secret",
        "/runner/latest/%2e%2e%2f%2e%2e%2fetc%2fpasswd",
        "/runner/latest//x86_64/keel-runner",
        "/runner/latest/./SHA256SUMS",
        "/runner/latest/x86_64/keel-runner%00",
        "/runner/..%2f..%2fetc/passwd",
        "/runner/%2e%2e/SHA256SUMS",
        "/runner/0.0.0/SHA256SUMS",
        "/runner/version/extra",
    ] {
        let resp = get(&app, uri).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn only_get_and_head_are_answered() {
    let dir = dist_dir();
    let app = app(&dir.path(), None);
    let head = Request::head("/runner/latest/SHA256SUMS")
        .body(Body::empty())
        .expect("request");
    let resp = send(&app, head).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().contains_key(header::ETAG));
    let post = Request::post("/runner/latest/SHA256SUMS")
        .body(Body::empty())
        .expect("request");
    assert_eq!(
        send(&app, post).await.status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn a_missing_dist_dir_is_404_everywhere() {
    let parent = tempfile::tempdir().expect("tempdir");
    let app = app(&parent.path().join("absent"), Some("1.2.3"));
    for uri in [
        "/runner/version",
        "/runner/latest/SHA256SUMS",
        "/runner/latest/x86_64/keel-runner",
        "/runner/1.2.3/requirements.txt",
    ] {
        assert_eq!(
            get(&app, uri).await.status(),
            StatusCode::NOT_FOUND,
            "{uri}"
        );
    }
}

#[test]
fn a_missing_dist_dir_does_not_stop_the_server_from_starting() {
    let env: HashMap<&str, String> = HashMap::from([
        ("KEEL_PUBLIC_URL", "https://keel.example.com".to_owned()),
        ("KEEL_DATABASE_URL", "postgres://unused/keel".to_owned()),
        ("KEEL_MASTER_KEY_FILE", "/nonexistent/master.key".to_owned()),
        ("KEEL_GITHUB_CLIENT_ID", "gh-id".to_owned()),
        ("KEEL_GITHUB_CLIENT_SECRET", "gh-secret".to_owned()),
        (
            "KEEL_RUNNER_DIST_DIR",
            "/nonexistent/keel/runner".to_owned(),
        ),
    ]);
    let cfg = Config::resolve(Flags::default(), &|k| env.get(k).cloned()).expect("config");
    assert_eq!(
        cfg.runner_dist_dir,
        std::path::PathBuf::from("/nonexistent/keel/runner")
    );
    // Building the routes touches nothing on disk.
    let _ = runner_dist::router::<()>(RunnerDist::from_config(&cfg));
}

#[test]
fn the_dist_dir_defaults_to_the_image_path() {
    let env: HashMap<&str, String> = HashMap::from([
        ("KEEL_PUBLIC_URL", "https://keel.example.com".to_owned()),
        ("KEEL_DATABASE_URL", "postgres://unused/keel".to_owned()),
        ("KEEL_MASTER_KEY_FILE", "/nonexistent/master.key".to_owned()),
        ("KEEL_GITHUB_CLIENT_ID", "gh-id".to_owned()),
        ("KEEL_GITHUB_CLIENT_SECRET", "gh-secret".to_owned()),
    ]);
    let cfg = Config::resolve(Flags::default(), &|k| env.get(k).cloned()).expect("config");
    assert_eq!(
        cfg.runner_dist_dir,
        std::path::PathBuf::from("/usr/share/keel/runner")
    );
}

#[tokio::test]
async fn version_reports_the_build() {
    let dir = dist_dir();
    let app = app(&dir.path(), Some("1.2.3"));
    let resp = get(&app, "/runner/version").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-cache");
    let v: Value = serde_json::from_slice(&body(resp).await).expect("json");
    assert_eq!(v["version"], "1.2.3");
    assert_eq!(v["commit"], "0123456789abcdef");
}

#[tokio::test]
async fn an_untagged_build_reports_an_unknown_version() {
    let dir = dist_dir();
    let app = app(&dir.path(), None);
    let v: Value =
        serde_json::from_slice(&body(get(&app, "/runner/version").await).await).expect("json");
    assert_eq!(v["version"], Value::Null);
}

#[tokio::test]
async fn the_release_version_serves_the_same_files_and_latest_stays_an_alias() {
    let dir = dist_dir();
    let app = app(&dir.path(), Some("1.2.3"));
    let a = get(&app, "/runner/latest/x86_64/keel-executor").await;
    let b = get(&app, "/runner/1.2.3/x86_64/keel-executor").await;
    assert_eq!(a.status(), StatusCode::OK);
    assert_eq!(b.status(), StatusCode::OK);
    assert_eq!(a.headers()[header::ETAG], b.headers()[header::ETAG]);
    assert_eq!(body(a).await, body(b).await);
    // Another version is not this server's release.
    assert_eq!(
        get(&app, "/runner/1.2.4/x86_64/keel-executor")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[test]
fn release_paths_are_exactly_the_contract() {
    for ok in [
        "SHA256SUMS",
        "requirements.txt",
        "keel_events.tar.gz",
        "x86_64/keel-runner",
        "aarch64/keel-executor",
        "aarch64/keel-relay",
    ] {
        assert!(runner_dist::is_release_path(ok), "{ok}");
    }
    for bad in [
        "",
        "/SHA256SUMS",
        "x86_64",
        "x86_64/",
        "arm64/keel-runner",
        "amd64/keel-runner",
        "x86_64/keel-runner/",
        "../SHA256SUMS",
        "x86_64/../SHA256SUMS",
        "x86_64\\keel-runner",
    ] {
        assert!(!runner_dist::is_release_path(bad), "{bad}");
    }
}
