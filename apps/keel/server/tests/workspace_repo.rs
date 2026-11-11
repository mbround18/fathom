//! Spec 066: workspaces, the primary repository, the catalog, forge connections and host add.
//!
//! The harness for the stories' tests (T033, T040 to T046). Foundation (T005) checks only that
//! migration 0014 applied as data-model.md describes.

mod common;
mod repos;

use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use common::TestDb;
use keel_core::ports::{ChangeRequest, Forge, ForgeAccess, ForgeFuture, OpenedChange};
use keel_protocol::{
    ApiBase, BranchName, CatalogView, ChangeOutcome, Cmd, CredentialKind, Event, ForgeProvider,
    GroupName, HostGroups, HostInventoryStatus, HostName, HostProfiles, ObjectName, RepoPath,
    RepoUrl, RepositorySpec, SecretValue, Transport,
};
use keel_server::hosts;
use keel_server::proposals::{Connector, Forges};
use repos::{Harness, Source, bind, manifest, op_error, sign_in};
use uuid::Uuid;

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

// ---- the catalog, proposals and SC-004 (T040 to T046) ------------------------------------------

/// The fixture: a synthetic copy of an Ansible repository with sops files.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../crates/keel-core/tests/fixtures/ansible-repo"
);
/// A host key the fixture's `keel_servers` rule and file list.
const KEEL_KEY: &str = "age1psaqfhh8a5h5ekl9ulhkkrwa4430q7mupsaqfhh8a5h5ekl9ulhkkrwa44";
/// A well-formed host key nothing in the fixture lists.
const NEW_KEY: &str = "age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";

/// Every `ENC[…]` value in the fixture's sops files: the strings that must never leave Keel.
fn ciphertexts() -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(Path::new(FIXTURE).join("secrets")).expect("secrets") {
        let text = std::fs::read_to_string(entry.expect("entry").path()).expect("read");
        let mut rest = text.as_str();
        while let Some(at) = rest.find("ENC[") {
            let end = rest[at..].find(']').map_or(rest.len(), |e| at + e + 1);
            let value = &rest[at..end];
            if let Some(data) = value.split(',').find_map(|p| p.strip_prefix("data:")) {
                out.push(data.to_owned());
            }
            out.push(value.to_owned());
            rest = &rest[end..];
        }
    }
    assert!(!out.is_empty(), "the fixture has ciphertext");
    out
}

fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let dest = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            std::fs::create_dir_all(&dest).expect("mkdir");
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).expect("copy");
        }
    }
}

/// What the fake forge was asked to open.
#[derive(Clone, Default)]
struct FakeForge {
    opened: Arc<Mutex<Vec<ChangeRequest>>>,
}

impl Forge for FakeForge {
    fn probe(&self) -> ForgeFuture<'_, ForgeAccess> {
        Box::pin(async {
            Ok(ForgeAccess {
                can_push: true,
                default_branch: "main".to_owned(),
            })
        })
    }

    fn open_change<'a>(&'a self, change: &'a ChangeRequest) -> ForgeFuture<'a, OpenedChange> {
        Box::pin(async move {
            self.opened.lock().unwrap().push(change.clone());
            Ok(OpenedChange {
                url: "https://forge.example/ops/infra/pulls/7".to_owned(),
                branch: change.new_branch.clone(),
                number: 7,
            })
        })
    }
}

