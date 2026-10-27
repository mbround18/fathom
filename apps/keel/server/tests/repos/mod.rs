//! Shared support for the binding, receiver and hook tests (spec 055 US1): a server with a
//! master key and local repositories allowed, a signed-in owner with a fresh step-up, a
//! workspace and project, and local bare repositories built with the `git` CLI (isolated
//! configuration, fixed dates). Nothing touches the network.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use keel_core::ports::SystemClock;
use keel_protocol::{Cmd, CmdEnvelope, Event, EventEnvelope, Preview, RequestId};
use keel_server::config::{Config, Flags};
use keel_server::seal::MasterKey;
use keel_server::serve::{self, AppState};
use keel_server::signin::{self, Established};
use secrecy::SecretString;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use web_auth::session::{Identity, Provider, Tokens};

use crate::common::TestDb;

pub const ORIGIN: &str = "https://keel.example.com";

/// A valid manifest whose `web` job runs `playbooks/web.yaml`. `CREDS` is replaced by the
/// job's credential list.
pub const MANIFEST: &str = "version: 1\nprofiles: { debian: {} }\njobs:\n  web:\n    \
                            playbook: playbooks/web.yaml\n    profiles: [debian]\n    \
                            inventory: prod\n    credentials: CREDS\n";

pub fn manifest(creds: &[&str]) -> String {
    MANIFEST.replace("CREDS", &format!("[{}]", creds.join(", ")))
}

pub struct Harness {
    pub db: TestDb,
    pub app: AppState,
    pub owner: Established,
    pub workspace: Uuid,
    pub project: Uuid,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

impl Harness {
    pub async fn new(test: &str) -> Option<Self> {
        let db = TestDb::new(test).await?;
        let web = tempfile::tempdir().expect("tempdir");
        std::fs::write(web.path().join("index.html"), "<!doctype html>").expect("index");
        let data = tempfile::tempdir().expect("tempdir");
        let env: HashMap<&str, String> = HashMap::from([
            ("KEEL_PUBLIC_URL", ORIGIN.to_owned()),
            ("KEEL_DATABASE_URL", "postgres://unused/keel".to_owned()),
            ("KEEL_MASTER_KEY_FILE", "/nonexistent/master.key".to_owned()),
            ("KEEL_GITHUB_CLIENT_ID", "gh-id".to_owned()),
            ("KEEL_GITHUB_CLIENT_SECRET", "gh-secret".to_owned()),
            ("KEEL_STATIC_DIR", web.path().display().to_string()),
            ("KEEL_DATA_DIR", data.path().display().to_string()),
        ]);
        let cfg = Config::resolve(Flags::default(), &|k| env.get(k).cloned()).expect("config");
        let providers = serve::build_providers(&cfg).await.expect("providers");
        let app = AppState::new(
            Arc::new(cfg),
            providers,
            db.app.clone(),
            Arc::new(SystemClock),
        )
        .with_master_key(MasterKey::from_bytes([7; 32]))
        .allowing_local_repos();
        let owner = sign_in(&app, "1", "owner").await;
        let (workspace, project) = (Uuid::now_v7(), Uuid::now_v7());
        sqlx::query("insert into workspaces (id, slug, name) values ($1, 'ops', 'Ops')")
            .bind(workspace)
            .execute(&db.owner)
            .await
            .expect("workspace");
        sqlx::query(
            "insert into projects (id, workspace_id, slug, name) values ($1, $2, 'infra', 'Infra')",
        )
        .bind(project)
        .bind(workspace)
        .execute(&db.owner)
        .await
        .expect("project");
        let h = Self {
            db,
            app,
            owner,
            workspace,
            project,
            _dirs: (web, data),
        };
        h.step_up(&h.owner).await;
        Some(h)
    }

    /// Marks the session as just verified, so step-up previews confirm without an assertion.
    pub async fn step_up(&self, est: &Established) {
        sqlx::query("update sessions set verified_at = now() where id = $1")
            .bind(&est.session)
            .execute(&self.db.owner)
            .await
            .expect("verified_at");
    }

    pub async fn call(&self, req: Request<Body>) -> (StatusCode, Vec<u8>) {
        let r = serve::router(self.app.clone())
            .oneshot(req)
            .await
            .expect("infallible");
        let status = r.status();
        let body = r.into_body().collect().await.expect("body").to_bytes();
        (status, body.to_vec())
    }

    /// Runs one command as `est` and returns its events.
    pub async fn cmd_as(&self, est: &Established, cmd: Cmd) -> Vec<Event> {
        let pair = est.cookies[0]
            .to_str()
            .expect("ascii")
            .split(';')
            .next()
            .expect("pair")
            .to_owned();
        let body = serde_json::to_vec(&CmdEnvelope {
            id: RequestId(1),
            cmd,
        })
        .expect("json");
        let req = Request::post("/api/v1/cmd")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, pair)
            .header("x-csrf-token", &est.csrf)
            .body(Body::from(body))
            .expect("req");
        let (status, body) = self.call(req).await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let envs: Vec<EventEnvelope> = serde_json::from_slice(&body).expect("events");
        envs.into_iter().map(|e| e.event).collect()
    }

