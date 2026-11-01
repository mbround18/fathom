//! Run approval (spec 064): a run held by an approval policy keeps the terms it was held with;
//! approvers approve or deny it with a fresh step-up; four-eyes keeps the requester from
//! approving their own run; a held run expires when its window closes; and approvers can count
//! what waits for them.
//!
//! The fixture is a workspace with one host (`web-1`), a binding whose `web` job runs on pushes
//! under `playbooks/`, a policy that holds every run, and five people: the owner, two approvers,
//! an operator and a viewer. No runner is attached, so a queued run stays queued.

mod common;
mod repos;

use keel_protocol::{ApprovalReason, Cmd, Event, RunApproval, RunInfo, RunPhase, RunReason};
use keel_server::signin::Established;
use repos::{Harness, Source, bind, preview, sign_in};
use serde_json::Value;
use uuid::Uuid;

/// The `web` job runs on pushes under `playbooks/`.
const MANIFEST: &str = "version: 1\nprofiles: { debian: {} }\njobs:\n  web:\n    \
                        playbook: playbooks/web.yaml\n    profiles: [debian]\n    \
                        inventory: prod\n    paths: ['playbooks/**']\n";

struct Fx {
    h: Harness,
    src: Source,
    binding: Uuid,
    job: Uuid,
    policy: Uuid,
    ann: Established,
    ben: Established,
    olga: Established,
    vic: Established,
}

/// Signs in a user and gives them `role` on the workspace through a team of their own.
async fn member(h: &Harness, sub: &str, name: &str, role: &str) -> Established {
    let est = sign_in(&h.app, sub, name).await;
    let team = Uuid::now_v7();
    let owner = &h.db.owner;
    sqlx::query("insert into teams (id, name) values ($1, $2)")
        .bind(team)
        .bind(format!("team-{name}"))
        .execute(owner)
        .await
        .expect("team");
    sqlx::query("insert into team_members (team_id, user_id) values ($1, $2)")
        .bind(team)
        .bind(est.user.id)
        .execute(owner)
        .await
        .expect("member");
    sqlx::query("insert into grants (id, team_id, workspace_id, role) values ($1, $2, $3, $4)")
        .bind(Uuid::now_v7())
        .bind(team)
        .bind(h.workspace)
        .bind(role)
        .execute(owner)
        .await
        .expect("grant");
    h.step_up(&est).await;
    est
}

impl Fx {
    /// A fixture whose policy holds every run for `needed` approvals, with four-eyes or not.
    async fn new(test: &str, needed: i32, four_eyes: bool) -> Option<Self> {
        let h = Harness::new(test).await?;
        let src = Source::new(MANIFEST);
        let binding = bind(&h, &src).await;
        let owner = &h.db.owner;
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
        sqlx::query(
            "insert into inventory_hosts (revision_id, host, groups) values ($1, 'web-1', '{web}')",
        )
        .bind(revision)
        .execute(owner)
        .await
        .expect("inventory host");
        let host = Uuid::now_v7();
        sqlx::query(
            "insert into hosts (id, workspace_id, inventory_name, allowed_profiles) \
             values ($1, $2, 'web-1', '{debian}')",
        )
        .bind(host)
        .bind(h.workspace)
        .execute(owner)
        .await
        .expect("host");
        sqlx::query(
            "insert into host_profiles (host_id, profile_id) \
             select $1, id from profiles where workspace_id = $2 and name = 'debian'",
        )
        .bind(host)
        .bind(h.workspace)
        .execute(owner)
        .await
        .expect("host profile");
        let policy = Uuid::now_v7();
        sqlx::query(
            "insert into approval_policies (id, workspace_id, priority, requires_approval, \
             approvals_needed, four_eyes) values ($1, $2, 1, true, $3, $4)",
        )
        .bind(policy)
        .bind(h.workspace)
        .bind(needed)
        .bind(four_eyes)
        .execute(owner)
        .await
        .expect("policy");
        let (job,): (Uuid,) = sqlx::query_as("select id from jobs where binding_id = $1")
            .bind(binding)
            .fetch_one(owner)
            .await
            .expect("job");
        let ann = member(&h, "11", "ann", "approver").await;
        let ben = member(&h, "12", "ben", "approver").await;
        let olga = member(&h, "13", "olga", "operator").await;
        let vic = member(&h, "14", "vic", "viewer").await;
        Some(Self {
            h,
            src,
            binding,
            job,
            policy,
            ann,
            ben,
            olga,
            vic,
        })
    }