/// A harness whose `ops` workspace has the fixture as its primary binding, synced and
/// catalogued, and hosts `keel` (key listed) and `alpha` (no key reported).
async fn primed(test: &str, forge: Option<FakeForge>) -> Option<(Harness, Uuid)> {
    let mut h = Harness::new(test).await?;
    if let Some(f) = forge {
        let connect: Connector = Arc::new(move |_target| Ok(Arc::new(f.clone()) as Arc<dyn Forge>));
        h.app = h
            .app
            .clone()
            .with_forges(Forges::default().with_connector(connect));
    }
    hosts::bootstrap(&h.app).await.expect("bootstrap");
    let src = Source::new(&repos::manifest(&[]));
    copy_tree(Path::new(FIXTURE), &src.work);
    src.commit("README.md", "fixture\n", "fixture");
    src.push();
    let binding = bind(&h, &src).await;
    sqlx::query("update workspaces set primary_binding = $1 where id = $2")
        .bind(binding)
        .bind(h.workspace)
        .execute(&h.db.owner)
        .await
        .expect("primary");
    let out = h.cmd(Cmd::SyncBinding { binding }).await;
    assert!(
        !out.iter().any(|e| matches!(e, Event::OpError { .. })),
        "{out:?}"
    );
    for (name, key) in [("keel", Some(KEEL_KEY)), ("alpha", None)] {
        sqlx::query(
            "insert into hosts (id, workspace_id, inventory_name, age_recipient) \
             values ($1, $2, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(h.workspace)
        .bind(name)
        .bind(key)
        .execute(&h.db.owner)
        .await
        .expect("host");
    }
    // The source can go: everything the tests read is in the binding's mirror.
    drop(src);
    Some((h, binding))
}

async fn catalog(h: &Harness) -> CatalogView {
    match h
        .cmd(Cmd::GetCatalog {
            workspace: h.workspace,
        })
        .await
        .as_slice()
    {
        [Event::Catalog(c)] => c.clone(),
        other => panic!("{other:?}"),
    }
}

fn change(out: &[Event]) -> ChangeOutcome {
    match out {
        [Event::Confirmed { .. }, Event::ChangeProposed(c)] => c.clone(),
        [Event::Confirmed { .. }, Event::HostAdded { change, .. }] => change.clone(),
        other => panic!("{other:?}"),
    }
}

fn host_add(h: &Harness, name: &str, groups: &[&str]) -> Cmd {
    Cmd::PreviewHostAdd {
        workspace: h.workspace,
        inventory_name: HostName::new(name).expect("name"),
        groups: HostGroups(
            groups
                .iter()
                .map(|g| GroupName::new(*g).expect("group"))
                .collect(),
        ),
        profiles: HostProfiles::default(),
        inventory: None,
    }
}

#[tokio::test]
async fn a_primary_sync_catalogues_the_repository() {
    let Some((h, binding)) = primed("repo_catalog", None).await else {
        return;
    };
    let c = catalog(&h).await;
    assert_eq!(c.binding, binding);
    assert!(!c.stale);
    assert!(
        c.playbooks
            .iter()
            .any(|p| p.path == "playbooks/keel-server.yaml")
    );
    assert!(c.roles.iter().any(|r| r.name == "base"));
    assert!(
        c.inventories
            .iter()
            .any(|i| i.path == "inventory/hosts.yml")
    );
    let file = c
        .secrets
        .files
        .iter()
        .find(|f| f.path == "secrets/keel_servers.sops.yaml")
        .expect("sops file");
    assert!(file.read_by.iter().any(|j| j == "keel-server"));
    // Decrypt state: keel's key is a recipient; alpha has reported none.
    let keel = c
        .hosts
        .iter()
        .find(|x| x.inventory_name == "keel")
        .expect("keel");
    assert!(
        keel.decrypts
            .iter()
            .any(|p| p == "secrets/keel_servers.sops.yaml")
    );
    let alpha = c
        .hosts
        .iter()
        .find(|x| x.inventory_name == "alpha")
        .expect("alpha");
    assert!(alpha.decrypts.is_empty() && alpha.age_recipient.is_none());

    // A catalog follows the primary only: a non-primary workspace gets the refusal.
    sqlx::query("update workspaces set primary_binding = null where id = $1")
        .bind(h.workspace)
        .execute(&h.db.owner)
        .await
        .expect("unset");
    let out = h
        .cmd(Cmd::GetCatalog {
            workspace: h.workspace,
        })
        .await;
    assert_eq!(repos::op_error(&out), "no_primary_repository");
    assert!(
        h.audit_actions()
            .await
            .iter()
            .all(|(a, _)| a != "catalog_build")
    );
    h.finish().await;
}

#[tokio::test]
async fn hosts_report_their_inventory_status() {
    let Some((h, _)) = primed("repo_inventory_status", None).await else {
        return;
    };
    sqlx::query("insert into hosts (id, workspace_id, inventory_name) values ($1, $2, 'newbox')")
        .bind(Uuid::now_v7())
        .bind(h.workspace)
        .execute(&h.db.owner)
        .await
        .expect("host");
    let out = h
        .cmd(Cmd::ListHosts {
            workspace: h.workspace,
            profile: None,
            drift: None,
            changed_outside: None,
        })
        .await;
    let [Event::Hosts { items }] = out.as_slice() else {
        panic!("{out:?}")
    };
    let status = |n: &str| {
        items
            .iter()
            .find(|i| i.inventory_name == n)
            .expect(n)
            .inventory_status
    };
    assert_eq!(status("keel"), HostInventoryStatus::Listed);
    assert_eq!(status("alpha"), HostInventoryStatus::Listed);
    assert_eq!(status("newbox"), HostInventoryStatus::PendingInventory);
    h.finish().await;
}

#[tokio::test]
async fn a_job_is_drafted_and_proposed_by_hand_without_a_forge() {
    let Some((h, _)) = primed("repo_job_propose", None).await else {
        return;
    };
    let out = h
        .cmd(Cmd::DraftJob {
            workspace: h.workspace,
            playbook: RepoPath::new("playbooks/cleanup.yaml").expect("path"),
        })
        .await;
    let [Event::JobDraft { yaml, .. }] = out.as_slice() else {
        panic!("{out:?}")
    };
    assert!(yaml.contains("playbook: playbooks/cleanup.yaml"), "{yaml}");

    // A name the manifest already has is refused at preview.
    let out = h
        .cmd(Cmd::PreviewJobPropose {
            workspace: h.workspace,
            playbook: RepoPath::new("playbooks/cleanup.yaml").expect("path"),
            name: ObjectName::new("debian").expect("name"),
        })
        .await;
    assert_eq!(repos::op_error(&out), "job_exists");

    let (_, out) = h
        .confirm(Cmd::PreviewJobPropose {
            workspace: h.workspace,
            playbook: RepoPath::new("playbooks/cleanup.yaml").expect("path"),
            name: ObjectName::new("cleanup").expect("name"),
        })
        .await;
    let ChangeOutcome::Manual { diff, reason } = change(&out) else {
        panic!("{out:?}")
    };
    assert!(diff.contains("+++ b/.keel/jobs.yaml"), "{diff}");
    assert!(diff.contains("+  cleanup:"), "{diff}");
    assert!(reason.contains("no forge connection"), "{reason}");
    assert!(
        h.audit_actions()
            .await
            .iter()
            .any(|(a, d)| a == "job_proposed" && d == "allowed")
    );
    h.finish().await;
}

#[tokio::test]
async fn sops_recipients_are_proposed_and_re_encryption_is_manual() {
    let Some((h, _)) = primed("repo_sops_recipient", None).await else {
        return;
    };
    let id = |n: &'static str| {
        let pool = h.db.owner.clone();
        let ws = h.workspace;
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "select id from hosts where workspace_id = $1 and inventory_name = $2",
            )
            .bind(ws)
            .bind(n)
            .fetch_one(&pool)
            .await
            .expect("id")
        }
    };
    let alpha = id("alpha").await;
    let out = h.cmd(Cmd::PreviewSopsRecipient { host: alpha }).await;
    assert_eq!(repos::op_error(&out), "no_age_recipient");

    sqlx::query("update hosts set age_recipient = $1 where id = $2")
        .bind(NEW_KEY)
        .bind(alpha)
        .execute(&h.db.owner)
        .await
        .expect("key");
    let (p, out) = h.confirm(Cmd::PreviewSopsRecipient { host: alpha }).await;
    assert!(p.summary.contains("sops updatekeys"), "{}", p.summary);
    let ChangeOutcome::Manual { diff, reason } = change(&out) else {
        panic!("{out:?}")
    };
    assert!(diff.contains(&format!("+          - {NEW_KEY}")) || diff.contains(NEW_KEY));
    assert!(reason.contains("sops updatekeys"), "{reason}");
    h.finish().await;
}

