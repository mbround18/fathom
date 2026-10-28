//! Runs, jobs and approval policies (spec 055 US2, T069 and T071).
//!
//! A sync that moves a binding ([`on_push`]) starts a run for each job whose `paths` match the
//! changed files. A run is pinned when it is made: its commit under `refs/keel/runs/<id>` (pins
//! are kept, so a run can always be re-read or reversed) and the inventory's newest revision.
//! Its hosts are the revision's hosts that the job's `limit` and the run's own select, that are
//! enrolled in the workspace, not revoked, and serve one of the job's profiles. The approval
//! policy then decides between `pending_approval` and `queued`. A signed-mode binding's run is
//! checked first ([`crate::review::early_check`]): hosts whose new commits are not all signed by
//! a trusted signer are grouped by base for review, and the run waits in `needs_review`.
//!
//! Reads need `View`, starting and cancelling `Operate`, and policy changes `Administer` with a
//! fresh passkey.

use std::collections::BTreeSet;

use keel_core::WorkspaceId;
use keel_core::authz::{self, Capability};
use keel_core::manifest::JobDefinition;
use keel_core::policy::{self, Policy, PolicyMatch};
use keel_core::ports::CommitId;
use keel_core::run::{RunEvent, RunState, RunStatus, StateReason};
use keel_core::target::{self, Target};
use keel_git::Mirror;
use keel_protocol::{
    Cmd, Event, FirstFailure, HostStats, JobInfo, PolicyCheck, PolicyInfo, PolicySpec, RunHostInfo,
    RunInfo, RunPhase, RunSummary,
};
use serde_json::{Value, json};
use sqlx::PgConnection;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::bindings::{
    binding_workspace, db_failed, enum_from, internal, not_found, op_error, require, subject, ts,
};
use crate::dispatch::{PendingOp, Request};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::serve::AppState;
use crate::sync::Trigger;

pub use crate::queue::{RunSettings, Runs, history};

/// The most runs `ListRuns` returns.
const LIST_LIMIT: i64 = 200;

// ---- creating runs ---------------------------------------------------------------------------

/// A sync that moved a binding.
pub struct Push<'a> {
    pub binding: Uuid,
    pub workspace: Uuid,
    pub previous: &'a str,
    pub head: &'a str,
    pub trigger: &'a Trigger,
    pub signed_mode: bool,
}

/// Starts a run for every job whose `paths` match what the push changed. Failures are logged:
/// the sync itself already succeeded.
pub async fn on_push(app: &AppState, p: Push<'_>) {
    let dir = crate::sync::data_dir(app);
    let (binding, prev, head) = (p.binding, p.previous.to_owned(), p.head.to_owned());
    let changed = tokio::task::spawn_blocking(move || {
        Mirror::open(&dir, binding)?.changed_paths(Some(&CommitId(prev)), &CommitId(head))
    })
    .await;
    let changed: BTreeSet<String> = match changed {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            tracing::error!(error = %e, binding = %binding, "reading the push's changed paths failed");
            return;
        }
        Err(_) => return,
    };
    let jobs: Vec<(Uuid, String, Value)> = match sqlx::query_as(
        "select id, name, definition from jobs where binding_id = $1 \
         and removed_at_commit is null and blocked_reason is null order by name",
    )
    .bind(binding)
    .fetch_all(&app.db)
    .await
    {
        Ok(j) => j,
        Err(e) => {
            tracing::error!(error = %e, "loading jobs failed");
            return;
        }
    };
    let (trigger, delivery, requested_by) = match p.trigger {
        Trigger::Delivery { delivery, .. } => ("delivery", Some(*delivery), None),
        Trigger::Poll => ("poll", None, None),
        Trigger::Create(a) | Trigger::Manual(a) => ("manual", None, user_of(a)),
    };
    let actor = p.trigger.actor();
    for (job, name, def) in jobs {
        let Ok(def) = serde_json::from_value::<JobDefinition>(def) else {
            continue;
        };
        let Some(paths) = &def.paths else { continue };
        if !target::paths_match(paths, &changed) {
            continue;
        }
        let new = NewRun {
            workspace: p.workspace,
            binding,
            job,
            job_name: &name,
            def: &def,
            trigger,
            delivery,
            requested_by,
            commit: p.head,
            run_limit: None,
            signed_mode: p.signed_mode,
            actor: &actor,
            id: None,
            only_hosts: None,
        };
        if let Err(e) = create_run(app, &new).await {
            tracing::error!(error = %e, job = %name, "creating a run failed");
        }
    }
}

const fn user_of(a: &Actor) -> Option<Uuid> {
    match a {
        Actor::User { id, .. } => Some(*id),
        _ => None,
    }
}

struct NewRun<'a> {
    workspace: Uuid,
    binding: Uuid,
    job: Uuid,
    job_name: &'a str,
    def: &'a JobDefinition,
    trigger: &'static str,
    delivery: Option<Uuid>,
    requested_by: Option<Uuid>,
    commit: &'a str,
    run_limit: Option<&'a str>,
    signed_mode: bool,
    actor: &'a Actor,
    /// The run's id, when it was chosen beforehand (a baseline signs it).
    id: Option<Uuid>,
    /// Only these of the planned hosts (a baseline).
    only_hosts: Option<&'a [String]>,
}

/// What a run of a job would reach.
struct Plan {
    revision: Option<Uuid>,
    /// Enrolled hosts the run reaches, by inventory name.
    hosts: Vec<(Uuid, String)>,
    policy: Policy,
    explanation: policy::Explanation,
    /// Why the run cannot start, if it cannot.
    unrunnable: Option<StateReason>,
}

