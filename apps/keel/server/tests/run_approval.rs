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

use keel_protocol::{Cmd, Event, RunApproval, RunInfo, RunPhase};
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
                    preview_id: p.preview_id.clone(),
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

    /// A run started by a push: it has no requester.
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
        let (run,): (Uuid,) =
            sqlx::query_as("select id from runs where requested_by is null order by created_at desc limit 1")
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
