//! Converge (spec 068 US1): bring hosts to what their assigned profiles say.
//!
//! A converge is one parent run (`kind = 'converge'`, no job) and one child run per distinct job
//! of its plan (`parent_run_id`). Each host walks its steps in order: its first step's
//! `run_hosts` row is `queued` once the parent is approved, the rest wait `blocked`, and the
//! `converge_advance` trigger (migration 0019) moves the next one when a step ends. A failed step
//! skips that host's later steps; other hosts carry on. While a host's step is out, the converge
//! holds the host (`converge_hosts.state = 'holding'`) and dispatch sends it nothing else
//! ([`crate::queue`]).
//!
//! One approval covers the whole plan, held with the strictest terms of its steps' policies. A
//! newer converge replaces a held one on the same hosts (`converge.replace`); a retry re-plans the
//! hosts that did not succeed and is approved again (`converge.retry`).
//!
//! Every command answers `feature_disabled` unless `KEEL_CONVERGE` is on.

use std::collections::{BTreeMap, BTreeSet};

use keel_core::WorkspaceId;
use keel_core::authz::Capability;
use keel_core::converge as core;
use keel_core::manifest::Manifest;
use keel_core::policy::{self, Policy};
use keel_core::ports::CommitId;
use keel_core::run::{RunState, RunStatus, StateReason};
use keel_git::Mirror;
use keel_protocol::{
    ApprovalTerms, Cmd, ConvergeHostState, ConvergeHostView, ConvergePlan, ConvergeStepView,
    ConvergeTarget, ConvergeView, Event, HostPlan, PlannedStep, RunSummary, StepRefusal,
    StepRefusalReason, StepState, TargetSelection,
};
use serde_json::json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use tracing::Instrument;
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::bindings::{binding_workspace, db_failed, enum_from, not_found, op_error, require, ts};
use crate::dispatch::{PendingOp, Request};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::serve::AppState;

const PREVIEW: &str = "preview_converge";
const RETRY: &str = "preview_converge_retry";

/// Routes the converge commands.
pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    if !app.cfg.converge {
        return op_error(
            "feature_disabled",
            "Converge is not enabled on this server.",
        );
    }
    match cmd {
        Cmd::PreviewConverge(target) => preview(app, req, target).await,
        Cmd::PreviewConvergeRetry { run_id, replan } => {
            preview_retry(app, req, run_id, replan).await
        }
        Cmd::ConfirmConverge {
            preview_token,
            step_up,
        }
        | Cmd::ConfirmConvergeRetry {
            preview_token,
            step_up,
        } => crate::dispatch::confirm(app, req, preview_token, step_up).await,
        Cmd::CancelConverge { run_id } => cancel(app, req, run_id).await,
        Cmd::GetConverge { run_id } => get(app, req, run_id).await,
        _ => crate::bindings::internal(),
    }
}

// ---- planning --------------------------------------------------------------------------------

/// What a converge would do, as previewed and as made.
struct Planned {
    workspace: Uuid,
    binding: Uuid,
    commit: String,
    signed_mode: bool,
    check: bool,
    revision: Uuid,
    inventory: String,
    hosts: Vec<HostPlan>,
    terms: core::ApprovalTerms,
    replaces: Vec<Uuid>,
    blocked_by: Vec<Uuid>,
}

impl Planned {
    /// The hosts that will run: no refusal and at least one step.
    fn runnable(&self) -> impl Iterator<Item = &HostPlan> {
        self.hosts
            .iter()
            .filter(|h| h.refusals.is_empty() && !h.steps.is_empty())
    }

    fn plan(&self) -> ConvergePlan {
        ConvergePlan {
            binding_id: self.binding,
            commit: self.commit.clone(),
            inventory_revision: self.revision,
            check: self.check,
            hosts: self.hosts.clone(),
            policy: ApprovalTerms {
                requires_approval: self.terms.requires_approval,
                approvals_needed: self.terms.approvals_needed,
                four_eyes: self.terms.four_eyes,
                approval_ttl_secs: self.terms.approval_ttl_secs,
            },
            review: Vec::new(),
            replaces: self.replaces.clone(),
            blocked_by: self.blocked_by.clone(),
        }
    }

    /// Each job in the order it first appears, with the hosts that run it.
    fn jobs(&self) -> Vec<(String, Vec<Named>)> {
        let mut jobs: Vec<(String, Vec<Named>)> = Vec::new();
        for h in self.runnable() {
            for s in &h.steps {
                match jobs.iter_mut().find(|(j, _)| *j == s.job) {
                    Some((_, hosts)) => hosts.push((h.host_id, h.host.clone())),
                    None => jobs.push((s.job.clone(), vec![(h.host_id, h.host.clone())])),
                }
            }
        }
        jobs
    }
}

