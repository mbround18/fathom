//! Inventory commands and repository inventory sync (spec 055 T054, US4). Needs the test
//! cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod repos;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use keel_protocol::{
    ActionName, Cmd, CmdEnvelope, Event, EventEnvelope, InventoryInfo, InventoryRevisionInfo,
    InventoryRevisionSummary, InventorySource, InventoryStatus, InventoryText, Label, ObjectName,
    RequestId, TokenActions,
};
use keel_server::inventory::{PinRefusal, pin_latest};
use repos::{Harness, Source, bind, op_error, preview};
use uuid::Uuid;

const V1: &str = "web:\n  hosts:\n    w1:\n      http_port: 80\n";
const V2: &str = "web:\n  hosts:\n    w1:\n      http_port: 8080\n    w2:\n";

fn text(s: &str) -> InventoryText {
    InventoryText::new(s).expect("text")
}

type Revision = (
    InventoryInfo,
    InventoryRevisionInfo,
    Vec<InventoryRevisionSummary>,
);

fn revision(events: &[Event]) -> Revision {
    match events {
        [
            Event::InventoryRevision {
                inventory,
                revision,
                history,
            },
        ]
        | [
            Event::Confirmed { .. },
            Event::InventoryRevision {
                inventory,
                revision,
                history,
            },
        ] => (inventory.clone(), revision.clone(), history.clone()),
        other => panic!("expected a revision, got {other:?}"),
    }
}

/// The op error a confirm answered with.
fn confirm_error(events: &[Event]) -> String {
    match events {
        [Event::OpError { code, .. }] | [Event::Confirmed { .. }, Event::OpError { code, .. }] => {
            code.clone()
        }
        other => panic!("expected an op error, got {other:?}"),
    }
}

/// Creates the managed inventory `prod` with `content` and returns its id.
async fn create(h: &Harness, content: &str, var_schema: Option<&str>) -> Uuid {
    let (_, out) = h
        .confirm(Cmd::PreviewInventoryCreate {
            workspace: h.workspace,
            name: ObjectName::new("prod").expect("name"),
            content: text(content),
            var_schema: var_schema.map(text),
        })
        .await;
    let (inv, rev, _) = revision(&out);
    assert_eq!(inv.source, InventorySource::Managed);
    assert_eq!(rev.summary.seq, 1);
    inv.id
}

fn edit(inventory: Uuid, base_seq: u32, content: &str) -> Cmd {
    Cmd::PreviewInventoryEdit {
        inventory,
        base_seq,
        content: text(content),
    }
}

async fn host_rows(h: &Harness, inventory: Uuid, seq: i32) -> Vec<(String, Vec<String>)> {
    sqlx::query_as(
        "select h.host, h.groups from inventory_hosts h \
         join inventory_revisions r on r.id = h.revision_id \
         where r.inventory_id = $1 and r.seq = $2 order by h.host",
    )
    .bind(inventory)
    .bind(seq)
    .fetch_all(&h.db.owner)
    .await
    .expect("hosts")
}

#[tokio::test]
async fn an_edit_makes_a_revision_only_when_the_content_changes() {
    let Some(h) = Harness::new("inv_revision_on_change").await else {
        return;
    };
    let id = create(&h, V1, None).await;
    assert_eq!(
        host_rows(&h, id, 1).await,
        [("w1".to_owned(), vec!["web".to_owned()])]
    );

    // The same content, and the same content with only a comment added, change nothing.
    assert_eq!(op_error(&h.cmd(edit(id, 1, V1)).await), "no_change");
    let commented = format!("# primary web tier\n{V1}");
    assert_eq!(op_error(&h.cmd(edit(id, 1, &commented)).await), "no_change");

    let (_, out) = h.confirm(edit(id, 1, V2)).await;
    let (inv, rev, history) = revision(&out);
    assert_eq!((inv.head_seq, inv.host_count), (Some(2), 2));
    assert_eq!(rev.summary.seq, 2);
    assert_eq!(rev.summary.author.as_deref(), Some("owner"));
    assert_eq!(rev.source.as_deref(), Some(V2));
    assert_eq!(
        history.iter().map(|s| s.seq).collect::<Vec<_>>(),
        [2, 1],
        "newest first"
    );
    assert_eq!(host_rows(&h, id, 2).await.len(), 2);

    // An older revision is still readable as it was.
    let (_, old, _) = revision(
        &h.cmd(Cmd::GetInventoryRevision {
            inventory: id,
            seq: Some(1),
        })
        .await,
    );
    assert_eq!(old.source.as_deref(), Some(V1));
    assert_eq!(old.hosts.len(), 1);

    let actions = h.audit_actions().await;
    for want in ["inventory_created", "inventory_revised"] {
        assert!(
            actions.iter().any(|(a, d)| a == want && d == "allowed"),
            "{want}: {actions:?}"
        );
    }
    h.finish().await;
}

