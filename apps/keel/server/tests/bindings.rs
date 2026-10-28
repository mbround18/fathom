//! Binding and credential commands (spec 055 T042, T047, T048, T051) against local bare
//! repositories. Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod repos;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use keel_protocol::{
    ActionName, ActiveStatus, BindingUpdate, BranchName, Cmd, CmdEnvelope, CredentialKind, Event,
    EventEnvelope, Label, ObjectName, RequestId, SecretValue, TokenActions,
};
use repos::{Harness, Source, bind, manifest, op_error, preview, sign_in};

fn name(s: &str) -> ObjectName {
    ObjectName::new(s).expect("name")
}

async fn binding(
    h: &Harness,
    id: uuid::Uuid,
) -> (keel_protocol::BindingInfo, Vec<keel_protocol::JobSummary>) {
    match h.cmd(Cmd::GetBinding { binding: id }).await.as_slice() {
        [Event::Binding { binding, jobs }] => (binding.clone(), jobs.clone()),
        other => panic!("get binding: {other:?}"),
    }
}

#[tokio::test]
async fn create_test_fetches_then_syncs_jobs_from_the_manifest() {
    let Some(h) = Harness::new("create_test_fetches").await else {
        return;
    };
    let src = Source::new(&manifest(&[]));
    let head = repos::git(&src.work, &["rev-parse", "HEAD"]);

    let p = preview(
        &h.cmd(Cmd::PreviewBindingCreate(repos::spec(&h, &src)))
            .await,
    );
    assert!(p.summary.contains(&head[..12]), "{}", p.summary);
    assert!(!p.step_up);
    // Nothing is stored before Confirm.
    let (n,): (i64,) = sqlx::query_as("select count(*) from repo_bindings")
        .fetch_one(&h.db.owner)
        .await
        .expect("count");
    assert_eq!(n, 0);

    let id = bind(&h, &src).await;
    let (info, jobs) = binding(&h, id).await;
    assert_eq!(info.last_commit.as_deref(), Some(head.as_str()));
    assert_eq!(info.status, ActiveStatus::Active);
    assert!(info.last_sync_error.is_none(), "{:?}", info.last_sync_error);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].name, "web");
    assert_eq!(jobs[0].playbook, "playbooks/web.yaml");
    assert!(jobs[0].blocked.is_none());

    // The same URL and branch twice in one project is refused.
    let dup = h
        .cmd(Cmd::PreviewBindingCreate(repos::spec(&h, &src)))
        .await;
    assert_eq!(op_error(&dup), "binding_exists");

    let actions = h.audit_actions().await;
    assert!(
        actions.iter().any(|(a, _)| a == "binding_created"),
        "{actions:?}"
    );
    assert!(
        actions.iter().any(|(a, _)| a == "binding_synced"),
        "{actions:?}"
    );
    h.finish().await;
}

#[tokio::test]
async fn an_invalid_manifest_keeps_the_previous_jobs() {
    let Some(h) = Harness::new("invalid_manifest_keeps_jobs").await else {
        return;
    };
    let src = Source::new(&manifest(&[]));
    let id = bind(&h, &src).await;

    let bad = src.commit(
        ".keel/jobs.yaml",
        "version: 1\njobs:\n  web:\n    playbook: ../x\n",
        "bad",
    );
    src.push();
    let out = h.cmd(Cmd::SyncBinding { binding: id }).await;
    assert!(
        matches!(out.as_slice(), [Event::BindingSynced { commit, .. }] if *commit == bad),
        "{out:?}"
    );
    let (info, jobs) = binding(&h, id).await;
    assert_eq!(info.last_commit.as_deref(), Some(bad.as_str()));
    assert_eq!(info.last_sync_error.as_deref(), Some("manifest_invalid"));
    assert!(!info.manifest_problems.is_empty());
    assert_eq!(jobs.len(), 1, "the previous jobs stay");

    // A fixed manifest clears the error.
    src.commit(".keel/jobs.yaml", &manifest(&[]), "fixed");
    src.push();
    h.cmd(Cmd::SyncBinding { binding: id }).await;
    let (info, _) = binding(&h, id).await;
    assert!(info.last_sync_error.is_none());
    assert!(info.manifest_problems.is_empty());
    h.finish().await;
}