    /// Runs `cmd` as `est`; a preview is confirmed, and the confirmation's events returned.
    async fn answer(&self, est: &Established, cmd: Cmd) -> Vec<Event> {
        let out = self.h.cmd_as(est, cmd).await;
        let [Event::Preview(p)] = out.as_slice() else {
            return out;
        };
        self.h
            .cmd_as(
                est,
                Cmd::Confirm {
                    preview_id: p.preview_id,
                    step_up: None,
                },
            )
            .await
    }

    /// A manual run started by `est`; held by the policy.
    async fn start(&self, est: &Established) -> Uuid {
        let p = preview(
            &self
                .h
                .cmd_as(
                    est,
                    Cmd::PreviewRunStart {
                        job: self.job,
                        limit: None,
                    },
                )
                .await,
        );
        let out = self
            .h
            .cmd_as(
                est,
                Cmd::Confirm {
                    preview_id: p.preview_id,
                    step_up: None,
                },
            )
            .await;
        match out.as_slice() {
            [Event::Confirmed { .. }, Event::Run { run, .. }] => run.id,
            other => panic!("start: {other:?}"),
        }
    }

    /// A run started by a push that nobody asked for. A manual sync records whoever synced as
    /// the requester, so the run is then made to look as a polled push does: no requester.
    async fn push_run(&self, content: &str) -> Uuid {
        self.src.commit("playbooks/web.yaml", content, "change");
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
        let (run,): (Uuid,) = sqlx::query_as(
            "update runs set trigger = 'poll', requested_by = null where id = \
             (select id from runs order by created_at desc limit 1) returning id",
        )
        .fetch_one(&self.h.db.owner)
        .await
        .expect("push run");
        run
    }

    async fn state(&self, run: Uuid) -> (String, Option<String>) {
        sqlx::query_as("select state, state_reason from runs where id = $1")
            .bind(run)
            .fetch_one(&self.h.db.owner)
            .await
            .expect("run")
    }

    async fn info(&self, run: Uuid) -> RunInfo {
        match self.h.cmd(Cmd::GetRun { run }).await.as_slice() {
            [Event::Run { run, .. }] => run.clone(),
            other => panic!("get run: {other:?}"),
        }
    }

    /// `(action, decision, reason, context)` of the audit entries targeting `run`.
    async fn audit(&self, run: Uuid) -> Vec<(String, String, Option<String>, Value)> {
        sqlx::query_as(
            "select action, decision, reason, context from audit_event \
             where target ->> 'run' = $1::text order by seq",
        )
        .bind(run)
        .fetch_all(&self.h.db.owner)
        .await
        .expect("audit")
    }

    async fn finish(self) {
        self.h.finish().await;
    }
}

/// The OpError code, or the Denied reason, at the end of `out`.
fn refusal(out: &[Event]) -> String {
    match out.last() {
        Some(Event::OpError { code, .. }) => code.clone(),
        Some(Event::Denied { reason, .. }) => format!("denied:{reason}"),
        other => panic!("expected a refusal, got {out:?} ({other:?})"),
    }
}

/// The run in a confirmation's reply.
fn replied(out: &[Event]) -> RunInfo {
    match out {
        [Event::Confirmed { .. }, Event::Run { run, .. }] => run.clone(),
        other => panic!("expected the run, got {other:?}"),
    }
}