/// Plans a run of `def` with `run_limit`.
async fn plan(
    conn: &mut PgConnection,
    workspace: Uuid,
    job_name: &str,
    def: &JobDefinition,
    run_limit: Option<&str>,
) -> Result<Result<Plan, target::LimitError>, sqlx::Error> {
    let policies = load_policies(&mut *conn, workspace).await?;
    let limit = run_limit.or(def.limit.as_deref());
    let explanation = policy::resolve(
        &policies,
        &policy::Subject {
            job: job_name,
            inventory: &def.inventory,
            limit,
        },
    );
    let policy = explanation.winner.clone();
    let pinned = crate::inventory::pin_latest(&mut *conn, workspace, &def.inventory).await?;
    let pinned = match pinned {
        Ok(p) => p,
        Err(refusal) => {
            tracing::info!(
                code = refusal.code(),
                job = job_name,
                "no inventory revision to pin"
            );
            return Ok(Ok(Plan {
                revision: None,
                hosts: Vec::new(),
                policy,
                explanation,
                unrunnable: Some(StateReason::InventoryInvalid),
            }));
        }
    };
    let targets: Vec<Target<'_>> = pinned
        .hosts
        .iter()
        .map(|h| Target {
            host: &h.host,
            groups: &h.groups,
        })
        .collect();
    let by_job = match target::select(def.limit.as_deref(), &targets) {
        Ok(s) => s,
        Err(e) => return Ok(Err(e)),
    };
    let by_run = match target::select(run_limit, &targets) {
        Ok(s) => s,
        Err(e) => return Ok(Err(e)),
    };
    let names: Vec<String> = by_job
        .intersection(&by_run)
        .map(|s| (*s).to_owned())
        .collect();
    let hosts: Vec<(Uuid, String)> = sqlx::query_as(
        "select h.id, h.inventory_name from hosts h where h.workspace_id = $1 \
         and h.inventory_name = any($2) and h.state <> 'revoked' \
         and (cardinality($3::text[]) = 0 or exists (select 1 from host_profiles hp \
              join profiles p on p.id = hp.profile_id \
              where hp.host_id = h.id and p.name = any($3))) \
         order by h.inventory_name",
    )
    .bind(workspace)
    .bind(&names)
    .bind(&def.profiles)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Ok(Plan {
        revision: Some(pinned.revision),
        hosts,
        policy,
        explanation,
        unrunnable: None,
    }))
}

/// Makes a run: pins it, plans it, records it and, when it needs no approval, queues it.
async fn create_run(app: &AppState, new: &NewRun<'_>) -> Result<Uuid, sqlx::Error> {
    let id = new.id.unwrap_or_else(Uuid::now_v7);
    let dir = crate::sync::data_dir(app);
    let (binding, commit) = (new.binding, new.commit.to_owned());
    let pinned = tokio::task::spawn_blocking(move || {
        Mirror::open(&dir, binding)?.pin(id, &CommitId(commit))
    })
    .await;
    let pinned = matches!(pinned, Ok(Ok(())));
    let mut tx = app.db.begin().await?;
    let plan = match plan(&mut tx, new.workspace, new.job_name, new.def, new.run_limit).await? {
        Ok(p) => p,
        Err(e) => {
            // A job limit the manifest accepted but targeting cannot read: fail closed.
            tracing::warn!(error = %e, job = new.job_name, "a run's limit could not be read");
            Plan {
                revision: None,
                hosts: Vec::new(),
                policy: Policy::catch_all(Uuid::nil()),
                explanation: policy::resolve(
                    &[],
                    &policy::Subject {
                        job: new.job_name,
                        inventory: &new.def.inventory,
                        limit: None,
                    },
                ),
                unrunnable: Some(StateReason::InventoryInvalid),
            }
        }
    };
    let mut plan = plan;
    if let Some(only) = new.only_hosts {
        plan.hosts.retain(|h| only.contains(&h.1));
    }
    let mut unrunnable = if pinned {
        plan.unrunnable
    } else {
        Some(StateReason::ArtifactMissing)
    };
    // Signed mode: the early check (advisory; every host verifies on its own).
    let mut early: Option<crate::review::Checked> = None;
    if new.signed_mode && unrunnable.is_none() && !plan.hosts.is_empty() {
        match crate::review::early_check(
            app,
            &mut tx,
            new.workspace,
            new.binding,
            new.job_name,
            new.def.inventory_source,
            new.commit,
            &plan.hosts,
        )
        .await?
        {
            crate::review::Early::Managed => unrunnable = Some(StateReason::InventoryInvalid),
            crate::review::Early::Checked { bases, groups } => early = Some((bases, groups)),
        }
    }
    let needs_review = early.as_ref().is_some_and(|(_, g)| !g.is_empty());
    let status = match unrunnable {
        Some(r) => RunStatus {
            state: RunState::Failed,
            reason: Some(r),
        },
        None if plan.hosts.is_empty() => RunStatus {
            state: RunState::Failed,
            reason: None,
        },
        None => RunStatus::initial(needs_review, plan.policy.requires_approval),
    };
    let names: Vec<String> = plan.hosts.iter().map(|h| h.1.clone()).collect();
    let ended = status.state == RunState::Failed;
    sqlx::query(
        "insert into runs (id, workspace_id, job_id, binding_id, job_name, trigger, delivery_id, \
         requested_by, commit, inventory_revision_id, inventory_name, resolved_hosts, run_limit, \
         policy_id, signed_mode, state, state_reason, queued_at, ended_at, summary) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
         case when $16 = 'queued' then now() end, case when $18 then now() end, \
         case when $18 then $19 end)",
    )
    .bind(id)
    .bind(new.workspace)
    .bind(new.job)
    .bind(new.binding)
    .bind(new.job_name)
    .bind(new.trigger)
    .bind(new.delivery)
    .bind(new.requested_by)
    .bind(new.commit)
    .bind(plan.revision)
    .bind(&new.def.inventory)
    .bind(&names)
    .bind(new.run_limit)
    .bind((!plan.policy.id.is_nil()).then_some(plan.policy.id))
    .bind(new.signed_mode)
    .bind(status.state.as_str())
    .bind(status.reason.map(|r| r.as_str()))
    .bind(ended)
    .bind(json!(RunSummary::default()))
    .execute(&mut *tx)
    .await?;
    if !ended {
        let ids: Vec<Uuid> = plan.hosts.iter().map(|h| h.0).collect();
        sqlx::query(
            "insert into run_hosts (run_id, host_id, host_name) \
             select $1, * from unnest($2::uuid[], $3::text[])",
        )
        .bind(id)
        .bind(&ids)
        .bind(&names)
        .execute(&mut *tx)
        .await?;
    }
    let mut groups = Vec::new();
    if !ended && let Some((bases, drafts)) = &early {
        let (hosts, base): (Vec<Uuid>, Vec<Option<String>>) = bases.iter().cloned().unzip();
        sqlx::query(
            "update run_hosts rh set base_commit = b.base \
             from unnest($2::uuid[], $3::text[]) as b(host, base) \
             where rh.run_id = $1 and rh.host_id = b.host",
        )
        .bind(id)
        .bind(&hosts)
        .bind(&base)
        .execute(&mut *tx)
        .await?;
        groups = crate::review::insert_groups(&mut tx, id, new.commit, drafts).await?;
    }
    audit::record(
        &mut tx,
        Entry {
            actor: new.actor,
            action: "run_created",
            workspace: Some(WorkspaceId(new.workspace)),
            target: json!({ "run": id, "job": new.job }),
            decision: Decision::Allowed,
            reason: status.reason.map(|r| r.as_str()),
            context: json!({
                "trigger": new.trigger,
                "commit": new.commit,
                "inventory_revision": plan.revision,
                "hosts": names,
                "limit": new.run_limit,
                "state": status.state.as_str(),
                "policy": (!plan.policy.id.is_nil()).then_some(plan.policy.id),
                "policy_reason": plan.explanation.reason,
                "review_groups": groups,
            }),
        },
    )
    .await?;
    tx.commit().await?;
    for group in groups {
        app.repos.push(
            new.workspace,
            Event::ReviewRequested {
                group,
                run: id,
                workspace: new.workspace,
            },
        );
    }
    crate::queue::push_state(app, new.workspace, id, status);
    if status.state == RunState::Queued {
        crate::queue::kick(app);
    }
    Ok(id)
}

