//! Spec 068 US4 (FR-010) and FR-020: a path-triggered run is covered on hosts a pending
//! converge will apply it to, and a push that changes only group or host vars files narrows the
//! run to those hosts.

mod common;
mod converge_support;
mod repos;

use converge_support::Fixture;
use keel_protocol::{Cmd, ConvergeTarget, Event, RunInfo, TargetSelection};
use keel_server::runs::{Push, on_push};
use keel_server::sync::Trigger;
use uuid::Uuid;

/// Commits `file` to the bound repository, pushes, syncs; returns the new head.
async fn push(f: &Fixture, file: &str, content: &str) -> String {
    let head = f.src.commit(file, content, "change");
    f.src.push();
    let out = f.h.cmd(Cmd::SyncBinding { binding: f.binding }).await;
    assert!(
        matches!(out.as_slice(), [Event::BindingSynced { .. }]),
        "{out:?}"
    );
    head
}

async fn head(f: &Fixture) -> String {
    sqlx::query_scalar("select last_commit from repo_bindings where id = $1")
        .bind(f.binding)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("binding head")
}

/// Replays the push `previous..head` as the poller would: a path run at `head`.
async fn replay(f: &Fixture, previous: &str, head: &str) {
    on_push(
        &f.h.app,
        Push {
            binding: f.binding,
            workspace: f.h.workspace,
            previous,
            head,
            trigger: &Trigger::Poll,
            signed_mode: false,
        },
    )
    .await;
}

/// The path runs of `job` (not converge steps), oldest first, with their commit.
async fn path_runs(f: &Fixture, job: &str) -> Vec<(Uuid, String)> {
    sqlx::query_as(
        "select id, commit from runs where job_name = $1 and parent_run_id is null \
         order by created_at, id",
    )
    .bind(job)
    .fetch_all(&f.h.db.owner)
    .await
    .expect("runs")
}

/// Each host of `run` as (name, state, covered_by).
async fn hosts(f: &Fixture, run: Uuid) -> Vec<(String, String, Option<Uuid>)> {
    sqlx::query_as(
        "select host_name, state, covered_by from run_hosts where run_id = $1 order by host_name",
    )
    .bind(run)
    .fetch_all(&f.h.db.owner)
    .await
    .expect("run hosts")
}

async fn info(f: &Fixture, run: Uuid) -> RunInfo {
    match f.h.cmd(Cmd::GetRun { run }).await.as_slice() {
        [Event::Run { run, .. }] => run.clone(),
        other => panic!("get run: {other:?}"),
    }
}

async fn covered_audits(f: &Fixture) -> Vec<serde_json::Value> {
    sqlx::query_scalar(
        "select jsonb_build_object('target', target, 'context', context) from audit_event \
         where action = 'run_host.covered' order by seq",
    )
    .fetch_all(&f.h.db.owner)
    .await
    .expect("audit")
}

#[tokio::test]
async fn a_path_run_is_covered_by_a_pending_converge_at_the_same_or_a_newer_commit() {
    // Approval held: the converge stays pending, its hosts waiting, nothing dispatches.
    let Some(f) = Fixture::new("cov_commits", true).await else {
        return;
    };
    let c0 = head(&f).await;
    let h1 = push(&f, "playbooks/k3s-audit.yaml", "- hosts: all\n# v2\n").await;
    let runs = path_runs(&f, "k3s-audit").await;
    assert_eq!(runs.len(), 1, "the push starts a k3s-audit run");
    assert_eq!(
        hosts(&f, runs[0].0).await,
        vec![("master-1".into(), "queued".into(), None)],
        "no converge yet: it queues"
    );
    let h2 = push(&f, "README.md", "converge base\n").await;
    assert_eq!(path_runs(&f, "k3s-audit").await.len(), 1);
    let converge = f.start().await;
    let (at,): (String,) = sqlx::query_as("select commit from runs where id = $1")
        .bind(converge)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("converge");
    assert_eq!(at, h2);

    // The converge's commit descends from h1: k3s-master covers k3s-audit on master-1.
    replay(&f, &c0, &h1).await;
    // The same commit covers too.
    replay(&f, &c0, &h2).await;
    let runs = path_runs(&f, "k3s-audit").await;
    assert_eq!(runs.len(), 3);
    for (run, commit) in &runs[1..] {
        assert_eq!(
            hosts(&f, *run).await,
            vec![("master-1".into(), "covered".into(), Some(converge))],
            "run at {commit}"
        );
        let (state, ended): (String, bool) =
            sqlx::query_as("select state, ended_at is not null from runs where id = $1")
                .bind(run)
                .fetch_one(&f.h.db.owner)
                .await
                .expect("run");
        assert_eq!((state.as_str(), ended), ("succeeded", true));
    }
    let audits = covered_audits(&f).await;
    assert_eq!(audits.len(), 2, "{audits:?}");
    assert_eq!(audits[0]["target"]["run"], serde_json::json!(runs[1].0));
    assert_eq!(audits[0]["target"]["host"], serde_json::json!(f.master));
    assert_eq!(
        audits[0]["context"]["covered_by"],
        serde_json::json!(converge)
    );

    // A push past the converge's commit is not covered: it queues.
    push(&f, "playbooks/k3s-audit.yaml", "- hosts: all\n# v3\n").await;
    let runs = path_runs(&f, "k3s-audit").await;
    assert_eq!(runs.len(), 4);
    assert_eq!(
        hosts(&f, runs[3].0).await,
        vec![("master-1".into(), "queued".into(), None)]
    );
    assert_eq!(covered_audits(&f).await.len(), 2);
    f.h.finish().await;
}

