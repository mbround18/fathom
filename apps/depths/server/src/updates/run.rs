//! The update runner (research R2, R3, R4, R7; FR-009 to FR-018, FR-023, FR-027).
//!
//! A run is a batch (an administrator's `Confirm`) or a scheduled pass, and runs its stacks in
//! order, one at a time, each under the stack's `Ops` lock. For each stack:
//!
//! 1. **Pull** each new image by digest (`docker pull <repo>@<D>`), once per digest.
//! 2. **Check** the pulled image: root escalation and the compose lint ([`hold`]). A hold leaves
//!    the tag and the containers as they are, tags the image `depths-keep/<s>:<svc>-held`, and is
//!    audited `image_hold`.
//! 3. **Keep** the running image as `depths-keep/<s>:<svc>-previous`, then **tag** the new image
//!    as the compose reference and **recreate** only the changed services
//!    (`up -d --no-deps --pull never`). The compose file is never written.
//! 4. **Wait** for health (research R3), then, when the cleanup setting is on, **clean** the image
//!    the previous keep tag pointed at (`docker image rm`, never `-f`, never `prune`).
//! 5. A failed wait **rolls back**: the previous image is tagged back and `up` runs again. The
//!    digest is rejected and the image kept as `…-held`. A second failure is `RollbackFailed`
//!    and waits for a person; there is no third attempt.
//!
//! The run's journal (`update-state.json → journal`) is written with fsync before each mutating
//! step. A run found open at start ([`interrupt_open_runs`]) is closed as `Interrupted`, never
//! resumed or rolled back.
//!
//! 054's guard is not in the tree: a guard refusal can only come back from `up`, recognised by
//! [`GUARD_REFUSED`] in its stderr, and is a hold when the old containers still run.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use depths_core::StackName;
use depths_core::compose_lint::ComposePolicy;
use depths_core::plan::{self, ImagePin, RollbackPin, RunStep, StackContext};
use depths_core::ports::{
    CancelToken, ContainerRuntime, HealthStatus, OutputChunk, OutputSink, StepOutcome,
};
use depths_core::security::DaemonMode;
use depths_core::updates::hold::{self, LintDecision, Trigger, UserCheck};
use depths_core::updates::keep::{self, KeepKind};
use depths_protocol::{
    Actor, AuditAction, AuditOutcome, AuditServiceUpdate, AuditUpdate, Event, EventEnvelope,
    OpOutcome, OperationId, Provider, RequestId, RunId, RunPhase, RunTrigger, ServiceChange,
    StackOutcome, StackRun, UpdateRun,
};
use tokio::sync::mpsc;

use super::Updates;
use super::store::{KeepRecord, Rejected};
use crate::audit;
use crate::ops::{Ops, Refusal};

/// How often the health wait reads the containers (research R3).
pub const POLL: Duration = Duration::from_secs(2);

/// What 054's guard puts in a refused request's error, as `up` relays it on stderr.
pub const GUARD_REFUSED: &str = "refused by depths-guard";

/// The actor of a scheduled run (FR-016).
pub fn scheduled_actor() -> Actor {
    Actor {
        provider: Provider::Depths,
        subject: "scheduled".into(),
        username: "Depths (scheduled)".into(),
    }
}

/// One service a stack's update recreates, as the preview showed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePin {
    pub service: String,
    /// The compose `image:` as written.
    pub reference: String,
    /// `<repo>@<digest>`: what is pulled.
    pub pull: String,
    /// The digest the preview showed (index or manifest).
    pub digest: String,
    pub platform: Option<String>,
    /// The new image's id when known before the pull (the config digest).
    pub to_image: Option<String>,
    /// The image id the service runs now.
    pub from_image: String,
    pub from_digest: Option<String>,
    pub version: Option<String>,
    /// The service's compose `user:`.
    pub compose_user: Option<String>,
    /// The running container's user (`""`: the image default).
    pub old_user: String,
    /// Containers the service runs; at least one is waited for.
    pub replicas: u32,
}