/// What a baseline run is made from (spec 055 US8: the run id is signed before it exists).
pub(crate) struct BaselineRun<'a> {
    pub id: Uuid,
    pub workspace: Uuid,
    pub binding: Uuid,
    pub job: Uuid,
    pub job_name: &'a str,
    pub def: &'a JobDefinition,
    pub commit: &'a str,
    pub hosts: &'a [String],
}

/// Makes a baseline run: a manual signed-mode run of `hosts` only, with a chosen id.
pub(crate) async fn create_baseline_run(
    app: &AppState,
    caller: &Caller,
    b: BaselineRun<'_>,
) -> Result<Uuid, sqlx::Error> {
    let actor = caller.actor();
    create_run(
        app,
        &NewRun {
            workspace: b.workspace,
            binding: b.binding,
            job: b.job,
            job_name: b.job_name,
            def: b.def,
            trigger: "manual",
            delivery: None,
            requested_by: Some(caller.user.id),
            commit: b.commit,
            run_limit: None,
            signed_mode: true,
            actor: &actor,
            id: Some(b.id),
            only_hosts: Some(b.hosts),
        },
    )
    .await
}

/// The hosts a run of `def` reaches, whether its policy asks for four-eyes (a run without a
/// policy does), and its approval TTL in seconds; `None` when it cannot run.
pub(crate) async fn plan_hosts(
    conn: &mut PgConnection,
    workspace: Uuid,
    job_name: &str,
    def: &JobDefinition,
) -> Result<Option<(Vec<(Uuid, String)>, bool, i64)>, sqlx::Error> {
    Ok(match plan(conn, workspace, job_name, def, None).await? {
        Ok(p) if p.unrunnable.is_none() => Some((
            p.hosts,
            p.policy.id.is_nil() || p.policy.four_eyes,
            if p.policy.id.is_nil() {
                crate::review::MAX_OVERRIDE_SECS
            } else {
                i64::from(p.policy.approval_ttl_secs)
            },
        )),
        _ => None,
    })
}

// ---- commands --------------------------------------------------------------------------------

