//! The real tenant host: tab limits, per-user backend reuse and teardown on sign-out.

mod common;

use std::sync::Arc;
use std::time::Duration;

use fathom_server::host::HostedTenantHost;
use fathom_server::oidc::OidcClient;
use fathom_server::session::{EndReason, Identity, SessionConfig, SessionStore, Tokens};
use fathom_server::ws::{AttachError, TenantHost};
use secrecy::SecretString;
use tokio::sync::mpsc::unbounded_channel;

fn tokens() -> Tokens {
    Tokens {
        id_token: SecretString::from("id".to_owned()),
        access_token: SecretString::from("at".to_owned()),
        refresh_token: Some(SecretString::from("rt".to_owned())),
        expires_at: tokio::time::Instant::now() + Duration::from_secs(300),
    }
}

fn identity() -> Identity {
    Identity {
        issuer: "https://idp".into(),
        sub: "sub-1".into(),
        username: "alice".into(),
        groups: vec!["dev".into()],
        email: None,
    }
}

#[tokio::test]
async fn tabs_share_one_backend_are_capped_and_sign_out_ends_it() {
    fathom::startup::install_crypto_provider();
    let idp = common::MockIdp::start().await;
    let mut cfg = idp.config("http://localhost:8080");
    cfg.limits.max_tabs_per_user = 2;
    let cfg = Arc::new(cfg);
    let oidc = Arc::new(OidcClient::discover(&cfg).await.unwrap());
    let sessions = SessionStore::new(SessionConfig::default());
    let base = kube::Config::new("http://127.0.0.1:1".parse().unwrap());
    let host = HostedTenantHost::with_base(cfg, oidc, sessions.clone(), Ok(base));
    let s = sessions.create(identity(), tokens()).unwrap();

    let (tx, _rx) = unbounded_channel();
    let a = host.attach(&s, tx.clone()).await.unwrap();
    let b = host.attach(&s, tx.clone()).await.unwrap();
    assert_eq!(host.registry().len(), 1);
    match host.attach(&s, tx.clone()).await {
        Err(AttachError::Limit(l)) => assert_eq!(l.max, 2),
        other => panic!("expected tab limit, got {:?}", other.err()),
    }
    let info = a.subscription_info().unwrap();
    assert_eq!(info.limits.unwrap().max_tabs, 2);
    drop((a, b));

    sessions.end(&s.id, EndReason::SignedOut);
    for _ in 0..100 {
        if host.registry().is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("backend outlived the user's last session");
}

#[tokio::test]
async fn not_ready_without_incluster_config() {
    fathom::startup::install_crypto_provider();
    let idp = common::MockIdp::start().await;
    let cfg = Arc::new(idp.config("http://localhost:8080"));
    let oidc = Arc::new(OidcClient::discover(&cfg).await.unwrap());
    let sessions = SessionStore::new(SessionConfig::default());
    let host = HostedTenantHost::with_base(cfg, oidc, sessions.clone(), Err("none".into()));
    assert!(host.ready().await.is_err());
    let s = sessions.create(identity(), tokens()).unwrap();
    let (tx, _rx) = unbounded_channel();
    assert!(matches!(
        host.attach(&s, tx).await,
        Err(AttachError::Unavailable(_))
    ));
}
