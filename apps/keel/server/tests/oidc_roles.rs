//! Instance roles from the identity provider (spec 060, US5, T019; research R7).
//!
//! - Without a mapping, concurrent first sign-ins make exactly one owner (FR-023).
//! - With one, the highest mapped role wins on every OIDC sign-in (FR-025); a change bumps the
//!   epoch and is audited with reason `identity_provider` (FR-027).
//! - A required match that is missing refuses the sign-in, writes no user row and audits
//!   `role_not_mapped` (FR-026); so does another provider while a match is required.
//! - The last active owner is never demoted: they stay owner, `idp_role` records the mapped
//!   role, and the audit reason is `idp_names_no_owner` (FR-028).
//! - `PreviewUserRole` on an idp-managed user is refused with `role_managed_by_idp` (FR-029).
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::TestDb;
use http_body_util::BodyExt;
use keel_core::ports::SystemClock;
use keel_protocol::{Cmd, CmdEnvelope, Event, EventEnvelope, InstanceRole, RequestId};
use keel_server::config::{Config, Flags};
use keel_server::serve::{self, AppState};
use keel_server::signin::{self, EstablishError, Established};
use secrecy::SecretString;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use web_auth::session::{Identity, Provider, Tokens};

const ORIGIN: &str = "https://keel.example.com";
const ISSUER: &str = "https://idp.example.com/realms/home";

struct Harness {
    db: TestDb,
    app: AppState,
}

impl Harness {
    /// `mapping`: extra `KEEL_OIDC_*` role settings; empty means no mapping.
    async fn new(test: &str, mapping: &[(&str, &str)]) -> Option<Self> {
        let db = TestDb::new(test).await?;
        let env: HashMap<String, String> = [
            ("KEEL_PUBLIC_URL", ORIGIN),
            ("KEEL_DATABASE_URL", "postgres://unused/keel"),
            ("KEEL_MASTER_KEY_FILE", "/nonexistent/master.key"),
            ("KEEL_GITHUB_CLIENT_ID", "gh-id"),
            ("KEEL_GITHUB_CLIENT_SECRET", "gh-secret"),
            ("KEEL_OIDC_ISSUER", ISSUER),
            ("KEEL_OIDC_CLIENT_ID", "keel"),
        ]
        .into_iter()
        .chain(mapping.iter().copied())
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let cfg = Config::resolve(Flags::default(), &|k| env.get(k).cloned()).expect("config");
        // No provider objects: OIDC discovery would need a live issuer, and `establish` takes
        // the verified identity directly.
        let app = AppState::new(
            Arc::new(cfg),
            Vec::new(),
            db.app.clone(),
            Arc::new(SystemClock),
        );
        Some(Self { db, app })
    }

    async fn try_sign_in(
        &self,
        provider: Provider,
        sub: &str,
        groups: &[&str],
    ) -> Result<Established, EstablishError> {
        sign_in_with(&self.app, provider, sub, groups).await
    }

    async fn oidc(&self, sub: &str, groups: &[&str]) -> Established {
        self.try_sign_in(Provider::Oidc, sub, groups)
            .await
            .unwrap_or_else(|e| panic!("establish {sub}: {e:?}"))
    }

    async fn row(&self, id: Uuid) -> (String, String, Option<String>, i64) {
        sqlx::query_as(
            "select instance_role, role_source, idp_role, authz_epoch from users where id = $1",
        )
        .bind(id)
        .fetch_one(&self.db.owner)
        .await
        .expect("user row")
    }

    async fn users(&self) -> i64 {
        sqlx::query_scalar("select count(*) from users")
            .fetch_one(&self.db.owner)
            .await
            .expect("count")
    }

    async fn audited(&self, action: &str) -> Vec<Value> {
        let rows: Vec<(Value,)> = sqlx::query_as(
            "select to_jsonb(a) - 'hash' - 'prev_hash' from audit_event a \
             where action = $1 order by seq",
        )
        .bind(action)
        .fetch_all(&self.db.owner)
        .await
        .expect("audit rows");
        rows.into_iter().map(|r| r.0).collect()
    }