/// Runs one run, job or policy command.
pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::ListRuns {
            workspace,
            binding,
            host,
            actor,
            state,
            from,
            to,
        } => {
            if let Err(d) = require(app, req, "list_runs", Capability::View, workspace).await {
                return d;
            }
            let parse = |i: Option<keel_protocol::Instant>| -> Result<Option<OffsetDateTime>, ()> {
                i.map(|i| OffsetDateTime::parse(i.as_str(), &Rfc3339).map_err(|_| ()))
                    .transpose()
            };
            let (Ok(from), Ok(to)) = (parse(from), parse(to)) else {
                return op_error("bad_instant", "from and to must be RFC 3339 times.");
            };
            let filter = Filter {
                workspace,
                binding,
                host: host.map(|h| h.as_str().to_owned()),
                actor,
                state: state.map(|s| crate::bindings::enum_str(&s)),
                from,
                to,
            };
            match list_runs(app, &filter).await {
                Ok(items) => vec![Event::Runs { items }],
                Err(e) => db_failed(&e),
            }
        }
        Cmd::GetRun { run } => {
            let ws = match run_workspace(app, run).await {
                Ok(Some(ws)) => ws,
                Ok(None) => return not_found("run"),
                Err(e) => return db_failed(&e),
            };
            if let Err(d) = require(app, req, "get_run", Capability::View, ws).await {
                return d;
            }
            match get_run(app, run).await {
                Ok(Some(e)) => vec![e],
                Ok(None) => not_found("run"),
                Err(e) => db_failed(&e),
            }
        }
        Cmd::SubscribeRun { run, from_seq } => {
            let ws = match run_workspace(app, run).await {
                Ok(Some(ws)) => ws,
                Ok(None) => return not_found("run"),
                Err(e) => return db_failed(&e),
            };
            if let Err(d) = require(app, req, "subscribe_run", Capability::View, ws).await {
                return d;
            }
            match history(app, run, from_seq).await {
                Ok(events) => {
                    let last = events
                        .iter()
                        .filter_map(|e| match e {
                            Event::RunEvent { seq, .. } | Event::RunLog { seq, .. } => Some(*seq),
                            _ => None,
                        })
                        .max()
                        .unwrap_or(from_seq);
                    let mut out = vec![Event::RunSubscribed { run, seq: last }];
                    out.extend(events);
                    out
                }
                Err(e) => db_failed(&e),
            }
        }
        // The socket drops the subscription before dispatching; nothing to answer.
        Cmd::UnsubscribeRun { .. } => Vec::new(),
        Cmd::PreviewRunStart { job, limit } => {
            preview_run_start(app, req, job, limit.map(|l| l.as_str().to_owned())).await
        }
        Cmd::PreviewCancel { run } => preview_cancel(app, req, run).await,
        Cmd::ListJobs { binding } => {
            let ws = match binding_workspace(app, binding).await {
                Ok(Some(ws)) => ws,
                Ok(None) => return not_found("binding"),
                Err(e) => return db_failed(&e),
            };
            if let Err(d) = require(app, req, "list_jobs", Capability::View, ws).await {
                return d;
            }
            match list_jobs(app, binding).await {
                Ok(items) => vec![Event::Jobs { binding, items }],
                Err(e) => db_failed(&e),
            }
        }
        Cmd::ListPolicies { workspace } => {
            if let Err(d) = require(app, req, "list_policies", Capability::View, workspace).await {
                return d;
            }
            policies_event(app, workspace).await
        }
        Cmd::PreviewPolicyUpsert(spec) => preview_policy_upsert(app, req, spec).await,
        Cmd::PreviewPolicyRemove { policy } => preview_policy_remove(app, req, policy).await,
        Cmd::ExplainPolicy { job, limit } => {
            let Some((_, ws, name, def, _)) = (match load_job(app, job).await {
                Ok(j) => j,
                Err(e) => return db_failed(&e),
            }) else {
                return not_found("job");
            };
            if let Err(d) = require(app, req, "explain_policy", Capability::View, ws).await {
                return d;
            }
            let policies = match load_policies(&app.db, ws).await {
                Ok(p) => p,
                Err(e) => return db_failed(&e),
            };
            let limit = limit.map(|l| l.as_str().to_owned());
            let x = policy::resolve(
                &policies,
                &policy::Subject {
                    job: &name,
                    inventory: &def.inventory,
                    limit: limit.as_deref().or(def.limit.as_deref()),
                },
            );
            vec![Event::PolicyExplanation {
                job,
                limit,
                winner: x.winner.id,
                requires_approval: x.winner.requires_approval,
                reason: x.reason,
                checks: x
                    .checks
                    .into_iter()
                    .map(|c| PolicyCheck {
                        policy: c.policy,
                        priority: c.priority,
                        matched: c.matched,
                        why: c.why,
                    })
                    .collect(),
            }]
        }
        _ => internal(),
    }
}

fn pending<F, Fut>(f: F) -> PendingOp
where
    F: FnOnce(AppState, Caller) -> Fut + Send + 'static,
    Fut: Future<Output = Vec<Event>> + Send + 'static,
{
    Box::new(move |app, caller| Box::pin(f(app, caller)))
}

fn preview(app: &AppState, req: &Request<'_>, draft: Draft, op: PendingOp) -> Vec<Event> {
    let key = req.caller.preview_key(req.session);
    if key.is_empty() {
        return internal();
    }
    vec![Event::Preview(app.previews.create(&key, draft, op))]
}

/// Re-checks `cap` when a confirmed operation runs (grants may have changed since the preview).
fn recheck(caller: &Caller, action: &str, cap: Capability, ws: Uuid) -> Result<(), Vec<Event>> {
    let refused = match &caller.token {
        Some(t) if t.workspace.0 != ws => Some("token_scope"),
        _ => authz::require(&subject(caller), cap, WorkspaceId(ws), &[])
            .err()
            .map(|r| r.code()),
    };
    match refused {
        None => Ok(()),
        Some(reason) => Err(vec![Event::Denied {
            action: action.to_owned(),
            reason: reason.to_owned(),
        }]),
    }
}

async fn run_workspace(app: &AppState, run: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    let row: Option<(Uuid,)> = sqlx::query_as("select workspace_id from runs where id = $1")
        .bind(run)
        .fetch_optional(&app.db)
        .await?;
    Ok(row.map(|r| r.0))
}

type JobRow = (Uuid, Uuid, String, JobDefinition, Option<String>);

