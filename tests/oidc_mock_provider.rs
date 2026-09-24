//! Self-tests for the mock OIDC provider harness (033 T009).

#[path = "support/mock_oidc.rs"]
mod mock_oidc;

use base64::Engine;
use mock_oidc::{MockOidc, MockUser};
use serde_json::Value;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn claims(jwt: &str) -> Value {
    let payload = jwt.split('.').nth(1).unwrap();
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn token(m: &MockOidc, form: &[(&str, &str)]) -> (u16, Value) {
    let r = client()
        .post(format!("{}/token", m.issuer()))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(
            form.iter()
                .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
                .collect::<Vec<_>>()
                .join("&"),
        )
        .send()
        .await
        .unwrap();
    (r.status().as_u16(), r.json().await.unwrap())
}

#[tokio::test]
async fn discovery_and_jwks_are_served() {
    let m = MockOidc::start().await;
    let doc: Value = client()
        .get(format!("{}/.well-known/openid-configuration", m.issuer()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(doc["issuer"], m.issuer());
    let jwks: Value = client()
        .get(doc["jwks_uri"].as_str().unwrap())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(jwks["keys"][0]["alg"], "RS256");
}

#[tokio::test]
async fn auth_code_flow_echoes_nonce_and_codes_are_single_use() {
    let m = MockOidc::start().await;
    let r = client()
        .get(format!(
            "{}/authorize?redirect_uri=http://127.0.0.1:9/callback&state=st&nonce=n1",
            m.issuer()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 302);
    let loc = r.headers()["location"].to_str().unwrap().to_owned();
    assert!(loc.contains("state=st"));
    let code = loc
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap();
    let (st, body) = token(&m, &[("grant_type", "authorization_code"), ("code", code)]).await;
    assert_eq!(st, 200);
    let c = claims(body["id_token"].as_str().unwrap());
    assert_eq!(c["nonce"], "n1");
    assert_eq!(c["groups"][0], "platform");
    let (st, body) = token(&m, &[("grant_type", "authorization_code"), ("code", code)]).await;
    assert_eq!((st, body["error"].as_str()), (400, Some("invalid_grant")));
}

#[tokio::test]
async fn refresh_rotates_and_reuse_revokes_the_family() {
    let m = MockOidc::start().await;
    let dev: Value = client()
        .post(format!("{}/device", m.issuer()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let dc = dev["device_code"].as_str().unwrap();
    const DEVICE: &str = "urn:ietf:params:oauth:grant-type:device_code";
    m.approve_device(dc);
    let (_, t1) = token(&m, &[("grant_type", DEVICE), ("device_code", dc)]).await;
    let rt1 = t1["refresh_token"].as_str().unwrap().to_owned();
    let (st, t2) = token(
        &m,
        &[("grant_type", "refresh_token"), ("refresh_token", &rt1)],
    )
    .await;
    assert_eq!(st, 200);
    let rt2 = t2["refresh_token"].as_str().unwrap().to_owned();
    assert_ne!(rt1, rt2);
    // Replaying the spent token fails and revokes the live one.
    let (st, _) = token(
        &m,
        &[("grant_type", "refresh_token"), ("refresh_token", &rt1)],
    )
    .await;
    assert_eq!(st, 400);
    let (st, _) = token(
        &m,
        &[("grant_type", "refresh_token"), ("refresh_token", &rt2)],
    )
    .await;
    assert_eq!(st, 400);
}

#[tokio::test]
async fn device_flow_is_pending_until_approved() {
    let m = MockOidc::start().await;
    let dev: Value = client()
        .post(format!("{}/device", m.issuer()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let dc = dev["device_code"].as_str().unwrap();
    const DEVICE: &str = "urn:ietf:params:oauth:grant-type:device_code";
    let (st, b) = token(&m, &[("grant_type", DEVICE), ("device_code", dc)]).await;
    assert_eq!(
        (st, b["error"].as_str()),
        (400, Some("authorization_pending"))
    );
    m.approve_device(dc);
    let (st, _) = token(&m, &[("grant_type", DEVICE), ("device_code", dc)]).await;
    assert_eq!(st, 200);
}

#[tokio::test]
async fn key_rotation_publishes_both_then_retires_old() {
    let m = MockOidc::start().await;
    let new_kid = m.rotate_key();
    let jwks = |m: &MockOidc| {
        let url = format!("{}/jwks", m.issuer());
        async move {
            client()
                .get(url)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    assert_eq!(jwks(&m).await["keys"].as_array().unwrap().len(), 2);
    m.retire_old_keys();
    let after = jwks(&m).await;
    assert_eq!(after["keys"].as_array().unwrap().len(), 1);
    assert_eq!(after["keys"][0]["kid"], new_kid);
    let jwt = m.mint_id_token("fathom-desktop", None, serde_json::json!({}));
    let header = jwt.split('.').next().unwrap();
    let h: Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(header)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(h["kid"], new_kid);
}

#[tokio::test]
async fn failure_injection_and_claim_overrides() {
    let m = MockOidc::start().await;
    m.fail_token(503, "temporarily_unavailable");
    let (st, _) = token(
        &m,
        &[("grant_type", "refresh_token"), ("refresh_token", "x")],
    )
    .await;
    assert_eq!(st, 503);
    m.clear_failure();
    m.set_user(MockUser {
        groups: vec![],
        ..MockUser::alice()
    });
    let jwt = m.mint_id_token("other", None, serde_json::json!({"groups": null}));
    let c = claims(&jwt);
    assert_eq!(c["aud"], "other");
    assert!(c.get("groups").is_none());
}
