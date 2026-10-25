//! The sign-in slice end to end, in-process: a fake Discord provider, the real router.

mod support;

use axum::body::Body;
use http::{Request, StatusCode, header};
use support::*;
use url::Url;

#[tokio::test]
async fn healthz_answers_with_the_security_headers() {
    let a = app();
    let r = a.get("/healthz", None).await;
    assert_eq!(r.status(), StatusCode::OK);
    let h = r.headers();
    assert!(
        h[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .starts_with("default-src 'none'")
    );
    assert!(h.contains_key(header::STRICT_TRANSPORT_SECURITY));
    assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(h[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(h[header::REFERRER_POLICY], "no-referrer");
    assert!(!h.contains_key(header::SERVER));
}

#[tokio::test]
async fn me_signed_out_is_a_401_naming_the_providers() {
    let a = app();
    let r = a.get("/api/v1/me", None).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
    let b = json(r).await;
    assert_eq!(b["error"], "unauthenticated");
    assert_eq!(b["providers"], serde_json::json!(["discord"]));
}

#[tokio::test]
async fn unknown_api_and_auth_paths_are_json_404s() {
    let a = app();
    for p in [
        "/api",
        "/api/v1/nope",
        "/api/v2/me",
        "/auth/nope",
        "/auth/login/nope",
    ] {
        let r = a.get(p, None).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{p}");
        let b = json(r).await;
        assert!(b["error"].is_string(), "{p}: {b}");
    }
    // A wrong method on a known route is a JSON 405.
    let r = a
        .send(Request::put("/api/v1/me").body(Body::empty()).unwrap())
        .await;
    assert!(r.status().is_client_error());
}

#[tokio::test]
async fn the_owner_signs_in_as_admin() {
    let a = app();
    let r = a.get("/auth/login/discord", None).await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let cookie = set_cookies(&r)
        .into_iter()
        .find(|c| c.starts_with("__Host-depths_login="))
        .unwrap();
    assert!(
        cookie.contains("Secure") && cookie.contains("HttpOnly"),
        "{cookie}"
    );
    let redirect = Url::parse(location(&r)).unwrap();
    let cb = redirect
        .query_pairs()
        .find(|(k, _)| k == "redirect_uri")
        .unwrap()
        .1;
    assert_eq!(cb, "https://depth.boop.ninja/auth/callback/discord");

    let session = a.sign_in(OWNER).await;
    let r = a.get("/api/v1/me", Some(&session)).await;
    assert_eq!(r.status(), StatusCode::OK);
    let me = json(r).await;
    assert_eq!(me["role"], "admin");
    assert_eq!(me["owner"], true);
    assert_eq!(me["identity"]["provider"], "discord");
    assert_eq!(me["identity"]["subject"], OWNER);
    assert!(me["csrf_token"].as_str().unwrap().len() > 20);
    assert_eq!(me["features"]["shell"], true);
}

#[tokio::test]
async fn the_session_cookie_is_host_secure_httponly_lax() {
    let a = app();
    let r = a.get("/auth/login", None).await;
    let state = Url::parse(location(&r))
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let login = cookie_pair(&r, "__Host-depths_login");
    let r = a
        .get(
            &format!("/auth/callback/discord?code=7&state={state}"),
            Some(&login),
        )
        .await;
    let c = set_cookies(&r)
        .into_iter()
        .find(|c| c.starts_with("__Host-depths_session="))
        .unwrap();
    for attr in ["Secure", "HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(c.contains(attr), "{attr} missing from {c}");
    }
    assert!(!c.contains("Domain"), "{c}");
}

#[tokio::test]
async fn others_sign_in_as_pending() {
    let a = app();
    let session = a.sign_in("555").await;
    let me = json(a.get("/api/v1/me", Some(&session)).await).await;
    assert_eq!(me["role"], "pending");
    assert_eq!(me["owner"], false);
}

#[tokio::test]
async fn me_reports_the_files_mode_and_limits() {
    // 053 T025: the default is `full`; the limits are the configured ones, for everyone.
    let a = app();
    let session = a.sign_in(OWNER).await;
    let me = json(a.get("/api/v1/me", Some(&session)).await).await;
    assert_eq!(me["features"]["files"], "full", "{me}");
    assert!(me["features"].get("files_reason").is_none(), "{me}");
    let files = &a.state.cfg.files;
    assert_eq!(
        me["limits"]["files"],
        serde_json::json!({
            "editor_max": files.editor_max,
            "download_max": files.download_max,
            "upload_max": files.upload_max,
        })
    );

    let a = app_with(|s| {
        let mut cfg = (*s.cfg).clone();
        cfg.features.files = depths_server::config::FilesMode::ReadOnly;
        s.cfg = std::sync::Arc::new(cfg);
    });
    let session = a.sign_in("555").await;
    let me = json(a.get("/api/v1/me", Some(&session)).await).await;
    assert_eq!(me["features"]["files"], "read_only", "{me}");
}

#[tokio::test]
async fn me_reports_files_off_with_its_reason_to_admins_only() {
    // 053 T019: a helper that cannot run here turns the feature off; admins see why.
    let a = app();
    a.state.files.force_off(depths_server::files::NO_IMAGE);
    let admin = a.sign_in(OWNER).await;
    let me = json(a.get("/api/v1/me", Some(&admin)).await).await;
    assert_eq!(me["features"]["files"], "off", "{me}");
    assert_eq!(me["features"]["files_reason"], "no helper image", "{me}");
    let other = a.sign_in("555").await;
    let me = json(a.get("/api/v1/me", Some(&other)).await).await;
    assert_eq!(me["features"]["files"], "off", "{me}");
    assert!(me["features"].get("files_reason").is_none(), "{me}");
}

#[tokio::test]
async fn sign_ins_nobody_admitted_cannot_lock_the_admins_out() {
    // T099 F1: anyone with a Discord account can sign in and hold a pending session. Enough of
    // them fill the global session cap; the owner must still get in, by evicting one of them.
    let a = app();
    let cap = web_auth::session::SessionConfig::default().max_sessions;
    let sessions = &a.state.auth.sessions;
    for i in 0..cap {
        sessions
            .create(discord_identity(&format!("9{i:03}")), fake_tokens())
            .unwrap();
    }
    assert_eq!(sessions.len(), cap);

    let session = a.sign_in(OWNER).await;
    let me = json(a.get("/api/v1/me", Some(&session)).await).await;
    assert_eq!(me["role"], "admin");
    assert_eq!(sessions.len(), cap, "one pending session made room");

    // Someone nobody admitted is still turned away at the cap, and evicts nobody.
    let r = a.get("/auth/login?provider=discord", None).await;
    let state = Url::parse(location(&r))
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let login = cookie_pair(&r, "__Host-depths_login");
    let r = a
        .get(
            &format!("/auth/callback/discord?code=777&state={state}"),
            Some(&login),
        )
        .await;
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(sessions.len(), cap);
    let me = a.get("/api/v1/me", Some(&session)).await;
    assert_eq!(me.status(), StatusCode::OK, "the owner's session survives");
}

#[tokio::test]
async fn a_callback_without_its_login_cookie_is_refused_without_echoing() {
    let a = app();
    let r = a
        .get("/auth/callback/discord?code=1&state=%3Cscript%3Ex", None)
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(
        set_cookies(&r)
            .iter()
            .all(|c| !c.starts_with("__Host-depths_session="))
    );
    let body = text(r).await;
    assert!(!body.contains("script"), "{body}");
    assert!(body.contains("href=\"/\""));
}

#[tokio::test]
async fn a_replayed_or_forged_state_is_refused() {
    let a = app();
    let r = a.get("/auth/login", None).await;
    let login = cookie_pair(&r, "__Host-depths_login");
    let r = a
        .get("/auth/callback/discord?code=1&state=forged", Some(&login))
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_return_path_is_always_root() {
    let a = app();
    // A return path in the login request is not followed.
    let r = a
        .get(
            "/auth/login?provider=discord&return_to=https://evil.example",
            None,
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let session = a.sign_in("9").await;
    assert!(!session.is_empty());
}

#[tokio::test]
async fn logout_needs_the_csrf_token_and_origin() {
    let a = app();
    let session = a.sign_in(OWNER).await;
    let csrf = json(a.get("/api/v1/me", Some(&session)).await).await["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned();

    // No token.
    let r = a
        .send(
            Request::post("/auth/logout")
                .header(header::COOKIE, &session)
                .header(header::ORIGIN, ORIGIN)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // Wrong origin.
    let r = a
        .send(
            Request::post("/auth/logout")
                .header(header::COOKIE, &session)
                .header(header::ORIGIN, "https://evil.example")
                .header("x-csrf-token", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // Right.
    let r = a
        .send(
            Request::post("/auth/logout")
                .header(header::COOKIE, &session)
                .header(header::ORIGIN, ORIGIN)
                .header("x-csrf-token", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(
        set_cookies(&r)
            .iter()
            .any(|c| c.starts_with("__Host-depths_session=") && c.contains("Max-Age=0"))
    );
    let r = a.get("/api/v1/me", Some(&session)).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_keeps_a_valid_session_and_ends_a_rejected_one() {
    let a = app();
    for (subject, expect) in [(OWNER, StatusCode::OK), ("2", StatusCode::UNAUTHORIZED)] {
        let session = a.sign_in(subject).await;
        let csrf = json(a.get("/api/v1/me", Some(&session)).await).await["csrf_token"]
            .as_str()
            .unwrap()
            .to_owned();
        let r = a
            .send(
                Request::post("/auth/refresh")
                    .header(header::COOKIE, &session)
                    .header(header::ORIGIN, ORIGIN)
                    .header("x-csrf-token", &csrf)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(r.status(), expect, "{subject}");
        let after = a.get("/api/v1/me", Some(&session)).await.status();
        assert_eq!(
            after == StatusCode::OK,
            expect == StatusCode::OK,
            "{subject}"
        );
    }
}

#[tokio::test]
async fn sign_in_is_rate_limited_per_client() {
    let a = app();
    let mut statuses = Vec::new();
    for _ in 0..7 {
        statuses.push(a.get("/auth/login", None).await);
    }
    assert!(
        statuses[..5]
            .iter()
            .all(|r| r.status() == StatusCode::SEE_OTHER)
    );
    let last = statuses.pop().unwrap();
    assert_eq!(last.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(last.headers().contains_key(header::RETRY_AFTER));
    assert_eq!(json(last).await["error"], "rate_limited");
}

#[tokio::test]
async fn the_spa_is_served_with_a_fallback_and_no_traversal() {
    let a = app();
    let r = a.get("/", None).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()[header::CACHE_CONTROL], "no-cache");
    assert!(text(r).await.contains("Depths"));
    // Client-side routes get index.html.
    let r = a.get("/stacks/web", None).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(text(r).await.contains("Depths"));
    // Assets are immutable; a missing asset is a 404, not index.html.
    let r = a.get("/assets/app-abc.js", None).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(
        r.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    let r = a.get("/assets/missing.js", None).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    // Nothing outside the static dir.
    for p in [
        "/../secret.txt",
        "/%2e%2e/secret.txt",
        "/assets/../../secret.txt",
    ] {
        let r = a.get(p, None).await;
        assert!(!text(r).await.contains("top secret"), "{p}");
    }
}

#[tokio::test]
async fn the_ws_upgrade_needs_the_app_origin() {
    let a = app();
    let r = a
        .send(
            Request::get("/api/v1/ws")
                .header(header::ORIGIN, "https://evil.example")
                .header(header::CONNECTION, "upgrade")
                .header(header::UPGRADE, "websocket")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn readyz_needs_docker_and_never_shows_its_error() {
    let a = app();
    let r = a.get("/readyz", None).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(json(r).await["status"], "ready");

    let b = app();
    *b.runtime.unavailable.lock().unwrap() = Some("permission denied on docker.sock".into());
    let r = b.get("/readyz", None).await;
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = text(r).await;
    assert!(body.contains("not_ready"), "{body}");
    assert!(!body.contains("docker.sock"), "{body}");
}

#[tokio::test]
async fn the_audit_export_is_jsonl_for_admins_only() {
    let a = app();
    let admin = a.sign_in(OWNER).await;
    let pending = a.sign_in("555").await;

    let r = a.get("/api/v1/audit/export", Some(&admin)).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()[header::CONTENT_TYPE], "application/x-ndjson");
    assert!(
        r.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
    let body = text(r).await;
    let lines: Vec<serde_json::Value> = body
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // Each sign-in, and (spec 052) each person's first network.
    assert_eq!(lines.len(), 4, "{body}");
    let count = |a: &str| lines.iter().filter(|l| l["action"] == a).count();
    assert_eq!(count("sign_in"), 2, "{body}");
    assert_eq!(count("network_first"), 2, "{body}");

    // A filter narrows; a bad one is a 400.
    let r = a
        .get("/api/v1/audit/export?action=sign_out", Some(&admin))
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(text(r).await.is_empty());
    let r = a
        .get("/api/v1/audit/export?action=nonsense", Some(&admin))
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);

    // Pending: refused, and the refusal is audited.
    let r = a.get("/api/v1/audit/export", Some(&pending)).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(json(r).await["error"], "forbidden");
    assert!(
        a.audit_entries()
            .iter()
            .any(|e| e.action == depths_protocol::AuditAction::Denied && e.actor.subject == "555")
    );

    // Signed out: 401.
    let r = a.get("/api/v1/audit/export", None).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refused_callbacks_are_audited_as_denied() {
    let a = app();
    // No login cookie (another browser).
    let r = a.get("/auth/callback/discord?code=1&state=x", None).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    // A replayed state: the first callback consumes it.
    let r = a.get("/auth/login", None).await;
    let state = Url::parse(location(&r))
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let login = cookie_pair(&r, "__Host-depths_login");
    let cb = format!("/auth/callback/discord?code=7&state={state}");
    assert_eq!(
        a.get(&cb, Some(&login)).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        a.get(&cb, Some(&login)).await.status(),
        StatusCode::BAD_REQUEST
    );

    let entries = a.audit_entries();
    let denied: Vec<_> = entries
        .iter()
        .filter(|e| e.action == depths_protocol::AuditAction::Denied)
        .collect();
    assert_eq!(denied.len(), 2, "{entries:?}");
    for e in &denied {
        assert!(e.actor.subject.is_empty());
        match &e.outcome {
            depths_protocol::AuditOutcome::Denied { reason } => {
                assert!(reason.starts_with("sign-in refused"), "{reason}")
            }
            o => panic!("{o:?}"),
        }
    }
    assert!(
        entries
            .iter()
            .any(|e| e.action == depths_protocol::AuditAction::SignIn && e.actor.subject == "7")
    );
}

#[tokio::test]
async fn static_serving_refuses_traversal_dotfiles_and_odd_paths() {
    let a = app();
    let web = a.stacks.parent().unwrap().join("web");
    std::fs::write(web.join(".env"), "top secret").unwrap();
    std::fs::write(web.join("assets/.hidden"), "top secret").unwrap();
    for p in [
        "/.env",
        "/assets/.hidden",
        "/..%2fsecret.txt",
        "/%2e%2e/secret.txt",
        "/%2E%2E%2Fsecret.txt",
        "/assets/%2e%2e/%2e%2e/secret.txt",
        "//secret.txt",
        "/assets//app-abc.js",
        "/%5c..%5csecret.txt",
        "/assets/..%5c..%5csecret.txt",
        "/.git/config",
    ] {
        let r = a.get(p, None).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{p}");
        assert_eq!(r.headers()[header::CONTENT_TYPE], "application/json", "{p}");
        let body = text(r).await;
        assert!(!body.contains("top secret"), "{p}");
        assert_eq!(body, r#"{"error":"not_found"}"#, "{p}");
    }
    // A directory is never listed.
    let r = a.get("/assets/", None).await;
    assert!(!text(r).await.contains("app-abc.js"));
    // The ordinary paths still work.
    for p in ["/", "/assets/app-abc.js", "/stacks/web/"] {
        assert_eq!(a.get(p, None).await.status(), StatusCode::OK, "{p}");
    }
}

#[test]
fn hostile_paths_are_recognised() {
    use depths_server::http::path_is_hostile;
    for p in [
        "/",
        "/index.html",
        "/assets/a-1.js",
        "/stacks/web/",
        "/auth/callback/discord",
    ] {
        assert!(!path_is_hostile(p), "{p}");
    }
    for p in [
        "", "a", "/..", "/./x", "/x/..", "/%2e", "/a%2fb", "/a%5Cb", "/a\\b", "//", "/a//b",
    ] {
        assert!(path_is_hostile(p), "{p}");
    }
}

/// T093: the whole header set, on a 200, a 401, a 404 and a static file.
#[tokio::test]
async fn every_response_carries_the_full_header_set() {
    let a = app();
    for (path, status, cache) in [
        ("/healthz", StatusCode::OK, "no-cache"),
        ("/api/v1/me", StatusCode::UNAUTHORIZED, "no-store"),
        ("/api/v1/nope", StatusCode::NOT_FOUND, "no-store"),
        (
            "/assets/app-abc.js",
            StatusCode::OK,
            "public, max-age=31536000, immutable",
        ),
        ("/", StatusCode::OK, "no-cache"),
    ] {
        let r = a.get(path, None).await;
        assert_eq!(r.status(), status, "{path}");
        let h = r.headers();
        assert_eq!(
            h[header::CONTENT_SECURITY_POLICY],
            depths_server::headers::CSP,
            "{path}"
        );
        assert_eq!(
            h[header::STRICT_TRANSPORT_SECURITY],
            depths_server::headers::HSTS,
            "{path}"
        );
        let pp = h["permissions-policy"].to_str().unwrap();
        assert_eq!(pp, depths_server::headers::PERMISSIONS_POLICY, "{path}");
        for p in [
            "camera=()",
            "microphone=()",
            "geolocation=()",
            "usb=()",
            "payment=()",
        ] {
            assert!(pp.contains(p), "{path}: {p}");
        }
        assert_eq!(h["cross-origin-opener-policy"], "same-origin", "{path}");
        assert_eq!(h["cross-origin-resource-policy"], "same-origin", "{path}");
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{path}");
        assert_eq!(h[header::CACHE_CONTROL], cache, "{path}");
        assert!(!h.contains_key(header::SERVER), "{path}");
        assert!(
            !h.keys().any(|k| k.as_str().starts_with("access-control-")),
            "{path}"
        );
    }
}

/// T072: the periodic revalidation. Subject "2" is rejected by the fake provider, "3" finds it
/// unreachable, the owner is valid.
#[tokio::test]
async fn the_refresh_loop_ends_rejected_sessions_and_backs_off_when_unreachable() {
    use depths_server::refresh::{BACKOFF_MIN, EVERY, Pass, Refresher};
    use std::time::Duration;
    let a = app_with(|s| {
        s.limits = std::sync::Arc::new(depths_server::limits::Limits {
            auth: depths_server::limits::RateLimiter::new(600, 100, 100),
            ..Default::default()
        })
    });
    let ok = a.sign_in(OWNER).await;
    let rejected = a.sign_in("2").await;
    let unreachable = a.sign_in("3").await;
    let r = Refresher::default();
    let start = tokio::time::Instant::now();

    // Nothing is due before the period.
    assert_eq!(r.pass(&a.state, start).await, Pass::default());

    let t1 = start + EVERY + Duration::from_secs(1);
    let p = r.pass(&a.state, t1).await;
    assert_eq!(
        p,
        Pass {
            checked: 3,
            ended: 1,
            deferred: 1
        }
    );
    for (cookie, alive) in [(&ok, true), (&rejected, false), (&unreachable, true)] {
        let s = a.get("/api/v1/me", Some(cookie)).await.status();
        assert_eq!(s == StatusCode::OK, alive, "{cookie}");
    }

    // The unreachable one is retried after the backoff, the valid one only next period.
    let id = |c: &str| web_auth::session::session_handle(c.split_once('=').unwrap().1);
    assert_eq!(r.next_check(&id(&unreachable)), Some(t1 + BACKOFF_MIN));
    assert_eq!(r.next_check(&id(&ok)), Some(t1 + EVERY));
    assert_eq!(r.next_check(&id(&rejected)), None);
    let p = r.pass(&a.state, t1 + Duration::from_secs(10)).await;
    assert_eq!(p.checked, 0);
    let t2 = t1 + BACKOFF_MIN;
    let p = r.pass(&a.state, t2).await;
    assert_eq!(p.deferred, 1);
    assert_eq!(r.next_check(&id(&unreachable)), Some(t2 + BACKOFF_MIN * 2));
}

// ---------------------------------------------------------------------------
// Spec 053 US3: folder downloads and confirmed uploads (T047)
// ---------------------------------------------------------------------------

mod transfer {
    use super::*;

    use std::net::IpAddr;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use depths_protocol::{AuditAction, AuditOutcome, MountId, VersionReason};
    use depths_server::files::{FileKey, HistorySink};
    use http_body_util::BodyExt as _;
    use serde_json::{Value, json};
    use support::volumes::{self, FakeVolumeHelper};
    use support::ws::{self, Socket};

    const CAFE: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 9));

    struct Vol {
        a: App,
        helper: Arc<FakeVolumeHelper>,
        root: PathBuf,
        mount: MountId,
        _dir: tempfile::TempDir,
    }

    /// `valheim` mounting a volume with `a.txt`, `sub/b.txt` and `sub/deep/c.txt`.
    fn vol(env: &[(&str, &str)]) -> Vol {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vol");
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::write(root.join("a.txt"), b"hello a").unwrap();
        std::fs::write(root.join("sub/b.txt"), b"hello b").unwrap();
        std::fs::write(root.join("sub/deep/c.txt"), b"hello c").unwrap();
        let a = app_with_env(env, |_| {});
        let (helper, mount) = volumes::install(&a, root.clone());
        Vol {
            a,
            helper,
            root,
            mount,
            _dir: dir,
        }
    }

    /// The owner's socket and session cookie, and the session's CSRF token.
    struct Owner {
        s: Socket,
        _server: tokio::task::JoinHandle<()>,
        cookie: String,
        csrf: String,
        next: u64,
    }

    async fn owner(v: &Vol) -> Owner {
        let (s, server, cookie) = ws::open(&v.a, OWNER).await;
        let csrf = ws::csrf_of(&v.a, &cookie).await;
        Owner {
            s,
            _server: server,
            cookie,
            csrf,
            next: 1,
        }
    }

    impl Owner {
        async fn ask(&mut self, cmd: Value) -> Value {
            self.next += 1;
            ws::ask(&mut self.s, self.next, cmd).await["event"].clone()
        }

        /// `PreviewFileOp{Upload}` of `files` into `dir`.
        async fn preview(&mut self, v: &Vol, dir: &str, files: &[(&str, u64)]) -> Value {
            let files: Vec<Value> = files
                .iter()
                .map(|(name, size)| json!({"name": name, "size": size}))
                .collect();
            self.ask(
                json!({"type": "PreviewFileOp", "data": {"stack": "valheim", "op": {
                    "kind": "upload",
                    "options": {"mount": v.mount, "dir": dir, "files": files},
                }}}),
            )
            .await
        }

        async fn confirm(&mut self, preview: &Value) -> Value {
            let id = preview["data"]["preview_id"].clone();
            self.ask(json!({"type": "Confirm", "data": {"preview_id": id}}))
                .await
        }

        /// A previewed and confirmed upload of `files` into `dir`; its preview id.
        async fn confirmed(&mut self, v: &Vol, dir: &str, files: &[(&str, u64)]) -> String {
            let p = self.preview(v, dir, files).await;
            assert_eq!(p["type"], "FilePreview", "{p}");
            let done = self.confirm(&p).await;
            assert_eq!(done["type"], "FileOpDone", "{done}");
            p["data"]["preview_id"].as_str().unwrap().to_owned()
        }

        fn post(&self, preview: &str, name: &str, body: Vec<u8>) -> Request<Body> {
            Request::post(upload_uri(preview, name))
                .header(header::COOKIE, &self.cookie)
                .header(header::ORIGIN, ORIGIN)
                .header("x-csrf-token", &self.csrf)
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(header::CONTENT_LENGTH, body.len())
                .body(Body::from(body))
                .unwrap()
        }

        async fn upload(&self, v: &Vol, preview: &str, name: &str, body: &[u8]) -> Response {
            v.a.send(self.post(preview, name, body.to_vec())).await
        }
    }

    type Response = http::Response<Body>;

    fn upload_uri(preview: &str, name: &str) -> String {
        let q: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("preview", preview)
            .append_pair("name", name)
            .finish();
        format!("/api/v1/stacks/valheim/files/upload?{q}")
    }

    fn download_uri(mount: &MountId, path: &str) -> String {
        let q: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("mount", mount.as_str())
            .append_pair("path", path)
            .finish();
        format!("/api/v1/stacks/valheim/files/download?{q}")
    }

    async fn bytes(r: Response) -> Vec<u8> {
        r.into_body().collect().await.unwrap().to_bytes().to_vec()
    }

    fn audited(a: &App, action: AuditAction) -> Vec<depths_protocol::AuditEntry> {
        a.audit_entries()
            .into_iter()
            .filter(|e| e.action == action)
            .collect()
    }

    fn rw_starts(v: &Vol) -> usize {
        v.helper
            .specs()
            .iter()
            .filter(|s| s.mode == depths_core::ports::OpMode::ReadWrite)
            .count()
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[tokio::test]
    async fn an_upload_preview_names_the_folder_and_every_file_and_marks_replacements() {
        let v = vol(&[]);
        let mut o = owner(&v).await;
        let p = o.preview(&v, "sub", &[("b.txt", 3), ("new.bin", 4)]).await;
        assert_eq!(p["type"], "FilePreview", "{p}");
        let summary = p["data"]["summary"].as_str().unwrap();
        for word in ["sub", "b.txt", "new.bin"] {
            assert!(summary.contains(word), "{summary}");
        }
        let uploads = p["data"]["uploads"].as_array().unwrap();
        assert_eq!(uploads.len(), 2, "{p}");
        assert_eq!(uploads[0]["name"], "b.txt");
        assert_eq!(uploads[0]["size"], 3);
        assert_eq!(uploads[0]["exists"], true);
        let fp = uploads[0]["fingerprint"].as_str().unwrap();
        assert!(fp.starts_with("sha256:") && fp.contains(":7:"), "{fp}");
        assert_eq!(uploads[1]["name"], "new.bin");
        assert_eq!(uploads[1]["exists"], false);
        assert!(uploads[1]["fingerprint"].is_null(), "{p}");
        assert_eq!(rw_starts(&v), 0, "a preview writes nothing");

        // A name twice, a folder's name, or a file above upload_max is refused at the preview.
        let p = o.preview(&v, "", &[("x", 1), ("x", 2)]).await;
        assert_eq!(p["data"]["kind"], "invalid_name", "{p}");
        let p = o.preview(&v, "", &[("sub", 1)]).await;
        assert_eq!(p["data"]["kind"], "exists", "{p}");
    }

    #[tokio::test]
    async fn an_upload_preview_expires_like_any_other() {
        let v = vol(&[]);
        let mut o = owner(&v).await;
        v.a.state
            .files
            .set_preview_ttl(std::time::Duration::from_millis(200));
        let p = o.preview(&v, "", &[("late.txt", 2)]).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let r = o.confirm(&p).await;
        assert_eq!(r["type"], "PreviewVoid", "{r}");
        assert_eq!(r["data"]["reason"], "expired", "{r}");

        let id = o.confirmed(&v, "", &[("late.txt", 2)]).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let r = o.upload(&v, &id, "late.txt", b"hi").await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert_eq!(json(r).await["kind"], "not_confirmed");
        assert!(!v.root.join("late.txt").exists());
    }

    #[tokio::test]
    async fn a_post_needs_a_confirmed_preview_covering_its_name_and_size() {
        let v = vol(&[]);
        let mut o = owner(&v).await;
        let p = o
            .preview(&v, "sub", &[("one.txt", 3), ("two.txt", 5)])
            .await;
        let id = p["data"]["preview_id"].as_str().unwrap().to_owned();

        // Not confirmed yet.
        let r = o.upload(&v, &id, "one.txt", b"one").await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert_eq!(json(r).await["kind"], "not_confirmed");
        assert_eq!(o.confirm(&p).await["type"], "FileOpDone");

        // A name the preview did not cover, a size it did not, an unknown preview.
        for (preview, name, body) in [
            (id.as_str(), "three.txt", &b"333"[..]),
            (id.as_str(), "one.txt", &b"four"[..]),
            (
                "00000000-0000-4000-8000-000000000000",
                "one.txt",
                &b"one"[..],
            ),
        ] {
            let r = o.upload(&v, preview, name, body).await;
            assert_eq!(r.status(), StatusCode::FORBIDDEN, "{name}");
            let j = json(r).await;
            assert_eq!(j["kind"], "not_confirmed", "{j}");
            assert_eq!(j["error"], "not_confirmed", "{j}");
        }
        assert_eq!(rw_starts(&v), 0);

        let r = o.upload(&v, &id, "one.txt", b"one").await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let fp = json(r).await["fingerprint"].as_str().unwrap().to_owned();
        assert!(fp.starts_with("sha256:") && fp.contains(":3:"), "{fp}");
        assert_eq!(std::fs::read(v.root.join("sub/one.txt")).unwrap(), b"one");

        // Each name once.
        let r = o.upload(&v, &id, "one.txt", b"one").await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let r = o.upload(&v, &id, "two.txt", b"two!!").await;
        assert_eq!(r.status(), StatusCode::CREATED);

        let ups = audited(&v.a, AuditAction::FilesUpload);
        assert_eq!(ups.len(), 2, "{ups:?}");
        assert!(ups.iter().all(|e| e.outcome == AuditOutcome::Succeeded));
        let mut sizes: Vec<_> = ups.iter().map(|e| e.file.as_ref().unwrap().bytes).collect();
        sizes.sort();
        assert_eq!(sizes, [Some(3), Some(5)]);
        let refused = audited(&v.a, AuditAction::FilesRefused);
        assert!(refused.len() >= 4, "{refused:?}");
    }

    #[tokio::test]
    async fn an_upload_needs_the_csrf_header_and_the_origin() {
        let v = vol(&[]);
        let mut o = owner(&v).await;
        let id = o.confirmed(&v, "", &[("c.txt", 1)]).await;
        let mut no_token = o.post(&id, "c.txt", b"c".to_vec());
        no_token.headers_mut().remove("x-csrf-token");
        assert_eq!(v.a.send(no_token).await.status(), StatusCode::FORBIDDEN);
        let mut elsewhere = o.post(&id, "c.txt", b"c".to_vec());
        elsewhere.headers_mut().insert(
            header::ORIGIN,
            http::HeaderValue::from_static("https://evil.example"),
        );
        assert_eq!(v.a.send(elsewhere).await.status(), StatusCode::FORBIDDEN);
        assert!(!v.root.join("c.txt").exists());
        // The name is still unclaimed.
        let r = o.upload(&v, &id, "c.txt", b"c").await;
        assert_eq!(r.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn an_upload_that_lost_a_race_is_409_and_audited_as_failed() {
        let v = vol(&[]);
        let mut o = owner(&v).await;
        let id = o.confirmed(&v, "", &[("new.txt", 3), ("a.txt", 3)]).await;
        // Someone made new.txt, and changed a.txt, after the preview.
        std::fs::write(v.root.join("new.txt"), b"theirs").unwrap();
        std::fs::write(v.root.join("a.txt"), b"changed a").unwrap();

        let r = o.upload(&v, &id, "new.txt", b"new").await;
        assert_eq!(r.status(), StatusCode::CONFLICT);
        assert_eq!(json(r).await["kind"], "exists");
        let r = o.upload(&v, &id, "a.txt", b"new").await;
        assert_eq!(r.status(), StatusCode::CONFLICT);
        assert_eq!(json(r).await["kind"], "changed_on_disk");
        assert_eq!(std::fs::read(v.root.join("new.txt")).unwrap(), b"theirs");
        assert_eq!(std::fs::read(v.root.join("a.txt")).unwrap(), b"changed a");

        let ups = audited(&v.a, AuditAction::FilesUpload);
        assert_eq!(ups.len(), 2, "{ups:?}");
        for e in &ups {
            assert!(
                matches!(e.outcome, AuditOutcome::Failed { .. }),
                "{:?}",
                e.outcome
            );
            assert_eq!(e.file.as_ref().unwrap().bytes, Some(3));
        }
    }

    #[derive(Default)]
    struct Kept(Mutex<Vec<(FileKey, VersionReason, Vec<u8>)>>);

    impl HistorySink for Kept {
        fn keep(&self, file: &FileKey, _: &str, reason: VersionReason, bytes: &[u8]) {
            self.0
                .lock()
                .unwrap()
                .push((file.clone(), reason, bytes.to_vec()));
        }
    }

    #[tokio::test]
    async fn an_upload_that_replaces_a_file_keeps_what_it_replaced() {
        let v = vol(&[]);
        let kept = Arc::new(Kept::default());
        v.a.state.files.set_history(kept.clone());
        let mut o = owner(&v).await;
        let id = o.confirmed(&v, "sub", &[("b.txt", 5)]).await;
        let r = o.upload(&v, &id, "b.txt", b"fresh").await;
        assert_eq!(r.status(), StatusCode::CREATED);
        assert_eq!(std::fs::read(v.root.join("sub/b.txt")).unwrap(), b"fresh");
        let kept = kept.0.lock().unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].0.path.as_str(), "sub/b.txt");
        assert_eq!(kept[0].1, VersionReason::Upload);
        assert_eq!(kept[0].2, b"hello b");
    }

    #[tokio::test]
    async fn upload_max_limits_the_upload_route_while_other_routes_keep_theirs() {
        let v = vol(&[("DEPTHS_FILES_UPLOAD_MAX", "32768")]);
        let mut o = owner(&v).await;
        let p = o.preview(&v, "", &[("huge.bin", 32_769)]).await;
        assert_eq!(p["data"]["kind"], "too_large", "{p}");

        // 20 KiB is above MAX_BODY_BYTES and under upload_max.
        let big = vec![7u8; 20 * 1024];
        let id = o.confirmed(&v, "", &[("big.bin", big.len() as u64)]).await;
        let r = o.upload(&v, &id, "big.bin", &big).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        assert_eq!(std::fs::read(v.root.join("big.bin")).unwrap(), big);

        let r = o.upload(&v, &id, "big.bin", &vec![7u8; 40_000]).await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let r =
            v.a.send(
                Request::post("/auth/refresh")
                    .header(header::COOKIE, &o.cookie)
                    .header(header::ORIGIN, ORIGIN)
                    .header("x-csrf-token", &o.csrf)
                    .header(header::CONTENT_LENGTH, big.len())
                    .body(Body::from(big.clone()))
                    .unwrap(),
            )
            .await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn a_folder_downloads_as_a_tar_named_after_it() {
        let v = vol(&[]);
        let cookie = v.a.sign_in(OWNER).await;
        let r = v.a.get(&download_uri(&v.mount, "sub"), Some(&cookie)).await;
        assert_eq!(r.status(), StatusCode::OK);
        let h = r.headers().clone();
        assert_eq!(h[header::CONTENT_TYPE], "application/x-tar");
        assert_eq!(h[header::CACHE_CONTROL], "no-store");
        assert_eq!(h["x-content-type-options"], "nosniff");
        let disposition = h[header::CONTENT_DISPOSITION].to_str().unwrap();
        assert!(disposition.starts_with("attachment;"), "{disposition}");
        assert!(
            disposition.contains("filename=\"sub.tar\""),
            "{disposition}"
        );
        let tar = bytes(r).await;
        assert_eq!(tar.len() % 512, 0, "a whole archive");
        assert!(contains(&tar, b"sub/b.txt"));
        assert!(contains(&tar, b"sub/deep/c.txt"));
        assert!(contains(&tar, b"hello c"));

        let downloads = audited(&v.a, AuditAction::FilesDownload);
        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].file.as_ref().unwrap().bytes, Some(14));

        // The mount root is named after the mount.
        let r = v.a.get(&download_uri(&v.mount, ""), Some(&cookie)).await;
        assert_eq!(r.status(), StatusCode::OK);
        let disposition = r.headers()[header::CONTENT_DISPOSITION].to_str().unwrap();
        assert!(
            disposition.contains(&format!("\"{}.tar\"", volumes::VOLUME)),
            "{disposition}"
        );
    }

    #[tokio::test]
    async fn a_download_above_download_max_is_413_with_its_size_and_audited() {
        let v = vol(&[("DEPTHS_FILES_DOWNLOAD_MAX", "10")]);
        let cookie = v.a.sign_in(OWNER).await;
        std::fs::write(v.root.join("big.txt"), "x".repeat(11)).unwrap();
        let r =
            v.a.get(&download_uri(&v.mount, "big.txt"), Some(&cookie))
                .await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let j = json(r).await;
        assert_eq!(j["kind"], "too_large", "{j}");
        assert_eq!(j["bytes"], 11, "{j}");
        let r = v.a.get(&download_uri(&v.mount, "sub"), Some(&cookie)).await;
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let j = json(r).await;
        assert_eq!(j["kind"], "too_large", "{j}");
        assert!(j["bytes"].as_u64().unwrap() > 10, "{j}");

        let downloads = audited(&v.a, AuditAction::FilesDownload);
        assert_eq!(downloads.len(), 2, "{downloads:?}");
        for e in &downloads {
            assert!(
                matches!(e.outcome, AuditOutcome::Failed { .. }),
                "{:?}",
                e.outcome
            );
            assert!(e.file.as_ref().unwrap().bytes.unwrap() > 10);
        }
    }

    #[tokio::test]
    async fn downloads_are_403_with_a_kind_for_no_access_a_restricted_network_and_files_off() {
        let v = vol(&[]);
        let owner = v.a.sign_in(OWNER).await;
        let uri = download_uri(&v.mount, "a.txt");

        let r = v.a.get_from(CAFE, &uri, Some(&owner)).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert_eq!(json(r).await["kind"], "untrusted_network");

        // Signed in with access to another stack only.
        let other = v.a.sign_in("4242").await;
        ws::grant(&v.a, "4242", &["minecraft"]);
        let r = v.a.get(&uri, Some(&other)).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert_eq!(json(r).await["kind"], "not_permitted");

        let off = vol(&[("DEPTHS_FEATURE_FILES", "off")]);
        let cookie = off.a.sign_in(OWNER).await;
        let r = off
            .a
            .get(&download_uri(&off.mount, "a.txt"), Some(&cookie))
            .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert_eq!(json(r).await["kind"], "feature_disabled");
    }

    /// The resident set, from `/proc/self/statm`.
    fn rss() -> u64 {
        let statm = std::fs::read_to_string("/proc/self/statm").unwrap();
        let pages: u64 = statm.split_whitespace().nth(1).unwrap().parse().unwrap();
        pages * 4096
    }

    /// SC-006: a 1 GiB download streams through at most 64 MiB of memory.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "slow: set DEPTHS_SLOW_TESTS=1 and run with --ignored"]
    async fn a_one_gib_download_streams_in_bounded_memory() {
        if std::env::var("DEPTHS_SLOW_TESTS").as_deref() != Ok("1") {
            eprintln!("skipped: DEPTHS_SLOW_TESTS is not 1");
            return;
        }
        const GIB: u64 = 1 << 30;
        let v = vol(&[("DEPTHS_FILES_DOWNLOAD_MAX", "2GiB")]);
        let f = std::fs::File::create(v.root.join("huge.bin")).unwrap();
        f.set_len(GIB).unwrap();
        let cookie = v.a.sign_in(OWNER).await;
        let before = rss();
        let r =
            v.a.get(&download_uri(&v.mount, "huge.bin"), Some(&cookie))
                .await;
        assert_eq!(r.status(), StatusCode::OK);
        let mut body = r.into_body();
        let (mut got, mut peak) = (0u64, before);
        while let Some(frame) = body.frame().await {
            if let Ok(data) = frame.unwrap().into_data() {
                got += data.len() as u64;
                peak = peak.max(rss());
            }
        }
        assert_eq!(got, GIB);
        let grew = peak.saturating_sub(before);
        assert!(grew <= 64 << 20, "RSS grew by {} MiB", grew >> 20);
    }
}
