//! Spec 066: workspaces, the primary repository, the catalog, forge connections and host add.
//!
//! The harness for the stories' tests (T033, T040 to T046). Foundation (T005) checks only that
//! migration 0014 applied as data-model.md describes.

mod common;

use common::TestDb;

#[tokio::test]
async fn migration_0014_adds_the_spec_066_schema() {
    let Some(db) = TestDb::new("repo_schema").await else {
        return;
    };
    for (table, column) in [
        ("workspaces", "primary_binding"),
        ("workspaces", "primary_set_by"),
        ("workspaces", "primary_set_at"),
        ("hosts", "age_recipient"),
        ("forge_connections", "api_base"),
        ("binding_catalogs", "catalog"),
    ] {
        let (n,): (i64,) = sqlx::query_as(
            "select count(*) from information_schema.columns \
             where table_schema = current_schema() and table_name = $1 and column_name = $2",
        )
        .bind(table)
        .bind(column)
        .fetch_one(&db.owner)
        .await
        .unwrap();
        assert_eq!(n, 1, "{table}.{column}");
    }
    // Only an age public key fits `hosts.age_recipient`.
    let (n,): (i64,) = sqlx::query_as(
        "select count(*) from pg_constraint where conrelid = 'hosts'::regclass \
         and pg_get_constraintdef(oid) like '%age_recipient%age1%'",
    )
    .fetch_one(&db.owner)
    .await
    .unwrap();
    assert_eq!(n, 1);
    // `forge_token` is a credential kind.
    let (def,): (String,) = sqlx::query_as(
        "select pg_get_constraintdef(oid) from pg_constraint where conname = 'credentials_kind_check' \
         and connamespace = current_schema()::regnamespace",
    )
    .fetch_one(&db.owner)
    .await
    .unwrap();
    assert!(def.contains("forge_token"), "{def}");
    // The serving role may use the new tables.
    for table in ["forge_connections", "binding_catalogs"] {
        let (ok,): (bool,) = sqlx::query_as("select has_table_privilege('keel_app', $1, 'insert')")
            .bind(table)
            .fetch_one(&db.owner)
            .await
            .unwrap();
        assert!(ok, "{table}");
    }
    db.drop().await;
}

// ---- T033: the primary repository and the forge connection (US1, US2) -------------------------

mod repos;

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use keel_protocol::{
    ApiBase, BranchName, Cmd, CredentialKind, Event, ForgeProvider, ObjectName, RepoUrl,
    RepositorySpec, SecretValue, Transport,
};
use repos::{Harness, Source, bind, manifest, op_error, sign_in};
use uuid::Uuid;

const TOKEN: &str = "forge-token-plugh";

fn repository_spec(src: &Source) -> RepositorySpec {
    RepositorySpec {
        url: RepoUrl::new(src.url()).expect("url"),
        branch: BranchName::new("main").expect("branch"),
        transport: Transport::Https,
        credential: None,
        known_host: None,
        poll_interval: None,
        allow_force_push: false,
        signed_mode: false,
        override_quorum: 1,
    }
}

async fn primary_of(h: &Harness) -> Option<Uuid> {
    sqlx::query_scalar("select primary_binding from workspaces where id = $1")
        .bind(h.workspace)
        .fetch_one(&h.db.owner)
        .await
        .expect("primary")
}

async fn credential(h: &Harness, name: &str, kind: CredentialKind, value: &str) -> Uuid {
    let (_, out) = h
        .confirm(Cmd::PreviewCredentialCreate {
            workspace: h.workspace,
            name: ObjectName::new(name).expect("name"),
            kind,
            value: SecretValue::new(value).expect("value"),
        })
        .await;
    match out.as_slice() {
        [_, Event::Credentials { items }] => {
            items.iter().find(|c| c.name == name).expect("created").id
        }
        other => panic!("credential: {other:?}"),
    }
}

