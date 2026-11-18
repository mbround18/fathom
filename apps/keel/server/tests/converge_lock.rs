//! Converge step ordering and the per-host hold (spec 068 T017, T020, T021).
//!
//! The `converge_advance` trigger moves a host's next step when one ends; the
//! `converge_hosts_one_holding` index lets one converge hold a host at a time.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod repos;

use repos::Harness;
use uuid::Uuid;

async fn host(h: &Harness, name: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into hosts (id, workspace_id, inventory_name, allowed_profiles) \
         values ($1, $2, $3, '{debian}')",
    )
    .bind(id)
    .bind(h.workspace)
    .bind(name)
    .execute(&h.db.owner)
    .await
    .expect("host");
    id
}

/// A converge parent in `state`.
async fn parent(h: &Harness, state: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into runs (id, workspace_id, job_name, kind, trigger, commit, inventory_name, \
         state) values ($1, $2, 'converge', 'converge', 'manual', 'abc', 'prod', $3)",
    )
    .bind(id)
    .bind(h.workspace)
    .bind(state)
    .execute(&h.db.owner)
    .await
    .expect("parent");
    id
}

/// One child run (one job) of `parent`.
async fn child(h: &Harness, parent: Uuid, job: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into runs (id, workspace_id, job_name, trigger, commit, inventory_name, state, \
         parent_run_id) values ($1, $2, $3, 'manual', 'abc', 'prod', 'queued', $4)",
    )
    .bind(id)
    .bind(h.workspace)
    .bind(job)
    .bind(parent)
    .execute(&h.db.owner)
    .await
    .expect("child");
    id
}

/// Plans `jobs` (child runs, in order) for `host` under `parent`; the first step is `first`.
async fn plan(h: &Harness, parent: Uuid, host: Uuid, steps: &[(Uuid, &str)], first: &str) {
    sqlx::query(
        "insert into converge_hosts (run_id, host_id, state, profiles) \
         values ($1, $2, 'waiting', '{debian}')",
    )
    .bind(parent)
    .bind(host)
    .execute(&h.db.owner)
    .await
    .expect("converge host");
    for (pos, (run, job)) in steps.iter().enumerate() {
        sqlx::query(
            "insert into run_hosts (run_id, host_id, host_name, state) values ($1, $2, 'h', $3)",
        )
        .bind(run)
        .bind(host)
        .bind(if pos == 0 { first } else { "blocked" })
        .execute(&h.db.owner)
        .await
        .expect("run host");
        sqlx::query(
            "insert into converge_steps (run_id, host_id, position, job_name, step_run_id) \
             values ($1, $2, $3, $4, $5)",
        )
        .bind(parent)
        .bind(host)
        .bind(i16::try_from(pos).unwrap())
        .bind(job)
        .bind(run)
        .execute(&h.db.owner)
        .await
        .expect("step");
    }
}

async fn set(h: &Harness, run: Uuid, host: Uuid, state: &str) {
    sqlx::query("update run_hosts set state = $3 where run_id = $1 and host_id = $2")
        .bind(run)
        .bind(host)
        .bind(state)
        .execute(&h.db.app)
        .await
        .expect("update run host");
}

async fn step(h: &Harness, run: Uuid, host: Uuid) -> (String, Option<String>) {
    sqlx::query_as("select state, skip_reason from run_hosts where run_id = $1 and host_id = $2")
        .bind(run)
        .bind(host)
        .fetch_one(&h.db.owner)
        .await
        .expect("step")
}

async fn hold(h: &Harness, parent: Uuid, host: Uuid) -> (String, bool) {
    sqlx::query_as(
        "select state, ended_at is not null from converge_hosts where run_id = $1 and host_id = $2",
    )
    .bind(parent)
    .bind(host)
    .fetch_one(&h.db.owner)
    .await
    .expect("converge host")
}

struct Three {
    parent: Uuid,
    a: Uuid,
    b: Uuid,
    c: Uuid,
}

/// A converge of three steps (`a`, `b`, `c`) on `hosts`, the first queued and holding.
async fn three(h: &Harness, hosts: &[Uuid]) -> Three {
    let parent = parent(h, "running").await;
    let a = child(h, parent, "a").await;
    let b = child(h, parent, "b").await;
    let c = child(h, parent, "c").await;
    for host in hosts {
        plan(h, parent, *host, &[(a, "a"), (b, "b"), (c, "c")], "queued").await;
    }
    Three { parent, a, b, c }
}

#[tokio::test]
async fn a_succeeded_step_queues_the_next_and_the_last_ends_the_hold() {
    let Some(h) = Harness::new("converge_advance_success").await else {
        return;
    };
    let x = host(&h, "x").await;
    let t = three(&h, &[x]).await;
    sqlx::query("update converge_hosts set state = 'holding' where run_id = $1")
        .bind(t.parent)
        .execute(&h.db.app)
        .await
        .unwrap();
    set(&h, t.a, x, "sent").await;
    set(&h, t.a, x, "running").await;
    assert_eq!(step(&h, t.b, x).await.0, "blocked", "not before a ends");
    set(&h, t.a, x, "succeeded").await;
    assert_eq!(step(&h, t.b, x).await.0, "queued");
    assert_eq!(step(&h, t.c, x).await.0, "blocked");
    assert_eq!(hold(&h, t.parent, x).await, ("holding".into(), false));
    set(&h, t.b, x, "succeeded").await;
    set(&h, t.c, x, "succeeded").await;
    assert_eq!(hold(&h, t.parent, x).await, ("succeeded".into(), true));
    h.finish().await;
}