/// A refusal as an `OpError` code and message.
type Refused = (&'static str, String);

/// A host: its id and inventory name.
type Named = (Uuid, String);

type HostRow = (Uuid, String, String, Vec<String>, Vec<String>);

/// Plans a converge of `target` at `commit` (the binding's newest when `None`).
async fn plan(
    app: &AppState,
    workspace: Uuid,
    target: &ConvergeTarget,
    commit: Option<String>,
) -> Result<Result<Planned, Refused>, sqlx::Error> {
    let span = tracing::info_span!("converge.plan", binding = %target.binding_id);
    async move {
        let row: Option<(Option<String>, bool, String, bool)> = sqlx::query_as(
            "select last_commit, signed_mode, status, manifest_v068 from repo_bindings \
             where id = $1",
        )
        .bind(target.binding_id)
        .fetch_optional(&app.db)
        .await?;
        let Some((last, signed_mode, status, v068)) = row else {
            return Ok(Err((
                "not_found",
                "That binding does not exist.".to_owned(),
            )));
        };
        if status != "active" {
            return Ok(Err((
                "binding_disabled",
                "The binding is disabled.".to_owned(),
            )));
        }
        let Some(commit) = commit.or(last) else {
            return Ok(Err((
                "not_synced",
                "The binding has not synced a commit yet.".to_owned(),
            )));
        };
        if signed_mode {
            // Signed-mode review of a converge's steps is not built yet (068 T018 partial).
            return Ok(Err((
                "signed_mode_unsupported",
                "Converge does not support signed-mode bindings yet.".to_owned(),
            )));
        }
        let m = match crate::sync::manifest_at(app, target.binding_id, &commit, signed_mode).await?
        {
            Ok(m) => m,
            Err(_) => {
                return Ok(Err((
                    "manifest_invalid",
                    "The binding's manifest at this commit is not valid.".to_owned(),
                )));
            }
        };
        let cov = match core::coverage(&m) {
            Ok(c) => c,
            Err(e) => return Ok(Err(("manifest_invalid", e.to_string()))),
        };
        let selected: Option<Vec<Uuid>> = match &target.hosts {
            TargetSelection::All => None,
            TargetSelection::Hosts { ids } => Some(ids.clone()),
        };
        let rows: Vec<HostRow> = sqlx::query_as(
            "select h.id, h.inventory_name, h.state, h.allowed_profiles, \
               array(select p.name from host_profiles hp join profiles p on p.id = hp.profile_id \
                     where hp.host_id = h.id order by p.name) \
             from hosts h where h.workspace_id = $1 \
               and (($2::uuid[] is null and h.state <> 'revoked') or h.id = any($2)) \
             order by h.inventory_name",
        )
        .bind(workspace)
        .bind(&selected)
        .fetch_all(&app.db)
        .await?;
        let too_old: BTreeSet<Uuid> = if v068 {
            app.runs
                .lacking(
                    rows.iter().map(|r| r.0),
                    keel_exec::frames::CAP_MANIFEST_V068,
                )
                .into_iter()
                .collect()
        } else {
            BTreeSet::new()
        };
        let mut hosts = Vec::new();
        for (id, name, state, allowed, assigned) in rows {
            match host_plan(
                &m,
                &cov,
                id,
                name,
                &state,
                &allowed,
                assigned,
                too_old.contains(&id),
            ) {
                Ok(h) => hosts.push(h),
                Err(e) => return Ok(Err(("manifest_invalid", e))),
            }
        }
        let inventories: BTreeSet<&str> = hosts
            .iter()
            .filter(|h| h.refusals.is_empty())
            .flat_map(|h| &h.steps)
            .filter_map(|s| m.jobs.get(&s.job))
            .map(|j| j.definition.inventory.as_str())
            .collect();
        let inventory = match inventories.len() {
            0 => {
                return Ok(Err((
                    "no_steps",
                    "No selected host has a step to run.".to_owned(),
                )));
            }
            1 => inventories
                .into_iter()
                .next()
                .unwrap_or_default()
                .to_owned(),
            _ => {
                return Ok(Err((
                    "inventory_mixed",
                    "The plan's jobs use more than one inventory.".to_owned(),
                )));
            }
        };
        let mut conn = app.db.acquire().await?;
        let pinned = match crate::inventory::pin_latest(&mut conn, workspace, &inventory).await? {
            Ok(p) => p,
            Err(_) => {
                return Ok(Err((
                    "inventory_invalid",
                    format!("Inventory {inventory} has no valid revision to run against."),
                )));
            }
        };
        // A host the pinned revision does not list cannot be reached through it.
        let listed: BTreeSet<&str> = pinned.hosts.iter().map(|h| h.host.as_str()).collect();
        hosts.retain(|h| listed.contains(h.host.as_str()));
        let jobs: BTreeSet<&str> = hosts
            .iter()
            .filter(|h| h.refusals.is_empty())
            .flat_map(|h| &h.steps)
            .map(|s| s.job.as_str())
            .collect();
        if jobs.is_empty() {
            return Ok(Err((
                "no_steps",
                "No selected host has a step to run.".to_owned(),
            )));
        }
        let policies = crate::runs::load_policies(&mut *conn, workspace).await?;
        let winners: Vec<Policy> = jobs
            .iter()
            .map(|job| {
                policy::resolve_check(
                    &policies,
                    &policy::Subject {
                        job,
                        inventory: &inventory,
                        limit: None,
                    },
                    target.check,
                )
                .winner
            })
            .collect();
        let terms = core::combine_policies(&winners);
        let ids: Vec<Uuid> = hosts
            .iter()
            .filter(|h| h.refusals.is_empty() && !h.steps.is_empty())
            .map(|h| h.host_id)
            .collect();
        let replaces = open_converges(
            &mut conn,
            workspace,
            &ids,
            &["needs_review", "pending_approval"],
            &["waiting"],
        )
        .await?;
        let blocked_by = open_converges(
            &mut conn,
            workspace,
            &ids,
            &["queued", "running"],
            &["waiting", "holding"],
        )
        .await?;
        Ok(Ok(Planned {
            workspace,
            binding: target.binding_id,
            commit,
            signed_mode,
            check: target.check,
            revision: pinned.revision,
            inventory,
            hosts,
            terms,
            replaces,
            blocked_by,
        }))
    }
    .instrument(span)
    .await
}

/// Spec 068 US3: the steps a converge would give `host` with `profiles` assigned, from the
/// workspace's first active binding (oldest first) whose manifest gives it any. Empty when no
/// usable binding does (signed mode, not synced, invalid manifest are skipped).
pub(crate) async fn steps_with(
    app: &AppState,
    workspace: Uuid,
    host: Uuid,
    name: &str,
    allowed: &[String],
    profiles: &[String],
) -> Result<Vec<PlannedStep>, sqlx::Error> {
    let bindings: Vec<(Uuid, String)> = sqlx::query_as(
        "select b.id, b.last_commit from repo_bindings b join projects p on p.id = b.project_id \
         where p.workspace_id = $1 and b.status = 'active' and not b.signed_mode \
           and b.last_commit is not null order by b.id",
    )
    .bind(workspace)
    .fetch_all(&app.db)
    .await?;
    for (binding, commit) in bindings {
        let Ok(m) = crate::sync::manifest_at(app, binding, &commit, false).await? else {
            continue;
        };
        let Ok(cov) = core::coverage(&m) else {
            continue;
        };
        if let Ok(h) = host_plan(
            &m,
            &cov,
            host,
            name.to_owned(),
            "online",
            allowed,
            profiles.to_vec(),
            false,
        ) && !h.steps.is_empty()
        {
            return Ok(h.steps);
        }
    }
    Ok(Vec::new())
}

/// One host's plan: its steps from its assigned profiles, and why any cannot run.
#[allow(clippy::too_many_arguments)]
fn host_plan(
    m: &Manifest,
    cov: &core::Coverage,
    id: Uuid,
    name: String,
    state: &str,
    allowed: &[String],
    assigned: Vec<String>,
    too_old: bool,
) -> Result<HostPlan, String> {
    // Profiles this manifest does not declare (another binding's) are not this converge's.
    let profiles: Vec<String> = assigned
        .into_iter()
        .filter(|p| m.profiles.contains_key(p))
        .collect();
    let planned = core::host_plan(&profiles, m, cov).map_err(|e| e.to_string())?;
    let mut steps = Vec::new();
    let mut refusals = Vec::new();
    if state == "revoked" {
        refusals.push(StepRefusal {
            job: String::new(),
            reason: StepRefusalReason::HostRevoked,
        });
    }
    if too_old {
        refusals.push(StepRefusal {
            job: String::new(),
            reason: StepRefusalReason::RunnerTooOld,
        });
    }
    for (i, step) in planned.into_iter().enumerate() {
        let Some(job) = m.jobs.get(&step.job) else {
            continue;
        };
        let def = &job.definition;
        if !def.profiles.iter().any(|p| allowed.contains(p)) {
            refusals.push(StepRefusal {
                job: step.job.clone(),
                reason: StepRefusalReason::OutsideCeiling,
            });
        }
        let mut credential_names: Vec<String> = def.credentials.clone();
        credential_names.extend(def.env.values().cloned());
        credential_names.sort();
        credential_names.dedup();
        steps.push(PlannedStep {
            position: u16::try_from(i + 1).unwrap_or(u16::MAX),
            job: step.job,
            via: step.via,
            params: BTreeMap::new(),
            credential_names,
        });
    }
    if steps.is_empty() && refusals.is_empty() {
        refusals.push(StepRefusal {
            job: String::new(),
            reason: StepRefusalReason::NoSteps,
        });
    }
    Ok(HostPlan {
        host_id: id,
        host: name,
        profiles,
        steps,
        refusals,
    })
}

/// Converges in `workspace` in one of `states` with a host of `hosts` in one of `host_states`.
async fn open_converges(
    conn: &mut PgConnection,
    workspace: Uuid,
    hosts: &[Uuid],
    states: &[&str],
    host_states: &[&str],
) -> Result<Vec<Uuid>, sqlx::Error> {
    let states: Vec<String> = states.iter().map(|s| (*s).to_owned()).collect();
    let host_states: Vec<String> = host_states.iter().map(|s| (*s).to_owned()).collect();
    sqlx::query_scalar(
        "select r.id from runs r where r.workspace_id = $1 and r.kind = 'converge' \
           and r.state = any($3) and exists (select 1 from converge_hosts ch \
             where ch.run_id = r.id and ch.host_id = any($2) and ch.state = any($4)) \
         order by r.created_at, r.id",
    )
    .bind(workspace)
    .bind(hosts)
    .bind(&states)
    .bind(&host_states)
    .fetch_all(conn)
    .await
}

// ---- previews --------------------------------------------------------------------------------

fn pending<F, Fut>(f: F) -> PendingOp
where
    F: FnOnce(AppState, Caller) -> Fut + Send + 'static,
    Fut: Future<Output = Vec<Event>> + Send + 'static,
{
    Box::new(move |app, caller| Box::pin(f(app, caller)))
}

async fn preview(app: &AppState, req: &Request<'_>, target: ConvergeTarget) -> Vec<Event> {
    let ws = match binding_workspace(app, target.binding_id).await {
        Ok(Some(ws)) => ws,
        Ok(None) => return not_found("binding"),
        Err(e) => return db_failed(&e),
    };
    if let Err(d) = require(app, req, PREVIEW, Capability::Operate, ws).await {
        return d;
    }
    respond(app, req, PREVIEW, ws, target, None, None).await
}

async fn preview_retry(app: &AppState, req: &Request<'_>, run: Uuid, replan: bool) -> Vec<Event> {
    type Row = (Uuid, Option<Uuid>, String, bool, bool);
    let row: Option<Row> = match sqlx::query_as(
        "select workspace_id, binding_id, commit, check_mode, ended_at is not null from runs \
         where id = $1 and kind = 'converge'",
    )
    .bind(run)
    .fetch_optional(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let Some((ws, Some(binding), commit, check, ended)) = row else {
        return not_found("converge");
    };
    if let Err(d) = require(app, req, RETRY, Capability::Operate, ws).await {
        return d;
    }
    if !ended {
        return op_error("not_finished", "The converge has not finished yet.");
    }
    let ids: Vec<Uuid> = match sqlx::query_scalar(
        "select host_id from converge_hosts where run_id = $1 and state <> 'succeeded'",
    )
    .bind(run)
    .fetch_all(&app.db)
    .await
    {
        Ok(ids) => ids,
        Err(e) => return db_failed(&e),
    };
    if ids.is_empty() {
        return op_error("nothing_to_retry", "Every host of the converge succeeded.");
    }
    let target = ConvergeTarget {
        binding_id: binding,
        hosts: TargetSelection::Hosts { ids },
        check,
    };
    respond(
        app,
        req,
        RETRY,
        ws,
        target,
        (!replan).then_some(commit),
        Some(run),
    )
    .await
}

/// Plans and answers `ConvergePlan` and its `Preview`; confirming makes the converge.
async fn respond(
    app: &AppState,
    req: &Request<'_>,
    action: &'static str,
    ws: Uuid,
    target: ConvergeTarget,
    commit: Option<String>,
    retry_of: Option<Uuid>,
) -> Vec<Event> {
    let planned = match plan(app, ws, &target, commit.clone()).await {
        Ok(Ok(p)) => p,
        Ok(Err((code, message))) => return op_error(code, &message),
        Err(e) => return db_failed(&e),
    };
    let shown = planned.plan();
    let affected: Vec<String> = planned.runnable().map(|h| h.host.clone()).collect();
    let steps: usize = planned.runnable().map(|h| h.steps.len()).sum();
    let summary = format!(
        "{} {} host(s) at {} ({} step(s), inventory {}){}{}.",
        if retry_of.is_some() {
            "Retry the converge of"
        } else {
            "Converge"
        },
        affected.len(),
        &planned.commit[..planned.commit.len().min(12)],
        steps,
        planned.inventory,
        if planned.terms.requires_approval {
            ", awaiting approval"
        } else {
            ""
        },
        if planned.replaces.is_empty() {
            String::new()
        } else {
            format!(", replacing {} held converge(s)", planned.replaces.len())
        },
    );
    let op = pending(move |app, caller| async move {
        if let Err(d) = crate::runs::recheck(&caller, action, Capability::Operate, ws) {
            return d;
        }
        // Re-planned at confirm: hosts, policies and holds may have moved since the preview.
        let planned = match plan(&app, ws, &target, commit).await {
            Ok(Ok(p)) => p,
            Ok(Err((code, message))) => return op_error(code, &message),
            Err(e) => return db_failed(&e),
        };
        match create(
            &app,
            &caller.actor(),
            Some(caller.user.id),
            &planned,
            retry_of,
        )
        .await
        {
            Ok(Ok(run_id)) => vec![Event::ConvergeStarted { run_id }],
            Ok(Err((code, message))) => op_error(code, &message),
            Err(e) => db_failed(&e),
        }
    });
    let key = req.caller.preview_key(req.session);
    if key.is_empty() {
        return crate::bindings::internal();
    }
    let p = app.previews.create(
        &key,
        Draft {
            action: action.to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    );
    vec![
        Event::ConvergePlan {
            preview_token: p.preview_id,
            plan: shown,
        },
        Event::Preview(p),
    ]
}

// ---- making ----------------------------------------------------------------------------------

/// Makes the converge: replaces held ones on the same hosts, then records the parent, a child
/// run per job (each pinned), each host's steps and holds, and queues the first steps when no
/// approval is needed.
async fn create(
    app: &AppState,
    actor: &Actor,
    user: Option<Uuid>,
    p: &Planned,
    retry_of: Option<Uuid>,
) -> Result<Result<Uuid, Refused>, sqlx::Error> {
    let id = Uuid::now_v7();
    let jobs = p.jobs();
    let children: Vec<(Uuid, String, Vec<Named>)> = jobs
        .into_iter()
        .map(|(job, hosts)| (Uuid::now_v7(), job, hosts))
        .collect();
    let dir = crate::sync::data_dir(app);
    let (binding, commit) = (p.binding, p.commit.clone());
    let to_pin: Vec<Uuid> = children.iter().map(|c| c.0).collect();
    let pinned = tokio::task::spawn_blocking(move || {
        let mirror = Mirror::open(&dir, binding)?;
        to_pin
            .iter()
            .try_for_each(|run| mirror.pin(*run, &CommitId(commit.clone())))
    })
    .await;
    if !matches!(pinned, Ok(Ok(()))) {
        return Ok(Err((
            "artifact_missing",
            "The commit could not be pinned.".to_owned(),
        )));
    }
    let then = RunStatus::initial(false, p.terms.requires_approval);
    let names: Vec<String> = p.runnable().map(|h| h.host.clone()).collect();
    let mut tx = app.db.begin().await?;
    // Spec 068 US7: with required checks the converge waits for them before approval or its
    // first steps; `checks::poll_once` moves it on.
    let contexts = crate::checks::required(&mut tx, p.binding).await?;
    let status = if crate::checks::gates(then, &contexts) {
        crate::checks::waiting()
    } else {
        then
    };
    let queued = status.state == RunState::Queued;
    // Held converges on these hosts are replaced (one approval covers the newest plan).
    let replaced: Vec<Uuid> = sqlx::query_scalar(
        "update runs set state = 'cancelled', state_reason = 'replaced', ended_at = now() \
         where id = any($1) and kind = 'converge' \
           and state in ('needs_review', 'pending_approval') returning id",
    )
    .bind(&p.replaces)
    .fetch_all(&mut *tx)
    .await?;
    for old in &replaced {
        abandon(&mut tx, *old).await?;
        audit::record(
            &mut tx,
            Entry {
                actor,
                action: "converge.replace",
                workspace: Some(WorkspaceId(p.workspace)),
                target: json!({ "run": old }),
                decision: Decision::Allowed,
                reason: Some("replaced"),
                context: json!({ "by": id }),
            },
        )
        .await?;
    }
    sqlx::query(
        "insert into runs (id, workspace_id, kind, job_id, binding_id, job_name, trigger, \
         requested_by, commit, inventory_revision_id, inventory_name, resolved_hosts, \
         signed_mode, check_mode, retry_of, state, queued_at, summary, params, \
         approvals_needed, four_eyes, approval_expires_at) \
         values ($1, $2, 'converge', null, $3, 'converge', 'manual', $4, $5, $6, $7, $8, $9, \
         $10, $11, $17, case when $17 = 'queued' then now() end, $13, '{}', \
         case when $12 = 'pending_approval' then $14 end, \
         case when $12 = 'pending_approval' then $15 end, \
         case when $12 = 'pending_approval' then now() + $16 * interval '1 second' end)",
    )
    .bind(id)
    .bind(p.workspace)
    .bind(p.binding)
    .bind(user)
    .bind(&p.commit)
    .bind(p.revision)
    .bind(&p.inventory)
    .bind(&names)
    .bind(p.signed_mode)
    .bind(p.check)
    .bind(retry_of)
    // $12 is where the converge goes once any required checks pass; $17 is where it starts.
    .bind(then.state.as_str())
    .bind(json!(RunSummary::default()))
    .bind(i32::from(p.terms.approvals_needed.max(1)))
    .bind(p.terms.four_eyes)
    .bind(i64::from(p.terms.approval_ttl_secs))
    .bind(status.state.as_str())
    .execute(&mut *tx)
    .await?;
    if status.state == RunState::WaitingChecks {
        crate::checks::hold(&mut tx, id, &contexts).await?;
    }
    for (child, job, hosts) in &children {
        let host_names: Vec<String> = hosts.iter().map(|h| h.1.clone()).collect();
        sqlx::query(
            "insert into runs (id, workspace_id, parent_run_id, job_id, binding_id, job_name, \
             trigger, requested_by, commit, inventory_revision_id, inventory_name, \
             resolved_hosts, signed_mode, check_mode, state, queued_at, summary, params) \
             values ($1, $2, $3, (select j.id from jobs j where j.binding_id = $4 \
               and j.name = $5 and j.removed_at_commit is null), $4, $5, 'manual', $6, $7, $8, \
               $9, $10, $11, $12, 'queued', now(), $13, '{}')",
        )
        .bind(child)
        .bind(p.workspace)
        .bind(id)
        .bind(p.binding)
        .bind(job)
        .bind(user)
        .bind(&p.commit)
        .bind(p.revision)
        .bind(&p.inventory)
        .bind(&host_names)
        .bind(p.signed_mode)
        .bind(p.check)
        .bind(json!(RunSummary::default()))
        .execute(&mut *tx)
        .await?;
    }
    for h in p.runnable() {
        sqlx::query("insert into converge_hosts (run_id, host_id, profiles) values ($1, $2, $3)")
            .bind(id)
            .bind(h.host_id)
            .bind(&h.profiles)
            .execute(&mut *tx)
            .await?;
        for (i, step) in h.steps.iter().enumerate() {
            let Some((child, ..)) = children.iter().find(|c| c.1 == step.job) else {
                continue;
            };
            let first = i == 0;
            sqlx::query(
                "insert into run_hosts (run_id, host_id, host_name, state) values ($1, $2, $3, $4)",
            )
            .bind(child)
            .bind(h.host_id)
            .bind(&h.host)
            .bind(if first && queued { "queued" } else { "blocked" })
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "insert into converge_steps (run_id, host_id, position, job_name, step_run_id, \
                 via) values ($1, $2, $3, $4, $5, $6)",
            )
            .bind(id)
            .bind(h.host_id)
            .bind(i16::try_from(i).unwrap_or(i16::MAX))
            .bind(&step.job)
            .bind(child)
            .bind(&step.via)
            .execute(&mut *tx)
            .await?;
        }
    }
    audit::record(
        &mut tx,
        Entry {
            actor,
            action: if retry_of.is_some() {
                "converge.retry"
            } else {
                "converge.create"
            },
            workspace: Some(WorkspaceId(p.workspace)),
            target: json!({ "run": id, "binding": p.binding }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "commit": p.commit,
                "inventory_revision": p.revision,
                "hosts": names,
                "jobs": children.iter().map(|c| &c.1).collect::<Vec<_>>(),
                "check": p.check,
                "state": status.state.as_str(),
                "required_checks": contexts,
                "approvals_needed": p.terms.approvals_needed,
                "four_eyes": p.terms.four_eyes,
                "replaces": replaced,
                "retry_of": retry_of,
            }),
        },
    )
    .await?;
    tx.commit().await?;
    for old in replaced {
        crate::queue::push_state(
            app,
            p.workspace,
            old,
            RunStatus {
                state: RunState::Cancelled,
                reason: Some(StateReason::Replaced),
            },
        );
    }
    crate::queue::push_state(app, p.workspace, id, status);
    if queued {
        crate::queue::kick(app);
    }
    Ok(Ok(id))
}

/// Spec 068 US2 (FR-006): converges a host that just enrolled to its profiles, as
/// `system:enroll`, when `KEEL_CONVERGE` is on and its enrollment kept auto-converge (a host
/// with no enrollment row, such as an accepted join request, counts as kept). The workspace's
/// active bindings are tried oldest first; the first whose plan gives the host a step gets the
/// converge, routed through the approval policy like any other. When none does,
/// `converge.enroll_skipped` is audited with each binding's reason. Called after the enrollment
/// committed, so nothing here can undo it.
///
/// # Errors
///
/// A database error; the enrollment stands.
pub async fn on_enrolled(app: &AppState, host: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    if !app.cfg.converge {
        return Ok(None);
    }
    let span = tracing::info_span!("converge.on_enrolled", %host, run = tracing::field::Empty);
    async move {
        let row: Option<(Uuid, bool)> = sqlx::query_as(
            "select h.workspace_id, coalesce((select e.auto_converge from host_enrollments e \
               where e.host_id = h.id order by e.used_at desc nulls last limit 1), true) \
             from hosts h where h.id = $1 and h.state <> 'revoked'",
        )
        .bind(host)
        .fetch_optional(&app.db)
        .await?;
        let Some((ws, true)) = row else {
            return Ok(None);
        };
        let bindings: Vec<Uuid> = sqlx::query_scalar(
            "select b.id from repo_bindings b join projects p on p.id = b.project_id \
             where p.workspace_id = $1 and b.status = 'active' order by b.id",
        )
        .bind(ws)
        .fetch_all(&app.db)
        .await?;
        let mut skipped = Vec::new();
        for binding in bindings {
            let target = ConvergeTarget {
                binding_id: binding,
                hosts: TargetSelection::Hosts { ids: vec![host] },
                check: false,
            };
            let refused = match plan(app, ws, &target, None).await? {
                Ok(p) if p.runnable().any(|h| h.host_id == host) => {
                    match create(app, &Actor::Enroll, None, &p, None).await? {
                        Ok(run) => {
                            tracing::Span::current().record("run", tracing::field::display(run));
                            return Ok(Some(run));
                        }
                        Err(r) => r,
                    }
                }
                Ok(_) => ("no_steps", "The host has no step to run.".to_owned()),
                Err(r) => r,
            };
            skipped.push(json!({ "binding": binding, "code": refused.0, "message": refused.1 }));
        }
        let mut conn = app.db.acquire().await?;
        audit::record(
            &mut conn,
            Entry {
                actor: &Actor::Enroll,
                action: "converge.enroll_skipped",
                workspace: Some(WorkspaceId(ws)),
                target: json!({ "host": host }),
                decision: Decision::Allowed,
                reason: Some(if skipped.is_empty() {
                    "no_binding"
                } else {
                    "no_plan"
                }),
                context: json!({ "bindings": skipped }),
            },
        )
        .await?;
        Ok(None)
    }
    .instrument(span)
    .await
}

/// Releases an approved converge: each host's first step is queued (spec 068 FR: one approval
/// covers the plan). A run that is not a converge has no steps, so nothing changes.
pub(crate) async fn release(tx: &mut PgConnection, run: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update run_hosts rh set state = 'queued' from converge_steps s \
         where s.run_id = $1 and s.position = 0 \
           and rh.run_id = s.step_run_id and rh.host_id = s.host_id and rh.state = 'blocked'",
    )
    .bind(run)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Abandons a converge's steps that have not gone out: each host's current step is cancelled
/// (the trigger skips its later ones and ends its hold), and children left with nothing to run
/// end `cancelled`. Steps already out are left to finish or be cancelled by the runner. A run
/// that is not a converge has no children, so nothing changes.
pub(crate) async fn abandon(tx: &mut PgConnection, run: Uuid) -> Result<(), sqlx::Error> {
    // Queued steps first: each is its host's current step.
    sqlx::query(
        "update run_hosts rh set state = 'cancelled', ended_at = now() from converge_steps s \
         where s.run_id = $1 and rh.run_id = s.step_run_id and rh.host_id = s.host_id \
           and rh.state = 'queued'",
    )
    .bind(run)
    .execute(&mut *tx)
    .await?;
    // Hosts with nothing out and nothing queued: their first blocked step is the current one.
    sqlx::query(
        "update run_hosts rh set state = 'cancelled', ended_at = now() \
         from (select distinct on (s.host_id) s.step_run_id, s.host_id \
                 from converge_steps s join run_hosts r \
                   on r.run_id = s.step_run_id and r.host_id = s.host_id \
                where s.run_id = $1 and r.state = 'blocked' \
                order by s.host_id, s.position) cur \
         where rh.run_id = cur.step_run_id and rh.host_id = cur.host_id \
           and rh.state = 'blocked' \
           and not exists (select 1 from converge_steps s2 join run_hosts r2 \
                             on r2.run_id = s2.step_run_id and r2.host_id = s2.host_id \
                            where s2.run_id = $1 and s2.host_id = cur.host_id \
                              and r2.state in ('sent', 'running'))",
    )
    .bind(run)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update runs c set state = 'cancelled', ended_at = now() \
         where c.parent_run_id = $1 and c.ended_at is null \
           and not exists (select 1 from run_hosts rh where rh.run_id = c.id \
                             and rh.state in ('blocked', 'queued', 'sent', 'running'))",
    )
    .bind(run)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

// ---- cancelling ------------------------------------------------------------------------------

async fn cancel(app: &AppState, req: &Request<'_>, run: Uuid) -> Vec<Event> {
    const ACTION: &str = "cancel_converge";
    let ws = match converge_workspace(app, run).await {
        Ok(Some(ws)) => ws,
        Ok(None) => return not_found("converge"),
        Err(e) => return db_failed(&e),
    };
    if let Err(d) = require(app, req, ACTION, Capability::Operate, ws).await {
        return d;
    }
    let result: Result<Option<Vec<(Uuid, Uuid)>>, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let ended: Option<bool> =
            sqlx::query_scalar("select ended_at is not null from runs where id = $1 for update")
                .bind(run)
                .fetch_optional(&mut *tx)
                .await?;
        if ended != Some(false) {
            return Ok(None);
        }
        sqlx::query(
            "update runs set state = 'cancelled', state_reason = 'cancelled_by_user', \
             ended_at = now() where id = $1",
        )
        .bind(run)
        .execute(&mut *tx)
        .await?;
        let out: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "select rh.run_id, rh.host_id from converge_steps s join run_hosts rh \
               on rh.run_id = s.step_run_id and rh.host_id = s.host_id \
             where s.run_id = $1 and rh.state in ('sent', 'running')",
        )
        .bind(run)
        .fetch_all(&mut *tx)
        .await?;
        abandon(&mut tx, run).await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &req.caller.actor(),
                action: "converge.cancel",
                workspace: Some(WorkspaceId(ws)),
                target: json!({ "run": run }),
                decision: Decision::Allowed,
                reason: Some("cancelled_by_user"),
                context: json!({ "in_flight": out.len() }),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(Some(out))
    }
    .await;
    match result {
        Ok(Some(out)) => {
            for (step, host) in out {
                crate::queue::cancel(app, step, host);
            }
            crate::queue::push_state(
                app,
                ws,
                run,
                RunStatus {
                    state: RunState::Cancelled,
                    reason: Some(StateReason::CancelledByUser),
                },
            );
            push_hosts(app, run).await;
            // Hosts with nothing in flight are released now; their other runs go out at once.
            crate::queue::kick(app);
            reply(app, run).await
        }
        Ok(None) => op_error("not_cancellable", "The converge has already ended."),
        Err(e) => db_failed(&e),
    }
}