#[tokio::test]
async fn an_edit_from_a_stale_base_is_refused() {
    let Some(h) = Harness::new("inv_stale_base").await else {
        return;
    };
    let id = create(&h, V1, None).await;
    // Two editors start from revision 1; the second to confirm loses.
    let first = preview(&h.cmd(edit(id, 1, V2)).await);
    h.confirm(edit(id, 1, "web:\n  hosts:\n    w9:\n")).await;
    let out = h
        .cmd(Cmd::Confirm {
            preview_id: first.preview_id,
            step_up: None,
        })
        .await;
    assert_eq!(confirm_error(&out), "stale_base");
    // A preview from the old base is refused outright.
    assert_eq!(op_error(&h.cmd(edit(id, 1, V2)).await), "stale_base");
    assert_eq!(op_error(&h.cmd(edit(id, 7, V2)).await), "stale_base");
    h.finish().await;
}

#[tokio::test]
async fn revisions_cannot_be_changed_or_removed() {
    let Some(h) = Harness::new("inv_immutable").await else {
        return;
    };
    let id = create(&h, V1, None).await;
    for sql in [
        "update inventory_revisions set status = 'invalid' where inventory_id = $1",
        "delete from inventory_revisions where inventory_id = $1",
        "update inventory_hosts set groups = '{}' where revision_id in \
         (select id from inventory_revisions where inventory_id = $1)",
        "delete from inventory_hosts where revision_id in \
         (select id from inventory_revisions where inventory_id = $1)",
    ] {
        let err = sqlx::query(sql)
            .bind(id)
            .execute(&h.db.app)
            .await
            .expect_err(sql);
        let code = err
            .as_database_error()
            .and_then(|d| d.code())
            .map(|c| c.into_owned());
        assert_eq!(code.as_deref(), Some("42501"), "{sql}: {err}");
    }
    h.finish().await;
}

#[tokio::test]
async fn a_deprecated_inventory_accepts_no_new_runs() {
    let Some(h) = Harness::new("inv_deprecated").await else {
        return;
    };
    let id = create(&h, V1, None).await;
    let mut conn = h.db.app.acquire().await.expect("conn");
    let pinned = pin_latest(&mut conn, h.workspace, "prod")
        .await
        .expect("pin")
        .expect("pinned");
    assert_eq!(pinned.hosts.len(), 1);

    let (_, out) = h
        .confirm(Cmd::PreviewInventoryDeprecate { inventory: id })
        .await;
    match out.as_slice() {
        [Event::Confirmed { .. }, Event::Inventories { items }] => {
            assert_eq!(items[0].status, InventoryStatus::Deprecated);
        }
        other => panic!("deprecate: {other:?}"),
    }
    assert_eq!(
        pin_latest(&mut conn, h.workspace, "prod")
            .await
            .expect("pin"),
        Err(PinRefusal::Deprecated)
    );
    drop(conn);
    assert_eq!(
        op_error(
            &h.cmd(Cmd::PreviewInventoryDeprecate { inventory: id })
                .await
        ),
        "no_change"
    );
    assert!(
        h.audit_actions()
            .await
            .iter()
            .any(|(a, _)| a == "inventory_deprecated")
    );
    h.finish().await;
}