    async fn cmd(&self, est: &Established, cmd: Cmd) -> Vec<Event> {
        let body = serde_json::to_vec(&CmdEnvelope {
            id: RequestId(1),
            cmd,
        })
        .expect("json");
        let pair = est.cookies[0].to_str().expect("ascii");
        let req = Request::post("/api/v1/cmd")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, pair.split(';').next().expect("pair"))
            .header("x-csrf-token", &est.csrf)
            .body(Body::from(body))
            .expect("req");
        let r = serve::router(self.app.clone())
            .oneshot(req)
            .await
            .expect("infallible");
        assert_eq!(r.status(), StatusCode::OK);
        let body = r.into_body().collect().await.expect("body").to_bytes();
        serde_json::from_slice::<Vec<EventEnvelope>>(&body)
            .expect("events")
            .into_iter()
            .map(|e| e.event)
            .collect()
    }
}

async fn sign_in_with(
    app: &AppState,
    provider: Provider,
    sub: &str,
    groups: &[&str],
) -> Result<Established, EstablishError> {
    let issuer = match provider {
        Provider::Oidc => ISSUER,
        _ => "https://github.com",
    };
    let who = Identity {
        provider,
        issuer: issuer.into(),
        sub: sub.into(),
        username: format!("user-{sub}"),
        groups: groups.iter().map(|g| (*g).to_owned()).collect(),
        email: None,
    };
    let tokens = Tokens {
        id_token: SecretString::from(String::new()),
        access_token: SecretString::from("provider-access-token".to_owned()),
        refresh_token: None,
        expires_at: tokio::time::Instant::now() + Duration::from_secs(3600),
    };
    signin::establish(app, who, tokens, json!({ "provider": provider.as_str() })).await
}

const MAPPED: &[(&str, &str)] = &[
    ("KEEL_OIDC_ROLE_CLAIM", "realm_access.roles"),
    ("KEEL_OIDC_OWNER_VALUES", "keel-owners"),
    ("KEEL_OIDC_ADMIN_VALUES", "keel-admins"),
    ("KEEL_OIDC_USER_VALUES", "keel-users"),
    ("KEEL_OIDC_ROLE_REQUIRED", "true"),
];