/// The services a stack's pins recreate, and the plan's image pins (the preview's argv and the
/// user `Update` path, FR-026, use the same).
pub fn image_pins(pins: &[ServicePin]) -> (Vec<depths_core::ServiceName>, Vec<ImagePin>) {
    let services = pins
        .iter()
        .filter_map(|p| depths_core::ServiceName::parse(&p.service).ok())
        .collect();
    let image = pins
        .iter()
        .map(|p| ImagePin {
            pull: p.pull.clone(),
            platform: p.platform.clone(),
            image: p.to_image.clone().unwrap_or_else(|| p.pull.clone()),
            reference: p.reference.clone(),
            services: depths_core::ServiceName::parse(&p.service)
                .into_iter()
                .collect(),
        })
        .collect();
    (services, image)
}

/// One stack of a run.
#[derive(Debug, Clone)]
pub struct StackJob {
    pub stack: String,
    /// The content hash the preview bound; a different one now is stale.
    pub stack_hash: String,
    pub pins: Vec<ServicePin>,
    /// The confirm acknowledged the compose findings (manual runs only).
    pub acknowledged: bool,
    /// `ApplyHeld`: an administrator overrode the root-escalation hold.
    pub apply_held: bool,
}

/// A confirmed run, ready for [`Runner::run`].
pub struct Batch {
    pub run_id: RunId,
    pub trigger: RunTrigger,
    pub ip: IpAddr,
    pub jobs: Vec<StackJob>,
    /// Cancelling it stops the stack in progress and skips the rest.
    pub cancel: CancelToken,
    /// The `Confirm` it answers: `UpdateRunStarted` is its reply.
    pub reply_to: Option<RequestId>,
}

/// How the runner waits between health reads; tests pass a no-op.
pub type Sleep = Arc<dyn Fn(Duration) + Send + Sync>;

pub struct Runner {
    pub ops: Arc<Ops>,
    pub updates: Arc<Updates>,
    pub sleep: Sleep,
}

/// What one stack came to, for its audit entry and `UpdateStackDone`.
struct Done {
    outcome: StackOutcome,
    action: AuditAction,
    audited: AuditOutcome,
    hold: Option<String>,
    rejected: Option<String>,
    argv: Vec<Vec<String>>,
}

impl Done {
    fn skipped(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            audited: AuditOutcome::Failed {
                exit_code: None,
                stderr_tail: format!("skipped: {reason}"),
            },
            outcome: StackOutcome::Skipped { reason },
            action: AuditAction::ImageUpdate,
            hold: None,
            rejected: None,
            argv: Vec::new(),
        }
    }

    fn failed(
        outcome: StackOutcome,
        action: AuditAction,
        why: String,
        argv: Vec<Vec<String>>,
    ) -> Self {
        Self {
            outcome,
            action,
            audited: AuditOutcome::Failed {
                exit_code: None,
                stderr_tail: why,
            },
            hold: None,
            rejected: None,
            argv,
        }
    }
}

/// Sends a stack's output as `OpOutput`.
struct Sink {
    operation_id: OperationId,
    tx: mpsc::Sender<EventEnvelope>,
}

impl OutputSink for Sink {
    fn emit(&self, chunk: OutputChunk) -> bool {
        send(
            &self.tx,
            Event::OpOutput {
                operation_id: self.operation_id,
                chunk: chunk.text,
            },
        );
        true
    }
}

fn send(tx: &mpsc::Sender<EventEnvelope>, event: Event) {
    let _ = tx.blocking_send(EventEnvelope {
        reply_to: None,
        event,
    });
}

/// The stack being run.
struct Cur<'a> {
    name: StackName,
    sink: Sink,
    cancel: &'a CancelToken,
    argv: Vec<Vec<String>>,
    actor: Actor,
    on_behalf_of: Option<Actor>,
    ip: IpAddr,
}

impl Runner {
    fn runtime(&self) -> &Arc<dyn ContainerRuntime> {
        self.ops.runtime()
    }