// ---- held terms (T007, T008) -----------------------------------------------------------------

#[tokio::test]
async fn a_held_run_keeps_its_terms_whatever_happens_to_the_policy() {
    let Some(f) = Fx::new("approval_held_terms", 2, true).await else {
        return;
    };
    let run = f.start(&f.olga).await;
    assert_eq!(f.state(run).await.0, "pending_approval");
    let terms = |f: &Fx| {
        let pool = f.h.db.owner.clone();
        async move {
            let row: (i32, bool, i64) = sqlx::query_as(
                "select approvals_needed, four_eyes, \
                 extract(epoch from approval_expires_at - created_at)::bigint from runs where id = $1",
            )
            .bind(run)
            .fetch_one(&pool)
            .await
            .expect("terms");
            row
        }
    };
    let held = terms(&f).await;
    assert_eq!((held.0, held.1), (2, true));
    assert!((86_399..=86_401).contains(&held.2), "{held:?}");

    // Editing the policy, then removing it, leaves the run's terms as they were (FR-009).
    sqlx::query(
        "update approval_policies set approvals_needed = 1, four_eyes = false, \
         approval_ttl = interval '1 hour' where id = $1",
    )
    .bind(f.policy)
    .execute(&f.h.db.owner)
    .await
    .expect("edit policy");
    assert_eq!(terms(&f).await, held);
    sqlx::query("delete from approval_policies where id = $1")
        .bind(f.policy)
        .execute(&f.h.db.owner)
        .await
        .expect("remove policy");
    assert_eq!(terms(&f).await, held);

    // RunInfo carries the held terms.
    let info = f.info(run).await;
    let a: RunApproval = info.approval.expect("approval");
    assert_eq!((a.needed, a.four_eyes), (2, true));
    assert!(a.approvals.is_empty() && a.denial.is_none());
    assert_eq!(info.state, RunPhase::PendingApproval);
    f.finish().await;
}

#[tokio::test]
async fn a_run_that_was_never_held_has_no_approval() {
    let Some(f) = Fx::new("approval_never_held", 1, false).await else {
        return;
    };
    sqlx::query("update approval_policies set requires_approval = false where id = $1")
        .bind(f.policy)
        .execute(&f.h.db.owner)
        .await
        .expect("policy");
    let run = f.start(&f.olga).await;
    assert_eq!(f.state(run).await.0, "queued");
    assert_eq!(f.info(run).await.approval, None);
    f.finish().await;
}

// ---- US1: approve (T012) ---------------------------------------------------------------------

fn approve(run: Uuid) -> Cmd {
    Cmd::PreviewRunApprove { run }
}

fn actions(audit: &[(String, String, Option<String>, Value)]) -> Vec<&str> {
    audit.iter().map(|(a, ..)| a.as_str()).collect()
}

#[tokio::test]
async fn one_approval_queues_a_run_needing_one() {
    let Some(f) = Fx::new("approval_one_of_one", 1, false).await else {
        return;
    };
    let run = f.start(&f.olga).await;
    // The preview says what the answer does.
    let out = f.h.cmd_as(&f.ann, approve(run)).await;
    let [Event::Preview(p)] = out.as_slice() else {
        panic!("{out:?}");
    };
    assert!(p.summary.contains("queues the run"), "{}", p.summary);
    assert!(p.step_up);
    let info = replied(&f.answer(&f.ann, approve(run)).await);
    assert_eq!(info.state, RunPhase::Queued);
    let a = info.approval.expect("approval");
    assert_eq!(a.approvals.len(), 1);
    assert_eq!(a.approvals[0].user, f.ann.user.id);
    assert_eq!(a.approvals[0].role.as_deref(), Some("approver"));
    let (queued,): (bool,) = sqlx::query_as("select queued_at is not null from runs where id = $1")
        .bind(run)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("queued_at");
    assert!(queued);
    let audit = f.audit(run).await;
    let approved = audit
        .iter()
        .find(|(a, ..)| a == "run_approved")
        .expect("run_approved");
    assert_eq!(approved.1, "allowed");
    assert_eq!(approved.3["role"], "approver");
    assert_eq!(approved.3["approvals"], 1);
    assert_eq!(approved.3["needed"], 1);
    assert_eq!(approved.3["policy"], f.policy.to_string());
    assert!(actions(&audit).contains(&"run_approval_queued"));
    // A second answer finds it decided.
    assert_eq!(
        refusal(&f.answer(&f.ben, approve(run)).await),
        "run_answered"
    );
    f.finish().await;
}