/// A current job: its binding, workspace, name, definition and why it is blocked.
pub(crate) async fn load_job(app: &AppState, job: Uuid) -> Result<Option<JobRow>, sqlx::Error> {
    let row: Option<(Uuid, Uuid, String, Value, Option<String>)> = sqlx::query_as(
        "select j.binding_id, p.workspace_id, j.name, j.definition, j.blocked_reason \
         from jobs j join repo_bindings b on b.id = j.binding_id \
         join projects p on p.id = b.project_id \
         where j.id = $1 and j.removed_at_commit is null",
    )
    .bind(job)
    .fetch_optional(&app.db)
    .await?;
    Ok(row.and_then(|(binding, ws, name, def, blocked)| {
        let def = serde_json::from_value(def).ok()?;
        Some((binding, ws, name, def, blocked))
    }))
}

async fn preview_run_start(
    app: &AppState,
    req: &Request<'_>,
    job: Uuid,
    limit: Option<String>,
) -> Vec<Event> {
    const ACTION: &str = "preview_run_start";
    let Some((binding, ws, name, def, blocked)) = (match load_job(app, job).await {
        Ok(j) => j,
        Err(e) => return db_failed(&e),
    }) else {
        return not_found("job");
    };
    if let Err(d) = require(app, req, ACTION, Capability::Operate, ws).await {
        return d;
    }
    if let Some(why) = blocked {
        return op_error("job_blocked", &format!("The job is blocked: {why}."));
    }
    let row: Option<(Option<String>, bool, String)> = match sqlx::query_as(
        "select last_commit, signed_mode, status from repo_bindings where id = $1",
    )
    .bind(binding)
    .fetch_optional(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let Some((Some(commit), signed_mode, status)) = row else {
        return op_error("not_synced", "The binding has not synced a commit yet.");
    };
    if status != "active" {
        return op_error("binding_disabled", "The binding is disabled.");
    }
    let mut conn = match app.db.acquire().await {
        Ok(c) => c,
        Err(e) => return db_failed(&e),
    };
    let p = match plan(&mut conn, ws, &name, &def, limit.as_deref()).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return op_error(e.code(), &e.to_string()),
        Err(e) => return db_failed(&e),
    };
    drop(conn);
    if p.unrunnable.is_some() {
        return op_error(
            "inventory_invalid",
            &format!(
                "Inventory {} has no valid revision to run against.",
                def.inventory
            ),
        );
    }
    if p.hosts.is_empty() {
        return op_error("no_hosts", "No enrolled host matches the job and limit.");
    }
    let affected: Vec<String> = p.hosts.iter().map(|h| h.1.clone()).collect();
    let summary = format!(
        "Run job {name} at {} on {} host(s) of inventory {} (revision {}){}{}.",
        &commit[..commit.len().min(12)],
        affected.len(),
        def.inventory,
        p.revision.map(|r| r.to_string()).unwrap_or_default(),
        limit
            .as_deref()
            .map(|l| format!(", limit {l}"))
            .unwrap_or_default(),
        if p.policy.requires_approval {
            ", awaiting approval"
        } else {
            ""
        },
    );
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Operate, ws) {
            return d;
        }
        let actor = caller.actor();
        let new = NewRun {
            workspace: ws,
            binding,
            job,
            job_name: &name,
            def: &def,
            trigger: "manual",
            delivery: None,
            requested_by: Some(caller.user.id),
            commit: &commit,
            run_limit: limit.as_deref(),
            signed_mode,
            actor: &actor,
            id: None,
            only_hosts: None,
        };
        match create_run(&app, &new).await {
            Ok(run) => match get_run(&app, run).await {
                Ok(Some(e)) => vec![e],
                Ok(None) => internal(),
                Err(e) => db_failed(&e),
            },
            Err(e) => db_failed(&e),
        }
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

async fn preview_cancel(app: &AppState, req: &Request<'_>, run: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_cancel";
    let row: Option<(Uuid, String, String, Vec<String>)> = match sqlx::query_as(
        "select workspace_id, job_name, state, resolved_hosts from runs where id = $1",
    )
    .bind(run)
    .fetch_optional(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let Some((ws, job, state, hosts)) = row else {
        return not_found("run");
    };
    if let Err(d) = require(app, req, ACTION, Capability::Operate, ws).await {
        return d;
    }
    if !cancellable(&state) {
        return op_error("not_cancellable", &format!("The run is {state}."));
    }
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Operate, ws) {
            return d;
        }
        match cancel_run(&app, &caller, run, ws).await {
            Ok(Ok(status)) => vec![Event::RunState {
                run,
                state: enum_from(status.state.as_str()).unwrap_or(RunPhase::Cancelled),
                reason: status.reason.and_then(|r| enum_from(r.as_str())),
            }],
            Ok(Err(state)) => op_error("not_cancellable", &format!("The run is {state}.")),
            Err(e) => db_failed(&e),
        }
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary: format!("Cancel run {run} of job {job}."),
            affected: hosts,
            step_up: false,
        },
        op,
    )
}

fn cancellable(state: &str) -> bool {
    state
        .parse::<RunState>()
        .is_ok_and(|s| keel_core::run::transition(s, &RunEvent::CancelledByUser).is_ok())
}

