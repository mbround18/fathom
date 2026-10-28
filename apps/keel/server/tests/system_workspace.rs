//! The owner-only System workspace (spec 060 US7, T043).
//!
//! - Configure creates `keel-system` once and reuses it; the slug is reserved.
//! - An instance admin is refused `owner_only` on it (hosts, runs), never sees it in lists, and
//!   nobody can add a grant or an API token on it.
//! - The admin lists across workspaces (`ListEnrollments`, `ListAllApiTokens`) leave its rows
//!   out for non-owners, and those rows can't be revoked or rolled by them.
//! - Enrolment there is limited to the allowed profiles (`profile_not_allowed`).
//! - Remove revokes its hosts, disables its bindings, archives it and deletes the configuration
//!   in one audited change, keeping its history.
//! - A stale `version` is `system_conflict`.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod repos;

use keel_protocol::{
    ActionName, Cmd, Event, GrantProfiles, HostName, HostProfiles, Label, ProfileName, Slug,
    TeamRole, TokenActions,
};
use keel_server::hosts;
use keel_server::signin::Established;
use repos::{Harness, op_error, preview, sign_in};
use serde_json::Value;
use uuid::Uuid;

fn profiles(names: &[&str]) -> HostProfiles {
    HostProfiles(
        names
            .iter()
            .map(|n| ProfileName::new(*n).expect("profile"))
            .collect(),
    )
}

fn denied(events: &[Event]) -> String {
    match events {
        [Event::Denied { reason, .. }] => reason.clone(),
        other => panic!("expected Denied, got {other:?}"),
    }
}

/// Configures the System workspace as the owner; returns its id and the new version.
async fn configure(h: &Harness, allowed: &[&str], version: i64) -> (Uuid, i64) {
    let (_, out) = h
        .confirm(Cmd::PreviewSystemConfigure {
            allowed_profiles: profiles(allowed),
            version,
        })
        .await;
    match out.as_slice() {
        [
            Event::Confirmed { .. },
            Event::System {
                configured: true,
                workspace: Some(ws),
                allowed_profiles,
                version,
                ..
            },
        ] => {
            assert_eq!(allowed_profiles, &allowed.to_vec());
            (*ws, *version)
        }
        other => panic!("configure: {other:?}"),
    }
}

async fn admin(h: &Harness) -> Established {
    let est = sign_in(&h.app, "2", "admin").await;
    sqlx::query("update users set instance_role = 'admin' where id = $1")
        .bind(est.user.id)
        .execute(&h.db.owner)
        .await
        .expect("admin");
    h.step_up(&est).await;
    est
}

async fn audited(h: &Harness, action: &str) -> Vec<Value> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        "select to_jsonb(a) - 'hash' - 'prev_hash' from audit_event a where action = $1 \
         order by seq",
    )
    .bind(action)
    .fetch_all(&h.db.owner)
    .await
    .expect("audit");
    rows.into_iter().map(|r| r.0).collect()
}

async fn workspaces(h: &Harness, est: &Established) -> Vec<String> {
    match h.cmd_as(est, Cmd::ListWorkspaces).await.as_slice() {
        [Event::Workspaces { items }] => items.iter().map(|w| w.slug.clone()).collect(),
        other => panic!("workspaces: {other:?}"),
    }
}

#[tokio::test]
async fn configure_creates_once_and_reserves_the_slug() {
    let Some(h) = Harness::new("system_configure").await else {
        return;
    };
    // Nothing configured: GetSystem says so.
    assert!(matches!(
        h.cmd(Cmd::GetSystem).await.as_slice(),
        [Event::System {
            configured: false,
            version: 0,
            ..
        }]
    ));
    let (ws, v1) = configure(&h, &["debian"], 0).await;
    assert_eq!(v1, 1);
    assert_eq!(h.app.system_workspace(), Some(ws));
    let (ws2, v2) = configure(&h, &["debian", "web"], v1).await;
    assert_eq!(ws2, ws, "the second configure reuses the workspace");
    assert_eq!(v2, 2);
    let (n,): (i64,) = sqlx::query_as("select count(*) from workspaces where system")
        .fetch_one(&h.db.owner)
        .await
        .expect("count");
    assert_eq!(n, 1);
    let audit = audited(&h, "system_configure").await;
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[1]["target"]["workspace"], ws.to_string());
    assert_eq!(audit[1]["context"]["before"], serde_json::json!(["debian"]));

    // A stale version is a conflict, at preview and at confirm.
    let stale = h
        .cmd(Cmd::PreviewSystemConfigure {
            allowed_profiles: profiles(&["web"]),
            version: v1,
        })
        .await;
    assert_eq!(op_error(&stale), "system_conflict");
    let p = preview(
        &h.cmd(Cmd::PreviewSystemConfigure {
            allowed_profiles: profiles(&["web"]),
            version: v2,
        })
        .await,
    );
    let _ = configure(&h, &["debian"], v2).await;
    let out = h
        .cmd(Cmd::Confirm {
            preview_id: p.preview_id,
            step_up: None,
        })
        .await;
    assert!(
        out.iter()
            .any(|e| matches!(e, Event::OpError { code, .. } if code == "system_conflict")),
        "{out:?}"
    );

    // The slug is reserved for everyone, owner included.
    let out = h
        .cmd(Cmd::PreviewWorkspaceCreate {
            slug: Slug::new("keel-system").unwrap(),
            name: Label::new("Mine").unwrap(),
        })
        .await;
    assert_eq!(op_error(&out), "slug_reserved");
    h.finish().await;
}