#[tokio::test]
async fn two_approvals_are_needed_when_the_policy_says_two() {
    let Some(f) = Fx::new("approval_two_of_two", 2, false).await else {
        return;
    };
    let run = f.start(&f.olga).await;
    let out = f.h.cmd_as(&f.ann, approve(run)).await;
    let [Event::Preview(p)] = out.as_slice() else {
        panic!("{out:?}");
    };
    assert!(p.summary.contains("approval 1 of 2"), "{}", p.summary);
    let info = replied(&f.answer(&f.ann, approve(run)).await);
    assert_eq!(info.state, RunPhase::PendingApproval);
    assert_eq!(info.approval.expect("approval").approvals.len(), 1);
    let info = replied(&f.answer(&f.ben, approve(run)).await);
    assert_eq!(info.state, RunPhase::Queued);
    assert_eq!(info.approval.expect("approval").approvals.len(), 2);
    let audit = f.audit(run).await;
    let counts: Vec<&Value> = audit
        .iter()
        .filter(|(a, ..)| a == "run_approved")
        .map(|(.., c)| &c["approvals"])
        .collect();
    assert_eq!(counts, [&Value::from(1), &Value::from(2)]);
    f.finish().await;
}

#[tokio::test]
async fn four_eyes_keeps_the_requester_from_approving_their_own_run() {
    let Some(f) = Fx::new("approval_four_eyes", 1, true).await else {
        return;
    };
    // The owner may approve, but not a run they asked for themselves.
    let run = f.start(&f.h.owner).await;
    assert_eq!(
        refusal(&f.answer(&f.h.owner, approve(run)).await),
        "four_eyes_requester"
    );
    assert_eq!(f.state(run).await.0, "pending_approval");
    let audit = f.audit(run).await;
    let refused = audit
        .iter()
        .find(|(a, ..)| a == "run_approval_refused")
        .expect("refusal audited");
    assert_eq!(refused.1, "denied");
    assert_eq!(refused.2.as_deref(), Some("four_eyes_requester"));
    assert_eq!(refused.3["state"], "pending_approval");
    // Someone else may.
    assert_eq!(
        replied(&f.answer(&f.ann, approve(run)).await).state,
        RunPhase::Queued
    );
    f.finish().await;
}

#[tokio::test]
async fn four_eyes_lets_anyone_approve_a_run_nobody_requested() {
    let Some(f) = Fx::new("approval_four_eyes_push", 1, true).await else {
        return;
    };
    let run = f.push_run("- hosts: web\n").await;
    assert_eq!(f.state(run).await.0, "pending_approval");
    assert_eq!(
        replied(&f.answer(&f.h.owner, approve(run)).await).state,
        RunPhase::Queued
    );
    f.finish().await;
}

#[tokio::test]
async fn the_same_approver_counts_once() {
    let Some(f) = Fx::new("approval_counts_once", 2, false).await else {
        return;
    };
    let run = f.start(&f.olga).await;
    replied(&f.answer(&f.ann, approve(run)).await);
    assert_eq!(
        refusal(&f.answer(&f.ann, approve(run)).await),
        "already_answered"
    );
    assert_eq!(f.state(run).await.0, "pending_approval");
    let (n,): (i64,) = sqlx::query_as("select count(*) from approvals where run_id = $1")
        .bind(run)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("count");
    assert_eq!(n, 1);
    f.finish().await;
}

