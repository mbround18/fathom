//! Governance (spec 055 US6, T033): workspaces, teams, grants, users and API tokens.
//!
//! - An operator in team A acting on team B's workspace is `Denied`, and the denial is audited
//!   against that workspace (SC-006).
//! - Removing a grant bumps every affected user's `authz_epoch` with the change's audit row, and
//!   their open socket gets `Revoked`.
//! - The last owner can be neither demoted nor removed, also when the race is lost at confirm.
//! - Role changes need step-up.
//! - API tokens are shown once, stored hashed, narrowed to one workspace and their actions, and
//!   can be revoked.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::TestDb;
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use keel_core::ports::SystemClock;
use keel_protocol::{
    ActionName, Cmd, CmdEnvelope, Event, EventEnvelope, GrantProfiles, InstanceRole, Label,
    PreviewId, ProfileName, RequestId, Slug, TeamRole, TokenActions,
};
use keel_server::config::{Config, Flags};
use keel_server::serve::{self, AppState};
use keel_server::signin::{self, Established};
use secrecy::SecretString;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;
use uuid::Uuid;
use web_auth::session::{Identity, Provider, Tokens};

const ORIGIN: &str = "https://keel.example.com";

struct Harness {
    db: TestDb,
    app: AppState,
}

impl Harness {
    async fn new(test: &str) -> Option<Self> {
        let db = TestDb::new(test).await?;
        let env: HashMap<&str, String> = HashMap::from([
            ("KEEL_PUBLIC_URL", ORIGIN.to_owned()),
            ("KEEL_DATABASE_URL", "postgres://unused/keel".to_owned()),
            ("KEEL_MASTER_KEY_FILE", "/nonexistent/master.key".to_owned()),
            ("KEEL_GITHUB_CLIENT_ID", "gh-id".to_owned()),
            ("KEEL_GITHUB_CLIENT_SECRET", "gh-secret".to_owned()),
        ]);
        let cfg = Config::resolve(Flags::default(), &|k| env.get(k).cloned()).expect("config");
        let providers = serve::build_providers(&cfg).await.expect("providers");
        let app = AppState::new(
            Arc::new(cfg),
            providers,
            db.app.clone(),
            Arc::new(SystemClock),
        );
        Some(Self { db, app })
    }

    async fn sign_in(&self, sub: &str, name: &str) -> Established {
        let who = Identity {
            provider: Provider::Github,
            issuer: "https://github.com".into(),
            sub: sub.into(),
            username: name.into(),
            groups: vec![],
            email: None,
        };
        let tokens = Tokens {
            id_token: SecretString::from(String::new()),
            access_token: SecretString::from("provider-access-token".to_owned()),
            refresh_token: None,
            expires_at: tokio::time::Instant::now() + Duration::from_secs(3600),
        };
        signin::establish(&self.app, who, tokens, json!({ "provider": "github" }))
            .await
            .unwrap_or_else(|e| panic!("establish: {e:?}"))
    }

    /// Marks the session as having verified a passkey just now (step-up is then satisfied).
    async fn verified(&self, est: &Established) {
        sqlx::query("update sessions set verified_at = now() where id = $1")
            .bind(&est.session)
            .execute(&self.db.owner)
            .await
            .expect("verified");
    }