#[tokio::test]
async fn admins_never_see_or_use_it() {
    let Some(h) = Harness::new("system_admins").await else {
        return;
    };
    let (ws, _) = configure(&h, &["debian"], 0).await;
    let admin = admin(&h).await;

    // Lists: the owner sees it, the admin does not.
    assert!(
        workspaces(&h, &h.owner)
            .await
            .contains(&"keel-system".to_owned())
    );
    let seen = workspaces(&h, &admin).await;
    assert!(seen.contains(&"ops".to_owned()), "{seen:?}");
    assert!(!seen.contains(&"keel-system".to_owned()), "{seen:?}");

    // System commands themselves.
    for cmd in [
        Cmd::GetSystem,
        Cmd::PreviewSystemRemove,
        Cmd::PreviewSystemConfigure {
            allowed_profiles: profiles(&["debian"]),
            version: 1,
        },
    ] {
        assert_eq!(denied(&h.cmd_as(&admin, cmd).await), "owner_only");
    }
    // Workspace commands on it.
    let out = h
        .cmd_as(
            &admin,
            Cmd::ListHosts {
                workspace: ws,
                profile: None,
                drift: None,
                changed_outside: None,
            },
        )
        .await;
    assert_eq!(denied(&out), "owner_only");
    let out = h
        .cmd_as(
            &admin,
            Cmd::ListRuns {
                workspace: ws,
                binding: None,
                host: None,
                actor: None,
                state: None,
                from: None,
                to: None,
            },
        )
        .await;
    assert_eq!(denied(&out), "owner_only");
    let out = h
        .cmd_as(&admin, Cmd::PreviewWorkspaceArchive { workspace: ws })
        .await;
    assert_eq!(denied(&out), "owner_only");

    // No grants, no API tokens: not for the admin, not for the owner.
    for est in [&admin, &h.owner] {
        let out = h
            .cmd_as(
                est,
                Cmd::PreviewGrantAdd {
                    team: Uuid::now_v7(),
                    workspace: ws,
                    role: TeamRole::Operator,
                    profiles: GrantProfiles::default(),
                },
            )
            .await;
        assert_eq!(denied(&out), "owner_only");
        let out = h
            .cmd_as(
                est,
                Cmd::PreviewApiTokenCreate {
                    name: Label::new("ci").unwrap(),
                    workspace: ws,
                    actions: TokenActions(vec![ActionName::new("list_hosts").unwrap()]),
                    expires_in_days: 7,
                },
            )
            .await;
        assert_eq!(denied(&out), "owner_only");
    }
    // The owner can't archive it from the workspace list either.
    let out = h.cmd(Cmd::PreviewWorkspaceArchive { workspace: ws }).await;
    assert_eq!(op_error(&out), "system_workspace");
    // Every refusal is audited.
    let denials: Vec<Value> = sqlx::query_as::<_, (Value,)>(
        "select to_jsonb(a) from audit_event a where decision = 'denied' and reason = 'owner_only'",
    )
    .fetch_all(&h.db.owner)
    .await
    .expect("audit")
    .into_iter()
    .map(|r| r.0)
    .collect();
    assert!(denials.len() >= 9, "{denials:?}");
    h.finish().await;
}