/// Cancels `run`: hosts not yet running stop here, running ones are told to stop.
async fn cancel_run(
    app: &AppState,
    caller: &Caller,
    run: Uuid,
    ws: Uuid,
) -> Result<Result<RunStatus, String>, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let (state,): (String,) =
        sqlx::query_as("select state from runs where id = $1 for no key update")
            .bind(run)
            .fetch_one(&mut *tx)
            .await?;
    let Ok(from) = state.parse::<RunState>() else {
        return Ok(Err(state));
    };
    let Ok(next) = keel_core::run::transition(from, &RunEvent::CancelledByUser) else {
        return Ok(Err(state));
    };
    sqlx::query("update runs set state = $2, state_reason = $3 where id = $1")
        .bind(run)
        .bind(next.state.as_str())
        .bind(next.reason.map(|r| r.as_str()))
        .execute(&mut *tx)
        .await?;
    // A host that was sent its run but has not started may drop it without a word: it is
    // cancelled here and told to stop, like a running one.
    let stop: Vec<(Uuid,)> = sqlx::query_as(
        "select host_id from run_hosts where run_id = $1 and state in ('sent', 'running')",
    )
    .bind(run)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query(
        "update run_hosts set state = 'cancelled', ended_at = now() \
         where run_id = $1 and state in ('queued', 'sent')",
    )
    .bind(run)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        Entry {
            actor: &caller.actor(),
            action: "run_cancelled",
            workspace: Some(WorkspaceId(ws)),
            target: json!({ "run": run }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({ "from": state }),
        },
    )
    .await?;
    tx.commit().await?;
    for (host,) in stop {
        crate::queue::cancel(app, run, host);
    }
    crate::queue::push_state(app, ws, run, next);
    crate::queue::maybe_finish(app, run).await;
    Ok(Ok(next))
}

// ---- reading runs ----------------------------------------------------------------------------

struct Filter {
    workspace: Uuid,
    binding: Option<Uuid>,
    host: Option<String>,
    actor: Option<Uuid>,
    state: Option<String>,
    from: Option<OffsetDateTime>,
    to: Option<OffsetDateTime>,
}

const RUN_COLUMNS: &str = "r.id, r.workspace_id, r.binding_id, r.job_id, r.job_name, r.trigger, \
     r.requested_by, r.commit, r.inventory_name, r.inventory_revision_id, \
     exists (select 1 from inventory_revisions cur join inventory_revisions newer \
       on newer.inventory_id = cur.inventory_id and newer.seq > cur.seq \
       where cur.id = r.inventory_revision_id) as newer, \
     r.run_limit, r.resolved_hosts, r.policy_id, r.state, r.state_reason, r.created_at, \
     r.started_at, r.ended_at, r.summary";

#[derive(sqlx::FromRow)]
struct RunRow {
    id: Uuid,
    workspace_id: Uuid,
    binding_id: Option<Uuid>,
    job_id: Option<Uuid>,
    job_name: String,
    trigger: String,
    requested_by: Option<Uuid>,
    commit: String,
    inventory_name: String,
    inventory_revision_id: Option<Uuid>,
    newer: bool,
    run_limit: Option<String>,
    resolved_hosts: Vec<String>,
    policy_id: Option<Uuid>,
    state: String,
    state_reason: Option<String>,
    created_at: OffsetDateTime,
    started_at: Option<OffsetDateTime>,
    ended_at: Option<OffsetDateTime>,
    summary: Option<Value>,
}

impl RunRow {
    fn info(self) -> RunInfo {
        RunInfo {
            id: self.id,
            workspace: self.workspace_id,
            binding: self.binding_id,
            job: self.job_id,
            job_name: self.job_name,
            trigger: enum_from(&self.trigger).unwrap_or(keel_protocol::RunTrigger::Manual),
            requested_by: self.requested_by,
            commit: self.commit,
            inventory: self.inventory_name,
            inventory_revision: self.inventory_revision_id,
            newer_inventory_revision: self.newer,
            limit: self.run_limit,
            hosts: self.resolved_hosts,
            policy: self.policy_id,
            state: enum_from(&self.state).unwrap_or(RunPhase::Failed),
            reason: self.state_reason.as_deref().and_then(enum_from),
            created_at: ts(self.created_at),
            started_at: self.started_at.map(ts),
            ended_at: self.ended_at.map(ts),
            summary: self.summary.and_then(|s| serde_json::from_value(s).ok()),
        }
    }
}

async fn list_runs(app: &AppState, f: &Filter) -> Result<Vec<RunInfo>, sqlx::Error> {
    let rows: Vec<RunRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {RUN_COLUMNS} from runs r where r.workspace_id = $1 \
         and ($2::uuid is null or r.binding_id = $2) \
         and ($3::text is null or $3 = any(r.resolved_hosts)) \
         and ($4::uuid is null or r.requested_by = $4) \
         and ($5::text is null or r.state = $5) \
         and ($6::timestamptz is null or r.created_at >= $6) \
         and ($7::timestamptz is null or r.created_at < $7) \
         order by r.created_at desc limit $8"
    )))
    .bind(f.workspace)
    .bind(f.binding)
    .bind(&f.host)
    .bind(f.actor)
    .bind(&f.state)
    .bind(f.from)
    .bind(f.to)
    .bind(LIST_LIMIT)
    .fetch_all(&app.db)
    .await?;
    Ok(rows.into_iter().map(RunRow::info).collect())
}