const MAPPED_OPTIONAL: &[(&str, &str)] = &[
    ("KEEL_OIDC_ROLE_CLAIM", "groups"),
    ("KEEL_OIDC_OWNER_VALUES", "keel-owners"),
    ("KEEL_OIDC_ADMIN_VALUES", "keel-admins"),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_sign_ins_make_exactly_one_owner() {
    let Some(h) = Harness::new("concurrent_first_sign_ins", &[]).await else {
        return;
    };
    let tasks: Vec<_> = (0..24)
        .map(|i| {
            let app = h.app.clone();
            tokio::spawn(async move {
                sign_in_with(&app, Provider::Github, &format!("racer-{i}"), &[])
                    .await
                    .unwrap_or_else(|e| panic!("establish: {e:?}"))
                    .user
                    .instance_role
            })
        })
        .collect();
    let mut owners = 0;
    for t in tasks {
        if t.await.expect("join") == keel_core::InstanceRole::Owner {
            owners += 1;
        }
    }
    assert_eq!(owners, 1);
    let (stored,): (i64,) =
        sqlx::query_as("select count(*) from users where instance_role = 'owner'")
            .fetch_one(&h.db.owner)
            .await
            .expect("owners");
    assert_eq!(stored, 1);
    let (others,): (i64,) = sqlx::query_as(
        "select count(*) from users where instance_role = 'user' and role_source = 'keel'",
    )
    .fetch_one(&h.db.owner)
    .await
    .expect("users");
    assert_eq!(others, 23);
    assert_eq!(h.app.role_mapping_info(), None);
    assert!(!h.app.idp_names_no_owner().await.expect("warning"));
    h.db.drop().await;
}

#[tokio::test]
async fn the_highest_mapped_role_wins() {
    let Some(h) = Harness::new("highest_mapped_role", MAPPED).await else {
        return;
    };
    // Mapped, the first account is not owner by default: only the claim decides.
    let u = h.oidc("u", &["keel-users", "offline_access"]).await;
    assert_eq!(u.user.instance_role, keel_core::InstanceRole::User);
    let a = h.oidc("a", &["keel-users", "keel-admins"]).await;
    assert_eq!(a.user.instance_role, keel_core::InstanceRole::Admin);
    let o = h
        .oidc("o", &["keel-admins", "keel-owners", "keel-users"])
        .await;
    assert_eq!(o.user.instance_role, keel_core::InstanceRole::Owner);
    assert_eq!(
        h.row(a.user.id).await,
        ("admin".into(), "idp".into(), Some("admin".into()), 1)
    );
    let created = h.audited("user_create").await;
    assert!(created.iter().all(|r| r["reason"] == "identity_provider"));

    let info = h.app.role_mapping_info().expect("mapping info");
    assert_eq!(info.provider, "oidc");
    assert_eq!(info.claim, "realm_access.roles");
    assert_eq!(info.owner, ["keel-owners"]);
    assert!(info.required);
    assert!(!h.app.idp_names_no_owner().await.expect("warning"));
    h.db.drop().await;
}

#[tokio::test]
async fn a_changed_role_bumps_the_epoch_and_is_audited() {
    let Some(h) = Harness::new("changed_mapped_role", MAPPED).await else {
        return;
    };
    let _owner = h.oidc("o", &["keel-owners"]).await;
    let first = h.oidc("a", &["keel-admins"]).await;
    let epoch = h.row(first.user.id).await.3;

    // Same role again: no bump, no audit.
    let again = h.oidc("a", &["keel-admins"]).await;
    assert_eq!(h.row(again.user.id).await.3, epoch);
    assert!(h.audited("user_role").await.is_empty());

    // Moved to the users group in the IdP.
    let moved = h.oidc("a", &["keel-users"]).await;
    assert_eq!(moved.user.instance_role, keel_core::InstanceRole::User);
    let (role, source, idp_role, new_epoch) = h.row(moved.user.id).await;
    assert_eq!(
        (role.as_str(), source.as_str(), idp_role.as_deref()),
        ("user", "idp", Some("user"))
    );
    assert_eq!(new_epoch, epoch + 1);
    assert_eq!(moved.user.authz_epoch, new_epoch);

    let rows = h.audited("user_role").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["reason"], "identity_provider");
    assert_eq!(rows[0]["decision"], "allowed");
    let target = &rows[0]["target"];
    assert_eq!(
        (&target["from"], &target["to"]),
        (&json!("admin"), &json!("user"))
    );

    // The session opened under the old role no longer works; the new one does.
    let stale: Vec<(String,)> = sqlx::query_as(
        "select s.id from sessions s join users u on u.id = s.user_id \
         where s.user_id = $1 and s.authz_epoch <> u.authz_epoch",
    )
    .bind(moved.user.id)
    .fetch_all(&h.db.owner)
    .await
    .expect("sessions");
    let stale: Vec<String> = stale.into_iter().map(|r| r.0).collect();
    assert!(stale.contains(&first.session) && stale.contains(&again.session));
    assert!(!stale.contains(&moved.session));
    h.db.drop().await;
}

#[tokio::test]
async fn no_match_refuses_writes_no_row_and_audits() {
    let Some(h) = Harness::new("role_not_mapped", MAPPED).await else {
        return;
    };
    let r = h
        .try_sign_in(Provider::Oidc, "nobody", &["offline_access"])
        .await;
    assert!(matches!(r, Err(EstablishError::NotMapped)), "{:?}", r.err());
    assert_eq!(h.users().await, 0);
    let rows = h.audited("sign_in").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["decision"], "denied");
    assert_eq!(rows[0]["reason"], "role_not_mapped");
    h.db.drop().await;
}

#[tokio::test]
async fn another_provider_is_refused_when_a_match_is_required() {
    let Some(h) = Harness::new("other_provider_required", MAPPED).await else {
        return;
    };
    let r = h.try_sign_in(Provider::Github, "gh", &[]).await;
    assert!(matches!(r, Err(EstablishError::NotMapped)), "{:?}", r.err());
    assert_eq!(h.users().await, 0);
    assert_eq!(h.audited("sign_in").await[0]["reason"], "role_not_mapped");
    h.db.drop().await;
}

