#![allow(dead_code)]
//! Shared harness: a fake Discord provider and the real router over a temp dir.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use depths_core::plan::RunStep;
use depths_core::ports::{
    CancelToken, ContainerRuntime, OutputChunk, OutputSink, OutputStream, ProjectContainers,
    RuntimeError, RuntimeSnapshot, StepOutcome, Validation,
};
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
        if s.identity.sub == "3" {
            return Err(ProviderError::Transient("discord is down".into()));
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

/// A docker that answers from a snapshot the test sets, and records what it was asked to run.
#[derive(Default)]
pub struct FakeRuntime {
    pub snapshot: Mutex<RuntimeSnapshot>,
    /// `Some(stderr)`: docker is unreachable.
    pub unavailable: Mutex<Option<String>>,
    pub ran: Mutex<Vec<Vec<String>>>,
    /// `Some(stderr)`: every step fails with it.
    pub fail_with: Mutex<Option<String>>,
}

impl ContainerRuntime for FakeRuntime {
    fn containers_by_project(&self) -> Result<RuntimeSnapshot, RuntimeError> {
        if let Some(e) = self.unavailable.lock().unwrap().clone() {
            return Err(RuntimeError::Unavailable(e));
        }
        Ok(self.snapshot.lock().unwrap().clone())
    }
    fn run_step(
        &self,
        step: &RunStep,
        _: Option<&[u8]>,
        sink: &dyn OutputSink,
        _: &CancelToken,
    ) -> Result<StepOutcome, RuntimeError> {
        self.ran.lock().unwrap().push(step.argv.clone());
        sink.emit(OutputChunk {
            stream: OutputStream::Stderr,
            text: "Container web-app-1  Started\n".into(),
        });
        Ok(match self.fail_with.lock().unwrap().clone() {
            Some(stderr_tail) => StepOutcome::Failed {
                exit_code: Some(1),
                stderr_tail,
            },
            None => StepOutcome::Succeeded,
        })
    }
    fn validate(&self, _: &RunStep, _: &[u8]) -> Result<Validation, RuntimeError> {
        Ok(Validation::Valid)
    }
    fn follow_events(
        &self,
        _: &(dyn Fn() + Sync),
        cancel: &CancelToken,
    ) -> Result<(), RuntimeError> {
        while !cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }
}

/// A container of compose project `project`, service `service`.
pub fn container(project: &str, service: &str, state: &str, status: &str) -> serde_json::Value {
    serde_json::json!({
        "id": format!("{project}-{service}-id"),
        "name": format!("{project}-{service}-1"),
        "image": "nginx",
        "state": state,
        "health": null,
        "status": status,
        "created_at": "2026-10-23 12:00:00 +0000 UTC",
        "command": "nginx",
        "ports": [],
        "project": project,
        "service": service,
        "cluster": null,
        "adoption": "not_adopted",
    })
}

pub struct App {
    _dir: tempfile::TempDir,
    pub router: axum::Router,
    pub state: AppState,
    pub stacks: PathBuf,
    pub runtime: Arc<FakeRuntime>,
}

pub fn app() -> App {
    app_with(|_| {})
}

pub fn app_with(tweak: impl FnOnce(&mut AppState)) -> App {
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
    let runtime = Arc::new(FakeRuntime::default());
    let mut state = AppState::new(
        Arc::new(cfg),
        vec![(
            "discord",
            Arc::new(FakeDiscord) as Arc<dyn IdentityProvider>,
        )],
        Arc::new(users),
        runtime.clone(),
    )
    .unwrap();
    tweak(&mut state);
    App {
        _dir: dir,
        router: router(state.clone()),
        state,
        stacks,
        runtime,
    }
}

impl App {
    /// Writes `<stacks>/<name>/compose.yaml` (and `.env`).
    pub fn add_stack(&self, name: &str, compose: &str, env: Option<&str>) {
        let d = self.stacks.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("compose.yaml"), compose).unwrap();
        if let Some(e) = env {
            std::fs::write(d.join(".env"), e).unwrap();
        }
    }

    /// Sets the containers docker reports.
    pub fn set_containers(&self, containers: Vec<serde_json::Value>) {
        let mut snap = RuntimeSnapshot::default();
        for c in containers {
            let project = c["project"].as_str().unwrap().to_owned();
            let entry = snap
                .projects
                .entry(project)
                .or_insert_with(|| ProjectContainers {
                    working_dir: None,
                    containers: Vec::new(),
                });
            entry.containers.push(serde_json::from_value(c).unwrap());
        }
        *self.runtime.snapshot.lock().unwrap() = snap;
    }

    /// Every audit entry so far.
    pub fn audit_entries(&self) -> Vec<depths_protocol::AuditEntry> {
        self.state
            .audit
            .page(&depths_protocol::AuditFilter::default(), None)
            .unwrap()
            .0
    }

    /// The raw bytes of every audit file.
    pub fn audit_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(self.state.audit.dir()).unwrap() {
            let p = e.unwrap().path();
            if p.is_file() {
                out.extend(std::fs::read(p).unwrap());
            }
        }
        out
    }

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