/// `Run`: the run and its hosts.
pub(crate) async fn get_run(app: &AppState, run: Uuid) -> Result<Option<Event>, sqlx::Error> {
    let row: Option<RunRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {RUN_COLUMNS} from runs r where r.id = $1"
    )))
    .bind(run)
    .fetch_optional(&app.db)
    .await?;
    let Some(row) = row else { return Ok(None) };
    type HostRow = (
        Uuid,
        String,
        String,
        Option<String>,
        Option<OffsetDateTime>,
        Option<OffsetDateTime>,
        Option<Value>,
        Option<Value>,
    );
    let hosts: Vec<HostRow> = sqlx::query_as(
        "select rh.host_id, rh.host_name, rh.state, rh.refusal, rh.started_at, rh.ended_at, \
         case when rr.run_id is null then null else json_build_object( \
           'ok', rr.ok, 'changed', rr.changed, 'failed', rr.failed, \
           'unreachable', rr.unreachable, 'skipped', rr.skipped, 'rescued', rr.rescued, \
           'ignored', rr.ignored)::jsonb end, rr.first_failure \
         from run_hosts rh left join run_host_results rr \
           on rr.run_id = rh.run_id and rr.host_id = rh.host_id \
         where rh.run_id = $1 order by rh.host_name",
    )
    .bind(run)
    .fetch_all(&app.db)
    .await?;
    let hosts = hosts
        .into_iter()
        .map(
            |(host, name, state, refusal, started, ended, stats, ff)| RunHostInfo {
                host,
                name,
                state: enum_from(&state).unwrap_or(keel_protocol::HostPhase::Failed),
                refusal,
                started_at: started.map(ts),
                ended_at: ended.map(ts),
                stats: stats.and_then(|s| serde_json::from_value::<HostStats>(s).ok()),
                first_failure: ff.and_then(|f| serde_json::from_value::<FirstFailure>(f).ok()),
            },
        )
        .collect();
    Ok(Some(Event::Run {
        run: row.info(),
        hosts,
    }))
}

async fn list_jobs(app: &AppState, binding: Uuid) -> Result<Vec<JobInfo>, sqlx::Error> {
    let rows: Vec<(Uuid, String, Value, Option<String>, String)> = sqlx::query_as(
        "select id, name, definition, blocked_reason, defined_at_commit from jobs \
         where binding_id = $1 and removed_at_commit is null order by name",
    )
    .bind(binding)
    .fetch_all(&app.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, name, def, blocked, at)| {
            let d: JobDefinition = serde_json::from_value(def).ok()?;
            Some(JobInfo {
                id,
                binding,
                name,
                playbook: d.playbook,
                profiles: d.profiles,
                inventory: d.inventory,
                limit: d.limit,
                paths: d.paths,
                credentials: d.credentials,
                blocked,
                defined_at_commit: at,
            })
        })
        .collect())
}

// ---- policies --------------------------------------------------------------------------------

type PolicyRow = (Uuid, i32, Value, bool, i32, bool, i64);

const POLICY_COLUMNS: &str = "id, priority, match, requires_approval, approvals_needed, four_eyes, \
     extract(epoch from approval_ttl)::bigint";

fn policy_of((id, priority, m, requires, needed, four_eyes, ttl): PolicyRow) -> Policy {
    // A match that cannot be read must not loosen approval: it matches nothing it should not,
    // and requires approval.
    let (matcher, requires) = match serde_json::from_value::<PolicyMatch>(m) {
        Ok(m) => (m, requires),
        Err(_) => (PolicyMatch::default(), true),
    };
    Policy {
        id,
        priority,
        matcher,
        requires_approval: requires,
        approvals_needed: u16::try_from(needed).unwrap_or(u16::MAX),
        four_eyes,
        approval_ttl_secs: u32::try_from(ttl).unwrap_or(u32::MAX),
    }
}

async fn load_policies<'e, E>(db: E, workspace: Uuid) -> Result<Vec<Policy>, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    let rows: Vec<PolicyRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {POLICY_COLUMNS} from approval_policies where workspace_id = $1 \
         order by priority, id"
    )))
    .bind(workspace)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(policy_of).collect())
}

fn policy_info(workspace: Uuid, p: Policy) -> PolicyInfo {
    PolicyInfo {
        id: p.id,
        workspace,
        priority: p.priority,
        catch_all: p.is_catch_all(),
        job: p.matcher.job,
        inventory: p.matcher.inventory,
        limit: p.matcher.limit,
        requires_approval: p.requires_approval,
        approvals_needed: p.approvals_needed,
        four_eyes: p.four_eyes,
        approval_ttl_secs: p.approval_ttl_secs,
    }
}

async fn policies_event(app: &AppState, workspace: Uuid) -> Vec<Event> {
    match load_policies(&app.db, workspace).await {
        Ok(ps) => vec![Event::Policies {
            workspace,
            items: ps.into_iter().map(|p| policy_info(workspace, p)).collect(),
        }],
        Err(e) => db_failed(&e),
    }
}

/// A policy row with its workspace first.
type WorkspacePolicyRow = (Uuid, Uuid, i32, Value, bool, i32, bool, i64);

async fn policy_row(app: &AppState, id: Uuid) -> Result<Option<(Uuid, Policy)>, sqlx::Error> {
    let row: Option<WorkspacePolicyRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select workspace_id, {POLICY_COLUMNS} from approval_policies where id = $1"
    )))
    .bind(id)
    .fetch_optional(&app.db)
    .await?;
    Ok(row.map(|(ws, id, pr, m, ra, an, fe, ttl)| (ws, policy_of((id, pr, m, ra, an, fe, ttl)))))
}

fn policy_error(e: &policy::PolicyError) -> Vec<Event> {
    op_error(e.code(), &format!("{e}."))
}