#[tokio::test]
async fn enrolment_is_limited_to_the_allowed_profiles() {
    let Some(h) = Harness::new("system_profiles").await else {
        return;
    };
    hosts::bootstrap(&h.app).await.expect("bootstrap");
    let (ws, _) = configure(&h, &["debian"], 0).await;
    for name in ["debian", "web"] {
        sqlx::query("insert into profiles (id, workspace_id, name) values ($1, $2, $3)")
            .bind(Uuid::now_v7())
            .bind(ws)
            .bind(name)
            .execute(&h.db.owner)
            .await
            .expect("profile");
    }
    let enrol = |names: &'static [&'static str]| Cmd::PreviewHostEnroll {
        workspace: ws,
        inventory_name: HostName::new("keel-1").unwrap(),
        profiles: profiles(names),
    };
    assert_eq!(
        op_error(&h.cmd(enrol(&["debian", "web"])).await),
        "profile_not_allowed"
    );
    let _ = preview(&h.cmd(enrol(&["debian"])).await);
    h.finish().await;
}

#[tokio::test]
async fn remove_revokes_disables_archives_and_keeps_history() {
    let Some(h) = Harness::new("system_remove").await else {
        return;
    };
    let (ws, _) = configure(&h, &["debian"], 0).await;
    let (host, project, binding, enrolment) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    sqlx::query("insert into hosts (id, workspace_id, inventory_name) values ($1, $2, 'keel-1')")
        .bind(host)
        .bind(ws)
        .execute(&h.db.owner)
        .await
        .expect("host");
    sqlx::query(
        "insert into projects (id, workspace_id, slug, name) values ($1, $2, 'keel', 'Keel')",
    )
    .bind(project)
    .bind(ws)
    .execute(&h.db.owner)
    .await
    .expect("project");
    sqlx::query(
        "insert into repo_bindings (id, project_id, url, url_key, branch, transport) \
         values ($1, $2, 'https://example.com/keel.git', 'example.com/keel', 'main', 'https')",
    )
    .bind(binding)
    .bind(project)
    .execute(&h.db.owner)
    .await
    .expect("binding");
    sqlx::query(
        "insert into host_enrollments (id, workspace_id, inventory_name, token_hash, expires_at) \
         values ($1, $2, 'keel-2', $3, now() + interval '15 minutes')",
    )
    .bind(enrolment)
    .bind(ws)
    .bind(vec![9u8; 32])
    .execute(&h.db.owner)
    .await
    .expect("enrolment");

    match h.cmd(Cmd::GetSystem).await.as_slice() {
        [
            Event::System {
                hosts, bindings, ..
            },
        ] => {
            assert_eq!(hosts, &vec![host]);
            assert_eq!(bindings, &vec![binding]);
        }
        other => panic!("{other:?}"),
    }

    let (p, out) = h.confirm(Cmd::PreviewSystemRemove).await;
    assert!(p.step_up);
    assert!(
        p.affected.contains(&"keel-1".to_owned()),
        "{:?}",
        p.affected
    );
    assert!(
        matches!(
            out.as_slice(),
            [
                Event::Confirmed { .. },
                Event::System {
                    configured: false,
                    ..
                }
            ]
        ),
        "{out:?}"
    );
    assert_eq!(h.app.system_workspace(), None);

    let (state,): (String,) = sqlx::query_as("select state from hosts where id = $1")
        .bind(host)
        .fetch_one(&h.db.owner)
        .await
        .expect("host");
    assert_eq!(state, "revoked");
    let (status,): (String,) = sqlx::query_as("select status from repo_bindings where id = $1")
        .bind(binding)
        .fetch_one(&h.db.owner)
        .await
        .expect("binding");
    assert_eq!(status, "disabled");
    let (burned,): (Option<String>,) =
        sqlx::query_as("select burned_reason from host_enrollments where id = $1")
            .bind(enrolment)
            .fetch_one(&h.db.owner)
            .await
            .expect("enrolment");
    assert_eq!(burned.as_deref(), Some("system_removed"));
    let (archived, system): (bool, bool) =
        sqlx::query_as("select archived_at is not null, system from workspaces where id = $1")
            .bind(ws)
            .fetch_one(&h.db.owner)
            .await
            .expect("workspace kept");
    assert!(archived && system);
    let (configs,): (i64,) = sqlx::query_as("select count(*) from system_config")
        .fetch_one(&h.db.owner)
        .await
        .expect("config");
    assert_eq!(configs, 0);

    let audit = audited(&h, "system_remove").await;
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["context"]["hosts_revoked"], 1);
    assert_eq!(audit[0]["context"]["bindings_disabled"], 1);
    // History stays: the configure row is still there.
    assert_eq!(audited(&h, "system_configure").await.len(), 1);

    // Nothing to remove twice; configuring again reuses (and un-archives) the workspace.
    assert_eq!(
        op_error(&h.cmd(Cmd::PreviewSystemRemove).await),
        "not_configured"
    );
    let (again, v) = configure(&h, &["debian"], 0).await;
    assert_eq!((again, v), (ws, 1));
    h.finish().await;
}