#[tokio::test]
async fn hosts_outside_the_converge_still_run() {
    let Some(f) = Fixture::new("cov_hosts", true).await else {
        return;
    };
    let c0 = head(&f).await;
    let h1 = push(&f, "playbooks/debian.yaml", "- hosts: all\n# v2\n").await;
    // A converge of master-1 only, at h1.
    let (token, _) = f
        .plan(Cmd::PreviewConverge(ConvergeTarget {
            binding_id: f.binding,
            hosts: TargetSelection::Hosts {
                ids: vec![f.master],
            },
            check: false,
        }))
        .await;
    let converge = f.confirm(token).await;
    replay(&f, &c0, &h1).await;
    let runs = path_runs(&f, "debian").await;
    assert_eq!(runs.len(), 2);
    assert_eq!(
        hosts(&f, runs[1].0).await,
        vec![
            ("master-1".into(), "covered".into(), Some(converge)),
            ("node-1".into(), "queued".into(), None),
        ]
    );
    let (ended,): (bool,) = sqlx::query_as("select ended_at is not null from runs where id = $1")
        .bind(runs[1].0)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("run");
    assert!(!ended, "node-1 still has to run");
    // A cancelled converge covers nothing.
    let out = f.h.cmd(Cmd::CancelConverge { run_id: converge }).await;
    assert!(
        !out.iter().any(|e| matches!(e, Event::OpError { .. })),
        "{out:?}"
    );
    replay(&f, &c0, &h1).await;
    let runs = path_runs(&f, "debian").await;
    assert_eq!(
        hosts(&f, runs[2].0).await,
        vec![
            ("master-1".into(), "queued".into(), None),
            ("node-1".into(), "queued".into(), None),
        ]
    );
    f.h.finish().await;
}

#[tokio::test]
async fn a_vars_only_push_narrows_the_run_to_its_hosts() {
    let Some(f) = Fixture::new("cov_narrow", true).await else {
        return;
    };
    for (host, groups) in [("node-1", "{keel_nodes}"), ("master-1", "{k3s_servers}")] {
        sqlx::query("update inventory_hosts set groups = $2::text[] where host = $1")
            .bind(host)
            .bind(groups)
            .execute(&f.h.db.owner)
            .await
            .expect("groups");
    }
    let expect = |hosts: &[&str], by: Option<&[&str]>| {
        (
            hosts.iter().map(|h| (*h).to_owned()).collect::<Vec<_>>(),
            by.map(|b| b.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>()),
        )
    };
    let mut seen = 0;
    for (file, want) in [
        (
            "inventory/group_vars/keel_nodes.yml",
            Some(expect(
                &["node-1"],
                Some(&["inventory/group_vars/keel_nodes.yml"]),
            )),
        ),
        (
            "inventory/host_vars/master-1.yaml",
            Some(expect(
                &["master-1"],
                Some(&["inventory/host_vars/master-1.yaml"]),
            )),
        ),
        (
            "inventory/group_vars/all.yml",
            Some(expect(&["master-1", "node-1"], None)),
        ),
        (
            "playbooks/debian.yaml",
            Some(expect(&["master-1", "node-1"], None)),
        ),
        // A group no host is in: no run, audited.
        ("inventory/group_vars/nobody.yml", None),
    ] {
        push(&f, file, "x: 1\n").await;
        let runs = path_runs(&f, "debian").await;
        match want {
            Some((names, by)) => {
                assert_eq!(runs.len(), seen + 1, "{file} starts a run");
                seen += 1;
                let mut run = info(&f, runs[seen - 1].0).await;
                run.hosts.sort();
                assert_eq!((run.hosts, run.narrowed_by), (names, by), "{file}");
            }
            None => assert_eq!(runs.len(), seen, "{file} starts no run"),
        }
    }
    let skipped: Vec<(String, Option<String>, serde_json::Value)> = sqlx::query_as(
        "select decision, reason, context -> 'narrowed_by' from audit_event \
         where action = 'run.skipped'",
    )
    .fetch_all(&f.h.db.owner)
    .await
    .expect("audit");
    assert_eq!(
        skipped,
        vec![(
            "allowed".into(),
            Some("no_hosts".into()),
            serde_json::json!(["inventory/group_vars/nobody.yml"])
        )]
    );
    f.h.finish().await;
}
