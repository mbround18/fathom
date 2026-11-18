//! Shared support for the spec 068 converge tests: a converge-enabled server bound to the
//! `converge-repo` fixture, a managed inventory `prod` with `node-1` (keel-node) and
//! `master-1` (k3s-master), and helpers to plan, confirm and watch converges.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use keel_exec::fake::{FakeHandle, FakeRunner};
use keel_exec::frames::CAP_MANIFEST_V068;
use keel_protocol::{Cmd, ConvergeTarget, Event, PreviewId, TargetSelection};
use keel_server::config::Config;
use keel_server::runs::{RunSettings, Runs};
use uuid::Uuid;

use crate::repos::{self, Harness};

pub const WITHIN: Duration = Duration::from_secs(20);

pub struct Fixture {
    pub h: Harness,
    pub binding: Uuid,
    /// `node-1` (keel-node) and `master-1` (k3s-master).
    pub node: Uuid,
    pub master: Uuid,
}

impl Fixture {
    pub async fn new(test: &str, approval: bool) -> Option<Self> {
        let mut h = Harness::new(test).await?;
        h.app.runs = Runs::new(RunSettings {
            ping_interval: Duration::from_millis(50),
            flush_interval: Duration::from_millis(20),
            ..RunSettings::default()
        });
        h.app.cfg = Arc::new(Config {
            converge: true,
            ..(*h.app.cfg).clone()
        });
        let (_src, binding) = repos::bind_converge_repo(&h).await;
        let owner = &h.db.owner;
        let inventory = Uuid::now_v7();
        sqlx::query(
            "insert into inventories (id, workspace_id, name, source) \
             values ($1, $2, 'prod', 'managed')",
        )
        .bind(inventory)
        .bind(h.workspace)
        .execute(owner)
        .await
        .expect("inventory");
        let revision = Uuid::now_v7();
        sqlx::query(
            "insert into inventory_revisions (id, inventory_id, seq, content, content_hash, \
             status) values ($1, $2, 1, '{}', 'h1', 'valid')",
        )
        .bind(revision)
        .bind(inventory)
        .execute(owner)
        .await
        .expect("revision");
        let mut ids = Vec::new();
        for (name, profile) in [("node-1", "keel-node"), ("master-1", "k3s-master")] {
            sqlx::query(
                "insert into inventory_hosts (revision_id, host, groups) values ($1, $2, '{}')",
            )
            .bind(revision)
            .bind(name)
            .execute(owner)
            .await
            .expect("inventory host");
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into hosts (id, workspace_id, inventory_name, allowed_profiles) \
                 values ($1, $2, $3, array['debian', $4])",
            )
            .bind(id)
            .bind(h.workspace)
            .bind(name)
            .bind(profile)
            .execute(owner)
            .await
            .expect("host");
            sqlx::query(
                "insert into host_profiles (host_id, profile_id) \
                 select $1, id from profiles where workspace_id = $2 and name = $3",
            )
            .bind(id)
            .bind(h.workspace)
            .bind(profile)
            .execute(owner)
            .await
            .expect("host profile");
            ids.push(id);
        }
        if !approval {
            sqlx::query(
                "insert into approval_policies (id, workspace_id, priority, requires_approval) \
                 values ($1, $2, 1, false)",
            )
            .bind(Uuid::now_v7())
            .bind(h.workspace)
            .execute(owner)
            .await
            .expect("policy");
        }
        Some(Self {
            h,
            binding,
            node: ids[0],
            master: ids[1],
        })
    }

    /// Connects a 068 runner for each host, built by `runner`.
    pub async fn attach(&self, runner: impl Fn(&str, Uuid) -> FakeRunner) -> Vec<FakeHandle> {
        let mut out = Vec::new();
        for (name, id) in [("node-1", self.node), ("master-1", self.master)] {
            let (stream, handle) = runner(name, id)
                .with_capabilities([CAP_MANIFEST_V068])
                .spawn();
            keel_server::queue::attach(self.h.app.clone(), id, stream)
                .await
                .expect("attach");
            out.push(handle);
        }
        out
    }

    pub fn target(&self) -> ConvergeTarget {
        ConvergeTarget {
            binding_id: self.binding,
            hosts: TargetSelection::All,
            check: false,
        }
    }

    /// Previews `cmd`, returns the plan's token after checking it answered a plan.
    pub async fn plan(&self, cmd: Cmd) -> (PreviewId, keel_protocol::ConvergePlan) {
        let out = self.h.cmd(cmd).await;
        match out.as_slice() {
            [
                Event::ConvergePlan {
                    preview_token,
                    plan,
                },
                Event::Preview(p),
            ] => {
                assert_eq!(*preview_token, p.preview_id);
                (*preview_token, plan.clone())
            }
            other => panic!("expected a plan, got {other:?}"),
        }
    }

    /// Previews and confirms a converge of every host; returns the parent run.
    pub async fn start(&self) -> Uuid {
        let (token, _) = self.plan(Cmd::PreviewConverge(self.target())).await;
        self.confirm(token).await
    }

    pub async fn confirm(&self, token: PreviewId) -> Uuid {
        let out = self
            .h
            .cmd(Cmd::ConfirmConverge {
                preview_token: token,
                step_up: None,
            })
            .await;
        match out.as_slice() {
            [Event::Confirmed { .. }, Event::ConvergeStarted { run_id }] => *run_id,
            other => panic!("confirm: {other:?}"),
        }
    }

    pub async fn ended(&self, run: Uuid) -> (String, Option<String>) {
        let start = Instant::now();
        loop {
            let (state, reason, ended): (String, Option<String>, bool) = sqlx::query_as(
                "select state, state_reason, ended_at is not null from runs where id = $1",
            )
            .bind(run)
            .fetch_one(&self.h.db.owner)
            .await
            .expect("run");
            if ended {
                return (state, reason);
            }
            assert!(start.elapsed() < WITHIN, "run {run} still {state}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Each child step as (host, position, job, run_hosts state, skip reason), in order.
    pub async fn steps(&self, parent: Uuid) -> Vec<(String, i16, String, String, Option<String>)> {
        sqlx::query_as(
            "select h.inventory_name, s.position, s.job_name, rh.state, rh.skip_reason \
             from converge_steps s join hosts h on h.id = s.host_id \
             join run_hosts rh on rh.run_id = s.step_run_id and rh.host_id = s.host_id \
             where s.run_id = $1 order by h.inventory_name, s.position",
        )
        .bind(parent)
        .fetch_all(&self.h.db.owner)
        .await
        .expect("steps")
    }

    pub async fn converge(&self, run: Uuid) -> keel_protocol::ConvergeView {
        match self
            .h
            .cmd(Cmd::GetConverge { run_id: run })
            .await
            .as_slice()
        {
            [Event::Converge(view)] => view.clone(),
            other => panic!("get converge: {other:?}"),
        }
    }
}

pub fn row(
    host: &str,
    pos: i16,
    job: &str,
    state: &str,
    skip: Option<&str>,
) -> (String, i16, String, String, Option<String>) {
    (
        host.into(),
        pos,
        job.into(),
        state.into(),
        skip.map(Into::into),
    )
}