#[tokio::test]
async fn workspace_repository_binds_on_default_and_becomes_primary() {
    let Some(h) = Harness::new("repo_primary_create").await else {
        return;
    };
    // The harness workspace has no `default` project: the confirm creates it.
    let src = Source::new(&manifest(&[]));
    let (p, out) = h
        .confirm(Cmd::PreviewWorkspaceRepository {
            workspace: h.workspace,
            spec: repository_spec(&src),
        })
        .await;
    assert_eq!(p.action, "workspace_repository");
    assert!(p.summary.contains("primary repository"), "{}", p.summary);
    let id = match out.as_slice() {
        [Event::Confirmed { .. }, Event::Binding { binding, .. }] => binding.id,
        other => panic!("{other:?}"),
    };
    assert_eq!(primary_of(&h).await, Some(id));
    let (slug, synced): (String, Option<String>) = sqlx::query_as(
        "select p.slug, b.last_commit from repo_bindings b \
         join projects p on p.id = b.project_id where b.id = $1",
    )
    .bind(id)
    .fetch_one(&h.db.owner)
    .await
    .expect("project");
    assert_eq!(slug, "default");
    assert!(synced.is_some(), "it synced on create");
    match h.cmd(Cmd::ListWorkspaces).await.as_slice() {
        [Event::Workspaces { items }] => {
            let ws = items.iter().find(|w| w.id == h.workspace).expect("listed");
            assert_eq!(ws.primary_binding, Some(id));
        }
        other => panic!("{other:?}"),
    }
    let actions = h.audit_actions().await;
    for a in ["binding_created", "workspace_primary_set"] {
        assert!(
            actions.iter().any(|(x, d)| x == a && d == "allowed"),
            "{a}: {actions:?}"
        );
    }
    // A second repository reuses the `default` project and takes over as primary.
    let other = Source::new(&manifest(&[]));
    h.confirm(Cmd::PreviewWorkspaceRepository {
        workspace: h.workspace,
        spec: repository_spec(&other),
    })
    .await;
    let (n,): (i64,) = sqlx::query_as(
        "select count(*) from projects where workspace_id = $1 and slug = 'default'",
    )
    .bind(h.workspace)
    .fetch_one(&h.db.owner)
    .await
    .expect("count");
    assert_eq!(n, 1);
    assert_ne!(primary_of(&h).await, Some(id));
    h.finish().await;
}

#[tokio::test]
async fn primary_is_set_cleared_and_checked() {
    let Some(h) = Harness::new("repo_primary_set").await else {
        return;
    };
    let src = Source::new(&manifest(&[]));
    let id = bind(&h, &src).await;
    assert_eq!(primary_of(&h).await, None, "a plain binding is not primary");

    let (_, out) = h
        .confirm(Cmd::PreviewWorkspacePrimary {
            workspace: h.workspace,
            binding: Some(id),
        })
        .await;
    match out.as_slice() {
        [Event::Confirmed { .. }, Event::Workspaces { items }] => {
            let ws = items.iter().find(|w| w.id == h.workspace).expect("listed");
            assert_eq!(ws.primary_binding, Some(id));
        }
        other => panic!("{other:?}"),
    }

    let out = h
        .cmd(Cmd::PreviewWorkspacePrimary {
            workspace: h.workspace,
            binding: Some(Uuid::now_v7()),
        })
        .await;
    assert_eq!(op_error(&out), "not_found");

    let stranger = sign_in(&h.app, "2", "stranger").await;
    let out = h
        .cmd_as(
            &stranger,
            Cmd::PreviewWorkspacePrimary {
                workspace: h.workspace,
                binding: None,
            },
        )
        .await;
    assert!(matches!(out.as_slice(), [Event::Denied { .. }]), "{out:?}");
    assert!(
        h.audit_actions()
            .await
            .iter()
            .any(|(a, d)| a == "preview_workspace_primary" && d == "denied")
    );

    h.confirm(Cmd::PreviewWorkspacePrimary {
        workspace: h.workspace,
        binding: None,
    })
    .await;
    assert_eq!(primary_of(&h).await, None);

    // Removing the primary binding clears it; nothing is promoted (US2 scenario 5).
    h.confirm(Cmd::PreviewWorkspacePrimary {
        workspace: h.workspace,
        binding: Some(id),
    })
    .await;
    h.confirm(Cmd::PreviewBindingRemove { binding: id }).await;
    assert_eq!(primary_of(&h).await, None);
    h.finish().await;
}

