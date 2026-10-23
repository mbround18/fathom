#![allow(dead_code)]
//! Shared harness: a fake Discord provider and the real router over a temp dir.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use depths_server::config::{Config, Flags};
use depths_server::http::{AppState, router};
use depths_server::users::Registry;
use http::{Request, Response, StatusCode, header};
use http_body_util::BodyExt as _;
use secrecy::SecretString;
use tower::ServiceExt as _;
use url::Url;
use web_auth::provider::{
    AuthRedirect, Completed, IdentityProvider, PendingLogin, ProviderError, Refreshed,
};
use web_auth::session::{Identity, Provider, Session, Tokens};

pub const ORIGIN: &str = "https://depth.boop.ninja";
pub const OWNER: &str = "140642000969007104";

/// Signs in whoever the `code` names: `code=<subject>`.
pub struct FakeDiscord;

#[async_trait::async_trait]
impl IdentityProvider for FakeDiscord {
    fn id(&self) -> &'static str {
        "discord"
    }
    async fn authorize(&self, redirect: &Url) -> Result<AuthRedirect, ProviderError> {
        let state = web_auth::session::random_token();
        let mut url = Url::parse("https://discord.example/oauth2/authorize").unwrap();
        url.query_pairs_mut()
            .append_pair("state", &state)
            .append_pair("redirect_uri", redirect.as_str());
        Ok(AuthRedirect {
            url,
            pending: PendingLogin {
                state,
                nonce: None,
                pkce_verifier: "v".into(),
                redirect: redirect.clone(),
            },
        })
    }
    async fn complete(
        &self,
        pending: PendingLogin,
        code: &str,
        state: &str,
    ) -> Result<Completed, ProviderError> {
        if pending.state != state {
            return Err(ProviderError::StateMismatch);
        }
        if code == "unreachable" {
            return Err(ProviderError::Transient("down".into()));
        }
        Ok(Completed {
            identity: Identity {
                provider: Provider::Discord,
                issuer: "https://discord.com".into(),
                sub: code.into(),
                username: format!("user{code}"),
                groups: vec![],
                email: None,
            },
            tokens: Tokens {
                id_token: SecretString::from(""),
                access_token: SecretString::from("at"),
                refresh_token: None,
                expires_at: tokio::time::Instant::now() + Duration::from_secs(3600),
            },
        })
    }
    async fn refresh(&self, s: &Session) -> Result<Refreshed, ProviderError> {
        if s.identity.sub == "2" {
            return Err(ProviderError::Rejected("revoked".into()));
        }
        Ok(Refreshed {
            tokens: Tokens {
                id_token: SecretString::from(""),
                access_token: SecretString::from("at2"),
                refresh_token: None,
                expires_at: tokio::time::Instant::now() + Duration::from_secs(3600),
            },
            identity: None,
        })
    }
    fn end_session_url(&self, _: &Session) -> Option<Url> {
        None
    }
}

pub struct App {
    _dir: tempfile::TempDir,
    pub router: axum::Router,
}

pub fn app() -> App {
    let dir = tempfile::tempdir().unwrap();
    let stacks = dir.path().join("stacks");
    let web = dir.path().join("web");
    std::fs::create_dir(&stacks).unwrap();
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(
        web.join("index.html"),
        "<!doctype html><title>Depths</title>",
    )
    .unwrap();
    std::fs::write(web.join("assets/app-abc.js"), "console.log(1)").unwrap();
    std::fs::write(dir.path().join("secret.txt"), "top secret").unwrap();
    let mut env: HashMap<String, String> = HashMap::new();
    for (k, v) in [
        ("DEPTHS_LISTEN", "0.0.0.0:8080"),
        ("ORIGIN_URL", ORIGIN),
        ("OAUTH_PROVIDER", "discord"),
        ("OAUTH_CLIENT_ID", "1234"),
        ("OAUTH_CLIENT_SECRET", "s3cret"),
        ("OWNER_USER_ID", OWNER),
    ] {
        env.insert(k.into(), v.into());
    }
    env.insert("DOCKGE_STACKS_DIR".into(), stacks.display().to_string());
    env.insert(
        "DEPTHS_DATA_DIR".into(),
        dir.path().join("data").display().to_string(),
    );
    env.insert("STATIC_DIR".into(), web.display().to_string());
    let lookup = move |k: &str| env.get(k).cloned();
    let cfg = Config::resolve(Flags::default(), &lookup).unwrap();
    cfg.prepare_data_dir().unwrap();
    let users = Registry::open(cfg.users_path(), cfg.owners.clone(), None).unwrap();
    let state = AppState::new(
        Arc::new(cfg),
        vec![(
            "discord",
            Arc::new(FakeDiscord) as Arc<dyn IdentityProvider>,
        )],
        Arc::new(users),
    );
    App {
        _dir: dir,
        router: router(state),
    }
}

impl App {
    pub async fn send(&self, req: Request<Body>) -> Response<Body> {
        self.router.clone().oneshot(req).await.unwrap()
    }

    pub async fn get(&self, path: &str, cookie: Option<&str>) -> Response<Body> {
        let mut b = Request::get(path);
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        self.send(b.body(Body::empty()).unwrap()).await
    }

    /// Runs the whole sign-in as `subject`; returns the session cookie pair.
    pub async fn sign_in(&self, subject: &str) -> String {
        let r = self.get("/auth/login?provider=discord", None).await;
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        let loc = Url::parse(location(&r)).unwrap();
        let state = loc
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        let login = cookie_pair(&r, "__Host-depths_login");
        let r = self
            .get(
                &format!("/auth/callback/discord?code={subject}&state={state}"),
                Some(&login),
            )
            .await;
        assert_eq!(r.status(), StatusCode::SEE_OTHER, "{r:?}");
        assert_eq!(location(&r), "/");
        cookie_pair(&r, "__Host-depths_session")
    }
}

pub fn location(r: &Response<Body>) -> &str {
    r.headers()[header::LOCATION].to_str().unwrap()
}

pub fn set_cookies(r: &Response<Body>) -> Vec<String> {
    r.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect()
}

pub fn cookie_pair(r: &Response<Body>, name: &str) -> String {
    let c = set_cookies(r)
        .into_iter()
        .find(|c| c.starts_with(&format!("{name}=")) && !c.contains("Max-Age=0"))
        .unwrap_or_else(|| panic!("no {name} cookie in {:?}", set_cookies(r)));
    c.split(';').next().unwrap().to_owned()
}

pub async fn json(r: Response<Body>) -> serde_json::Value {
    let b = r.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&b).unwrap()
}

pub async fn text(r: Response<Body>) -> String {
    let b = r.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(b.to_vec()).unwrap()
}