/// Inserts an API token on `ws` for `holder`, as if made before the workspace was System.
async fn token_on(h: &Harness, holder: Uuid, ws: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into api_tokens (id, name, owner_user_id, workspace_id, actions, hash, \
         expires_at) values ($1, 'ci', $2, $3, array['list_workspaces'], $4, \
         now() + interval '7 days')",
    )
    .bind(id)
    .bind(holder)
    .bind(ws)
    .bind(id.as_bytes().to_vec())
    .execute(&h.db.owner)
    .await
    .expect("token");
    id
}

fn enrolment_ids(out: &[Event]) -> Vec<Uuid> {
    match out {
        [Event::Enrollments { items }] => items.iter().map(|e| e.id).collect(),
        other => panic!("enrolments: {other:?}"),
    }
}

fn token_ids(out: &[Event]) -> Vec<Uuid> {
    match out {
        [Event::AllApiTokens { items }] => items.iter().map(|t| t.token.id).collect(),
        other => panic!("tokens: {other:?}"),
    }
}

#[tokio::test]
async fn admin_lists_leave_it_out_for_non_owners() {
    let Some(h) = Harness::new("system_lists").await else {
        return;
    };
    hosts::bootstrap(&h.app).await.expect("bootstrap");
    let (ws, _) = configure(&h, &["debian"], 0).await;
    // One enrolment token in each workspace, minted by the owner.
    let mut minted = Vec::new();
    for w in [ws, h.workspace] {
        sqlx::query("insert into profiles (id, workspace_id, name) values ($1, $2, 'debian')")
            .bind(Uuid::now_v7())
            .bind(w)
            .execute(&h.db.owner)
            .await
            .expect("profile");
        let (_, out) = h
            .confirm(Cmd::PreviewHostEnroll {
                workspace: w,
                inventory_name: HostName::new("host-1").unwrap(),
                profiles: profiles(&["debian"]),
            })
            .await;
        match out.as_slice() {
            [Event::Confirmed { .. }, Event::HostEnrollment { id, .. }] => minted.push(*id),
            other => panic!("enrol: {other:?}"),
        }
    }
    let (system_enrolment, ops_enrolment) = (minted[0], minted[1]);
    let admin = admin(&h).await;
    let system_token = token_on(&h, h.owner.user.id, ws).await;
    let ops_token = token_on(&h, admin.user.id, h.workspace).await;
    let all_enrolments = |workspace| Cmd::ListEnrollments {
        state: None,
        workspace,
    };
    let all_tokens = || Cmd::ListAllApiTokens {
        holder: None,
        state: None,
    };

    // The owner sees both workspaces' rows.
    let seen = enrolment_ids(&h.cmd(all_enrolments(None)).await);
    assert!(
        seen.contains(&system_enrolment) && seen.contains(&ops_enrolment),
        "{seen:?}"
    );
    let seen = token_ids(&h.cmd(all_tokens()).await);
    assert!(
        seen.contains(&system_token) && seen.contains(&ops_token),
        "{seen:?}"
    );

    // The admin sees only the other workspace's, even when asking for System's by id.
    assert_eq!(
        enrolment_ids(&h.cmd_as(&admin, all_enrolments(None)).await),
        vec![ops_enrolment]
    );
    assert!(enrolment_ids(&h.cmd_as(&admin, all_enrolments(Some(ws))).await).is_empty());
    assert_eq!(
        token_ids(&h.cmd_as(&admin, all_tokens()).await),
        vec![ops_token]
    );

    // Nor can the admin act on them: for the admin they don't exist.
    let out = h
        .cmd_as(
            &admin,
            Cmd::PreviewEnrollmentRevoke {
                enrollment: system_enrolment,
            },
        )
        .await;
    assert_eq!(op_error(&out), "not_found");
    let out = h
        .cmd_as(
            &admin,
            Cmd::PreviewEnrollmentRoll {
                enrollment: system_enrolment,
            },
        )
        .await;
    assert_eq!(op_error(&out), "not_found");
    let out = h
        .cmd_as(
            &admin,
            Cmd::PreviewApiTokenRevoke {
                token: system_token,
            },
        )
        .await;
    assert_eq!(denied(&out), "not_owner");
    h.finish().await;
}