/// A Forgejo API fake: `acme/infra` answers with push access for [`TOKEN`]; anything else, or
/// another token, is 401.
async fn fake_forge() -> String {
    async fn repo(
        axum::extract::Path((owner, name)): axum::extract::Path<(String, String)>,
        headers: HeaderMap,
    ) -> (StatusCode, String) {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        if auth != format!("token {TOKEN}") || owner != "acme" || name != "infra" {
            return (StatusCode::UNAUTHORIZED, r#"{"message":"no"}"#.into());
        }
        (
            StatusCode::OK,
            r#"{"default_branch":"main","permissions":{"push":true}}"#.into(),
        )
    }
    let app = Router::new().route("/api/v1/repos/{owner}/{name}", get(repo));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{addr}/api/v1")
}

async fn set_url(h: &Harness, binding: Uuid, url: &str) {
    sqlx::query("update repo_bindings set url = $2 where id = $1")
        .bind(binding)
        .bind(url)
        .execute(&h.db.owner)
        .await
        .expect("url");
}

async fn forge_info(h: &Harness) -> Option<keel_protocol::ForgeConnectionInfo> {
    match h
        .cmd(Cmd::GetForgeConnection {
            workspace: h.workspace,
        })
        .await
        .as_slice()
    {
        [Event::ForgeConnection(c)] => c.clone(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn forge_connection_is_set_probed_and_removed() {
    let Some(h) = Harness::new("repo_forge").await else {
        return;
    };
    let token = credential(&h, "forge", CredentialKind::ForgeToken, TOKEN).await;
    let wrong = credential(&h, "https", CredentialKind::GitHttpsToken, "x-token").await;
    let ws = h.workspace;

    assert!(forge_info(&h).await.is_none());
    let out = h.cmd(Cmd::ProbeForge { workspace: ws }).await;
    assert_eq!(op_error(&out), "forge_not_configured");

    // Without a primary repository there is no API base to default to.
    let out = h
        .cmd(Cmd::PreviewForgeConnection {
            workspace: ws,
            provider: ForgeProvider::Forgejo,
            api_base: None,
            credential: token,
        })
        .await;
    assert_eq!(op_error(&out), "no_primary_repository");
    // Only a forge_token will do.
    let out = h
        .cmd(Cmd::PreviewForgeConnection {
            workspace: ws,
            provider: ForgeProvider::Forgejo,
            api_base: None,
            credential: wrong,
        })
        .await;
    assert_eq!(op_error(&out), "credential_unavailable");

    // A primary binding, pointed at the fake's repository.
    let src = Source::new(&manifest(&[]));
    let id = bind(&h, &src).await;
    h.confirm(Cmd::PreviewWorkspacePrimary {
        workspace: ws,
        binding: Some(id),
    })
    .await;
    set_url(&h, id, "https://forge.example/acme/infra.git").await;

    let base = fake_forge().await;
    let (p, out) = h
        .confirm(Cmd::PreviewForgeConnection {
            workspace: ws,
            provider: ForgeProvider::Forgejo,
            api_base: Some(ApiBase::new(base.clone()).expect("api base")),
            credential: token,
        })
        .await;
    assert!(p.summary.contains(&base), "{}", p.summary);
    match out.as_slice() {
        [Event::Confirmed { .. }, Event::ForgeConnection(Some(c))] => {
            assert_eq!(c.provider, ForgeProvider::Forgejo);
            assert_eq!(c.api_base, base);
            assert_eq!(c.credential, token);
            assert!(c.probed_at.is_none());
        }
        other => panic!("{other:?}"),
    }

    let out = h.cmd(Cmd::ProbeForge { workspace: ws }).await;
    assert!(
        matches!(
            out.as_slice(),
            [Event::ForgeProbe {
                ok: true,
                can_push: true,
                message: None
            }]
        ),
        "{out:?}"
    );
    let c = forge_info(&h).await.expect("connection");
    assert!(c.probed_at.is_some());
    assert!(c.probe_error.is_none());

    // A refusal is reported and kept on the connection.
    set_url(&h, id, "https://forge.example/acme/locked.git").await;
    let out = h.cmd(Cmd::ProbeForge { workspace: ws }).await;
    match out.as_slice() {
        [
            Event::ForgeProbe {
                ok: false,
                can_push: false,
                message: Some(m),
            },
        ] => assert!(!m.contains(TOKEN), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(
        forge_info(&h)
            .await
            .expect("connection")
            .probe_error
            .is_some()
    );

    // A stranger can neither read, probe nor remove it.
    let stranger = sign_in(&h.app, "2", "stranger").await;
    for cmd in [
        Cmd::GetForgeConnection { workspace: ws },
        Cmd::ProbeForge { workspace: ws },
        Cmd::PreviewForgeRemove { workspace: ws },
    ] {
        let out = h.cmd_as(&stranger, cmd).await;
        assert!(matches!(out.as_slice(), [Event::Denied { .. }]), "{out:?}");
    }

    let (_, out) = h.confirm(Cmd::PreviewForgeRemove { workspace: ws }).await;
    assert!(
        matches!(
            out.as_slice(),
            [Event::Confirmed { .. }, Event::ForgeConnection(None)]
        ),
        "{out:?}"
    );
    let out = h.cmd(Cmd::PreviewForgeRemove { workspace: ws }).await;
    assert_eq!(op_error(&out), "forge_not_configured");

    let actions = h.audit_actions().await;
    for a in ["forge_connection_set", "forge_connection_remove"] {
        assert!(
            actions.iter().any(|(x, d)| x == a && d == "allowed"),
            "{a}: {actions:?}"
        );
    }
    assert!(!h.audit_text().await.contains(TOKEN));
    h.finish().await;
}
