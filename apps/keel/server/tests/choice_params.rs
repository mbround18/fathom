//! Spec 068 US8 (FR-014, T064, T065): `choice` params. `ListJobs` shows each param's allowed
//! values at the binding's current commit (from a `values` list or the directories under
//! `from`); `PreviewRunStart` refuses any other value, or an undeclared param, before approval;
//! and an accepted run carries its params and check flag to the runner.

mod common;
mod converge_support;
mod repos;

use std::collections::BTreeMap;

use converge_support::Fixture;
use keel_exec::fake::{FakeRunner, Script};
use keel_exec::frames::Frame;
use keel_protocol::{Cmd, Event, JobInfo, ParamView, Pattern};

const JOBS: &str = include_str!("fixtures/converge-repo/.keel/jobs.yaml");

const PROVISION: &str = "
  provision:
    playbook: playbooks/provision.yaml
    profiles: [debian]
    inventory: prod
    params:
      stack: { kind: choice, from: composes/, default: web }
      size: { kind: choice, values: [small, large] }
";

/// Adds the `provision` job and two stacks (`web`, `db`) plus a hidden directory; syncs. Gives
/// `node-1` the `debian` profile, which `provision` runs under.
async fn add_provision(f: &Fixture) {
    sqlx::query(
        "insert into host_profiles (host_id, profile_id) \
         select $1, id from profiles where workspace_id = $2 and name = 'debian' \
         on conflict do nothing",
    )
    .bind(f.node)
    .bind(f.h.workspace)
    .execute(&f.h.db.owner)
    .await
    .expect("debian");
    f.src
        .commit("playbooks/provision.yaml", "- hosts: all\n", "provision");
    for dir in ["web", "db", ".hidden"] {
        f.src.commit(
            &format!("composes/{dir}/compose.yaml"),
            "services: {}\n",
            dir,
        );
    }
    f.src
        .commit(".keel/jobs.yaml", &format!("{JOBS}{PROVISION}"), "jobs");
    f.src.push();
    sync(f).await;
}

async fn sync(f: &Fixture) {
    let out = f.h.cmd(Cmd::SyncBinding { binding: f.binding }).await;
    assert!(
        matches!(out.as_slice(), [Event::BindingSynced { .. }]),
        "{out:?}"
    );
}

async fn provision(f: &Fixture) -> JobInfo {
    match f
        .h
        .cmd(Cmd::ListJobs { binding: f.binding })
        .await
        .as_slice()
    {
        [Event::Jobs { items, .. }] => items
            .iter()
            .find(|j| j.name == "provision")
            .expect("provision listed")
            .clone(),
        other => panic!("list jobs: {other:?}"),
    }
}

fn choice(allowed: &[&str], default: Option<&str>) -> ParamView {
    ParamView {
        kind: "choice".to_owned(),
        allowed: Some(allowed.iter().map(|v| (*v).to_owned()).collect()),
        default: default.map(str::to_owned),
        unavailable: None,
    }
}

fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn start(job: &JobInfo, p: &[(&str, &str)], check: bool) -> Cmd {
    Cmd::PreviewRunStart {
        job: job.id,
        limit: Some(Pattern::new("node-1").expect("limit")),
        params: params(p),
        check,
    }
}