#[tokio::test]
async fn a_failed_step_skips_the_rest_of_that_host_only() {
    let Some(h) = Harness::new("converge_advance_failure").await else {
        return;
    };
    let x = host(&h, "x").await;
    let y = host(&h, "y").await;
    let t = three(&h, &[x, y]).await;
    set(&h, t.a, x, "failed").await;
    assert_eq!(
        step(&h, t.b, x).await,
        ("skipped".into(), Some("dependency_failed".into()))
    );
    assert_eq!(
        step(&h, t.c, x).await,
        ("skipped".into(), Some("dependency_failed".into()))
    );
    assert_eq!(hold(&h, t.parent, x).await, ("failed".into(), true));
    // The other host goes on.
    set(&h, t.a, y, "succeeded").await;
    assert_eq!(step(&h, t.b, y).await.0, "queued");
    assert_eq!(hold(&h, t.parent, y).await.0, "waiting");
    h.finish().await;
}

#[tokio::test]
async fn refused_and_lost_steps_fail_the_host_and_a_late_result_changes_nothing() {
    let Some(h) = Harness::new("converge_advance_lost").await else {
        return;
    };
    let x = host(&h, "x").await;
    let y = host(&h, "y").await;
    let t = three(&h, &[x, y]).await;
    set(&h, t.a, x, "refused").await;
    assert_eq!(hold(&h, t.parent, x).await.0, "failed");
    set(&h, t.a, y, "sent").await;
    set(&h, t.a, y, "lost").await;
    assert_eq!(hold(&h, t.parent, y).await.0, "failed");
    assert_eq!(step(&h, t.b, y).await.0, "skipped");
    // A runner replays `Finished` on the lost row (spec 060): `lost` is terminal, so the
    // trigger does not fire and the skipped steps stay skipped.
    set(&h, t.a, y, "succeeded").await;
    assert_eq!(step(&h, t.b, y).await.0, "skipped");
    assert_eq!(hold(&h, t.parent, y).await.0, "failed");
    h.finish().await;
}

#[tokio::test]
async fn cancelling_steps_cancels_the_hold() {
    let Some(h) = Harness::new("converge_advance_cancel").await else {
        return;
    };
    let x = host(&h, "x").await;
    let t = three(&h, &[x]).await;
    // A cancel takes every blocked and queued step of the converge in one statement.
    sqlx::query(
        "update run_hosts set state = 'cancelled', ended_at = now() \
         where run_id = any($1) and state in ('blocked', 'queued')",
    )
    .bind(vec![t.a, t.b, t.c])
    .execute(&h.db.app)
    .await
    .unwrap();
    assert_eq!(hold(&h, t.parent, x).await, ("cancelled".into(), true));
    for run in [t.a, t.b, t.c] {
        assert_eq!(step(&h, run, x).await.0, "cancelled");
    }
    // Cancelling only the queued first step skips the rest as `converge_cancelled`.
    let y = host(&h, "y").await;
    let u = three(&h, &[y]).await;
    set(&h, u.a, y, "cancelled").await;
    assert_eq!(
        step(&h, u.b, y).await,
        ("skipped".into(), Some("converge_cancelled".into()))
    );
    assert_eq!(hold(&h, u.parent, y).await.0, "cancelled");
    h.finish().await;
}

#[tokio::test]
async fn ordinary_runs_are_untouched() {
    let Some(h) = Harness::new("converge_advance_ordinary").await else {
        return;
    };
    let x = host(&h, "x").await;
    let run = Uuid::now_v7();
    sqlx::query(
        "insert into runs (id, workspace_id, job_name, trigger, commit, inventory_name, state) \
         values ($1, $2, 'web', 'manual', 'abc', 'prod', 'queued')",
    )
    .bind(run)
    .bind(h.workspace)
    .execute(&h.db.owner)
    .await
    .unwrap();
    sqlx::query("insert into run_hosts (run_id, host_id, host_name) values ($1, $2, 'x')")
        .bind(run)
        .bind(x)
        .execute(&h.db.owner)
        .await
        .unwrap();
    set(&h, run, x, "failed").await;
    assert_eq!(step(&h, run, x).await, ("failed".into(), None));
    h.finish().await;
}