#[tokio::test]
async fn disable_update_and_remove() {
    let Some(h) = Harness::new("disable_update_remove").await else {
        return;
    };
    let src = Source::new(&manifest(&[]));
    let id = bind(&h, &src).await;

    h.confirm(Cmd::PreviewBindingDisable { binding: id }).await;
    assert_eq!(binding(&h, id).await.0.status, ActiveStatus::Disabled);
    let out = h.cmd(Cmd::SyncBinding { binding: id }).await;
    assert_eq!(op_error(&out), "binding_disabled");
    h.confirm(Cmd::PreviewBindingEnable { binding: id }).await;

    h.confirm(Cmd::PreviewBindingUpdate(BindingUpdate {
        binding: id,
        branch: BranchName::new("main").expect("branch"),
        credential: None,
        known_host: None,
        poll_interval: Some(300),
        allow_force_push: true,
        override_quorum: 2,
    }))
    .await;
    let (info, _) = binding(&h, id).await;
    assert_eq!(info.poll_interval, Some(300));
    assert!(info.allow_force_push);
    assert_eq!(info.override_quorum, 2);

    // Signed mode refuses a job on a managed inventory (spec 055 US8).
    let out = h
        .cmd(Cmd::PreviewSignedMode {
            binding: id,
            on: true,
        })
        .await;
    assert_eq!(op_error(&out), "managed_inventory");
    src.commit(
        "inventories/prod/hosts.yaml",
        "all:\n  hosts:\n    web-1: {}\n",
        "inventory",
    );
    src.commit(
        ".keel/jobs.yaml",
        "version: 1\nprofiles: { debian: {} }\ninventories:\n  prod:\n    \
         path: inventories/prod/hosts.yaml\njobs:\n  web:\n    \
         playbook: playbooks/web.yaml\n    profiles: [debian]\n    inventory: prod\n",
        "repo inventory",
    );
    src.push();
    h.cmd(Cmd::SyncBinding { binding: id }).await;

    // Turning signed mode off needs step-up; on does not.
    let (on, _) = h
        .confirm(Cmd::PreviewSignedMode {
            binding: id,
            on: true,
        })
        .await;
    assert!(!on.step_up);
    assert!(binding(&h, id).await.0.signed_mode);
    let off = preview(
        &h.cmd(Cmd::PreviewSignedMode {
            binding: id,
            on: false,
        })
        .await,
    );
    assert!(off.step_up);

    h.confirm(Cmd::PreviewBindingRemove { binding: id }).await;
    let gone = h.cmd(Cmd::GetBinding { binding: id }).await;
    assert_eq!(op_error(&gone), "not_found");
    h.finish().await;
}

#[tokio::test]
async fn viewers_may_read_but_not_change_bindings() {
    let Some(h) = Harness::new("viewers_read_only").await else {
        return;
    };
    let src = Source::new(&manifest(&[]));
    let id = bind(&h, &src).await;
    let other = sign_in(&h.app, "2", "stranger").await;
    // No grant at all: even reading is denied.
    let out = h.cmd_as(&other, Cmd::GetBinding { binding: id }).await;
    assert!(matches!(out.as_slice(), [Event::Denied { .. }]), "{out:?}");
    let out = h
        .cmd_as(&other, Cmd::PreviewBindingRemove { binding: id })
        .await;
    assert!(matches!(out.as_slice(), [Event::Denied { .. }]), "{out:?}");
    assert!(
        h.audit_actions()
            .await
            .iter()
            .any(|(a, d)| a == "preview_binding_remove" && d == "denied")
    );
    h.finish().await;
}