#[tokio::test]
async fn operators_and_viewers_cannot_approve() {
    let Some(f) = Fx::new("approval_needs_grant", 1, false).await else {
        return;
    };
    let run = f.start(&f.olga).await;
    assert_eq!(
        refusal(&f.h.cmd_as(&f.olga, approve(run)).await),
        "denied:no_grant"
    );
    assert_eq!(
        refusal(&f.h.cmd_as(&f.vic, approve(run)).await),
        "denied:no_grant"
    );
    assert_eq!(f.state(run).await.0, "pending_approval");
    // An unknown run is not found.
    assert_eq!(
        refusal(&f.h.cmd_as(&f.ann, approve(Uuid::now_v7())).await),
        "not_found"
    );
    f.finish().await;
}

#[tokio::test]
async fn approving_needs_a_fresh_step_up() {
    let Some(f) = Fx::new("approval_step_up", 1, false).await else {
        return;
    };
    let run = f.start(&f.olga).await;
    sqlx::query("update sessions set verified_at = null where id = $1")
        .bind(&f.ann.session)
        .execute(&f.h.db.owner)
        .await
        .expect("stale");
    // Ann has no passkey on record, so she is sent to enrol one; either way nothing is written.
    let out = f.answer(&f.ann, approve(run)).await;
    assert!(
        matches!(out.as_slice(), [Event::PasskeyEnrolmentRequired])
            || refusal(&out) == "step_up_required",
        "{out:?}"
    );
    let (n,): (i64,) = sqlx::query_as("select count(*) from approvals where run_id = $1")
        .bind(run)
        .fetch_one(&f.h.db.owner)
        .await
        .expect("count");
    assert_eq!(n, 0);
    assert_eq!(f.state(run).await.0, "pending_approval");
    f.finish().await;
}

// ---- US2: deny (T016) ------------------------------------------------------------------------

fn deny(run: Uuid, reason: Option<&str>) -> Cmd {
    Cmd::PreviewRunDeny {
        run,
        reason: reason.map(|r| ApprovalReason::new(r).expect("reason")),
    }
}

impl Fx {
    /// The states of `run`'s hosts.
    async fn host_states(&self, run: Uuid) -> Vec<String> {
        let rows: Vec<(String,)> =
            sqlx::query_as("select state from run_hosts where run_id = $1 order by host_name")
                .bind(run)
                .fetch_all(&self.h.db.owner)
                .await
                .expect("run hosts");
        rows.into_iter().map(|(s,)| s).collect()
    }
}

#[tokio::test]
async fn a_denied_run_ends_with_its_reason() {
    let Some(f) = Fx::new("approval_deny", 1, false).await else {
        return;
    };
    let run = f.start(&f.olga).await;
    assert_eq!(f.host_states(run).await, ["queued"]);
    let info = replied(
        &f.answer(
            &f.ann,
            deny(run, Some("  Not during the freeze.\nRetry Monday.  ")),
        )
        .await,
    );
    assert_eq!(info.state, RunPhase::Denied);
    assert_eq!(info.reason, Some(RunReason::ApprovalDenied));
    assert!(info.ended_at.is_some());
    let denial = info.approval.expect("approval").denial.expect("denial");
    assert_eq!(denial.user, f.ann.user.id);
    assert_eq!(
        denial.reason.as_deref(),
        Some("Not during the freeze.\nRetry Monday.")
    );
    assert_eq!(
        f.state(run).await,
        ("denied".to_owned(), Some("approval_denied".to_owned()))
    );
    assert_eq!(f.host_states(run).await, ["cancelled"]);
    let audit = f.audit(run).await;
    let denied = audit
        .iter()
        .find(|(a, ..)| a == "run_denied")
        .expect("run_denied");
    assert_eq!(denied.1, "allowed");
    assert_eq!(denied.3["reason"], "Not during the freeze.\nRetry Monday.");
    // Nothing more can be said about it.
    assert_eq!(
        refusal(&f.answer(&f.ben, approve(run)).await),
        "run_answered"
    );
    assert_eq!(
        refusal(&f.answer(&f.ben, deny(run, None)).await),
        "run_answered"
    );
    f.finish().await;
}