// ---- reading ---------------------------------------------------------------------------------

async fn converge_workspace(app: &AppState, run: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("select workspace_id from runs where id = $1 and kind = 'converge'")
        .bind(run)
        .fetch_optional(&app.db)
        .await
}

async fn get(app: &AppState, req: &Request<'_>, run: Uuid) -> Vec<Event> {
    let ws = match converge_workspace(app, run).await {
        Ok(Some(ws)) => ws,
        Ok(None) => return not_found("converge"),
        Err(e) => return db_failed(&e),
    };
    if let Err(d) = require(app, req, "get_converge", Capability::View, ws).await {
        return d;
    }
    reply(app, run).await
}

async fn reply(app: &AppState, run: Uuid) -> Vec<Event> {
    match view(app, run).await {
        Ok(Some(v)) => vec![Event::Converge(v)],
        Ok(None) => not_found("converge"),
        Err(e) => db_failed(&e),
    }
}

type StepRow = (
    Uuid,
    i16,
    String,
    Uuid,
    String,
    Option<String>,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
);

/// The converge as `GetConverge` answers it.
pub(crate) async fn view(app: &AppState, run: Uuid) -> Result<Option<ConvergeView>, sqlx::Error> {
    let Some(info) = crate::runs::run_info(app, run).await? else {
        return Ok(None);
    };
    let retry_of: Option<Uuid> = sqlx::query_scalar("select retry_of from runs where id = $1")
        .bind(run)
        .fetch_one(&app.db)
        .await?;
    let hosts: Vec<(Uuid, String, String)> = sqlx::query_as(
        "select ch.host_id, h.inventory_name, ch.state from converge_hosts ch \
         join hosts h on h.id = ch.host_id where ch.run_id = $1 order by h.inventory_name",
    )
    .bind(run)
    .fetch_all(&app.db)
    .await?;
    let steps: Vec<StepRow> = sqlx::query_as(
        "select s.host_id, s.position, s.job_name, s.step_run_id, rh.state, \
           coalesce(rh.refusal, rh.skip_reason), rh.started_at, rh.ended_at \
         from converge_steps s join run_hosts rh \
           on rh.run_id = s.step_run_id and rh.host_id = s.host_id \
         where s.run_id = $1 order by s.host_id, s.position",
    )
    .bind(run)
    .fetch_all(&app.db)
    .await?;
    let hosts = hosts
        .into_iter()
        .map(|(host_id, host, state)| ConvergeHostView {
            host_id,
            host,
            state: enum_from(&state).unwrap_or(ConvergeHostState::Failed),
            steps: steps
                .iter()
                .filter(|s| s.0 == host_id)
                .map(|s| ConvergeStepView {
                    position: u16::try_from(s.1).unwrap_or(0).saturating_add(1),
                    job: s.2.clone(),
                    run_id: s.3,
                    state: enum_from(&s.4).unwrap_or(StepState::Failed),
                    reason: s.5.clone(),
                    started_at: s.6.map(ts),
                    ended_at: s.7.map(ts),
                })
                .collect(),
        })
        .collect();
    Ok(Some(ConvergeView {
        run: info,
        hosts,
        retry_of,
    }))
}