#[tokio::test]
async fn one_converge_holds_a_host_at_a_time() {
    let Some(h) = Harness::new("converge_one_holding").await else {
        return;
    };
    let x = host(&h, "x").await;
    let first = three(&h, &[x]).await;
    let second = three(&h, &[x]).await;
    let hold = |run: Uuid| {
        sqlx::query(
            "update converge_hosts set state = 'holding' where run_id = $1 and host_id = $2",
        )
        .bind(run)
        .bind(x)
    };
    hold(first.parent)
        .execute(&h.db.app)
        .await
        .expect("first holds");
    let err = hold(second.parent)
        .execute(&h.db.app)
        .await
        .expect_err("a second hold is refused");
    assert!(
        err.to_string().contains("converge_hosts_one_holding"),
        "{err}"
    );
    h.finish().await;
}

#[tokio::test]
async fn converge_rows_have_their_shape() {
    let Some(h) = Harness::new("converge_shape").await else {
        return;
    };
    let p = parent(&h, "queued").await;
    // A converge is never a child, and a child is never a converge.
    let err = sqlx::query(
        "insert into runs (id, workspace_id, job_name, kind, trigger, commit, inventory_name, \
         state, parent_run_id) values ($1, $2, 'converge', 'converge', 'manual', 'abc', 'prod', \
         'queued', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(h.workspace)
    .bind(p)
    .execute(&h.db.owner)
    .await
    .expect_err("nested converge");
    assert!(err.to_string().contains("runs_converge_shape"), "{err}");
    let err = sqlx::query(
        "insert into runs (id, workspace_id, job_name, kind, trigger, commit, inventory_name, \
         state, parent_run_id) values ($1, $2, 'web', 'rollback', 'manual', 'abc', 'prod', \
         'queued', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(h.workspace)
    .bind(p)
    .execute(&h.db.owner)
    .await
    .expect_err("rollback child");
    assert!(err.to_string().contains("runs_converge_shape"), "{err}");
    h.finish().await;
}

async fn ended(h: &Harness, run: Uuid) -> (String, bool) {
    sqlx::query_as("select state, ended_at is not null from runs where id = $1")
        .bind(run)
        .fetch_one(&h.db.owner)
        .await
        .expect("run")
}

#[tokio::test]
async fn a_restart_with_a_step_out_fails_that_host_and_ends_the_converge() {
    let Some(h) = Harness::new("converge_restart_in_flight").await else {
        return;
    };
    let x = host(&h, "x").await;
    let t = three(&h, &[x]).await;
    sqlx::query("update converge_hosts set state = 'holding' where run_id = $1")
        .bind(t.parent)
        .execute(&h.db.app)
        .await
        .unwrap();
    set(&h, t.a, x, "sent").await;
    keel_server::queue::recover_in_flight(&h.app)
        .await
        .expect("recover");
    assert_eq!(step(&h, t.a, x).await.0, "lost");
    assert_eq!(
        step(&h, t.b, x).await,
        ("skipped".into(), Some("dependency_failed".into()))
    );
    assert_eq!(hold(&h, t.parent, x).await, ("failed".into(), true));
    assert_eq!(ended(&h, t.parent).await, ("failed".into(), true));
    for c in [t.a, t.b, t.c] {
        assert!(ended(&h, c).await.1, "every step run ends");
    }
    h.finish().await;
}

#[tokio::test]
async fn a_restart_reconciles_a_hold_with_no_live_step() {
    let Some(h) = Harness::new("converge_restart_stale_hold").await else {
        return;
    };
    let x = host(&h, "x").await;
    let parent = parent(&h, "running").await;
    let a = child(&h, parent, "a").await;
    let b = child(&h, parent, "b").await;
    // Held, but nothing queued or out: no step will ever release it.
    plan(&h, parent, x, &[(a, "a"), (b, "b")], "blocked").await;
    sqlx::query("update converge_hosts set state = 'holding' where run_id = $1")
        .bind(parent)
        .execute(&h.db.app)
        .await
        .unwrap();
    keel_server::queue::recover_in_flight(&h.app)
        .await
        .expect("recover");
    assert_eq!(hold(&h, parent, x).await, ("failed".into(), true));
    assert_eq!(
        step(&h, b, x).await,
        ("skipped".into(), Some("dependency_failed".into()))
    );
    assert_eq!(ended(&h, parent).await, ("failed".into(), true));
    h.finish().await;
}

#[tokio::test]
async fn a_restart_leaves_a_hold_between_steps_alone() {
    let Some(h) = Harness::new("converge_restart_between").await else {
        return;
    };
    let x = host(&h, "x").await;
    let t = three(&h, &[x]).await;
    sqlx::query("update converge_hosts set state = 'holding' where run_id = $1")
        .bind(t.parent)
        .execute(&h.db.app)
        .await
        .unwrap();
    set(&h, t.a, x, "sent").await;
    set(&h, t.a, x, "succeeded").await;
    keel_server::queue::recover_in_flight(&h.app)
        .await
        .expect("recover");
    assert_eq!(step(&h, t.b, x).await.0, "queued");
    assert_eq!(hold(&h, t.parent, x).await, ("holding".into(), false));
    assert_eq!(ended(&h, t.parent).await, ("running".into(), false));
    h.finish().await;
}