#[tokio::test]
async fn a_host_add_enrols_and_opens_a_pull_request_on_a_new_branch() {
    let forge = FakeForge::default();
    let Some((h, _)) = primed("repo_host_add", Some(forge.clone())).await else {
        return;
    };
    // Without a connection, the change is manual even with a connector.
    let (_, out) = h.confirm(host_add(&h, "newbox", &["keel_servers"])).await;
    let [
        _,
        Event::HostAdded {
            enrollment,
            change: first,
        },
    ] = out.as_slice()
    else {
        panic!("{out:?}")
    };
    assert_eq!(enrollment.inventory_name, "newbox");
    assert!(
        matches!(first, ChangeOutcome::Manual { diff, .. } if diff.contains("+        newbox:"))
    );

    // With one, a pull request on keel/add-host-<name>-<id>, never a direct write.
    let (_, out) = h
        .confirm(Cmd::PreviewCredentialCreate {
            workspace: h.workspace,
            name: ObjectName::new("forge").expect("name"),
            kind: CredentialKind::ForgeToken,
            value: SecretValue::new("forge-token-value").expect("value"),
        })
        .await;
    let [_, Event::Credentials { items }] = out.as_slice() else {
        panic!("{out:?}")
    };
    sqlx::query(
        "insert into forge_connections (workspace_id, provider, api_base, credential_id) \
         values ($1, 'forgejo', 'https://forge.example/api/v1', $2)",
    )
    .bind(h.workspace)
    .bind(items[0].id)
    .execute(&h.db.owner)
    .await
    .expect("forge");
    let (_, out) = h
        .confirm(host_add(&h, "otherbox", &["compose_hosts", "keel_servers"]))
        .await;
    let ChangeOutcome::Opened { url, branch } = change(&out) else {
        panic!("{out:?}")
    };
    assert!(branch.starts_with("keel/add-host-otherbox-"), "{branch}");
    assert!(url.ends_with("/pulls/7"));
    let opened = forge.opened.lock().unwrap().clone();
    let [req] = opened.as_slice() else {
        panic!("{opened:?}")
    };
    assert_eq!(req.base_branch, "main");
    assert_eq!(req.new_branch, branch);
    let [file] = req.files.as_slice() else {
        panic!("{:?}", req.files)
    };
    assert_eq!(file.path, "inventory/hosts.yml");
    assert!(file.expected_blob_sha.is_some());
    let text = String::from_utf8(file.new_content.clone()).expect("utf8");
    assert_eq!(text.matches("otherbox:").count(), 2, "{text}");

    // A host the inventory already lists is refused at preview.
    let out = h.cmd(host_add(&h, "steam", &["keel_servers"])).await;
    assert_eq!(repos::op_error(&out), "host_exists");
    let audit = h.audit_text().await;
    assert!(audit.contains("host_add_proposed"));
    assert!(!audit.contains("forge-token-value"));
    h.finish().await;
}

