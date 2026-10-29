//! Keel updating itself through a run (spec 061 US4, T065).
//!
//! - Run params are checked against the job's declared ones.
//! - `PreviewKeelUpdate` refusals, each audited as `keel_update_refused` with its reason.
//! - Confirming queues a manual run of the System workspace's `keel-server` job with the digest
//!   param and audits `keel_update_requested`; a second request opens that run instead
//!   (`update_in_progress`, FR-018); a status that moved after the preview gives `digest_stale`.
//! - The run finishing audits `keel_update_result`, and the owner's status shows it as
//!   `last_update_run`.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it. Nothing calls a registry.

mod common;
mod repos;
mod update_fixture;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, header};
use keel_exec::fake::{FakeRunner, Script};
use keel_protocol::{
    ActionName, Cmd, CmdEnvelope, Event, EventEnvelope, Label, RequestId, RunTrigger, TokenActions,
    UpdateOutcome, UpdatePath,
};
use repos::{Harness, preview, sign_in};
use update_fixture::{
    NEW, OTHER, RUNNING, Setup, audited, build, offer, refusals, running, system,
};
use uuid::Uuid;

/// The reason or code of a refused command.
fn refused(events: &[Event]) -> String {
    match events {
        [Event::Denied { reason, .. }] => reason.clone(),
        [Event::OpError { code, .. }] => code.clone(),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn update(digest: &str) -> Cmd {
    Cmd::PreviewKeelUpdate {
        digest: digest.to_owned(),
    }
}

/// Previews and confirms an update to `digest`; returns the run it opened.
async fn update_to(h: &Harness, digest: &str) -> keel_protocol::RunInfo {
    let (_, out) = h.confirm(update(digest)).await;
    match out.as_slice() {
        [Event::Confirmed { .. }, Event::Run { run, .. }] => run.clone(),
        other => panic!("update: {other:?}"),
    }
}

#[tokio::test]
async fn run_params_must_be_declared_and_match_their_kind() {
    let Some(mut h) = Harness::new("upd_params").await else {
        return;
    };
    running(&mut h, build(), false);
    system(&h, Setup::FULL).await;
    let def: serde_json::Value =
        sqlx::query_scalar("select definition from jobs where name = 'keel-server'")
            .fetch_one(&h.db.owner)
            .await
            .expect("job");
    let def: keel_core::manifest::JobDefinition = serde_json::from_value(def).expect("definition");
    let check = |k: &str, v: &str| {
        keel_server::runs::check_params(&def, &BTreeMap::from([(k.to_owned(), v.to_owned())]))
    };
    assert!(check("keel_server_image_digest", NEW).is_ok());
    assert!(check("keel_server_image_digest", "latest").is_err());
    assert!(check("other", NEW).is_err());
    assert!(keel_server::runs::check_params(&def, &BTreeMap::new()).is_ok());
    h.finish().await;
}

#[tokio::test]
async fn every_refusal_is_audited_with_its_reason() {
    let Some(mut h) = Harness::new("upd_refusals").await else {
        return;
    };
    running(&mut h, build(), false);
    offer(&h, UpdateOutcome::UpdateAvailable, NEW);

    // No System workspace yet.
    assert_eq!(refused(&h.cmd(update(NEW)).await), "no_system_workspace");

    // An admin.
    let admin = sign_in(&h.app, "2", "admin").await;
    sqlx::query("update users set instance_role = 'admin' where id = $1")
        .bind(admin.user.id)
        .execute(&h.db.owner)
        .await
        .expect("admin");
    h.step_up(&admin).await;
    assert_eq!(refused(&h.cmd_as(&admin, update(NEW)).await), "owner_only");

    // An API token, whatever it was scoped to.
    let (_, out) = h
        .confirm(Cmd::PreviewApiTokenCreate {
            name: Label::new("ci").expect("label"),
            workspace: h.workspace,
            actions: TokenActions(vec![ActionName::new("list_runs").expect("action")]),
            expires_in_days: 7,
        })
        .await;
    let secret = match out.as_slice() {
        [_, Event::ApiTokenCreated { secret, .. }, ..] => secret.0.clone(),
        other => panic!("token: {other:?}"),
    };
    let body = serde_json::to_vec(&CmdEnvelope {
        id: RequestId(1),
        cmd: update(NEW),
    })
    .expect("json");
    let (_, body) = h
        .call(
            Request::post("/api/v1/cmd")
                .header(header::AUTHORIZATION, format!("Bearer {secret}"))
                .body(Body::from(body))
                .expect("req"),
        )
        .await;
    let envs: Vec<EventEnvelope> = serde_json::from_slice(&body).expect("events");
    let events: Vec<Event> = envs.into_iter().map(|e| e.event).collect();
    assert_eq!(refused(&events), "session_required");

    // A System workspace without a keel-server host.
    let sys = system(
        &h,
        Setup {
            host: false,
            ..Setup::FULL
        },
    )
    .await;
    assert_eq!(refused(&h.cmd(update(NEW)).await), "no_keel_host");

    // The host, but keel-server no longer allowed.
    let host = Uuid::now_v7();
    sqlx::query(
        "insert into hosts (id, workspace_id, inventory_name, allowed_profiles) \
         values ($1, $2, 'keel-1', '{keel-server}')",
    )
    .bind(host)
    .bind(sys.workspace)
    .execute(&h.db.owner)
    .await
    .expect("host");
    sqlx::query(
        "insert into host_profiles (host_id, profile_id) \
         select $1, id from profiles where workspace_id = $2 and name = 'keel-server'",
    )
    .bind(host)
    .bind(sys.workspace)
    .execute(&h.db.owner)
    .await
    .expect("host profile");
    sqlx::query("update system_config set allowed_profiles = '{debian}'")
        .execute(&h.db.owner)
        .await
        .expect("allowed");
    assert_eq!(refused(&h.cmd(update(NEW)).await), "profile_not_allowed");
    sqlx::query("update system_config set allowed_profiles = '{keel-server}'")
        .execute(&h.db.owner)
        .await
        .expect("allowed");

    // A digest that isn't the one on offer.
    assert_eq!(refused(&h.cmd(update(OTHER)).await), "digest_stale");

    assert_eq!(
        refusals(&h).await,
        [
            "no_system_workspace",
            "owner_only",
            "token_caller",
            "no_keel_host",
            "profile_not_allowed",
            "digest_stale",
        ]
    );
    let rows = audited(&h, "keel_update_refused").await;
    assert!(
        rows.iter()
            .all(|r| r["context"]["old_digest"] == RUNNING && r["decision"] == "denied"),
        "{rows:?}"
    );
    assert_eq!(rows[5]["context"]["new_digest"], OTHER);
    h.finish().await;
}

#[tokio::test]
async fn a_job_without_the_digest_param_is_refused() {
    let Some(mut h) = Harness::new("upd_no_param").await else {
        return;
    };
    running(&mut h, build(), false);
    offer(&h, UpdateOutcome::UpdateAvailable, NEW);
    system(
        &h,
        Setup {
            param: false,
            ..Setup::FULL
        },
    )
    .await;
    assert_eq!(refused(&h.cmd(update(NEW)).await), "job_missing_param");
    assert_eq!(refusals(&h).await, ["job_missing_param"]);
    h.finish().await;
}

#[tokio::test]
async fn a_status_that_moved_after_the_preview_is_stale_on_confirm() {
    let Some(mut h) = Harness::new("upd_stale").await else {
        return;
    };
    running(&mut h, build(), false);
    offer(&h, UpdateOutcome::UpdateAvailable, NEW);
    system(&h, Setup::FULL).await;
    let p = preview(&h.cmd(update(NEW)).await);
    offer(&h, UpdateOutcome::UpdateAvailable, OTHER);
    let out = h
        .cmd(Cmd::Confirm {
            preview_id: p.preview_id,
            step_up: None,
        })
        .await;
    assert!(
        out.iter()
            .any(|e| matches!(e, Event::OpError { code, .. } if code == "digest_stale")),
        "{out:?}"
    );
    assert_eq!(refusals(&h).await, ["digest_stale"]);
    let runs: i64 = sqlx::query_scalar("select count(*) from runs")
        .fetch_one(&h.db.owner)
        .await
        .expect("runs");
    assert_eq!(runs, 0);
    h.finish().await;
}

#[tokio::test]
async fn the_preview_says_what_will_happen_in_one_paragraph() {
    let Some(mut h) = Harness::new("upd_preview").await else {
        return;
    };
    running(&mut h, build(), false);
    offer(&h, UpdateOutcome::Rebuilt, NEW);
    system(&h, Setup::FULL).await;
    let p = preview(&h.cmd(update(NEW)).await);
    let s = &p.summary;
    assert!(!s.contains('\n'), "{s}");
    assert!(s.starts_with("Warning: "), "{s}");
    assert!(
        s.contains(&format!(
            "Update Keel from 0.2.0 (revision abcdef1, {RUNNING}) to 0.3.0 (revision fedcba9, {NEW})."
        )),
        "{s}"
    );
    assert!(s.contains("Runs job keel-server"), "{s}");
    assert!(s.contains("on keel-1"), "{s}");
    assert!(s.contains("Approval: no approval needed"), "{s}");
    assert_eq!(p.affected, ["keel-1"]);
    h.finish().await;
}

#[tokio::test]
async fn confirming_queues_one_update_run_and_its_end_is_audited() {
    let Some(mut h) = Harness::new("upd_run").await else {
        return;
    };
    running(&mut h, build(), false);
    offer(&h, UpdateOutcome::UpdateAvailable, NEW);
    let sys = system(&h, Setup::FULL).await;

    // The owner's status: the run path, nothing in flight.
    match h.cmd(Cmd::GetUpdateStatus).await.as_slice() {
        [Event::UpdateStatus(s)] => {
            assert_eq!(s.update_path, UpdatePath::Run);
            assert_eq!((s.update_run, &s.last_update_run), (None, &None));
        }
        other => panic!("{other:?}"),
    }

    let run = update_to(&h, NEW).await;
    assert_eq!(run.workspace, sys.workspace);
    assert_eq!(run.job_name, "keel-server");
    assert_eq!(run.trigger, RunTrigger::Manual);
    assert_eq!(
        run.params,
        BTreeMap::from([("keel_server_image_digest".to_owned(), NEW.to_owned())])
    );
    let trigger: String = sqlx::query_scalar("select trigger from runs where id = $1")
        .bind(run.id)
        .fetch_one(&h.db.owner)
        .await
        .expect("trigger");
    assert_eq!(trigger, "manual");
    let requested = audited(&h, "keel_update_requested").await;
    assert_eq!(requested.len(), 1, "{requested:?}");
    let c = &requested[0]["context"];
    assert_eq!(c["old_digest"], RUNNING);
    assert_eq!(c["new_digest"], NEW);
    assert_eq!(c["old_revision"], "abcdef1");
    assert_eq!(c["new_revision"], "fedcba9");
    assert_eq!(c["run"], run.id.to_string());
    assert_eq!(c["workspace"], sys.workspace.to_string());

    // In flight: the status names it, and a second request opens it instead.
    match h.cmd(Cmd::GetUpdateStatus).await.as_slice() {
        [Event::UpdateStatus(s)] => assert_eq!(s.update_run, Some(run.id)),
        other => panic!("{other:?}"),
    }
    let p = preview(&h.cmd(update(NEW)).await);
    assert!(
        p.summary.contains(&format!(
            "An update run is already in progress ({})",
            run.id
        )),
        "{}",
        p.summary
    );
    let again = update_to(&h, NEW).await;
    assert_eq!(again.id, run.id);
    let refused = audited(&h, "keel_update_refused").await;
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(refused[0]["reason"], "update_in_progress");
    assert_eq!(refused[0]["context"]["run"], run.id.to_string());
    let runs: i64 = sqlx::query_scalar("select count(*) from runs")
        .fetch_one(&h.db.owner)
        .await
        .expect("runs");
    assert_eq!(runs, 1);

    // A runner on keel-1 finishes it.
    let host = sys.host.expect("host");
    let (stream, _runner) = FakeRunner::new(host.to_string())
        .default_script(Script::finish(0))
        .spawn();
    keel_server::queue::attach(h.app.clone(), host, stream)
        .await
        .expect("attach");
    let start = Instant::now();
    let state = loop {
        let (state, ended): (String, bool) =
            sqlx::query_as("select state, ended_at is not null from runs where id = $1")
                .bind(run.id)
                .fetch_one(&h.db.owner)
                .await
                .expect("run");
        if ended {
            break state;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "still {state}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let results = audited(&h, "keel_update_result").await;
    assert_eq!(results.len(), 1, "{results:?}");
    let c = &results[0]["context"];
    assert_eq!(c["run"], run.id.to_string());
    assert_eq!(c["outcome"], state.as_str());
    assert_eq!(c["old_digest"], RUNNING);
    assert_eq!(c["new_digest"], NEW);
    assert_eq!(results[0]["actor"], "system");

    // The status keeps the result, so a failed update survives a reload.
    match h.cmd(Cmd::GetUpdateStatus).await.as_slice() {
        [Event::UpdateStatus(s)] => {
            assert_eq!(s.update_run, None);
            let last = s.last_update_run.as_ref().expect("last_update_run");
            assert_eq!(last.run, run.id);
            assert_eq!(last.outcome, state);
            assert_eq!(last.digest, NEW);
            assert!(last.ended_at.is_some());
        }
        other => panic!("{other:?}"),
    }
    h.finish().await;
}
