//! Impersonation access model (spec 031, T046 server part): the headers the API server sees,
//! claim prefixing, the missing-groups error state, and a service account that can only
//! impersonate.

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use fathom::backend::access::ClusterAccess;
use fathom_server::config::{AccessModel, HostedConfig};
use fathom_server::host::{HostedTenantHost, credentials_for};
use fathom_server::kubeauth::{Credentials, TokenCell};
use fathom_server::oidc::OidcClient;
use fathom_server::session::{Identity, SessionConfig, SessionStore, Tokens};
use fathom_server::tenant::HostedAccess;
use fathom_server::ws::{AttachError, TenantHost};
use secrecy::SecretString;
use tokio::sync::mpsc::unbounded_channel;

type Seen = Arc<Mutex<Vec<HeaderMap>>>;

/// A stand-in API server: the service account may only impersonate, so any request that does
/// not carry `Impersonate-User` is refused (no user-visible reads on the SA's own authority).
async fn fake_api(State(seen): State<Seen>, headers: HeaderMap) -> impl IntoResponse {
    seen.lock().unwrap().push(headers.clone());
    if headers.contains_key("impersonate-user") {
        (StatusCode::OK, "{}").into_response()
    } else {
        (StatusCode::FORBIDDEN, "sa has no reads").into_response()
    }
}

async fn start_api() -> (String, Seen) {
    let seen: Seen = Arc::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(fake_api).with_state(seen.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (url, seen)
}

fn env(issuer: &str) -> impl Fn(&str) -> Option<String> {
    let issuer = issuer.to_owned();
    move |k| match k {
        "FATHOM_EXTERNAL_URL" => Some("http://localhost:8080".into()),
        "FATHOM_OIDC_ISSUER" => Some(issuer.clone()),
        "FATHOM_OIDC_CLIENT_ID" => Some("fathom".into()),
        "FATHOM_ALLOW_INSECURE_LOCALHOST" => Some("1".into()),
        "FATHOM_ACCESS_MODEL" => Some("impersonation".into()),
        "FATHOM_USERNAME_PREFIX" => Some("oidc:".into()),
        "FATHOM_GROUPS_PREFIX" => Some("oidc:".into()),
        _ => None,
    }
}

fn cfg(issuer: &str) -> HostedConfig {
    HostedConfig::from_lookup(env(issuer), true).unwrap()
}

fn identity(groups: &[&str]) -> Identity {
    Identity {
        issuer: "https://idp".into(),
        sub: "sub-1".into(),
        username: "alice".into(),
        groups: groups.iter().map(|g| g.to_string()).collect(),
        email: None,
    }
}

fn cell() -> TokenCell {
    TokenCell::new(&SecretString::from("user-id-token".to_owned()), None).unwrap()
}

fn get(path: &str) -> http::Request<Vec<u8>> {
    http::Request::get(path).body(Vec::new()).unwrap()
}

#[tokio::test]
async fn requests_carry_prefixed_impersonation_headers_and_never_the_user_token() {
    fathom::startup::install_crypto_provider();
    let (url, seen) = start_api().await;
    let c = cfg("https://idp.example.com");
    assert_eq!(c.access_model, AccessModel::Impersonation);
    let creds = credentials_for(&c, &identity(&["dev", "ops"]), &cell()).unwrap();
    assert!(matches!(creds, Credentials::Impersonate(_)));

    // The base config carries the service account token, as in-cluster impersonation does.
    let mut base = kube::Config::new(url.parse().unwrap());
    base.auth_info.token = Some(SecretString::from("sa-token".to_owned()));
    let access = HostedAccess::new(base, creds, std::env::temp_dir().join("fathom-imp-test"));
    let client = access.build_client(None).await.unwrap();

    // A caller-supplied spoof must not survive the layer.
    let mut req = get("/api/v1/pods");
    req.headers_mut()
        .insert("impersonate-user", "system:admin".parse().unwrap());
    req.headers_mut()
        .insert("impersonate-group", "system:masters".parse().unwrap());
    client.request_text(req).await.unwrap();

    let seen = seen.lock().unwrap();
    let h = &seen[0];
    assert_eq!(h.get("impersonate-user").unwrap(), "oidc:alice");
    let groups: Vec<_> = h.get_all("impersonate-group").iter().collect();
    assert_eq!(groups, ["oidc:dev", "oidc:ops"]);
    assert_eq!(h.get("authorization").unwrap(), "Bearer sa-token");
    assert!(
        !format!("{h:?}").contains("user-id-token"),
        "the user's own token must never reach the API server in impersonation mode"
    );
}

#[tokio::test]
async fn service_account_alone_has_no_user_reads() {
    let (url, seen) = start_api().await;
    // No AuthLayer: exactly what the SA could do on its own authority.
    let res = reqwest::Client::new()
        .get(format!("{url}/api/v1/pods"))
        .bearer_auth("sa-token")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
    assert!(!seen.lock().unwrap()[0].contains_key("impersonate-user"));
}

#[test]
fn empty_groups_are_an_error_state_not_a_silent_no_access() {
    let c = cfg("https://idp.example.com");
    let err = credentials_for(&c, &identity(&[]), &cell()).unwrap_err();
    assert!(err.contains("no groups"), "{err}");
}

#[test]
fn unrepresentable_identity_is_refused() {
    let c = cfg("https://idp.example.com");
    assert!(credentials_for(&c, &identity(&["bad\ngroup"]), &cell()).is_err());
}

#[test]
fn chart_grants_the_service_account_only_impersonate() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../deploy/helm/fathom/templates/clusterrole-impersonate.yaml"
    );
    let y = std::fs::read_to_string(path).unwrap();
    assert!(y.contains("verbs: [\"impersonate\"]"));
    for v in [
        "get", "list", "watch", "create", "update", "delete", "patch",
    ] {
        assert!(
            !y.contains(&format!("\"{v}\"")),
            "impersonation ClusterRole must not grant `{v}`"
        );
    }
}

#[tokio::test]
async fn attach_without_groups_reports_unavailable_with_the_reason() {
    fathom::startup::install_crypto_provider();
    let idp = common::MockIdp::start().await;
    let c = Arc::new(cfg(&idp.base));
    let oidc = Arc::new(OidcClient::discover(&c).await.unwrap());
    let sessions = SessionStore::new(SessionConfig::default());
    let base = kube::Config::new("http://127.0.0.1:1".parse().unwrap());
    let host = HostedTenantHost::with_base(c, oidc, sessions.clone(), Ok(base));
    let tokens = Tokens {
        id_token: SecretString::from("id".to_owned()),
        access_token: SecretString::from("at".to_owned()),
        refresh_token: None,
        expires_at: tokio::time::Instant::now() + std::time::Duration::from_secs(300),
    };
    let session = sessions.create(identity(&[]), tokens).unwrap();
    let (tx, _rx) = unbounded_channel();
    match host.attach(&session, tx).await {
        Err(AttachError::Unavailable(why)) => assert!(why.contains("no groups"), "{why}"),
        Err(other) => panic!("wrong error: {other:?}"),
        Ok(_) => panic!("attach must fail without groups"),
    }
}