    async fn try_cmd(&self, est: &Established, cmd: Cmd) -> (StatusCode, Vec<Event>) {
        let body = serde_json::to_vec(&CmdEnvelope {
            id: RequestId(1),
            cmd,
        })
        .expect("json");
        let req = Request::post("/api/v1/cmd")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, cookie_pair(est))
            .header("x-csrf-token", &est.csrf)
            .body(Body::from(body))
            .expect("req");
        self.send(req).await
    }

    async fn bearer(&self, token: &str, cmd: Cmd) -> (StatusCode, Vec<Event>) {
        let body = serde_json::to_vec(&CmdEnvelope {
            id: RequestId(2),
            cmd,
        })
        .expect("json");
        let req = Request::post("/api/v1/cmd")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .expect("req");
        self.send(req).await
    }

    async fn send(&self, req: Request<Body>) -> (StatusCode, Vec<Event>) {
        let r = serve::router(self.app.clone())
            .oneshot(req)
            .await
            .expect("infallible");
        let status = r.status();
        let body = r.into_body().collect().await.expect("body").to_bytes();
        let events = serde_json::from_slice::<Vec<EventEnvelope>>(&body)
            .map(|v| v.into_iter().map(|e| e.event).collect())
            .unwrap_or_default();
        (status, events)
    }

    async fn cmd(&self, est: &Established, cmd: Cmd) -> Vec<Event> {
        let (status, events) = self.try_cmd(est, cmd).await;
        assert_eq!(status, StatusCode::OK);
        events
    }

    /// Sends a preview command and returns its `Preview`.
    async fn preview(&self, est: &Established, cmd: Cmd) -> keel_protocol::Preview {
        let events = self.cmd(est, cmd).await;
        match events.as_slice() {
            [Event::Preview(p)] => p.clone(),
            other => panic!("expected a Preview, got {other:?}"),
        }
    }

    async fn confirm(&self, est: &Established, id: PreviewId) -> Vec<Event> {
        self.cmd(
            est,
            Cmd::Confirm {
                preview_id: id,
                step_up: None,
            },
        )
        .await
    }

    /// Preview then confirm; returns the events after `Confirmed`.
    async fn apply(&self, est: &Established, cmd: Cmd) -> Vec<Event> {
        let p = self.preview(est, cmd).await;
        let mut events = self.confirm(est, p.preview_id).await;
        assert_eq!(
            events.first(),
            Some(&Event::Confirmed {
                preview_id: p.preview_id
            }),
            "{events:?}"
        );
        events.remove(0);
        events
    }

    async fn audit(&self) -> Vec<Value> {
        let rows: Vec<(Value,)> = sqlx::query_as(
            "select to_jsonb(a) - 'hash' - 'prev_hash' from audit_event a order by seq",
        )
        .fetch_all(&self.db.owner)
        .await
        .expect("audit rows");
        rows.into_iter().map(|r| r.0).collect()
    }

    async fn audited(&self, action: &str) -> Vec<Value> {
        self.audit()
            .await
            .into_iter()
            .filter(|r| r["action"] == action)
            .collect()
    }

    async fn epoch(&self, user: Uuid) -> i64 {
        let (e,): (i64,) = sqlx::query_as("select authz_epoch from users where id = $1")
            .bind(user)
            .fetch_one(&self.db.owner)
            .await
            .expect("epoch");
        e
    }

    // ---- setup through the commands themselves ----

    async fn workspace(&self, owner: &Established, slug: &str) -> Uuid {
        let events = self
            .apply(
                owner,
                Cmd::PreviewWorkspaceCreate {
                    slug: Slug::new(slug).unwrap(),
                    name: Label::new(slug.to_uppercase()).unwrap(),
                },
            )
            .await;
        let Some(Event::Workspaces { items }) = events.last() else {
            panic!("expected Workspaces, got {events:?}");
        };
        items.iter().find(|w| w.slug == slug).expect("created").id
    }

    async fn team(&self, owner: &Established, name: &str) -> Uuid {
        let events = self
            .apply(
                owner,
                Cmd::PreviewTeamCreate {
                    name: Label::new(name).unwrap(),
                },
            )
            .await;
        let Some(Event::Teams { items }) = events.last() else {
            panic!("expected Teams, got {events:?}");
        };
        items.iter().find(|t| t.name == name).expect("created").id
    }

    async fn member(&self, owner: &Established, team: Uuid, user: Uuid) {
        let events = self
            .apply(owner, Cmd::PreviewTeamMemberAdd { team, user })
            .await;
        assert!(
            matches!(events.last(), Some(Event::Teams { .. })),
            "{events:?}"
        );
    }

    async fn grant(&self, owner: &Established, team: Uuid, ws: Uuid, role: TeamRole) -> Uuid {
        let events = self
            .apply(
                owner,
                Cmd::PreviewGrantAdd {
                    team,
                    workspace: ws,
                    role,
                    profiles: GrantProfiles::default(),
                },
            )
            .await;
        let Some(Event::Teams { items }) = events.last() else {
            panic!("expected Teams, got {events:?}");
        };
        let t = items.iter().find(|t| t.id == team).expect("team");
        t.grants
            .iter()
            .find(|g| g.workspace == ws && g.role == role)
            .expect("grant")
            .id
    }
}

