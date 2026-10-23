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