#[tokio::test]
async fn optional_mapping_defaults_to_user_and_leaves_other_providers_alone() {
    let Some(h) = Harness::new("optional_mapping", MAPPED_OPTIONAL).await else {
        return;
    };
    // Not first-user-is-owner, even on an empty instance.
    let gh = h
        .try_sign_in(Provider::Github, "gh", &[])
        .await
        .expect("github");
    assert_eq!(gh.user.instance_role, keel_core::InstanceRole::User);
    assert_eq!(h.row(gh.user.id).await.1, "keel");
    let plain = h.oidc("plain", &["staff"]).await;
    assert_eq!(plain.user.instance_role, keel_core::InstanceRole::User);
    assert_eq!(
        h.row(plain.user.id).await,
        ("user".into(), "idp".into(), Some("user".into()), 1)
    );
    // Nobody is mapped to owner yet: System Settings warns.
    assert!(h.app.idp_names_no_owner().await.expect("warning"));
    let o = h.oidc("o", &["keel-owners"]).await;
    assert_eq!(o.user.instance_role, keel_core::InstanceRole::Owner);
    assert!(!h.app.idp_names_no_owner().await.expect("warning"));
    h.db.drop().await;
}

#[tokio::test]
async fn the_last_owner_is_never_demoted() {
    let Some(h) = Harness::new("last_owner_guard", MAPPED).await else {
        return;
    };
    let o = h.oidc("o", &["keel-owners"]).await;
    let epoch = h.row(o.user.id).await.3;

    let kept = h.oidc("o", &["keel-admins"]).await;
    assert_eq!(kept.user.instance_role, keel_core::InstanceRole::Owner);
    assert_eq!(
        h.row(o.user.id).await,
        ("owner".into(), "idp".into(), Some("admin".into()), epoch)
    );
    let rows = h.audited("user_role").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["reason"], "idp_names_no_owner");
    assert_eq!(rows[0]["target"]["to"], "owner");
    assert_eq!(rows[0]["target"]["mapped"], "admin");
    assert!(h.app.idp_names_no_owner().await.expect("warning"));

    // With a second owner, the first one is demoted like anyone else.
    let _second = h.oidc("o2", &["keel-owners"]).await;
    let demoted = h.oidc("o", &["keel-admins"]).await;
    assert_eq!(demoted.user.instance_role, keel_core::InstanceRole::Admin);
    let rows = h.audited("user_role").await;
    assert_eq!(rows.last().expect("row")["reason"], "identity_provider");
    assert!(!h.app.idp_names_no_owner().await.expect("warning"));
    h.db.drop().await;
}

#[tokio::test]
async fn preview_user_role_is_refused_for_idp_managed_users() {
    let Some(h) = Harness::new("role_managed_by_idp", MAPPED).await else {
        return;
    };
    let owner = h.oidc("o", &["keel-owners"]).await;
    let carol = h.oidc("c", &["keel-users"]).await;
    sqlx::query("update sessions set verified_at = now() where id = $1")
        .bind(&owner.session)
        .execute(&h.db.owner)
        .await
        .expect("verified");
    let events = h
        .cmd(
            &owner,
            Cmd::PreviewUserRole {
                user: carol.user.id,
                role: InstanceRole::Admin,
            },
        )
        .await;
    match events.as_slice() {
        [Event::OpError { code, .. }] => assert_eq!(code, "role_managed_by_idp"),
        other => panic!("expected role_managed_by_idp, got {other:?}"),
    }
    assert_eq!(h.row(carol.user.id).await.0, "user");

    // The Users list says who is managed by the IdP.
    let events = h.cmd(&owner, Cmd::ListUsers).await;
    let Some(Event::Users { items }) = events.first() else {
        panic!("expected Users, got {events:?}");
    };
    assert!(
        items
            .iter()
            .all(|u| u.role_source == keel_protocol::RoleSource::Idp)
    );
    h.db.drop().await;
}