async fn preview_policy_upsert(app: &AppState, req: &Request<'_>, spec: PolicySpec) -> Vec<Event> {
    const ACTION: &str = "preview_policy_upsert";
    let ws = spec.workspace;
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    let existing = match spec.id {
        Some(id) => match policy_row(app, id).await {
            Ok(Some((pws, p))) if pws == ws => Some(p),
            Ok(_) => return not_found("policy"),
            Err(e) => return db_failed(&e),
        },
        None => None,
    };
    let p = Policy {
        id: spec.id.unwrap_or_else(Uuid::now_v7),
        priority: spec.priority,
        matcher: PolicyMatch {
            job: spec.job.map(|j| j.as_str().to_owned()),
            inventory: spec.inventory.map(|i| i.as_str().to_owned()),
            limit: spec.limit.map(|l| l.as_str().to_owned()),
        },
        requires_approval: spec.requires_approval,
        approvals_needed: spec.approvals_needed,
        four_eyes: spec.four_eyes,
        approval_ttl_secs: spec.approval_ttl_secs,
    };
    if let Err(e) = policy::validate(&p, existing.as_ref()) {
        return policy_error(&e);
    }
    let summary = format!(
        "{} approval policy at priority {}: {}{}.",
        if existing.is_some() {
            "Change the"
        } else {
            "Add an"
        },
        p.priority,
        if p.requires_approval {
            format!(
                "requires {} approval(s){}",
                p.approvals_needed,
                if p.four_eyes { ", four eyes" } else { "" }
            )
        } else {
            "runs without approval".to_owned()
        },
        describe_match(&p.matcher),
    );
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        match upsert_policy(&app, &caller, ws, &p, existing.is_some()).await {
            Ok(None) => policies_event(&app, ws).await,
            Ok(Some(e)) => e,
            Err(e) => db_failed(&e),
        }
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected: Vec::new(),
            step_up: true,
        },
        op,
    )
}

fn describe_match(m: &PolicyMatch) -> String {
    let mut parts = Vec::new();
    if let Some(j) = &m.job {
        parts.push(format!("job {j}"));
    }
    if let Some(i) = &m.inventory {
        parts.push(format!("inventory {i}"));
    }
    if let Some(l) = &m.limit {
        parts.push(format!("limit {l}"));
    }
    if parts.is_empty() {
        ", for every run".to_owned()
    } else {
        format!(", for {}", parts.join(", "))
    }
}

/// Writes `p`; `Some(events)` is a refusal.
async fn upsert_policy(
    app: &AppState,
    caller: &Caller,
    ws: Uuid,
    p: &Policy,
    update: bool,
) -> Result<Option<Vec<Event>>, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let m = serde_json::to_value(&p.matcher).unwrap_or_else(|_| json!({}));
    let sql = if update {
        "update approval_policies set priority = $3, match = $4, requires_approval = $5, \
         approvals_needed = $6, four_eyes = $7, approval_ttl = make_interval(secs => $8) \
         where id = $1 and workspace_id = $2"
    } else {
        "insert into approval_policies (id, workspace_id, priority, match, requires_approval, \
         approvals_needed, four_eyes, approval_ttl) \
         values ($1, $2, $3, $4, $5, $6, $7, make_interval(secs => $8))"
    };
    let r = sqlx::query(sql)
        .bind(p.id)
        .bind(ws)
        .bind(p.priority)
        .bind(&m)
        .bind(p.requires_approval)
        .bind(i32::from(p.approvals_needed))
        .bind(p.four_eyes)
        .bind(f64::from(p.approval_ttl_secs))
        .execute(&mut *tx)
        .await;
    match r {
        Ok(_) => {}
        Err(e) if db_code(&e).as_deref() == Some("23505") => {
            return Ok(Some(op_error(
                "priority_taken",
                "Another policy already has that priority.",
            )));
        }
        Err(e) if db_code(&e).as_deref() == Some("23514") => {
            return Ok(Some(policy_error(&policy::PolicyError::CatchAllWeakened)));
        }
        Err(e) => return Err(e),
    }
    audit::record(
        &mut tx,
        Entry {
            actor: &caller.actor(),
            action: if update {
                "policy_updated"
            } else {
                "policy_created"
            },
            workspace: Some(WorkspaceId(ws)),
            target: json!({ "policy": p.id }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "priority": p.priority,
                "match": m,
                "requires_approval": p.requires_approval,
                "approvals_needed": p.approvals_needed,
                "four_eyes": p.four_eyes,
                "approval_ttl_secs": p.approval_ttl_secs,
            }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(None)
}

fn db_code(e: &sqlx::Error) -> Option<String> {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.into_owned())
}

async fn preview_policy_remove(app: &AppState, req: &Request<'_>, id: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_policy_remove";
    let (ws, p) = match policy_row(app, id).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("policy"),
        Err(e) => return db_failed(&e),
    };
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    if let Err(e) = policy::check_remove(&p) {
        return policy_error(&e);
    }
    let summary = format!(
        "Remove the approval policy at priority {}{}.",
        p.priority,
        describe_match(&p.matcher)
    );
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        let result: Result<(), sqlx::Error> = async {
            let mut tx = app.db.begin().await?;
            sqlx::query("delete from approval_policies where id = $1 and workspace_id = $2")
                .bind(id)
                .bind(ws)
                .execute(&mut *tx)
                .await?;
            audit::record(
                &mut tx,
                Entry {
                    actor: &caller.actor(),
                    action: "policy_removed",
                    workspace: Some(WorkspaceId(ws)),
                    target: json!({ "policy": id }),
                    decision: Decision::Allowed,
                    reason: None,
                    context: json!({ "priority": p.priority }),
                },
            )
            .await?;
            tx.commit().await
        }
        .await;
        match result {
            Ok(()) => policies_event(&app, ws).await,
            Err(e) if db_code(&e).as_deref() == Some("23514") => {
                policy_error(&policy::PolicyError::CatchAllPermanent)
            }
            Err(e) => db_failed(&e),
        }
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected: Vec::new(),
            step_up: true,
        },
        op,
    )
}