/// SC-004: no sops ciphertext in any reply, the stored catalog or the audit log.
#[tokio::test]
async fn no_sops_value_leaves_the_repository() {
    let Some((h, binding)) = primed("repo_sc004", None).await else {
        return;
    };
    let mut replies = Vec::new();
    replies.push(
        h.cmd(Cmd::GetCatalog {
            workspace: h.workspace,
        })
        .await,
    );
    for playbook in ["playbooks/keel-server.yaml", "playbooks/k3s-master.yaml"] {
        replies.push(
            h.cmd(Cmd::DraftJob {
                workspace: h.workspace,
                playbook: RepoPath::new(playbook).expect("path"),
            })
            .await,
        );
    }
    let (_, out) = h.confirm(host_add(&h, "newbox", &["keel_servers"])).await;
    replies.push(out);
    let (_, out) = h
        .confirm(Cmd::PreviewJobPropose {
            workspace: h.workspace,
            playbook: RepoPath::new("playbooks/cleanup.yaml").expect("path"),
            name: ObjectName::new("cleanup").expect("name"),
        })
        .await;
    replies.push(out);
    let replies = serde_json::to_string(&replies).expect("json");
    let (row,): (String,) =
        sqlx::query_as("select catalog::text from binding_catalogs where binding_id = $1")
            .bind(binding)
            .fetch_one(&h.db.owner)
            .await
            .expect("row");
    let audit = h.audit_text().await;
    for secret in ciphertexts() {
        assert!(!replies.contains(&secret), "a reply holds {secret}");
        assert!(!row.contains(&secret), "the catalog row holds {secret}");
        assert!(!audit.contains(&secret), "the audit log holds {secret}");
    }
    h.finish().await;
}