/// Previews a start that must be refused with `param_invalid`; returns the message.
async fn refused(f: &Fixture, cmd: Cmd) -> String {
    match f.h.cmd(cmd).await.as_slice() {
        [Event::OpError { code, message }] => {
            assert_eq!(code, "param_invalid", "{message}");
            message.clone()
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

async fn provision_runs(f: &Fixture) -> i64 {
    sqlx::query_scalar("select count(*) from runs where job_name = 'provision'")
        .fetch_one(&f.h.db.owner)
        .await
        .expect("runs")
}

#[tokio::test]
async fn allowed_values_are_listed_at_the_current_commit() {
    let Some(f) = Fixture::new("choice_list", false).await else {
        return;
    };
    add_provision(&f).await;
    let job = provision(&f).await;
    assert_eq!(
        job.params,
        Some(BTreeMap::from([
            ("size".to_owned(), choice(&["small", "large"], None)),
            // Directory names in name order; hidden ones are not choices.
            ("stack".to_owned(), choice(&["db", "web"], Some("web"))),
        ]))
    );
    // A job without params lists none.
    match f
        .h
        .cmd(Cmd::ListJobs { binding: f.binding })
        .await
        .as_slice()
    {
        [Event::Jobs { items, .. }] => {
            let debian = items.iter().find(|j| j.name == "debian").expect("debian");
            assert_eq!(debian.params, None);
        }
        other => panic!("list jobs: {other:?}"),
    }
    // A new stack directory is a choice once the binding syncs it.
    f.src
        .commit("composes/cache/compose.yaml", "services: {}\n", "cache");
    f.src.push();
    sync(&f).await;
    let job = provision(&f).await;
    assert_eq!(
        job.params.expect("params")["stack"],
        choice(&["cache", "db", "web"], Some("web"))
    );
    f.h.finish().await;
}

#[tokio::test]
async fn values_outside_the_choices_are_refused_before_approval() {
    // Approval held: a start that got past the checks would leave a pending run.
    let Some(f) = Fixture::new("choice_refuse", true).await else {
        return;
    };
    add_provision(&f).await;
    let job = provision(&f).await;
    // `size` has no default, so it is required (T074); these all set it.
    let size = ("size", "small");
    // Not a directory under composes/ at the commit.
    let m = refused(&f, start(&job, &[("stack", "cache"), size], false)).await;
    assert!(m.contains("param stack"), "{m}");
    assert!(
        !m.contains("cache"),
        "the refusal never echoes the value: {m}"
    );
    // Hidden directories are not choices; nor is a value of the wrong shape.
    refused(&f, start(&job, &[("stack", ".hidden"), size], false)).await;
    refused(&f, start(&job, &[("stack", "../web"), size], false)).await;
    // Not in the `values` list.
    refused(&f, start(&job, &[("size", "huge")], false)).await;
    // Not a declared param.
    let m = refused(&f, start(&job, &[("region", "eu"), size], false)).await;
    assert!(m.contains("does not declare the param region"), "{m}");
    assert_eq!(provision_runs(&f).await, 0, "no refused start made a run");
    // A listed value previews and waits for approval; `stack` falls back to its default.
    let out = f.h.cmd(start(&job, &[("stack", "db"), size], false)).await;
    assert!(matches!(out.as_slice(), [Event::Preview(_)]), "{out:?}");
    let out = f.h.cmd(start(&job, &[size], false)).await;
    assert!(matches!(out.as_slice(), [Event::Preview(_)]), "{out:?}");
    f.h.finish().await;
}

/// T074 without a database: `check_params`, which every run creation goes through (push,
/// baseline, update, Start), refuses a required param left out, naming it and no value.
#[test]
fn check_params_refuses_a_missing_required_choice() {
    let def: keel_core::manifest::JobDefinition = serde_json::from_value(serde_json::json!({
        "playbook": "playbooks/provision.yaml",
        "profiles": ["debian"],
        "inventory": "prod",
        "inventory_source": "repo",
        "limit": null,
        "vars": {},
        "credentials": [],
        "paths": null,
        "timeout_secs": null,
        "rollback": null,
        "params": {
            "stack": { "kind": "choice", "values": ["web", "db"], "default": "web" },
            "size": { "kind": "choice", "values": ["small", "large"] },
            "image": { "kind": "oci_digest" },
        },
    }))
    .expect("definition");
    let check = |p: &[(&str, &str)]| keel_server::runs::check_params(&def, &params(p));
    assert_eq!(check(&[]), Err("param size is required".to_owned()));
    assert_eq!(
        check(&[("stack", "db")]),
        Err("param size is required".to_owned())
    );
    // The digest and the defaulted choice stay optional.
    assert_eq!(check(&[("size", "small")]), Ok(()));
}

/// T074: a `choice` with no default (`size`) is required. A start without it is refused before
/// approval, naming the param; the job is manual-only, so a converge skips it.
#[tokio::test]
async fn a_required_choice_left_out_is_refused() {
    let Some(f) = Fixture::new("choice_required", true).await else {
        return;
    };
    add_provision(&f).await;
    let job = provision(&f).await;
    let m = refused(&f, start(&job, &[], false)).await;
    assert_eq!(m, "The run's param size is required.");
    let m = refused(&f, start(&job, &[("stack", "web")], true)).await;
    assert_eq!(m, "The run's param size is required.");
    assert_eq!(provision_runs(&f).await, 0, "no refused start made a run");
    f.h.finish().await;
}

/// T074: a profile without a recipe reaches the manual-only `provision`; a converge skips it
/// and the preview says so, without refusing the host.
#[tokio::test]
async fn a_converge_skips_a_manual_only_job_and_says_so() {
    let Some(f) = Fixture::new("choice_converge_skip", false).await else {
        return;
    };
    let jobs = JOBS.replacen("profiles:\n", "profiles:\n  stacks: {}\n", 1);
    let provision = PROVISION.replace("profiles: [debian]", "profiles: [debian, stacks]");
    f.src
        .commit("playbooks/provision.yaml", "- hosts: all\n", "provision");
    f.src
        .commit("composes/web/compose.yaml", "services: {}\n", "web");
    f.src
        .commit(".keel/jobs.yaml", &format!("{jobs}{provision}"), "jobs");
    f.src.push();
    sync(&f).await;
    sqlx::query(
        "insert into host_profiles (host_id, profile_id) \
         select $1, id from profiles where workspace_id = $2 and name = 'stacks' \
         on conflict do nothing",
    )
    .bind(f.node)
    .bind(f.h.workspace)
    .execute(&f.h.db.owner)
    .await
    .expect("stacks");
    let (_, plan) = f.plan(Cmd::PreviewConverge(f.target())).await;
    let node = plan
        .hosts
        .iter()
        .find(|h| h.host_id == f.node)
        .expect("node planned");
    assert!(node.steps.iter().all(|s| s.job != "provision"), "{node:?}");
    assert!(!node.steps.is_empty(), "the host's other steps still run");
    assert!(node.refusals.is_empty(), "{node:?}");
    assert_eq!(
        node.skipped,
        vec![keel_protocol::StepSkip {
            job: "provision".to_owned(),
            reason: keel_protocol::StepSkipReason::NeedsValue,
        }]
    );
    f.h.finish().await;
}

#[tokio::test]
async fn an_accepted_start_carries_its_params_and_check_to_the_runner() {
    let Some(f) = Fixture::new("choice_run", false).await else {
        return;
    };
    let mut runners = f
        .attach(|_, id| FakeRunner::new(id.to_string()).default_script(Script::finish(0)))
        .await;
    add_provision(&f).await;
    let job = provision(&f).await;
    let (p, out) =
        f.h.confirm(start(&job, &[("stack", "web"), ("size", "small")], true))
            .await;
    assert!(p.summary.starts_with("Check "), "{}", p.summary);
    assert!(
        p.summary.contains("Params: size=small, stack=web."),
        "{}",
        p.summary
    );
    let run = match out.as_slice() {
        [_, Event::Run { run, .. }] => run.clone(),
        other => panic!("run start: {other:?}"),
    };
    assert!(run.check);
    assert_eq!(run.params, params(&[("size", "small"), ("stack", "web")]));
    assert_eq!(f.ended(run.id).await.0, "succeeded");
    let mut assigned = Vec::new();
    while let Ok(frame) = runners[0].received.try_recv() {
        if let Frame::Assign { params, check, .. } = frame {
            assigned.push((params, check));
        }
    }
    assert_eq!(
        assigned,
        vec![(params(&[("size", "small"), ("stack", "web")]), true)]
    );
    f.h.finish().await;
}
