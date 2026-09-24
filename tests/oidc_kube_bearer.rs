//! US2 (033 T019): the kube client for an OIDC-opted context uses the current ID token, picks
//! up refreshed tokens mid-session, and never falls back to kubeconfig credentials once OIDC
//! fails.

use std::sync::{Arc, Mutex};

use fathom::backend::kube::build_oidc_client;
use fathom::backend::oidc::session::TokenSource;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn version_mock() -> Mock {
    Mock::given(method("GET"))
        .and(path("/version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "major": "1", "minor": "32", "gitVersion": "v1.32.2", "gitCommit": "x",
            "gitTreeState": "clean", "buildDate": "2026-01-01T00:00:00Z", "goVersion": "go1",
            "compiler": "gc", "platform": "linux/amd64"
        })))
}

fn client(
    server: &MockServer,
    source: TokenSource,
    kubeconfig_token: Option<&str>,
) -> kube::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = kube::Config::new(server.uri().parse().unwrap());
    if let Some(t) = kubeconfig_token {
        config.auth_info.token = Some(t.to_owned().into());
    }
    build_oidc_client(config, source).unwrap()
}

fn source(cell: &Arc<Mutex<Result<String, String>>>) -> TokenSource {
    let cell = cell.clone();
    Arc::new(move || cell.lock().unwrap().clone())
}

async fn auth_headers(server: &MockServer) -> Vec<Option<String>> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            r.headers
                .get("authorization")
                .map(|v| v.to_str().unwrap().to_owned())
        })
        .collect()
}

#[tokio::test]
async fn uses_the_current_token_and_follows_refreshes() {
    let server = MockServer::start().await;
    version_mock().mount(&server).await;
    let cell = Arc::new(Mutex::new(Ok("tok-1".to_owned())));
    let c = client(&server, source(&cell), None);

    c.apiserver_version().await.unwrap();
    *cell.lock().unwrap() = Ok("tok-2".to_owned()); // the refresh task swapped the token
    c.apiserver_version().await.unwrap();

    assert_eq!(
        auth_headers(&server).await,
        vec![
            Some("Bearer tok-1".to_owned()),
            Some("Bearer tok-2".to_owned())
        ]
    );
}

#[tokio::test]
async fn kubeconfig_credentials_are_never_used_for_oidc_contexts() {
    let server = MockServer::start().await;
    version_mock().mount(&server).await;
    let cell = Arc::new(Mutex::new(Ok("oidc-token".to_owned())));
    let c = client(&server, source(&cell), Some("kubeconfig-token"));
    c.apiserver_version().await.unwrap();
    assert_eq!(
        auth_headers(&server).await,
        vec![Some("Bearer oidc-token".to_owned())]
    );
}

#[tokio::test]
async fn oidc_failure_never_falls_back_or_sends_unauthenticated() {
    let server = MockServer::start().await;
    version_mock().mount(&server).await;
    let cell = Arc::new(Mutex::new(Ok("tok-1".to_owned())));
    let c = client(&server, source(&cell), Some("kubeconfig-token"));
    c.apiserver_version().await.unwrap();

    *cell.lock().unwrap() = Err("OIDC session ended; sign in again".to_owned());
    let err = c.apiserver_version().await.unwrap_err();
    assert!(err.to_string().contains("sign in again"), "{err}");
    let err = c.apiserver_version().await.unwrap_err();
    assert!(err.to_string().contains("sign in again"), "{err}");

    // Only the first (authenticated) request ever reached the API server.
    assert_eq!(auth_headers(&server).await.len(), 1);

    // Signing in again restores service on the same client.
    *cell.lock().unwrap() = Ok("tok-3".to_owned());
    c.apiserver_version().await.unwrap();
    assert_eq!(
        auth_headers(&server).await.last().unwrap().as_deref(),
        Some("Bearer tok-3")
    );
}
