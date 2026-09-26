//! Mock-IdP integration tests for the OIDC relying party (spec 031, T022).
mod common;

use common::{MockIdp, RefreshMode};
use fathom_server::oidc::{OidcClient, OidcError};
use secrecy::{ExposeSecret, SecretString};

const EXT: &str = "http://localhost:8080";

async fn setup() -> (MockIdp, OidcClient) {
    let idp = MockIdp::start().await;
    let client = OidcClient::discover(&idp.config(EXT))
        .await
        .expect("discover");
    (idp, client)
}

#[tokio::test]
async fn happy_path_yields_identity_and_tokens() {
    let (idp, oidc) = setup().await;
    let req = oidc.authorize();
    assert!(
        req.url
            .query()
            .unwrap()
            .contains("code_challenge_method=S256")
    );
    idp.grant_code("c1", &req.login.nonce);
    let (id, tokens) = oidc
        .complete("c1", &req.login.state, &req.login)
        .await
        .unwrap();
    assert_eq!(id.username, "alice");
    assert_eq!(id.groups, ["dev"]);
    assert_eq!(
        tokens.refresh_token.as_ref().unwrap().expose_secret(),
        "rt-1"
    );
}

#[tokio::test]
async fn bad_state_is_rejected_before_any_exchange() {
    let (idp, oidc) = setup().await;
    let req = oidc.authorize();
    idp.grant_code("c1", &req.login.nonce);
    let err = oidc.complete("c1", "forged", &req.login).await.unwrap_err();
    assert!(matches!(err, OidcError::StateMismatch), "{err:?}");
}

#[tokio::test]
async fn bad_nonce_is_rejected() {
    let (idp, oidc) = setup().await;
    let req = oidc.authorize();
    idp.grant_code("c1", &req.login.nonce);
    idp.set(|b| b.nonce_override = Some("someone-elses-nonce".into()));
    let err = oidc
        .complete("c1", &req.login.state, &req.login)
        .await
        .unwrap_err();
    assert!(matches!(err, OidcError::IdToken(_)), "{err:?}");
}

#[tokio::test]
async fn wrong_audience_is_rejected() {
    let (idp, oidc) = setup().await;
    let req = oidc.authorize();
    idp.grant_code("c1", &req.login.nonce);
    idp.set(|b| b.aud = "another-client".into());
    let err = oidc
        .complete("c1", &req.login.state, &req.login)
        .await
        .unwrap_err();
    assert!(matches!(err, OidcError::IdToken(_)), "{err:?}");
}

#[tokio::test]
async fn expired_id_token_is_rejected() {
    let (idp, oidc) = setup().await;
    let req = oidc.authorize();
    idp.grant_code("c1", &req.login.nonce);
    idp.set(|b| b.exp_in = -3600);
    let err = oidc
        .complete("c1", &req.login.state, &req.login)
        .await
        .unwrap_err();
    assert!(matches!(err, OidcError::IdToken(_)), "{err:?}");
}

#[tokio::test]
async fn code_cannot_be_replayed() {
    let (idp, oidc) = setup().await;
    let req = oidc.authorize();
    idp.grant_code("c1", &req.login.nonce);
    oidc.complete("c1", &req.login.state, &req.login)
        .await
        .unwrap();
    let err = oidc
        .complete("c1", &req.login.state, &req.login)
        .await
        .unwrap_err();
    assert!(matches!(err, OidcError::Exchange(_)), "{err:?}");
}

#[tokio::test]
async fn refresh_success_rotates_tokens() {
    let (idp, oidc) = setup().await;
    let t = oidc
        .refresh(&SecretString::from("rt-1".to_string()))
        .await
        .unwrap();
    assert_eq!(t.refresh_token.as_ref().unwrap().expose_secret(), "rt-r1");
    assert_eq!(idp.refresh_calls(), 1);
}

#[tokio::test]
async fn refresh_invalid_grant_is_permanent() {
    let (idp, oidc) = setup().await;
    idp.set(|b| b.refresh = RefreshMode::InvalidGrant);
    let err = oidc
        .refresh(&SecretString::from("rt-1".to_string()))
        .await
        .unwrap_err();
    assert!(matches!(err, OidcError::RefreshRejected(_)), "{err:?}");
}

/// Any other OAuth error response is the IdP's definitive answer (e.g. Rauthy answers a
/// disabled user's refresh with a non-standard `Disabled` code): revocation, not an outage.
#[tokio::test]
async fn refresh_other_error_responses_are_permanent_except_try_later_codes() {
    let (idp, oidc) = setup().await;
    let rt = || SecretString::from("rt-1".to_string());
    for code in ["Disabled", "access_denied", "invalid_request"] {
        idp.set(|b| b.refresh = RefreshMode::Error(code));
        let err = oidc.refresh(&rt()).await.unwrap_err();
        assert!(
            matches!(err, OidcError::RefreshRejected(_)),
            "{code}: {err:?}"
        );
    }
    for code in ["temporarily_unavailable", "server_error"] {
        idp.set(|b| b.refresh = RefreshMode::Error(code));
        let err = oidc.refresh(&rt()).await.unwrap_err();
        assert!(
            matches!(err, OidcError::RefreshTransient(_)),
            "{code}: {err:?}"
        );
    }
}

#[tokio::test]
async fn refresh_server_error_is_transient() {
    let (idp, oidc) = setup().await;
    idp.set(|b| b.refresh = RefreshMode::ServerError);
    let err = oidc
        .refresh(&SecretString::from("rt-1".to_string()))
        .await
        .unwrap_err();
    assert!(matches!(err, OidcError::RefreshTransient(_)), "{err:?}");
}

#[tokio::test]
async fn discovery_failure_is_reported() {
    let idp = MockIdp::start().await;
    let cfg = idp.config(EXT);
    drop(idp);
    assert!(matches!(
        OidcClient::discover(&cfg).await,
        Err(OidcError::Discovery(_))
    ));
}