#[tokio::test]
async fn credentials_are_sealed_never_returned_and_unblock_jobs_once_granted() {
    let Some(h) = Harness::new("credentials_sealed").await else {
        return;
    };
    let src = Source::new(&manifest(&["vault"]));
    let id = bind(&h, &src).await;
    let (_, jobs) = binding(&h, id).await;
    assert_eq!(jobs[0].blocked.as_deref(), Some("credential_unavailable"));

    const VALUE: &str = "correct-horse-battery-staple";
    let (p, out) = h
        .confirm(Cmd::PreviewCredentialCreate {
            workspace: h.workspace,
            name: name("vault"),
            kind: CredentialKind::VaultPassword,
            value: SecretValue::new(VALUE).expect("value"),
        })
        .await;
    assert!(p.step_up, "creating a credential needs step-up");
    let cred = match out.as_slice() {
        [_, Event::Credentials { items }] => items[0].id,
        other => panic!("{other:?}"),
    };
    let listed = serde_json::to_string(
        &h.cmd(Cmd::ListCredentials {
            workspace: h.workspace,
        })
        .await,
    )
    .expect("json");
    assert!(!listed.contains(VALUE));
    assert!(listed.contains("sha256:"));
    let (sealed,): (Vec<u8>,) = sqlx::query_as("select sealed from credentials where id = $1")
        .bind(cred)
        .fetch_one(&h.db.owner)
        .await
        .expect("sealed");
    assert!(!sealed.windows(VALUE.len()).any(|w| w == VALUE.as_bytes()));

    let (p, _) = h
        .confirm(Cmd::PreviewCredentialGrant {
            credential: cred,
            binding: id,
            profiles: vec![],
        })
        .await;
    assert!(p.step_up, "granting needs step-up");
    h.cmd(Cmd::SyncBinding { binding: id }).await;
    let (_, jobs) = binding(&h, id).await;
    assert!(jobs[0].blocked.is_none(), "{:?}", jobs[0].blocked);

    // Rotating keeps the name and changes the fingerprint.
    let before = listed.clone();
    h.confirm(Cmd::PreviewCredentialRotate {
        credential: cred,
        value: SecretValue::new("another-value").expect("value"),
    })
    .await;
    let after = serde_json::to_string(
        &h.cmd(Cmd::ListCredentials {
            workspace: h.workspace,
        })
        .await,
    )
    .expect("json");
    assert_ne!(before, after);

    h.confirm(Cmd::PreviewCredentialRevoke {
        credential: cred,
        binding: id,
    })
    .await;
    h.cmd(Cmd::SyncBinding { binding: id }).await;
    let (_, jobs) = binding(&h, id).await;
    assert_eq!(jobs[0].blocked.as_deref(), Some("credential_unavailable"));

    // Bad values are refused by kind, without echoing them.
    let out = h
        .cmd(Cmd::PreviewCredentialCreate {
            workspace: h.workspace,
            name: name("key"),
            kind: CredentialKind::GitSshKey,
            value: SecretValue::new("not-a-key-xyzzy").expect("value"),
        })
        .await;
    assert_eq!(op_error(&out), "credential_invalid");
    assert!(!serde_json::to_string(&out).expect("json").contains("xyzzy"));

    assert!(!h.audit_text().await.contains(VALUE));
    h.finish().await;
}

#[tokio::test]
async fn a_credential_a_binding_fetches_with_cannot_be_removed() {
    let Some(h) = Harness::new("credential_in_use").await else {
        return;
    };
    let src = Source::new(&manifest(&[]));
    let (_, out) = h
        .confirm(Cmd::PreviewCredentialCreate {
            workspace: h.workspace,
            name: name("git"),
            kind: CredentialKind::GitHttpsToken,
            value: SecretValue::new("user:tok").expect("value"),
        })
        .await;
    let cred = match out.as_slice() {
        [_, Event::Credentials { items }] => items[0].id,
        other => panic!("{other:?}"),
    };
    let mut spec = repos::spec(&h, &src);
    spec.credential = Some(cred);
    let (_, out) = h.confirm(Cmd::PreviewBindingCreate(spec)).await;
    assert!(
        matches!(out.as_slice(), [_, Event::Binding { .. }]),
        "{out:?}"
    );

    let out = h
        .cmd(Cmd::PreviewCredentialRemove { credential: cred })
        .await;
    assert_eq!(op_error(&out), "credential_in_use");
    h.finish().await;
}

#[tokio::test]
async fn a_token_may_list_bindings_but_never_sync_them() {
    let Some(h) = Harness::new("token_lists_bindings").await else {
        return;
    };
    let src = Source::new(&manifest(&[]));
    let id = bind(&h, &src).await;
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
    // Syncing writes, so a token may not name it.
    let out = h.cmd(token(&["list_bindings", "sync_binding"])).await;
    assert_eq!(op_error(&out), "action_not_allowed");

    let (_, out) = h.confirm(token(&["list_bindings"])).await;
    let secret = match out.as_slice() {
        [_, Event::ApiTokenCreated { secret, .. }, ..] => secret.0.clone(),
        other => panic!("token: {other:?}"),
    };
    let body = serde_json::to_vec(&CmdEnvelope {
        id: RequestId(1),
        cmd: Cmd::ListBindings {
            workspace: h.workspace,
        },
    })
    .expect("json");
    let req = Request::post("/api/v1/cmd")
        .header(header::AUTHORIZATION, format!("Bearer {secret}"))
        .body(Body::from(body))
        .expect("req");
    let (status, body) = h.call(req).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let envs: Vec<EventEnvelope> = serde_json::from_slice(&body).expect("events");
    assert!(
        matches!(envs.as_slice(), [e] if matches!(&e.event, Event::Bindings { items } if items.len() == 1 && items[0].id == id)),
        "{envs:?}"
    );
    h.finish().await;
}