    pub async fn cmd(&self, cmd: Cmd) -> Vec<Event> {
        self.cmd_as(&self.owner, cmd).await
    }

    /// Previews and confirms; panics unless the command answered a preview.
    pub async fn confirm(&self, cmd: Cmd) -> (Preview, Vec<Event>) {
        let p = preview(&self.cmd(cmd).await);
        let out = self
            .cmd(Cmd::Confirm {
                preview_id: p.preview_id,
                step_up: None,
            })
            .await;
        assert!(
            matches!(out.first(), Some(Event::Confirmed { .. })),
            "confirm: {out:?}"
        );
        (p, out)
    }

    pub async fn audit_actions(&self) -> Vec<(String, String)> {
        sqlx::query_as("select action, decision from audit_event order by seq")
            .fetch_all(&self.db.owner)
            .await
            .expect("audit")
    }

    /// The whole audit log as text, to check nothing secret reached it.
    pub async fn audit_text(&self) -> String {
        let rows: Vec<(Value,)> =
            sqlx::query_as("select to_jsonb(a) from audit_event a order by seq")
                .fetch_all(&self.db.owner)
                .await
                .expect("audit");
        rows.iter().map(|r| r.0.to_string()).collect()
    }

    pub async fn finish(self) {
        self.db.drop().await;
    }
}

pub async fn sign_in(app: &AppState, sub: &str, name: &str) -> Established {
    let who = Identity {
        provider: Provider::Github,
        issuer: "https://github.com".into(),
        sub: sub.into(),
        username: name.into(),
        groups: vec![],
        email: Some(format!("{name}@example.com")),
    };
    let tokens = Tokens {
        id_token: SecretString::from(String::new()),
        access_token: SecretString::from("provider-access-token".to_owned()),
        refresh_token: None,
        expires_at: tokio::time::Instant::now() + Duration::from_secs(3600),
    };
    signin::establish(app, who, tokens, json!({ "provider": "github" }))
        .await
        .unwrap_or_else(|e| panic!("establish: {e:?}"))
}

pub fn preview(events: &[Event]) -> Preview {
    match events {
        [Event::Preview(p)] => p.clone(),
        other => panic!("expected a preview, got {other:?}"),
    }
}

pub fn op_error(events: &[Event]) -> String {
    match events {
        [Event::OpError { code, .. }] => code.clone(),
        other => panic!("expected an op error, got {other:?}"),
    }
}

/// Runs `git` with no user or system configuration and fixed identities and dates.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.org")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.org")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf8")
        .trim()
        .to_owned()
}

/// A bare `origin.git` and a working clone that pushes to it, with a playbook and a manifest
/// on `main`.
pub struct Source {
    _dir: tempfile::TempDir,
    pub bare: PathBuf,
    pub work: PathBuf,
}

impl Source {
    pub fn new(manifest: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let bare = dir.path().join("origin.git");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&bare).expect("bare");
        std::fs::create_dir_all(&work).expect("work");
        git(&bare, &["init", "--bare", "-q", "-b", "main"]);
        git(&work, &["init", "-q", "-b", "main"]);
        git(
            &work,
            &["remote", "add", "origin", bare.to_str().expect("utf8")],
        );
        let s = Self {
            _dir: dir,
            bare,
            work,
        };
        s.write("playbooks/web.yaml", "- hosts: all\n");
        s.commit(".keel/jobs.yaml", manifest, "manifest");
        s.push();
        s
    }

    pub fn url(&self) -> String {
        format!("file://{}", self.bare.display())
    }

    fn write(&self, file: &str, content: &str) {
        let path = self.work.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("dir");
        }
        std::fs::write(path, content).expect("write");
    }

    /// Commits `file` and returns the new head.
    pub fn commit(&self, file: &str, content: &str, msg: &str) -> String {
        self.write(file, content);
        git(&self.work, &["add", "-A"]);
        git(&self.work, &["commit", "-q", "-m", msg]);
        git(&self.work, &["rev-parse", "HEAD"])
    }

    pub fn push(&self) {
        git(&self.work, &["push", "-q", "origin", "main"]);
    }
}

/// A binding spec for `src`'s `main` over HTTPS with no credential.
pub fn spec(h: &Harness, src: &Source) -> keel_protocol::BindingSpec {
    keel_protocol::BindingSpec {
        project: h.project,
        url: keel_protocol::RepoUrl::new(src.url()).expect("url"),
        branch: keel_protocol::BranchName::new("main").expect("branch"),
        transport: keel_protocol::Transport::Https,
        credential: None,
        known_host: None,
        poll_interval: None,
        allow_force_push: false,
        signed_mode: false,
        override_quorum: 1,
    }
}

/// Creates a binding for `src` and returns its id.
pub async fn bind(h: &Harness, src: &Source) -> Uuid {
    let (_, out) = h.confirm(Cmd::PreviewBindingCreate(spec(h, src))).await;
    match out.as_slice() {
        [Event::Confirmed { .. }, Event::Binding { binding, .. }] => binding.id,
        other => panic!("binding create: {other:?}"),
    }
}