#[tokio::test]
async fn restore_makes_a_new_revision_equal_to_an_old_one() {
    let Some(h) = Harness::new("inv_restore").await else {
        return;
    };
    let id = create(&h, V1, None).await;
    h.confirm(edit(id, 1, V2)).await;

    let out = h
        .cmd(Cmd::DiffInventory {
            inventory: id,
            from: 1,
            to: 2,
        })
        .await;
    let [Event::InventoryDiff { changes, .. }] = out.as_slice() else {
        panic!("diff: {out:?}");
    };
    let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
    assert!(paths.contains(&"hosts.w1.vars.http_port"), "{paths:?}");
    assert!(paths.contains(&"hosts.w2"), "{paths:?}");
    let port = changes
        .iter()
        .find(|c| c.path == "hosts.w1.vars.http_port")
        .expect("port");
    assert_eq!(
        (port.before.as_deref(), port.after.as_deref()),
        (Some("80"), Some("8080"))
    );

    let (_, out) = h
        .confirm(Cmd::PreviewInventoryRestore {
            inventory: id,
            seq: 1,
        })
        .await;
    let (_, rev, history) = revision(&out);
    assert_eq!(rev.summary.seq, 3);
    assert_eq!(rev.summary.restored_from, Some(1));
    assert_eq!(rev.source.as_deref(), Some(V1));
    assert_eq!(rev.summary.content_hash, history[2].content_hash);
    let out = h
        .cmd(Cmd::DiffInventory {
            inventory: id,
            from: 1,
            to: 3,
        })
        .await;
    assert!(
        matches!(out.as_slice(), [Event::InventoryDiff { changes, .. }] if changes.is_empty()),
        "{out:?}"
    );
    // Restoring what is already the newest content changes nothing.
    assert_eq!(
        op_error(
            &h.cmd(Cmd::PreviewInventoryRestore {
                inventory: id,
                seq: 1
            })
            .await
        ),
        "no_change"
    );
    assert!(
        h.audit_actions()
            .await
            .iter()
            .any(|(a, _)| a == "inventory_restored")
    );
    h.finish().await;
}

#[tokio::test]
async fn invalid_content_is_reported_and_never_saved() {
    let Some(h) = Harness::new("inv_invalid").await else {
        return;
    };
    let schema = r#"{"type":"object","properties":{"http_port":{"type":"integer"}}}"#;
    let id = create(&h, V1, Some(schema)).await;

    let bad = "web:\n  hosts:\n    w1:\n      http_port: eighty\n";
    let out = h
        .cmd(Cmd::ValidateInventory {
            workspace: h.workspace,
            content: text(bad),
            inventory: Some(id),
        })
        .await;
    let [Event::InventoryValidation { errors, hosts, .. }] = out.as_slice() else {
        panic!("validate: {out:?}");
    };
    assert_eq!(hosts.len(), 1);
    assert!(
        errors.iter().any(|e| e.at.contains("http_port")),
        "{errors:?}"
    );
    assert!(
        errors.iter().all(|e| !e.message.contains("eighty")),
        "values are never quoted"
    );
    assert_eq!(
        op_error(&h.cmd(edit(id, 1, bad)).await),
        "inventory_invalid"
    );
    assert_eq!(
        op_error(&h.cmd(edit(id, 1, "web: [unclosed\n")).await),
        "inventory_invalid"
    );

    // Without an inventory, only the shape is checked.
    let out = h
        .cmd(Cmd::ValidateInventory {
            workspace: h.workspace,
            content: text(bad),
            inventory: None,
        })
        .await;
    assert!(
        matches!(out.as_slice(), [Event::InventoryValidation { errors, .. }] if errors.is_empty()),
        "{out:?}"
    );
    h.finish().await;
}

async fn bearer(h: &Harness, secret: &str, cmd: Cmd) -> Vec<Event> {
    let body = serde_json::to_vec(&CmdEnvelope {
        id: RequestId(1),
        cmd,
    })
    .expect("json");
    let req = Request::post("/api/v1/cmd")
        .header(header::AUTHORIZATION, format!("Bearer {secret}"))
        .body(Body::from(body))
        .expect("req");
    let (status, body) = h.call(req).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let envs: Vec<EventEnvelope> = serde_json::from_slice(&body).expect("events");
    envs.into_iter().map(|e| e.event).collect()
}

