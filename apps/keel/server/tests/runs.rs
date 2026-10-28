//! Runs from source control (spec 055 US2, T063), against the fake runner (`keel-exec/fake`):
//! a push that touches a job's `paths` makes a run pinned to the fetched commit and the
//! inventory revision; a force-push makes none; results arrive within SC-003's 30 s; a runner
//! that stops answering pings is lost; logs past the cap keep their head and tail; a subscriber
//! resumes from a sequence number without gaps; and a token may read runs but never start or
//! cancel one.

mod common;
mod repos;

use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use keel_exec::events::HostStats;
use keel_exec::fake::{FakeRunner, Script};
use keel_exec::frames::{Frame, LogStream};
use keel_protocol::{
    ActionName, Cmd, CmdEnvelope, Event, EventEnvelope, Label, RequestId, RunPhase, TokenActions,
};
use keel_server::runs::{RunSettings, Runs};
use repos::{Harness, Source, bind, git, op_error};
use uuid::Uuid;

/// The `web` job runs on pushes under `playbooks/`.
const MANIFEST: &str = "version: 1\nprofiles: { debian: {} }\njobs:\n  web:\n    \
                        playbook: playbooks/web.yaml\n    profiles: [debian]\n    \
                        inventory: prod\n    paths: ['playbooks/**']\n";

const SC_003: Duration = Duration::from_secs(30);

struct Fixture {
    h: Harness,
    src: Source,
    binding: Uuid,
    revision: Uuid,
    hosts: Vec<(String, Uuid)>,
}