    fn now(&self) -> String {
        self.updates
            .clock()
            .now()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    /// Runs the batch to the end, whether or not anyone listens on `tx`. Blocking: call from
    /// `spawn_blocking`. Sends `UpdateRunStarted`, per stack `OpStarted`, `OpOutput`,
    /// `OpFinished` and `UpdateStackDone`, then `UpdateRunFinished`.
    pub fn run(&self, batch: Batch, tx: mpsc::Sender<EventEnvelope>) -> UpdateRun {
        let mut run = UpdateRun {
            run_id: batch.run_id,
            trigger: batch.trigger.clone(),
            stacks: batch
                .jobs
                .iter()
                .map(|j| StackRun {
                    stack: j.stack.clone(),
                    services: j
                        .pins
                        .iter()
                        .map(|p| ServiceChange {
                            service: p.service.clone(),
                            from_image: p.from_image.clone(),
                            to_digest: p.digest.clone(),
                            to_image: p.to_image.clone(),
                        })
                        .collect(),
                    operation_id: None,
                    phase: RunPhase::Pending,
                    outcome: None,
                })
                .collect(),
            started_at: self.now(),
            finished_at: None,
        };
        let _ = self
            .updates
            .store()
            .update_observed(|o| o.upsert_run(run.clone()));
        let _ = tx.blocking_send(EventEnvelope {
            reply_to: batch.reply_to,
            event: Event::UpdateRunStarted {
                run_id: run.run_id,
                trigger: run.trigger.clone(),
                stacks: batch.jobs.iter().map(|j| j.stack.clone()).collect(),
            },
        });
        let daemon = self.runtime().daemon_mode().unwrap_or(DaemonMode::Rootful);
        let policy = self.ops.compose_policy();
        for (i, job) in batch.jobs.iter().enumerate() {
            let (actor, on_behalf_of) = match &batch.trigger {
                RunTrigger::Manual { actor } => (actor.clone(), None),
                RunTrigger::Scheduled { on_behalf_of } => {
                    (scheduled_actor(), on_behalf_of.get(&job.stack).cloned())
                }
            };
            let trigger = match batch.trigger {
                RunTrigger::Manual { .. } => Trigger::Manual {
                    acknowledged: job.acknowledged,
                },
                RunTrigger::Scheduled { .. } => Trigger::Scheduled,
            };
            let (done, op_id) = if batch.cancel.is_cancelled() {
                (Done::skipped("cancelled"), None)
            } else {
                self.stack(
                    &mut run,
                    i,
                    job,
                    (&actor, &on_behalf_of),
                    trigger,
                    daemon,
                    policy,
                    &batch,
                    &tx,
                )
            };
            let mut entry = audit::entry(actor, done.action, batch.ip, done.audited.clone());
            entry.stack = Some(job.stack.clone());
            entry.operation_id = op_id;
            entry.argv = done.argv.clone();
            entry.on_behalf_of = on_behalf_of;
            entry.update = Some(AuditUpdate {
                run_id: run.run_id,
                services: job
                    .pins
                    .iter()
                    .map(|p| AuditServiceUpdate {
                        service: p.service.clone(),
                        from_image: p.from_image.clone(),
                        from_digest: p.from_digest.clone(),
                        to_image: p.to_image.clone(),
                        to_digest: p.digest.clone(),
                        version: p.version.clone(),
                    })
                    .collect(),
                rejected_digest: done.rejected.clone(),
                hold: done.hold.clone(),
            });
            let audit_at = entry.at.clone();
            let _ = self.ops.audit_log().record(&entry);
            run.stacks[i].phase = RunPhase::Done;
            run.stacks[i].outcome = Some(done.outcome.clone());
            let _ = self
                .updates
                .store()
                .update_observed(|o| o.upsert_run(run.clone()));
            send(
                &tx,
                Event::UpdateStackDone {
                    run_id: run.run_id,
                    stack: job.stack.clone(),
                    outcome: done.outcome,
                    audit_at,
                },
            );
        }
        run.finished_at = Some(self.now());
        let _ = self
            .updates
            .store()
            .update_observed(|o| o.upsert_run(run.clone()));
        send(
            &tx,
            Event::UpdateRunFinished {
                run_id: run.run_id,
                summary: run
                    .stacks
                    .iter()
                    .map(|s| {
                        (
                            s.stack.clone(),
                            s.outcome.clone().unwrap_or(StackOutcome::Interrupted),
                        )
                    })
                    .collect(),
            },
        );
        self.updates.notify_changed();
        run
    }

    /// One stack under its lock: claim, run, release, `OpFinished`.
    #[allow(clippy::too_many_arguments)]
    fn stack(
        &self,
        run: &mut UpdateRun,
        i: usize,
        job: &StackJob,
        (actor, on_behalf_of): (&Actor, &Option<Actor>),
        trigger: Trigger,
        daemon: DaemonMode,
        policy: ComposePolicy,
        batch: &Batch,
        tx: &mpsc::Sender<EventEnvelope>,
    ) -> (Done, Option<OperationId>) {
        let Ok(name) = StackName::parse(&job.stack) else {
            return (Done::skipped("not a valid stack name"), None);
        };
        let user = format!("{}:{}", word(&actor.provider), actor.subject);
        let operation_id =
            match self
                .ops
                .claim_update(&job.stack, actor.clone(), user, batch.cancel.clone())
            {
                Ok(id) => id,
                Err(Refusal::Busy { kind, actor, .. }) => {
                    return (
                        Done::skipped(format!("busy: {} {}", actor.username, kind_word(kind))),
                        None,
                    );
                }
                Err(_) => return (Done::skipped("busy"), None),
            };
        let hash_now = self
            .ops
            .stack_store()
            .entry(&name)
            .ok()
            .and_then(|e| e.content_hash)
            .map(|h| h.as_str().to_owned());
        let done = if hash_now.as_deref() != Some(job.stack_hash.as_str()) {
            Done::skipped("stale: preview again")
        } else {
            send(tx, Event::OpStarted { operation_id });
            run.stacks[i].operation_id = Some(operation_id);
            let mut cur = Cur {
                name,
                sink: Sink {
                    operation_id,
                    tx: tx.clone(),
                },
                cancel: &batch.cancel,
                argv: Vec::new(),
                actor: actor.clone(),
                on_behalf_of: on_behalf_of.clone(),
                ip: batch.ip,
            };
            let mut done = self.update(run, i, job, trigger, daemon, policy, &mut cur);
            done.argv = cur.argv;
            done
        };
        self.ops.release_update(&job.stack);
        if run.stacks[i].operation_id.is_some() {
            send(
                tx,
                Event::OpFinished {
                    operation_id,
                    outcome: op_outcome(&done.outcome),
                    guard: None,
                },
            );
        }
        (done, run.stacks[i].operation_id)
    }

    /// Writes the stack's phase (fsync) before the next mutating step.
    fn phase(&self, run: &mut UpdateRun, i: usize, phase: RunPhase) -> Result<(), String> {
        run.stacks[i].phase = phase;
        self.updates
            .store()
            .update_observed(|o| o.upsert_run(run.clone()))
            .map_err(|e| format!("the run journal could not be written: {e}"))
    }

    fn step(&self, cur: &mut Cur, step: &RunStep) -> Result<(), String> {
        cur.argv.push(step.argv.clone());
        match self.runtime().run_step(step, None, &cur.sink, cur.cancel) {
            Ok(StepOutcome::Succeeded) => Ok(()),
            Ok(StepOutcome::Failed { stderr_tail, .. }) => Err(stderr_tail),
            Ok(StepOutcome::Cancelled) => Err("cancelled".into()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn ctx<'a>(&'a self, name: &'a StackName) -> StackContext<'a> {
        let store = self.ops.stack_store();
        StackContext {
            stacks_root: store.root(),
            stack: name,
            global_env: store.global_env(),
            has_env: store.entry(name).map(|e| e.has_env).unwrap_or(false),
        }
    }

    /// Sets a keep tag and records it in `update-state.json → keep`. Returns the image the tag
    /// recorded before, if another.
    fn keep(
        &self,
        cur: &mut Cur,
        service: &str,
        kind: KeepKind,
        image: &str,
    ) -> Result<Option<String>, String> {
        let tag = keep::tag(cur.name.as_str(), service, kind);
        let step = plan::image_tag(&self.ctx(&cur.name), image, &tag);
        self.step(cur, &step)?;
        let at = self.now();
        let stack = cur.name.as_str().to_owned();
        self.updates
            .store()
            .update_observed(|o| {
                let before = o
                    .keep
                    .iter()
                    .position(|k| k.tag == tag)
                    .map(|p| o.keep.remove(p).image_id);
                o.keep.push(KeepRecord {
                    stack,
                    service: service.to_owned(),
                    tag: tag.clone(),
                    image_id: image.to_owned(),
                    at,
                });
                before.filter(|b| b != image)
            })
            .map_err(|e| format!("the keep tag could not be recorded: {e}"))
    }

    #[allow(clippy::too_many_arguments)]
    fn update(
        &self,
        run: &mut UpdateRun,
        i: usize,
        job: &StackJob,
        trigger: Trigger,
        daemon: DaemonMode,
        policy: ComposePolicy,
        cur: &mut Cur,
    ) -> Done {
        let (services, image_pins) = image_pins(&job.pins);
        let failed = |why: String, argv: &Vec<Vec<String>>| {
            Done::failed(
                StackOutcome::Failed,
                AuditAction::ImageUpdate,
                why,
                argv.clone(),
            )
        };

        // 1. Pull.
        if let Err(e) = self.phase(run, i, RunPhase::Pulling) {
            return failed(e, &cur.argv);
        }
        let phases = plan::image_update_phases(&self.ctx(&cur.name), &services, &image_pins);
        for s in &phases.pulls {
            if let Err(e) = self.step(cur, s) {
                return failed(format!("pull failed: {e}"), &cur.argv);
            }
        }

        // 2. Check what was pulled.
        if let Err(e) = self.phase(run, i, RunPhase::Checking) {
            return failed(e, &cur.argv);
        }
        let pulled: Vec<String> = job.pins.iter().map(|p| p.pull.clone()).collect();
        let infos = self.runtime().image_inspect(&pulled).unwrap_or_default();
        let new_id = |p: &ServicePin| -> (String, Option<String>) {
            // Docker shortens `docker.io/library/x` to `x` in RepoDigests, so match the digest.
            let at = format!("@{}", p.digest);
            let info = infos.iter().find(|i| {
                i.repo_digests
                    .iter()
                    .any(|d| d == &p.pull || d.ends_with(&at))
                    || i.id == p.pull
                    || p.to_image.as_deref() == Some(i.id.as_str())
            });
            match info {
                Some(i) if !i.id.is_empty() => (i.id.clone(), i.user.clone()),
                _ => (p.to_image.clone().unwrap_or_else(|| p.pull.clone()), None),
            }
        };
        let compose = self
            .ops
            .stack_store()
            .read(&cur.name)
            .map(|f| f.compose)
            .unwrap_or_default();
        let lint = hold::lint_check(&compose, policy, trigger);
        let codes = lint.escape_codes().join(", ");
        match lint.decision {
            LintDecision::Proceed => {}
            LintDecision::Refuse => {
                return Done::skipped(format!(
                    "compose_blocked: DEPTHS_COMPOSE_POLICY=enforce and {codes}"
                ));
            }
            LintDecision::NeedsAck => {
                return Done::skipped(format!("acknowledge the compose findings: {codes}"));
            }
            LintDecision::Hold => {
                return self.held(cur, job, &new_id, format!("compose: {codes}"), codes);
            }
        }
        if !job.apply_held {
            for p in &job.pins {
                let (_, new_user) = new_id(p);
                if let UserCheck::Hold(r) = hold::user_check(
                    p.compose_user.as_deref(),
                    &p.old_user,
                    new_user.as_deref(),
                    daemon,
                ) {
                    return self.held(
                        cur,
                        job,
                        &new_id,
                        format!("{}: {}", p.service, r.message),
                        r.code.as_str().to_owned(),
                    );
                }
            }
        }

        // 3. Keep the running images, tag the new ones, recreate.
        if let Err(e) = self.phase(run, i, RunPhase::Recreating) {
            return failed(e, &cur.argv);
        }
        let mut replaced: Vec<String> = Vec::new();
        for p in &job.pins {
            match self.keep(cur, &p.service, KeepKind::Previous, &p.from_image) {
                Ok(Some(old)) => replaced.push(old),
                Ok(None) => {}
                Err(e) => return failed(format!("keep tag: {e}"), &cur.argv),
            }
        }
        let name = cur.name.clone();
        let ctx = self.ctx(&name);
        for p in &job.pins {
            let (id, _) = new_id(p);
            if let Err(e) = self.step(cur, &plan::image_tag(&ctx, &id, &p.reference)) {
                return failed(format!("tag failed: {e}"), &cur.argv);
            }
        }
        let names: Vec<String> = job.pins.iter().map(|p| p.service.clone()).collect();
        let expect = job.pins.iter().map(|p| p.replicas.max(1) as usize).sum();
        let timeout = self.updates.store().settings().health_timeout_secs;
        let first = match phases.up.as_ref().map(|up| self.step(cur, up)) {
            Some(Err(e)) if e.contains(GUARD_REFUSED) && self.still_running(&cur.name, &names) => {
                // Put the reference back; nothing was recreated.
                for p in &job.pins {
                    let _ = self.step(cur, &plan::image_tag(&ctx, &p.from_image, &p.reference));
                }
                return self.held(cur, job, &new_id, e, "guard".into());
            }
            Some(Err(e)) => Err(format!("up failed: {e}")),
            _ => {
                if let Err(e) = self.phase(run, i, RunPhase::Waiting) {
                    return failed(e, &cur.argv);
                }
                self.wait(&cur.name, &names, expect, timeout)
            }
        };

        match first {
            Ok(()) => {
                // 4. Clean.
                if self.updates.store().settings().cleanup && !replaced.is_empty() {
                    let _ = self.phase(run, i, RunPhase::Cleaning);
                    self.clean(cur, &replaced);
                }
                Done {
                    outcome: StackOutcome::Updated,
                    action: AuditAction::ImageUpdate,
                    audited: AuditOutcome::Succeeded,
                    hold: None,
                    rejected: None,
                    argv: cur.argv.clone(),
                }
            }
            Err(why) => self.roll_back(
                run, i, job, &services, &names, expect, timeout, &new_id, why, cur,
            ),
        }
    }

    /// The services' containers are still running (a refused `up` changed nothing).
    fn still_running(&self, name: &StackName, services: &[String]) -> bool {
        let ids = self.ids_of(name, services);
        !ids.is_empty()
            && self
                .runtime()
                .containers_health(&ids)
                .is_ok_and(|h| h.len() == ids.len() && h.iter().all(|c| c.running))
    }

    /// A hold: the tag and the containers are left as they are, the pulled image is kept as
    /// `…-held`, audited `image_hold`.
    fn held(
        &self,
        cur: &mut Cur,
        job: &StackJob,
        new_id: &dyn Fn(&ServicePin) -> (String, Option<String>),
        why: String,
        code: String,
    ) -> Done {
        for p in &job.pins {
            let (id, _) = new_id(p);
            let _ = self.keep(cur, &p.service, KeepKind::Held, &id);
        }
        Done {
            outcome: StackOutcome::Held,
            action: AuditAction::ImageHold,
            audited: AuditOutcome::Failed {
                exit_code: None,
                stderr_tail: format!("held: {why}"),
            },
            hold: Some(code),
            rejected: None,
            argv: cur.argv.clone(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn roll_back(
        &self,
        run: &mut UpdateRun,
        i: usize,
        job: &StackJob,
        services: &[depths_core::ServiceName],
        names: &[String],
        expect: usize,
        timeout: u32,
        new_id: &dyn Fn(&ServicePin) -> (String, Option<String>),
        why: String,
        cur: &mut Cur,
    ) -> Done {
        let _ = self.phase(run, i, RunPhase::RollingBack);
        let pins: Vec<RollbackPin> = job
            .pins
            .iter()
            .map(|p| RollbackPin {
                image: p.from_image.clone(),
                reference: p.reference.clone(),
            })
            .collect();
        let steps = plan::image_rollback_steps(&self.ctx(&cur.name), services, &pins);
        let mut second = Ok(());
        for s in &steps {
            if let Err(e) = self.step(cur, s) {
                second = Err(format!("rollback step failed: {e}"));
                break;
            }
        }
        if second.is_ok() {
            second = self.wait(&cur.name, names, expect, timeout);
        }
        let at = self.now();
        let digests: Vec<String> = job.pins.iter().map(|p| p.digest.clone()).collect();
        let _ =
            self.updates.store().update_decisions(|d| {
                for p in &job.pins {
                    if !d.rejected.iter().any(|r| {
                        r.stack == job.stack && r.service == p.service && r.digest == p.digest
                    }) {
                        d.rejected.push(Rejected {
                            stack: job.stack.clone(),
                            service: p.service.clone(),
                            digest: p.digest.clone(),
                            at: at.clone(),
                            reason: format!("rolled back: {why}"),
                            run_id: Some(run.run_id),
                        });
                    }
                }
            });
        for p in &job.pins {
            let (id, _) = new_id(p);
            let _ = self.keep(cur, &p.service, KeepKind::Held, &id);
        }
        let rejected = Some(digests.join(","));
        match second {
            Ok(()) => Done {
                outcome: StackOutcome::RolledBack,
                action: AuditAction::ImageRollback,
                audited: AuditOutcome::Failed {
                    exit_code: None,
                    stderr_tail: format!("rolled back: {why}"),
                },
                hold: None,
                rejected,
                argv: cur.argv.clone(),
            },
            Err(again) => Done {
                rejected,
                ..Done::failed(
                    StackOutcome::RollbackFailed,
                    AuditAction::ImageRollback,
                    format!("{why}; rollback failed: {again}"),
                    cur.argv.clone(),
                )
            },
        }
    }

    /// Removes images a `…-previous` tag recorded before this run, when no keep tag records them
    /// any more. A daemon refusal (in use) is a note, not a failure. Each removal is audited
    /// `image_remove` with the image id and its repo digests.
    fn clean(&self, cur: &mut Cur, replaced: &[String]) {
        let kept: BTreeSet<String> = self
            .updates
            .store()
            .observed()
            .keep
            .iter()
            .map(|k| k.image_id.clone())
            .collect();
        let name = cur.name.clone();
        let ctx = self.ctx(&name);
        for id in replaced {
            if kept.contains(id) {
                continue;
            }
            let digests = self
                .runtime()
                .image_inspect(std::slice::from_ref(id))
                .ok()
                .and_then(|v| v.into_iter().next())
                .map(|i| i.repo_digests)
                .unwrap_or_default();
            match self.step(cur, &plan::image_rm(&ctx, id)) {
                Ok(()) => {
                    let mut e = audit::entry(
                        cur.actor.clone(),
                        AuditAction::ImageRemove,
                        cur.ip,
                        AuditOutcome::Succeeded,
                    );
                    e.on_behalf_of = cur.on_behalf_of.clone();
                    e.stack = Some(cur.name.as_str().to_owned());
                    e.argv = vec![plan::image_rm(&ctx, id).argv];
                    e.diff_summary = Some(format!("removed {id} ({})", digests.join(", ")));
                    let _ = self.ops.audit_log().record(&e);
                }
                Err(why) => {
                    cur.sink.emit(OutputChunk {
                        stream: depths_core::ports::OutputStream::Stderr,
                        text: format!("kept: in use: {id} ({})\n", why.trim()),
                    });
                }
            }
        }
    }

    fn ids_of(&self, name: &StackName, services: &[String]) -> Vec<String> {
        self.runtime()
            .containers_by_project()
            .ok()
            .and_then(|mut s| s.projects.remove(name.as_str()))
            .map(|p| {
                p.containers
                    .into_iter()
                    .filter(|c| c.service.as_ref().is_some_and(|s| services.contains(s)))
                    .map(|c| c.id)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The health wait (research R3). With a healthcheck a container must report healthy; an
    /// unhealthy one fails at once. Without one it must stay running with an unchanged restart
    /// count for the whole timeout. A missing replica, an exited container or the timeout fail.
    fn wait(
        &self,
        name: &StackName,
        services: &[String],
        expect: usize,
        timeout_secs: u32,
    ) -> Result<(), String> {
        let polls = (u64::from(timeout_secs) / POLL.as_secs()).max(1);
        let mut restarts: BTreeMap<String, u32> = BTreeMap::new();
        let mut last = String::from("timed out");
        for n in 0..=polls {
            if n > 0 {
                (self.sleep)(POLL);
            }
            let ids = self.ids_of(name, services);
            let health = self.runtime().containers_health(&ids).unwrap_or_default();
            if health.len() < expect {
                last = format!(
                    "{} of {expect} containers running after {timeout_secs}s",
                    health.len()
                );
                continue;
            }
            if let Some(c) = health.iter().find(|c| !c.running) {
                return Err(format!("container {} exited ({})", short(&c.id), c.status));
            }
            if let Some(c) = health.iter().find(|c| c.health == HealthStatus::Unhealthy) {
                return Err(format!("container {} is unhealthy", short(&c.id)));
            }
            for c in &health {
                let first = *restarts.entry(c.id.clone()).or_insert(c.restart_count);
                if c.restart_count != first {
                    return Err(format!("container {} is restarting", short(&c.id)));
                }
            }
            let checked = health.iter().all(|c| c.health != HealthStatus::None);
            if checked && health.iter().all(|c| c.health == HealthStatus::Healthy) {
                return Ok(());
            }
            let pending = health.iter().any(|c| c.health == HealthStatus::Starting);
            if n == polls {
                return if pending {
                    Err(format!("not healthy after {timeout_secs}s"))
                } else {
                    Ok(())
                };
            }
            last = format!("not healthy after {timeout_secs}s");
        }
        Err(last)
    }
}

fn short(id: &str) -> &str {
    let id = id.strip_prefix("sha256:").unwrap_or(id);
    &id[..id.len().min(12)]
}

/// A unit enum as the protocol spells it (`update`, `discord`).
fn word<T: serde::Serialize + std::fmt::Debug>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|j| j.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{v:?}").to_lowercase())
}

fn kind_word(kind: depths_protocol::OperationKind) -> String {
    word(&kind)
}

fn op_outcome(o: &StackOutcome) -> OpOutcome {
    match o {
        StackOutcome::Updated => OpOutcome::Succeeded,
        StackOutcome::Skipped { reason } if reason == "cancelled" => OpOutcome::Cancelled,
        other => OpOutcome::Failed {
            exit_code: None,
            stderr: format!("{other:?}"),
        },
    }
}

/// The start-up pass (FR-018): every open run is closed. A stack that had started becomes
/// `Interrupted`, audited `update_interrupted` with the run's actor and the images the stack's
/// containers really run; one that had not is `Skipped { reason: "interrupted" }`. Nothing is
/// resumed or rolled back.
pub fn interrupt_open_runs(updates: &Updates, ops: &Ops) -> usize {
    let open: Vec<UpdateRun> = updates.store().observed().open_runs().cloned().collect();
    let snapshot = ops.runtime().containers_by_project().unwrap_or_default();
    let now = updates
        .clock()
        .now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    for mut run in open.iter().cloned() {
        for s in &mut run.stacks {
            if s.outcome.is_some() {
                continue;
            }
            if s.phase == RunPhase::Pending {
                s.outcome = Some(StackOutcome::Skipped {
                    reason: "interrupted".into(),
                });
                continue;
            }
            s.outcome = Some(StackOutcome::Interrupted);
            let (actor, on_behalf_of) = match &run.trigger {
                RunTrigger::Manual { actor } => (actor.clone(), None),
                RunTrigger::Scheduled { on_behalf_of } => {
                    (scheduled_actor(), on_behalf_of.get(&s.stack).cloned())
                }
            };
            let images: Vec<String> = snapshot
                .projects
                .get(&s.stack)
                .map(|p| {
                    p.containers
                        .iter()
                        .map(|c| format!("{}={}", c.service.as_deref().unwrap_or("?"), c.image))
                        .collect()
                })
                .unwrap_or_default();
            let mut e = audit::entry(
                actor,
                AuditAction::UpdateInterrupted,
                IpAddr::from([127, 0, 0, 1]),
                AuditOutcome::Failed {
                    exit_code: None,
                    stderr_tail: format!("interrupted while {:?}", s.phase).to_lowercase(),
                },
            );
            e.stack = Some(s.stack.clone());
            e.operation_id = s.operation_id;
            e.on_behalf_of = on_behalf_of;
            e.diff_summary = Some(format!("running now: {}", images.join(", ")));
            e.update = Some(AuditUpdate {
                run_id: run.run_id,
                services: Vec::new(),
                rejected_digest: None,
                hold: None,
            });
            let _ = ops.audit_log().record(&e);
            s.phase = RunPhase::Done;
        }
        run.finished_at = Some(now.clone());
        let _ = updates
            .store()
            .update_observed(|o| o.upsert_run(run.clone()));
    }
    open.len()
}

#[cfg(test)]
mod tests;