/// Pushes each host's converge state to the workspace's viewers.
pub(crate) async fn push_hosts(app: &AppState, run: Uuid) {
    let rows: Result<Vec<(Uuid, Uuid, String)>, sqlx::Error> = sqlx::query_as(
        "select r.workspace_id, ch.host_id, ch.state from converge_hosts ch \
         join runs r on r.id = ch.run_id where ch.run_id = $1",
    )
    .bind(run)
    .fetch_all(&app.db)
    .await;
    match rows {
        Ok(rows) => {
            for (ws, host_id, state) in rows {
                if let Some(state) = enum_from(&state) {
                    app.repos.push(
                        ws,
                        Event::ConvergeChanged {
                            run_id: run,
                            host_id,
                            state,
                        },
                    );
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, run = %run, "reading converge hosts failed"),
    }
}

/// How a converge parent ends once no host is waiting or holding: succeeded when every host
/// succeeded, else failed. `None` while a host is still waiting or holding.
pub(crate) async fn outcome(
    conn: &mut PgConnection,
    run: Uuid,
) -> Result<Option<(RunStatus, RunSummary)>, sqlx::Error> {
    let states: Vec<String> =
        sqlx::query_scalar("select state from converge_hosts where run_id = $1")
            .bind(run)
            .fetch_all(&mut *conn)
            .await?;
    if states.iter().any(|s| s == "waiting" || s == "holding") {
        return Ok(None);
    }
    let mut summary = RunSummary::default();
    for s in &states {
        summary.hosts += 1;
        match s.as_str() {
            "succeeded" => summary.succeeded += 1,
            "failed" => summary.failed += 1,
            "cancelled" => summary.cancelled += 1,
            _ => {}
        }
    }
    let state = if summary.hosts > 0 && summary.succeeded == summary.hosts {
        RunState::Succeeded
    } else {
        RunState::Failed
    };
    Ok(Some((
        RunStatus {
            state,
            reason: None,
        },
        summary,
    )))
}