impl Fixture {
    async fn new(test: &str, settings: RunSettings) -> Option<Self> {
        let mut h = Harness::new(test).await?;
        h.app.runs = Runs::new(settings);
        let src = Source::new(MANIFEST);
        let binding = bind(&h, &src).await;
        let owner = &h.db.owner;
        // A managed inventory `prod` with one valid revision listing two web hosts.
        let inventory = Uuid::now_v7();
        sqlx::query(
            "insert into inventories (id, workspace_id, name, source) values ($1, $2, 'prod', 'managed')",
        )
        .bind(inventory)
        .bind(h.workspace)
        .execute(owner)
        .await
        .expect("inventory");
        let revision = Uuid::now_v7();
        sqlx::query(
            "insert into inventory_revisions (id, inventory_id, seq, content, content_hash, status) \
             values ($1, $2, 1, '{}', 'h1', 'valid')",
        )
        .bind(revision)
        .bind(inventory)
        .execute(owner)
        .await
        .expect("revision");
        let mut hosts = Vec::new();
        for name in ["web-1", "web-2"] {
            sqlx::query(
                "insert into inventory_hosts (revision_id, host, groups) values ($1, $2, '{web}')",
            )
            .bind(revision)
            .bind(name)
            .execute(owner)
            .await
            .expect("inventory host");
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into hosts (id, workspace_id, inventory_name, allowed_profiles) \
                 values ($1, $2, $3, '{debian}')",
            )
            .bind(id)
            .bind(h.workspace)
            .bind(name)
            .execute(owner)
            .await
            .expect("host");
            sqlx::query(
                "insert into host_profiles (host_id, profile_id) \
                 select $1, id from profiles where workspace_id = $2 and name = 'debian'",
            )
            .bind(id)
            .bind(h.workspace)
            .execute(owner)
            .await
            .expect("host profile");
            hosts.push((name.to_owned(), id));
        }
        // Runs here need no approval; the catch-all would hold them.
        sqlx::query(
            "insert into approval_policies (id, workspace_id, priority, requires_approval) \
             values ($1, $2, 1, false)",
        )
        .bind(Uuid::now_v7())
        .bind(h.workspace)
        .execute(owner)
        .await
        .expect("policy");
        Some(Self {
            h,
            src,
            binding,
            revision,
            hosts,
        })
    }

    /// Connects a fake runner for each host.
    async fn attach(
        &self,
        runner: impl Fn(&str, Uuid) -> FakeRunner,
    ) -> Vec<keel_exec::fake::FakeHandle> {
        let mut out = Vec::new();
        for (name, id) in &self.hosts {
            let (stream, handle) = runner(name, *id).spawn();
            keel_server::queue::attach(self.h.app.clone(), *id, stream)
                .await
                .expect("attach");
            out.push(handle);
        }
        out
    }

    /// Commits `file`, pushes and syncs; returns the new head.
    async fn push(&self, file: &str, content: &str) -> String {
        let head = self.src.commit(file, content, "change");
        self.src.push();
        let out = self
            .h
            .cmd(Cmd::SyncBinding {
                binding: self.binding,
            })
            .await;
        assert!(
            matches!(out.as_slice(), [Event::BindingSynced { .. }]),
            "{out:?}"
        );
        head
    }

    async fn runs(&self) -> Vec<(Uuid, String, Option<Uuid>, Vec<String>)> {
        sqlx::query_as(
            "select id, commit, inventory_revision_id, resolved_hosts from runs order by created_at",
        )
        .fetch_all(&self.h.db.owner)
        .await
        .expect("runs")
    }

    /// Waits until the run is terminal, up to `within`.
    async fn ended(&self, run: Uuid, within: Duration) -> (String, Option<String>) {
        let start = Instant::now();
        loop {
            let (state, reason, ended): (String, Option<String>, bool) = sqlx::query_as(
                "select state, state_reason, ended_at is not null from runs where id = $1",
            )
            .bind(run)
            .fetch_one(&self.h.db.owner)
            .await
            .expect("run");
            let s = (state, reason);
            if ended {
                return s;
            }
            assert!(start.elapsed() < within, "run {run} still {s:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn host_states(&self, run: Uuid) -> Vec<(String, String)> {
        sqlx::query_as(
            "select host_name, state from run_hosts where run_id = $1 order by host_name",
        )
        .bind(run)
        .fetch_all(&self.h.db.owner)
        .await
        .expect("run hosts")
    }

    async fn finish(self) {
        self.h.finish().await;
    }
}

fn settings() -> RunSettings {
    RunSettings {
        ping_interval: Duration::from_millis(50),
        flush_interval: Duration::from_millis(20),
        ..RunSettings::default()
    }
}

fn ok_stats() -> HostStats {
    HostStats {
        ok: 2,
        changed: 1,
        ..HostStats::default()
    }
}

#[tokio::test]
async fn a_push_touching_a_jobs_paths_makes_a_pinned_run_whose_results_arrive_in_time() {
    let Some(f) = Fixture::new("run_on_push", settings()).await else {
        return;
    };
    let _runners = f
        .attach(|name, id| {
            FakeRunner::new(id.to_string()).default_script(
                Script::finish(0)
                    .event(r#"{"v":1,"event":"task_start","task":"install","task_id":"t1","action":"apt","handler":false}"#)
                    .log(LogStream::Stdout, b"TASK [install]\n".to_vec())
                    .stats(name, ok_stats()),
            )
        })
        .await;
    // The first sync made no run; a change outside `paths` makes none either.
    f.push("README", "hello\n").await;
    assert!(f.runs().await.is_empty());

    let started = Instant::now();
    let head = f.push("playbooks/web.yaml", "- hosts: web\n").await;
    let runs = f.runs().await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    let (run, commit, revision, hosts) = runs[0].clone();
    assert_eq!(commit, head);
    assert_eq!(revision, Some(f.revision));
    assert_eq!(hosts, ["web-1", "web-2"]);
    // The commit is pinned for the run's lifetime.
    let mirror = keel_git::Mirror::open(&f.h.app.cfg.data_dir, f.binding).expect("mirror");
    assert_eq!(mirror.pinned(run).expect("pin").map(|c| c.0), Some(head));

    assert_eq!(f.ended(run, SC_003).await, ("succeeded".into(), None));
    let elapsed = started.elapsed();
    assert!(elapsed < SC_003, "SC-003: results took {elapsed:?}");
    eprintln!("SC-003: push to per-host results in {elapsed:?}");
    assert_eq!(
        f.host_states(run).await,
        [
            ("web-1".to_owned(), "succeeded".to_owned()),
            ("web-2".to_owned(), "succeeded".to_owned())
        ]
    );
    let results: Vec<(i32, i32)> =
        sqlx::query_as("select ok, changed from run_host_results where run_id = $1")
            .bind(run)
            .fetch_all(&f.h.db.owner)
            .await
            .expect("results");
    assert_eq!(results, [(2, 1), (2, 1)]);

    // Read back over the protocol.
    let out = f.h.cmd(Cmd::GetRun { run }).await;
    let [Event::Run { run: info, hosts }] = out.as_slice() else {
        panic!("{out:?}");
    };
    assert_eq!(info.state, RunPhase::Succeeded);
    assert!(!info.newer_inventory_revision);
    let summary = info.summary.expect("summary");
    assert_eq!((summary.hosts, summary.succeeded), (2, 2));
    assert_eq!(hosts.len(), 2);
    assert_eq!(hosts[0].stats.expect("stats").ok, 2);
    let out =
        f.h.cmd(Cmd::ListRuns {
            workspace: f.h.workspace,
            binding: Some(f.binding),
            host: None,
            actor: None,
            state: Some(RunPhase::Succeeded),
            from: None,
            to: None,
        })
        .await;
    assert!(
        matches!(out.as_slice(), [Event::Runs { items }] if items.len() == 1 && items[0].id == run),
        "{out:?}"
    );
    f.finish().await;
}

/// 058 A2 (T038): a host changed outside Keel is cleared once every job assigned to it has
/// succeeded on it in a run started after the newest outside run; not before.
#[tokio::test]
async fn changed_outside_clears_once_every_assigned_job_ran_after_it() {
    let Some(f) = Fixture::new("run_clears_changed_outside", settings()).await else {
        return;
    };
    let _runners = f
        .attach(|name, id| {
            FakeRunner::new(id.to_string())
                .default_script(Script::finish(0).stats(name, ok_stats()))
        })
        .await;
    let outside = |at: &str| keel_exec::frames::OutsideRun {
        at: at.into(),
        user: "root".into(),
        playbook: "playbooks/web.yaml".into(),
        commit: None,
        dirty: true,
        check: false,
    };
    // web-1 was changed before the run; web-2 says it will be changed after it.
    for ((_, id), at) in f
        .hosts
        .iter()
        .zip(["2026-01-01T00:00:00Z", "2999-01-01T00:00:00Z"])
    {
        let done = keel_server::outside::ingest(&f.h.app, *id, &[outside(at)], 0)
            .await
            .expect("ingest");
        assert!(done.flagged);
    }
    f.push("README", "hello\n").await;
    f.push("playbooks/web.yaml", "- hosts: web\n").await;
    let (run, ..) = f.runs().await[0].clone();
    assert_eq!(f.ended(run, SC_003).await.0, "succeeded");
    let flags = || async {
        sqlx::query_as::<_, (String, bool)>(
            "select inventory_name, changed_outside from hosts order by inventory_name",
        )
        .fetch_all(&f.h.db.owner)
        .await
        .expect("hosts")
    };
    let start = Instant::now();
    while flags().await[0].1 {
        assert!(start.elapsed() < SC_003, "web-1 is still flagged");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        flags().await,
        [("web-1".to_owned(), false), ("web-2".to_owned(), true)]
    );
    let actions = f.h.audit_actions().await;
    assert_eq!(
        actions
            .iter()
            .filter(|a| a.0 == "host_changed_outside_cleared")
            .count(),
        1,
        "{actions:?}"
    );
    f.finish().await;
}

#[tokio::test]
async fn a_force_push_is_history_rewritten_and_makes_no_run() {
    let Some(f) = Fixture::new("run_force_push", settings()).await else {
        return;
    };
    f.src
        .commit("playbooks/web.yaml", "- hosts: web\n", "local");
    git(&f.src.work, &["reset", "-q", "--hard", "HEAD~1"]);
    f.src
        .commit("playbooks/web.yaml", "- hosts: db\n", "rewritten");
    // Publish one history, sync, then replace it.
    git(&f.src.work, &["push", "-q", "-f", "origin", "main"]);
    f.h.cmd(Cmd::SyncBinding { binding: f.binding }).await;
    let before = f.runs().await.len();
    git(&f.src.work, &["reset", "-q", "--hard", "HEAD~1"]);
    f.src
        .commit("playbooks/web.yaml", "- hosts: other\n", "force");
    git(&f.src.work, &["push", "-q", "-f", "origin", "main"]);
    f.h.cmd(Cmd::SyncBinding { binding: f.binding }).await;
    assert_eq!(f.runs().await.len(), before);
    let (error,): (Option<String>,) =
        sqlx::query_as("select last_sync_error from repo_bindings where id = $1")
            .bind(f.binding)
            .fetch_one(&f.h.db.owner)
            .await
            .expect("binding");
    assert_eq!(error.as_deref(), Some("history_rewritten"));
    f.finish().await;
}

#[tokio::test]
async fn a_runner_that_misses_three_pings_is_lost() {
    let Some(f) = Fixture::new("run_runner_lost", settings()).await else {
        return;
    };
    // The run waits for its runners; they connect once it exists, so web-1 is lost mid-run.
    f.push("playbooks/web.yaml", "- hosts: web\n").await;
    let run = f.runs().await[0].0;
    let _runners = f
        .attach(|name, id| {
            let r = FakeRunner::new(id.to_string()).default_script(Script::hang());
            // web-2 answers; web-1 goes silent.
            r.answer_pings(name != "web-1")
        })
        .await;
    // web-2 hangs until cancelled; web-1 is lost after three missed pings.
    let start = Instant::now();
    loop {
        let states = f.host_states(run).await;
        if states[0].1 == "lost" {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "{states:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (online,): (String,) = sqlx::query_as("select state from hosts where id = $1")
        .bind(f.hosts[0].1)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("host");
    assert_eq!(online, "offline");
    // Cancel the rest: web-2 stops, and the run fails as lost.
    f.h.confirm(Cmd::PreviewCancel { run }).await;
    let ended = f.ended(run, Duration::from_secs(10)).await;
    assert_eq!(ended.0, "cancelled");
    assert_eq!(
        f.host_states(run).await,
        [
            ("web-1".to_owned(), "lost".to_owned()),
            ("web-2".to_owned(), "cancelled".to_owned())
        ]
    );
    f.finish().await;
}

#[tokio::test]
async fn a_runner_lost_while_the_run_is_alone_fails_it_runner_lost() {
    let Some(f) = Fixture::new("run_lost_fails", settings()).await else {
        return;
    };
    f.push("playbooks/web.yaml", "- hosts: web\n").await;
    let run = f.runs().await[0].0;
    let _runners = f
        .attach(|_, id| {
            FakeRunner::new(id.to_string())
                .default_script(Script::hang())
                .answer_pings(false)
        })
        .await;
    assert_eq!(
        f.ended(run, Duration::from_secs(10)).await,
        ("failed".into(), Some("runner_lost".into()))
    );
    f.finish().await;
}

/// A runner may end a run with `Finished` and no `Started`: exit 125 is a setup failure (a short
/// or bad pack, or a refusal frames.rs has no variant for), exit 130 a cancel before start. Both
/// are recorded as ended hosts, not protocol errors.
#[tokio::test]
async fn a_run_finished_without_starting_is_failed_on_125_and_cancelled_on_130() {
    for (code, host_state) in [(125, "failed"), (130, "cancelled")] {
        let Some(f) = Fixture::new(&format!("run_unstarted_{code}"), settings()).await else {
            return;
        };
        f.push("playbooks/web.yaml", "- hosts: web\n").await;
        let run = f.runs().await[0].0;
        let _runners = f
            .attach(|_, id| FakeRunner::new(id.to_string()).default_script(Script::unstarted(code)))
            .await;
        let (state, _) = f.ended(run, Duration::from_secs(10)).await;
        assert_ne!(state, "succeeded", "exit {code}");
        let hosts = f.host_states(run).await;
        assert!(!hosts.is_empty());
        assert!(
            hosts.iter().all(|(_, s)| s == host_state),
            "exit {code}: {hosts:?}"
        );
        f.finish().await;
    }
}

#[tokio::test]
async fn logs_past_the_cap_keep_head_and_tail_with_a_marker() {
    let cap = 64;
    let Some(f) = Fixture::new(
        "run_log_cap",
        RunSettings {
            log_cap: cap,
            ..settings()
        },
    )
    .await
    else {
        return;
    };
    let _runners = f
        .attach(|name, id| {
            let mut s = Script::finish(0).stats(name, ok_stats());
            if name == "web-1" {
                for i in 0..10 {
                    s = s.log(
                        LogStream::Stdout,
                        format!("line {i:02} ........\n").into_bytes(),
                    );
                }
            }
            FakeRunner::new(id.to_string()).default_script(s)
        })
        .await;
    f.push("playbooks/web.yaml", "- hosts: web\n").await;
    let run = f.runs().await[0].0;
    f.ended(run, SC_003).await;
    let chunks: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "select seq, stream, bytes from run_log_chunks where run_id = $1 order by seq",
    )
    .bind(run)
    .fetch_all(&f.h.db.owner)
    .await
    .expect("chunks");
    let text: String = chunks
        .iter()
        .map(|c| String::from_utf8_lossy(&c.2).into_owned())
        .collect();
    let stored: usize = chunks
        .iter()
        .filter(|c| !String::from_utf8_lossy(&c.2).contains("truncated"))
        .map(|c| c.2.len())
        .sum();
    assert!(stored <= cap as usize, "{stored} bytes stored: {text:?}");
    assert!(text.starts_with("line 00"), "{text:?}");
    assert!(text.contains("bytes truncated"), "{text:?}");
    assert!(text.ends_with("line 09 ........\n"), "{text:?}");
    let marker: (i64,) = sqlx::query_as(
        "select count(*) from run_events where run_id = $1 and kind = 'log_truncated'",
    )
    .bind(run)
    .fetch_one(&f.h.db.owner)
    .await
    .expect("marker");
    assert_eq!(marker.0, 1);
    f.finish().await;
}

#[tokio::test]
async fn subscribing_from_a_sequence_number_resumes_without_gaps() {
    let Some(f) = Fixture::new("run_subscribe", settings()).await else {
        return;
    };
    let mut live = f.h.app.runs.live();
    let _runners = f
        .attach(|name, id| {
            let mut s = Script::finish(0).stats(name, ok_stats());
            for i in 0..5 {
                s = s
                    .event(format!(
                        r#"{{"v":1,"event":"ok","host":"{name}","task":"t{i}","task_id":"{i}","action":"ping","result":{{}}}}"#
                    ))
                    .log(LogStream::Stdout, format!("{name} {i}\n").into_bytes());
            }
            FakeRunner::new(id.to_string()).default_script(s)
        })
        .await;
    f.push("playbooks/web.yaml", "- hosts: web\n").await;
    let run = f.runs().await[0].0;
    f.ended(run, SC_003).await;

    let seq = |e: &Event| match e {
        Event::RunEvent { seq, .. } | Event::RunLog { seq, .. } => Some(*seq),
        _ => None,
    };
    // Everything, in order, numbered 1..n.
    let all = keel_server::runs::history(&f.h.app, run, 0)
        .await
        .expect("history");
    let seqs: Vec<u64> = all.iter().filter_map(seq).collect();
    let n = seqs.len() as u64;
    assert!(n >= 20, "{seqs:?}");
    assert_eq!(seqs, (1..=n).collect::<Vec<_>>());
    // Resuming after 7 yields 8..n.
    let rest = keel_server::runs::history(&f.h.app, run, 7)
        .await
        .expect("history");
    assert_eq!(
        rest.iter().filter_map(seq).collect::<Vec<_>>(),
        (8..=n).collect::<Vec<_>>()
    );
    // Live delivery saw the same numbers, in order.
    let mut seen = Vec::new();
    while let Ok((r, e)) = live.try_recv() {
        if r == run
            && let Some(s) = seq(&e)
        {
            seen.push(s);
        }
    }
    assert_eq!(seen, (1..=n).collect::<Vec<_>>());
    f.finish().await;
}

#[tokio::test]
async fn a_token_may_list_runs_but_never_start_or_cancel_one() {
    let Some(f) = Fixture::new("run_token", settings()).await else {
        return;
    };
    let h = &f.h;
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
    for write in [
        "preview_run_start",
        "preview_cancel",
        "preview_policy_upsert",
    ] {
        let out = h.cmd(token(&["list_runs", write])).await;
        assert_eq!(op_error(&out), "action_not_allowed", "{write}");
    }
    let (_, out) = h
        .confirm(token(&["list_runs", "get_run", "list_jobs"]))
        .await;
    let secret = match out.as_slice() {
        [_, Event::ApiTokenCreated { secret, .. }, ..] => secret.0.clone(),
        other => panic!("token: {other:?}"),
    };
    let bearer = |cmd: Cmd| {
        let body = serde_json::to_vec(&CmdEnvelope {
            id: RequestId(1),
            cmd,
        })
        .expect("json");
        Request::post("/api/v1/cmd")
            .header(header::AUTHORIZATION, format!("Bearer {secret}"))
            .body(Body::from(body))
            .expect("req")
    };
    let (status, body) = h
        .call(bearer(Cmd::ListRuns {
            workspace: h.workspace,
            binding: None,
            host: None,
            actor: None,
            state: None,
            from: None,
            to: None,
        }))
        .await;
    assert_eq!(status, StatusCode::OK);
    let envs: Vec<EventEnvelope> = serde_json::from_slice(&body).expect("events");
    assert!(
        matches!(envs.as_slice(), [e] if matches!(&e.event, Event::Runs { .. })),
        "{envs:?}"
    );
    let jobs: Vec<(Uuid,)> = sqlx::query_as("select id from jobs where binding_id = $1")
        .bind(f.binding)
        .fetch_all(&h.db.owner)
        .await
        .expect("jobs");
    for cmd in [
        Cmd::PreviewRunStart {
            job: jobs[0].0,
            limit: None,
        },
        Cmd::PreviewCancel {
            run: Uuid::now_v7(),
        },
    ] {
        let (status, body) = h.call(bearer(cmd)).await;
        assert_eq!(status, StatusCode::OK);
        let envs: Vec<EventEnvelope> = serde_json::from_slice(&body).expect("events");
        assert!(
            matches!(envs.as_slice(), [e] if matches!(&e.event, Event::Denied { reason, .. } if reason == "token_scope")),
            "{envs:?}"
        );
    }
    f.finish().await;
}

#[tokio::test]
async fn a_manual_run_previews_its_targets_and_secrets_never_reach_the_record() {
    let Some(f) = Fixture::new("run_manual", settings()).await else {
        return;
    };
    let mut runners = f
        .attach(|_, id| FakeRunner::new(id.to_string()).default_script(Script::finish(0)))
        .await;
    let jobs: Vec<(Uuid,)> = sqlx::query_as("select id from jobs where binding_id = $1")
        .bind(f.binding)
        .fetch_all(&f.h.db.owner)
        .await
        .expect("jobs");
    let limit = keel_protocol::Pattern::new("web-2").expect("limit");
    let (p, out) =
        f.h.confirm(Cmd::PreviewRunStart {
            job: jobs[0].0,
            limit: Some(limit),
        })
        .await;
    assert!(p.affected.iter().any(|a| a == "web-2"), "{p:?}");
    assert!(!p.affected.iter().any(|a| a == "web-1"), "{p:?}");
    let run = match out.as_slice() {
        [_, Event::Run { run, .. }] => run.id,
        other => panic!("{other:?}"),
    };
    assert_eq!(f.ended(run, SC_003).await.0, "succeeded");
    // web-2 got Assign, Secrets and the pack; web-1 nothing.
    let mut frames = Vec::new();
    while let Ok(fr) = runners[1].received.try_recv() {
        frames.push(fr);
    }
    assert!(
        frames
            .iter()
            .any(|f| matches!(f, Frame::Assign { job, .. } if job == "web"))
    );
    assert!(frames.iter().any(|f| matches!(f, Frame::Secrets { .. })));
    assert!(
        frames
            .iter()
            .any(|f| matches!(f, Frame::Pack { last: true, .. }))
    );
    while let Ok(fr) = runners[0].received.try_recv() {
        assert!(!matches!(fr, Frame::Assign { .. }), "{fr:?}");
    }
    let (trigger,): (String,) = sqlx::query_as("select trigger from runs where id = $1")
        .bind(run)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("run");
    assert_eq!(trigger, "manual");
    f.finish().await;
}
