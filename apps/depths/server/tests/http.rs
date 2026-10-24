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