fn cookie_pair(e: &Established) -> String {
    let set = e.cookies[0].to_str().expect("ascii");
    set.split(';').next().expect("pair").to_owned()
}

fn denied(action: &str, reason: &str) -> Event {
    Event::Denied {
        action: action.to_owned(),
        reason: reason.to_owned(),
    }
}

// ---- WebSocket ------------------------------------------------------------------------------

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn spawn(h: &Harness) -> (std::net::SocketAddr, tokio::sync::oneshot::Sender<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let router = serve::router(h.app.clone());
    tokio::spawn(async move {
        let _ = serve::serve_until(listener, router, async {
            let _ = rx.await;
        })
        .await;
    });
    (addr, tx)
}

async fn open_socket(addr: std::net::SocketAddr, est: &Established) -> Socket {
    let mut req = format!("ws://{addr}/api/v1/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert(header::ORIGIN, ORIGIN.parse().unwrap());
    req.headers_mut()
        .insert(header::COOKIE, cookie_pair(est).parse().unwrap());
    let (mut s, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let hello = json!({
        "protocol_version": keel_protocol::PROTOCOL_VERSION,
        "client": "test/1",
        "csrf_token": est.csrf,
    });
    s.send(Message::Text(hello.to_string().into()))
        .await
        .unwrap();
    // A round trip, so the socket is past its Hello before the test goes on.
    s.send(Message::Text(r#"{"id":1,"cmd":{"type":"GetMe"}}"#.into()))
        .await
        .unwrap();
    assert!(matches!(
        next(&mut s).await,
        Ok(EventEnvelope {
            event: Event::Me(_),
            ..
        })
    ));
    s
}

async fn next(s: &mut Socket) -> Result<EventEnvelope, u16> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(5), s.next())
            .await
            .expect("socket answered in time");
        match m {
            Some(Ok(Message::Text(t))) => return Ok(serde_json::from_str(&t).expect("event")),
            Some(Ok(Message::Close(Some(f)))) => return Err(u16::from(f.code)),
            Some(Ok(Message::Close(None))) | None | Some(Err(_)) => return Err(0),
            Some(Ok(_)) => {}
        }
    }
}

// ---- tests ----------------------------------------------------------------------------------

#[tokio::test]
async fn an_operator_in_team_a_is_denied_on_team_b_workspace_and_audited() {
    let Some(h) = Harness::new("gov_scope").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let bob = h.sign_in("2", "bob").await;
    h.verified(&alice).await;
    let ws_a = h.workspace(&alice, "team-a-ws").await;
    let ws_b = h.workspace(&alice, "team-b-ws").await;
    let team_a = h.team(&alice, "A").await;
    let team_b = h.team(&alice, "B").await;
    h.member(&alice, team_a, bob.user.id).await;
    h.grant(&alice, team_a, ws_a, TeamRole::Operator).await;
    h.grant(&alice, team_b, ws_b, TeamRole::Operator).await;

    // Bob sees only his own workspace and team.
    let events = h.cmd(&bob, Cmd::ListWorkspaces).await;
    let [Event::Workspaces { items }] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(
        items.iter().map(|w| w.slug.as_str()).collect::<Vec<_>>(),
        ["team-a-ws"]
    );
    let events = h.cmd(&bob, Cmd::ListTeams).await;
    let [Event::Teams { items }] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(
        items.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["A"]
    );

    // Acting on team B's workspace: denied and audited against that workspace.
    let events = h
        .cmd(
            &bob,
            Cmd::PreviewGrantAdd {
                team: team_a,
                workspace: ws_b,
                role: TeamRole::Operator,
                profiles: GrantProfiles::default(),
            },
        )
        .await;
    assert_eq!(events, [denied("preview_grant_add", "no_grant")]);
    let rows = h.audited("preview_grant_add").await;
    let row = rows.last().expect("audited");
    assert_eq!(row["decision"], "denied");
    assert_eq!(row["reason"], "no_grant");
    assert_eq!(row["workspace_id"], json!(ws_b));
    assert_eq!(row["actor"], format!("user:{}", bob.user.id));

    let events = h
        .cmd(
            &bob,
            Cmd::PreviewWorkspaceUpdate {
                workspace: ws_b,
                name: Label::new("Mine now").unwrap(),
            },
        )
        .await;
    assert_eq!(events, [denied("preview_workspace_update", "no_grant")]);
    // An operator does not administer even its own workspace.
    let events = h
        .cmd(
            &bob,
            Cmd::PreviewProjectCreate {
                workspace: ws_a,
                slug: Slug::new("edge").unwrap(),
                name: Label::new("Edge").unwrap(),
            },
        )
        .await;
    assert_eq!(events, [denied("preview_project_create", "no_grant")]);
    // Instance-wide commands are for instance administrators.
    for (cmd, action) in [
        (
            Cmd::PreviewTeamCreate {
                name: Label::new("C").unwrap(),
            },
            "preview_team_create",
        ),
        (Cmd::ListUsers, "list_users"),
        (
            Cmd::PreviewUserRole {
                user: bob.user.id,
                role: InstanceRole::Admin,
            },
            "preview_user_role",
        ),
    ] {
        assert_eq!(
            h.cmd(&bob, cmd).await,
            [denied(action, "instance_admin_only")]
        );
        let rows = h.audited(action).await;
        assert_eq!(rows.last().expect("audited")["decision"], "denied");
    }
    // Nothing changed on team B's workspace.
    let (n,): (i64,) = sqlx::query_as("select count(*) from grants where workspace_id = $1")
        .bind(ws_b)
        .fetch_one(&h.db.owner)
        .await
        .unwrap();
    assert_eq!(n, 1);
    h.db.drop().await;
}

#[tokio::test]
async fn removing_a_grant_bumps_the_epoch_with_its_audit_row_and_revokes_the_socket() {
    let Some(h) = Harness::new("gov_revoke").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let bob = h.sign_in("2", "bob").await;
    h.verified(&alice).await;
    let ws = h.workspace(&alice, "infra").await;
    let team = h.team(&alice, "SRE").await;
    h.member(&alice, team, bob.user.id).await;
    let grant = h.grant(&alice, team, ws, TeamRole::Approver).await;
    let before = h.epoch(bob.user.id).await;
    let alice_before = h.epoch(alice.user.id).await;

    let (addr, stop) = spawn(&h).await;
    let mut s = open_socket(addr, &bob).await;

    let p = h.preview(&alice, Cmd::PreviewGrantRemove { grant }).await;
    assert_eq!(p.action, "grant_remove");
    assert_eq!(p.affected, ["bob"]);
    assert!(!p.step_up);
    // A preview changes nothing.
    assert_eq!(h.epoch(bob.user.id).await, before);

    let events = h.confirm(&alice, p.preview_id).await;
    assert!(
        matches!(events.last(), Some(Event::Teams { .. })),
        "{events:?}"
    );
    assert_eq!(h.epoch(bob.user.id).await, before + 1);
    assert_eq!(h.epoch(alice.user.id).await, alice_before);
    let rows = h.audited("grant_remove").await;
    let row = rows.last().expect("audited");
    assert_eq!(row["decision"], "allowed");
    assert_eq!(row["workspace_id"], json!(ws));
    assert_eq!(row["target"]["grant"], json!(grant));
    assert_eq!(row["target"]["epoch_bumped"], json!([bob.user.id]));

    let ev = next(&mut s).await.expect("Revoked");
    assert_eq!(ev.event, Event::Revoked);
    assert_eq!(next(&mut s).await.err(), Some(4401));
    assert_eq!(
        h.try_cmd(&bob, Cmd::GetMe).await.0,
        StatusCode::UNAUTHORIZED
    );

    // Signed in again, bob holds no grant.
    let bob = h.sign_in("2", "bob").await;
    let events = h.cmd(&bob, Cmd::GetMe).await;
    assert!(matches!(&events[0], Event::Me(me) if me.grants.is_empty()));
    let _ = stop.send(());
    h.db.drop().await;
}

#[tokio::test]
async fn removing_a_member_or_archiving_a_workspace_is_narrowing() {
    let Some(h) = Harness::new("gov_narrow").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let bob = h.sign_in("2", "bob").await;
    h.verified(&alice).await;
    let ws = h.workspace(&alice, "infra").await;
    let team = h.team(&alice, "SRE").await;
    h.member(&alice, team, bob.user.id).await;
    h.grant(&alice, team, ws, TeamRole::Viewer).await;

    let e0 = h.epoch(bob.user.id).await;
    let p = h
        .preview(&alice, Cmd::PreviewWorkspaceArchive { workspace: ws })
        .await;
    assert_eq!(p.affected, ["bob"]);
    h.confirm(&alice, p.preview_id).await;
    assert_eq!(h.epoch(bob.user.id).await, e0 + 1);
    let rows = h.audited("workspace_archive").await;
    assert_eq!(rows.last().unwrap()["workspace_id"], json!(ws));
    // Archived: no more grants on it.
    let bob = h.sign_in("2", "bob").await;
    let events = h.cmd(&bob, Cmd::ListWorkspaces).await;
    assert!(matches!(&events[0], Event::Workspaces { items } if items.is_empty()));
    // ...and it can no longer be changed.
    let events = h
        .cmd(
            &alice,
            Cmd::PreviewProjectCreate {
                workspace: ws,
                slug: Slug::new("edge").unwrap(),
                name: Label::new("Edge").unwrap(),
            },
        )
        .await;
    assert!(
        matches!(&events[0], Event::OpError { code, .. } if code == "workspace_archived"),
        "{events:?}"
    );

    let e1 = h.epoch(bob.user.id).await;
    h.apply(
        &alice,
        Cmd::PreviewTeamMemberRemove {
            team,
            user: bob.user.id,
        },
    )
    .await;
    assert_eq!(h.epoch(bob.user.id).await, e1 + 1);
    let rows = h.audited("team_member_remove").await;
    assert_eq!(rows.last().unwrap()["target"]["user"], json!(bob.user.id));
    h.db.drop().await;
}

#[tokio::test]
async fn the_last_owner_cannot_be_demoted_or_removed() {
    let Some(h) = Harness::new("gov_owner").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let carol = h.sign_in("3", "carol").await;
    h.verified(&alice).await;

    let events = h
        .cmd(
            &alice,
            Cmd::PreviewUserRole {
                user: alice.user.id,
                role: InstanceRole::Admin,
            },
        )
        .await;
    assert_eq!(events, [denied("preview_user_role", "last_owner")]);
    let rows = h.audited("preview_user_role").await;
    assert_eq!(rows.last().unwrap()["reason"], "last_owner");
    assert_eq!(rows.last().unwrap()["decision"], "denied");

    // Make carol an admin; an admin cannot touch an owner.
    let p = h
        .preview(
            &alice,
            Cmd::PreviewUserRole {
                user: carol.user.id,
                role: InstanceRole::Admin,
            },
        )
        .await;
    assert!(p.step_up);
    assert_eq!(p.affected, ["carol"]);
    h.confirm(&alice, p.preview_id).await;
    h.verified(&carol).await;
    let events = h
        .cmd(
            &carol,
            Cmd::PreviewUserRemove {
                user: alice.user.id,
            },
        )
        .await;
    assert_eq!(events, [denied("preview_user_remove", "owner_only")]);
    let events = h
        .cmd(
            &carol,
            Cmd::PreviewUserRole {
                user: carol.user.id,
                role: InstanceRole::Owner,
            },
        )
        .await;
    assert_eq!(events, [denied("preview_user_role", "owner_only")]);

    // Two owners: each may be demoted, but the race is settled at confirm.
    h.apply(
        &alice,
        Cmd::PreviewUserRole {
            user: carol.user.id,
            role: InstanceRole::Owner,
        },
    )
    .await;
    let carol = h.sign_in("3", "carol").await;
    h.verified(&carol).await;
    let demote_carol = h
        .preview(
            &alice,
            Cmd::PreviewUserRole {
                user: carol.user.id,
                role: InstanceRole::User,
            },
        )
        .await;
    let demote_alice = h
        .preview(
            &carol,
            Cmd::PreviewUserRole {
                user: alice.user.id,
                role: InstanceRole::User,
            },
        )
        .await;
    let carol_epoch = h.epoch(carol.user.id).await;
    h.confirm(&alice, demote_carol.preview_id).await;
    // Demotion is narrowing.
    assert_eq!(h.epoch(carol.user.id).await, carol_epoch + 1);
    // Carol's session ended with the demotion, so her pending preview cannot be used either way.
    assert_eq!(
        h.try_cmd(
            &carol,
            Cmd::Confirm {
                preview_id: demote_alice.preview_id,
                step_up: None
            }
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (owners,): (i64,) =
        sqlx::query_as("select count(*) from users where instance_role = 'owner'")
            .fetch_one(&h.db.owner)
            .await
            .unwrap();
    assert_eq!(owners, 1);

    // Removing: never oneself, never the last owner.
    let events = h
        .cmd(
            &alice,
            Cmd::PreviewUserRemove {
                user: alice.user.id,
            },
        )
        .await;
    assert_eq!(events, [denied("preview_user_remove", "self")]);
    h.db.drop().await;
}

#[tokio::test]
async fn the_last_owner_check_runs_again_at_confirm() {
    let Some(h) = Harness::new("gov_owner_race").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let carol = h.sign_in("3", "carol").await;
    h.verified(&alice).await;
    h.apply(
        &alice,
        Cmd::PreviewUserRole {
            user: carol.user.id,
            role: InstanceRole::Owner,
        },
    )
    .await;
    // Alice previews her own demotion while carol is an owner...
    let p = h
        .preview(
            &alice,
            Cmd::PreviewUserRole {
                user: alice.user.id,
                role: InstanceRole::User,
            },
        )
        .await;
    // ...then carol stops being one behind Keel's back.
    sqlx::query("update users set instance_role = 'user' where id = $1")
        .bind(carol.user.id)
        .execute(&h.db.owner)
        .await
        .unwrap();
    let events = h.confirm(&alice, p.preview_id).await;
    assert_eq!(
        events,
        [
            Event::Confirmed {
                preview_id: p.preview_id
            },
            denied("user_role", "last_owner")
        ]
    );
    let rows = h.audited("user_role").await;
    assert_eq!(rows.last().unwrap()["decision"], "denied");
    h.db.drop().await;
}

#[tokio::test]
async fn role_changes_need_step_up() {
    let Some(h) = Harness::new("gov_stepup").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let bob = h.sign_in("2", "bob").await;
    let p = h
        .preview(
            &alice,
            Cmd::PreviewUserRole {
                user: bob.user.id,
                role: InstanceRole::Admin,
            },
        )
        .await;
    assert!(p.step_up);
    // Not verified recently, and no passkey to verify with yet: enrol first.
    let events = h.confirm(&alice, p.preview_id).await;
    assert_eq!(events, [Event::PasskeyEnrolmentRequired]);
    let (role,): (String,) = sqlx::query_as("select instance_role from users where id = $1")
        .bind(bob.user.id)
        .fetch_one(&h.db.owner)
        .await
        .unwrap();
    assert_eq!(role, "user");
    h.verified(&alice).await;
    let events = h.confirm(&alice, p.preview_id).await;
    assert!(
        matches!(events.last(), Some(Event::Users { .. })),
        "{events:?}"
    );
    let rows = h.audited("user_role").await;
    let row = rows.last().unwrap();
    assert_eq!(row["target"]["from"], "user");
    assert_eq!(row["target"]["to"], "admin");
    h.db.drop().await;
}

#[tokio::test]
async fn removing_a_user_disables_them_and_ends_their_access() {
    let Some(h) = Harness::new("gov_remove").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let bob = h.sign_in("2", "bob").await;
    h.verified(&alice).await;
    let ws = h.workspace(&alice, "infra").await;
    let team = h.team(&alice, "SRE").await;
    h.member(&alice, team, bob.user.id).await;
    h.grant(&alice, team, ws, TeamRole::Viewer).await;
    h.verified(&bob).await;
    let created = h
        .apply(
            &bob,
            Cmd::PreviewApiTokenCreate {
                name: Label::new("ci").unwrap(),
                workspace: ws,
                actions: TokenActions(vec![ActionName::new("list_workspaces").unwrap()]),
                expires_in_days: 7,
            },
        )
        .await;
    let Some(Event::ApiTokenCreated { secret, .. }) = created.first() else {
        panic!("{created:?}");
    };
    let secret = secret.0.clone();

    let p = h
        .preview(&alice, Cmd::PreviewUserRemove { user: bob.user.id })
        .await;
    assert!(p.step_up);
    let events = h.confirm(&alice, p.preview_id).await;
    let Some(Event::Users { items }) = events.last() else {
        panic!("{events:?}");
    };
    assert!(items.iter().any(|u| u.id == bob.user.id && u.disabled));
    assert_eq!(
        h.try_cmd(&bob, Cmd::GetMe).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.bearer(&secret, Cmd::ListWorkspaces).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (members,): (i64,) = sqlx::query_as("select count(*) from team_members where user_id = $1")
        .bind(bob.user.id)
        .fetch_one(&h.db.owner)
        .await
        .unwrap();
    assert_eq!(members, 0);
    let rows = h.audited("user_remove").await;
    assert_eq!(rows.last().unwrap()["target"]["user"], json!(bob.user.id));
    h.db.drop().await;
}

#[tokio::test]
async fn api_tokens_are_shown_once_stored_hashed_scoped_and_revocable() {
    let Some(h) = Harness::new("gov_tokens").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    let bob = h.sign_in("2", "bob").await;
    h.verified(&alice).await;
    let ws = h.workspace(&alice, "infra").await;
    let other = h.workspace(&alice, "other").await;
    let team = h.team(&alice, "SRE").await;
    h.member(&alice, team, bob.user.id).await;
    h.grant(&alice, team, ws, TeamRole::Operator).await;

    // Bob may not mint a token for a workspace he cannot see.
    h.verified(&bob).await;
    let events = h
        .cmd(
            &bob,
            Cmd::PreviewApiTokenCreate {
                name: Label::new("ci").unwrap(),
                workspace: other,
                actions: TokenActions(vec![ActionName::new("list_workspaces").unwrap()]),
                expires_in_days: 7,
            },
        )
        .await;
    assert_eq!(events, [denied("preview_api_token_create", "no_grant")]);
    // ...nor one that runs session-only commands.
    let events = h
        .cmd(
            &bob,
            Cmd::PreviewApiTokenCreate {
                name: Label::new("ci").unwrap(),
                workspace: ws,
                actions: TokenActions(vec![ActionName::new("preview_api_token_create").unwrap()]),
                expires_in_days: 7,
            },
        )
        .await;
    assert!(
        matches!(&events[0], Event::OpError { code, .. } if code == "action_not_allowed"),
        "{events:?}"
    );

    let p = h
        .preview(
            &bob,
            Cmd::PreviewApiTokenCreate {
                name: Label::new("ci").unwrap(),
                workspace: ws,
                actions: TokenActions(vec![
                    ActionName::new("list_workspaces").unwrap(),
                    ActionName::new("preview_grant_add").unwrap(),
                ]),
                expires_in_days: 7,
            },
        )
        .await;
    assert!(p.step_up);
    let events = h.confirm(&bob, p.preview_id).await;
    let [
        Event::Confirmed { .. },
        Event::ApiTokenCreated { token, secret },
        Event::ApiTokens { items },
    ] = events.as_slice()
    else {
        panic!("{events:?}");
    };
    let secret = secret.0.clone();
    assert_eq!(token.workspace, ws);
    assert_eq!(items.len(), 1);
    // Stored hashed; never in the audit log.
    let (hash,): (Vec<u8>,) = sqlx::query_as("select hash from api_tokens where id = $1")
        .bind(token.id)
        .fetch_one(&h.db.owner)
        .await
        .unwrap();
    assert_eq!(hash, Sha256::digest(secret.as_bytes()).to_vec());
    let all = serde_json::to_string(&h.audit().await).unwrap();
    assert!(!all.contains(&secret));
    let rows = h.audited("api_token_create").await;
    assert_eq!(rows.last().unwrap()["workspace_id"], json!(ws));
    // Listing never shows it again.
    let listed = h.cmd(&bob, Cmd::ListApiTokens).await;
    assert!(!serde_json::to_string(&listed).unwrap().contains(&secret));

    // The token acts as bob, narrowed to its workspace and actions.
    let (status, events) = h.bearer(&secret, Cmd::ListWorkspaces).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        matches!(&events[0], Event::Workspaces { items } if items.len() == 1 && items[0].id == ws),
        "{events:?}"
    );
    let (_, events) = h.bearer(&secret, Cmd::ListTeams).await;
    assert_eq!(events, [denied("list_teams", "token_scope")]);
    // In scope by name, but bob is only an operator there.
    let (_, events) = h
        .bearer(
            &secret,
            Cmd::PreviewGrantAdd {
                team,
                workspace: ws,
                role: TeamRole::WsAdmin,
                profiles: GrantProfiles(vec![ProfileName::new("web").unwrap()]),
            },
        )
        .await;
    assert_eq!(events, [denied("preview_grant_add", "no_grant")]);

    // Revoked: the token stops working.
    h.apply(&bob, Cmd::PreviewApiTokenRevoke { token: token.id })
        .await;
    assert_eq!(
        h.bearer(&secret, Cmd::ListWorkspaces).await.0,
        StatusCode::UNAUTHORIZED
    );
    let rows = h.audited("api_token_revoke").await;
    assert_eq!(rows.last().unwrap()["target"]["token"], json!(token.id));
    h.db.drop().await;
}

#[tokio::test]
async fn an_owner_token_is_narrowed_to_its_workspace() {
    let Some(h) = Harness::new("gov_owner_token").await else {
        return;
    };
    let alice = h.sign_in("1", "alice").await;
    h.verified(&alice).await;
    let ws = h.workspace(&alice, "infra").await;
    let other = h.workspace(&alice, "other").await;
    let created = h
        .apply(
            &alice,
            Cmd::PreviewApiTokenCreate {
                name: Label::new("ops").unwrap(),
                workspace: ws,
                actions: TokenActions(
                    [
                        "list_workspaces",
                        "preview_workspace_update",
                        "preview_workspace_create",
                    ]
                    .into_iter()
                    .map(|a| ActionName::new(a).unwrap())
                    .collect(),
                ),
                expires_in_days: 1,
            },
        )
        .await;
    let Some(Event::ApiTokenCreated { secret, .. }) = created.first() else {
        panic!("{created:?}");
    };
    let secret = secret.0.clone();
    let (_, events) = h.bearer(&secret, Cmd::ListWorkspaces).await;
    assert!(matches!(&events[0], Event::Workspaces { items } if items.len() == 1));
    let (_, events) = h
        .bearer(
            &secret,
            Cmd::PreviewWorkspaceUpdate {
                workspace: other,
                name: Label::new("x").unwrap(),
            },
        )
        .await;
    assert_eq!(events, [denied("preview_workspace_update", "token_scope")]);
    let (_, events) = h
        .bearer(
            &secret,
            Cmd::PreviewWorkspaceCreate {
                slug: Slug::new("new").unwrap(),
                name: Label::new("New").unwrap(),
            },
        )
        .await;
    assert_eq!(events, [denied("preview_workspace_create", "token_scope")]);
    let (_, events) = h
        .bearer(
            &secret,
            Cmd::PreviewWorkspaceUpdate {
                workspace: ws,
                name: Label::new("Infrastructure").unwrap(),
            },
        )
        .await;
    assert!(matches!(&events[0], Event::Preview(_)), "{events:?}");
    h.db.drop().await;
}