#[tokio::test]
async fn a_token_may_list_inventories_but_never_edit_them() {
    let Some(h) = Harness::new("inv_token").await else {
        return;
    };
    let id = create(&h, V1, None).await;
    let token = |actions: &[&str]| Cmd::PreviewApiTokenCreate {
        name: Label::new("ci").expect("label"),
        workspace: h.workspace,
        actions: TokenActions(
            actions
                .iter()
                .map(|a| ActionName::new(*a).expect("action"))
                .collect(),
        ),
        expires_in_days: 7,
    };
    let out = h
        .cmd(token(&["list_inventories", "preview_inventory_edit"]))
        .await;
    assert_eq!(op_error(&out), "action_not_allowed");

    let (_, out) = h
        .confirm(token(&["list_inventories", "get_inventory_revision"]))
        .await;
    let secret = match out.as_slice() {
        [_, Event::ApiTokenCreated { secret, .. }, ..] => secret.0.clone(),
        other => panic!("token: {other:?}"),
    };
    let out = bearer(
        &h,
        &secret,
        Cmd::ListInventories {
            workspace: h.workspace,
        },
    )
    .await;
    assert!(
        matches!(out.as_slice(), [Event::Inventories { items }] if items.len() == 1 && items[0].id == id),
        "{out:?}"
    );
    let out = bearer(&h, &secret, edit(id, 1, V2)).await;
    assert!(
        matches!(out.as_slice(), [Event::Denied { reason, .. }] if reason == "token_scope"),
        "{out:?}"
    );
    h.finish().await;
}

const REPO_MANIFEST: &str = "version: 1\nprofiles: { debian: {} }\ninventories:\n  prod:\n    \
                             path: inventories/prod/hosts.yaml\njobs:\n  web:\n    \
                             playbook: playbooks/web.yaml\n    profiles: [debian]\n    \
                             inventory: prod\n";

async fn inventories(h: &Harness) -> Vec<InventoryInfo> {
    match h
        .cmd(Cmd::ListInventories {
            workspace: h.workspace,
        })
        .await
        .as_slice()
    {
        [Event::Inventories { items }] => items.clone(),
        other => panic!("list: {other:?}"),
    }
}

#[tokio::test]
async fn sync_adds_a_repo_revision_only_when_the_content_hash_changes() {
    let Some(h) = Harness::new("inv_repo_sync").await else {
        return;
    };
    let src = Source::new(REPO_MANIFEST);
    let first = src.commit("inventories/prod/hosts.yaml", V1, "inventory");
    src.push();
    let binding = bind(&h, &src).await;

    let items = inventories(&h).await;
    assert_eq!(items.len(), 1, "{items:?}");
    let inv = &items[0];
    assert_eq!(
        (inv.source, inv.binding, inv.path.as_deref()),
        (
            InventorySource::Repo,
            Some(binding),
            Some("inventories/prod/hosts.yaml")
        )
    );
    assert_eq!((inv.head_seq, inv.host_count), (Some(1), 1));
    let id = inv.id;

    // A commit that leaves the inventory alone adds no revision.
    src.commit("README.md", "hello\n", "readme");
    src.push();
    let out = h.cmd(Cmd::SyncBinding { binding }).await;
    assert!(
        matches!(
            out.as_slice(),
            [Event::BindingSynced {
                inventories_changed: false,
                ..
            }]
        ),
        "{out:?}"
    );
    assert_eq!(inventories(&h).await[0].head_seq, Some(1));

    // group_vars beside the inventory are part of its content.
    let second = src.commit(
        "inventories/prod/group_vars/web.yml",
        "tier: front\n",
        "vars",
    );
    src.push();
    let out = h.cmd(Cmd::SyncBinding { binding }).await;
    assert!(
        matches!(
            out.as_slice(),
            [Event::BindingSynced {
                inventories_changed: true,
                ..
            }]
        ),
        "{out:?}"
    );
    let (inv, rev, history) = revision(
        &h.cmd(Cmd::GetInventoryRevision {
            inventory: id,
            seq: None,
        })
        .await,
    );
    assert_eq!(inv.head_seq, Some(2));
    assert_eq!(rev.summary.source_commit.as_deref(), Some(second.as_str()));
    assert_eq!(history[1].source_commit.as_deref(), Some(first.as_str()));
    assert!(rev.content.contains("front"));
    assert_eq!(host_rows(&h, id, 2).await.len(), 1);

    // Repository inventories change in the repository, not in Keel.
    assert_eq!(
        op_error(&h.cmd(edit(id, 2, V2)).await),
        "inventory_from_repo"
    );
    assert!(
        h.audit_actions()
            .await
            .iter()
            .filter(|(a, _)| a == "inventory_synced")
            .count()
            == 2
    );
    h.finish().await;
}