#[tokio::test]
async fn one_denial_ends_a_run_part_way_approved() {
    let Some(f) = Fx::new("approval_deny_after_one", 2, false).await else {
        return;
    };
    let run = f.start(&f.olga).await;
    replied(&f.answer(&f.ann, approve(run)).await);
    let info = replied(&f.answer(&f.ben, deny(run, None)).await);
    assert_eq!(info.state, RunPhase::Denied);
    let a = info.approval.expect("approval");
    assert_eq!(a.approvals.len(), 1);
    assert_eq!(a.denial.expect("denial").reason, None);
    f.finish().await;
}

#[tokio::test]
async fn the_requester_may_deny_under_four_eyes() {
    let Some(f) = Fx::new("approval_deny_four_eyes", 1, true).await else {
        return;
    };
    let run = f.start(&f.h.owner).await;
    let info = replied(&f.answer(&f.h.owner, deny(run, Some("Wrong commit."))).await);
    assert_eq!(info.state, RunPhase::Denied);
    // Operators and viewers can't.
    let run = f.start(&f.olga).await;
    assert_eq!(
        refusal(&f.h.cmd_as(&f.olga, deny(run, None)).await),
        "denied:no_grant"
    );
    assert_eq!(
        refusal(&f.h.cmd_as(&f.vic, deny(run, None)).await),
        "denied:no_grant"
    );
    f.finish().await;
}

#[test]
fn a_reason_over_500_characters_is_refused() {
    assert!(ApprovalReason::new("x".repeat(500)).is_ok());
    assert!(ApprovalReason::new("x".repeat(501)).is_err());
}

#[tokio::test]
async fn an_approve_and_a_deny_racing_decide_the_run_once() {
    let Some(f) = Fx::new("approval_race", 2, false).await else {
        return;
    };
    for _ in 0..20 {
        let run = f.start(&f.olga).await;
        replied(&f.answer(&f.ann, approve(run)).await);
        let previewed = |out: Vec<Event>| match out.as_slice() {
            [Event::Preview(p)] => p.preview_id,
            other => panic!("preview: {other:?}"),
        };
        let yes = previewed(f.h.cmd_as(&f.ben, approve(run)).await);
        let no = previewed(f.h.cmd_as(&f.h.owner, deny(run, None)).await);
        let confirm = |preview_id| Cmd::Confirm {
            preview_id,
            step_up: None,
        };
        let (a, d) = tokio::join!(
            f.h.cmd_as(&f.ben, confirm(yes)),
            f.h.cmd_as(&f.h.owner, confirm(no))
        );
        let decided = |out: &[Event]| matches!(out, [Event::Confirmed { .. }, Event::Run { .. }]);
        let (state, _) = f.state(run).await;
        match (decided(&a), decided(&d)) {
            (true, false) => {
                assert_eq!(state, "queued");
                assert_eq!(refusal(&d), "run_answered");
            }
            (false, true) => {
                assert_eq!(state, "denied");
                assert_eq!(refusal(&a), "run_answered");
            }
            other => panic!("{other:?}: {a:?} / {d:?}"),
        }
        let audit = f.audit(run).await;
        let acts = actions(&audit);
        assert!(acts.contains(&"run_approval_refused"), "{acts:?}");
        assert_eq!(
            acts.iter()
                .filter(|a| **a == "run_approved" || **a == "run_denied")
                .count(),
            2,
            "{acts:?}"
        );
    }
    f.finish().await;
}
