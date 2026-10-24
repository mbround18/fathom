//! T039, T054: lifecycle operations, preview then confirm.
//!
//! `PreviewOp` checks the permission table, plans the operation ([`depths_core::plan`]) and
//! hands back exactly the argv the confirm will run, bound to the session, the stack's
//! `content_hash` and a five-minute expiry. `Confirm` claims the preview once, re-checks the
//! permission (the role may have changed in between) and the hash, refuses while another
//! operation runs on the stack, and runs the plan's steps unchanged.
//!
//! Every refusal of an action is audited as `denied`; every confirmed operation is audited
//! once it finishes, with its argv and outcome, before `OpFinished` goes out.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use depths_core::StackName;
use depths_core::plan::{self, Operation, Plan, StackContext, Step as PlanStep};
use depths_core::ports::{
    CancelToken, ContainerRuntime, ExecError, OutputChunk, OutputSink, RuntimeError, StackStore,
    StepOutcome, StoreError,
};
use depths_core::role::{self, Access, Role};
use depths_protocol::{
    Actor, AuditAction, AuditOutcome, DenyReason, Event, EventEnvelope, OpOutcome, OperationId,
    Preview, PreviewId, PreviewVoidReason, Reach, StackOp, Step, StepStdin,
};
use tokio::sync::mpsc;

use crate::audit::{self, Audit, OpRecord};
use crate::watch::Watch;

/// How long a preview may be confirmed.
pub const PREVIEW_TTL: Duration = Duration::from_secs(5 * 60);
/// Unconfirmed previews kept at once, server-wide.
const MAX_PREVIEWS: usize = 10_000;

/// Who is asking, as the socket knows them at the moment of the command.
#[derive(Debug, Clone)]
pub struct Who {
    pub session: String,
    /// The users-registry id (`provider:subject`).
    pub user: String,
    /// Re-read from the registry for every command.
    pub access: Access,
    pub actor: Actor,
    pub ip: IpAddr,
}

/// Why nothing ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Denied(DenyReason),
    Invalid(String),
    Busy {
        stack: String,
        operation_id: OperationId,
        actor: Actor,
    },
    Void(PreviewVoidReason),
}

impl From<Refusal> for Event {
    fn from(r: Refusal) -> Self {
        match r {
            Refusal::Denied(reason) => Event::Denied { reason },
            Refusal::Invalid(reason) => Event::Invalid { reason },
            Refusal::Busy {
                stack,
                operation_id,
                actor,
            } => Event::Busy {
                stack,
                operation_id,
                actor,
            },
            Refusal::Void(reason) => Event::PreviewVoid { reason },
        }
    }
}

struct Bound {
    session: String,
    stack: StackName,
    kind: AuditAction,
    op: Operation,
    plan: Plan,
    argv: Vec<Vec<String>>,
    stack_hash: String,
    expires: Instant,
    used: bool,
}

struct RunningOp {
    operation_id: OperationId,
    actor: Actor,
    user: String,
    cancel: CancelToken,
}

pub struct Ops {
    store: Arc<dyn StackStore>,
    runtime: Arc<dyn ContainerRuntime>,
    audit: Arc<Audit>,
    watch: Watch,
    ttl: Duration,
    previews: Mutex<HashMap<PreviewId, Bound>>,
    /// By stack name: one operation per stack at a time.
    running: Arc<Mutex<HashMap<String, RunningOp>>>,
}

/// A confirmed operation, ready to run. Send `OpStarted` first, then [`Job::run`].
pub struct Job {
    pub operation_id: OperationId,
    stack: StackName,
    plan: Plan,
    record: OpRecord,
    cancel: CancelToken,
    store: Arc<dyn StackStore>,
    runtime: Arc<dyn ContainerRuntime>,
    audit: Arc<Audit>,
    watch: Watch,
    running: Arc<Mutex<HashMap<String, RunningOp>>>,
}

fn to_core(op: &StackOp) -> Result<Operation, Refusal> {
    let service = |s: &depths_protocol::ServiceName| {
        depths_core::ServiceName::parse(s.as_str())
            .map_err(|e| Refusal::Invalid(format!("invalid service name: {e}")))
    };
    Ok(match op {
        StackOp::Up => Operation::Up,
        StackOp::Stop => Operation::Stop,
        StackOp::Restart => Operation::Restart,
        StackOp::Down {
            remove_volumes,
            remove_images,
        } => Operation::Down {
            remove_volumes: *remove_volumes,
            remove_images: *remove_images,
        },
        StackOp::Pull => Operation::Pull,
        StackOp::Update => Operation::Update,
        StackOp::Delete { remove_directory } => Operation::Delete {
            remove_directory: *remove_directory,
        },
        StackOp::ServiceUp { service: s } => Operation::ServiceUp {
            service: service(s)?,
        },
        StackOp::ServiceStop { service: s } => Operation::ServiceStop {
            service: service(s)?,
        },
        StackOp::ServiceRestart { service: s } => Operation::ServiceRestart {
            service: service(s)?,
        },
    })
}

fn service_of(op: &Operation) -> Option<&str> {
    match op {
        Operation::ServiceUp { service }
        | Operation::ServiceStop { service }
        | Operation::ServiceRestart { service } => Some(service.as_str()),
        _ => None,
    }
}

/// The steps as the preview shows them. File steps are not commands; they are shown as
/// `depths:<step>` pseudo-commands so the preview still lists everything, in order.
fn wire_steps(plan: &Plan, root: &std::path::Path, stack: &StackName) -> Vec<Step> {
    let dir = root.join(stack.as_str()).display().to_string();
    plan.steps
        .iter()
        .map(|s| match s {
            PlanStep::Run(r) => Step {
                argv: r.argv.clone(),
                cwd: r.cwd.display().to_string(),
                stdin: match r.stdin {
                    plan::StepStdin::Draft => StepStdin::Draft,
                    plan::StepStdin::None => StepStdin::None,
                },
            },
            PlanStep::WriteDraft => Step {
                argv: vec!["depths:write-draft".into(), dir.clone()],
                cwd: root.display().to_string(),
                stdin: StepStdin::Draft,
            },
            PlanStep::RemoveDir => Step {
                argv: vec!["depths:remove-dir".into(), dir.clone()],
                cwd: root.display().to_string(),
                stdin: StepStdin::None,
            },
        })
        .collect()
}

fn denial_reason(access: &Access) -> DenyReason {
    if access.role == Role::Pending {
        DenyReason::Pending
    } else {
        DenyReason::NotPermitted
    }
}

fn store_refusal(stack: &StackName, e: StoreError) -> Refusal {
    match e {
        StoreError::NotFound => Refusal::Invalid(format!("no stack named {}", stack.as_str())),
        e => Refusal::Invalid(e.to_string()),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, Refusal> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| Refusal::Invalid("the server could not complete the request".into()))
}

impl Ops {
    pub fn new(
        store: Arc<dyn StackStore>,
        runtime: Arc<dyn ContainerRuntime>,
        audit: Arc<Audit>,
        watch: Watch,
    ) -> Self {
        Self::with_ttl(store, runtime, audit, watch, PREVIEW_TTL)
    }

    pub fn with_ttl(
        store: Arc<dyn StackStore>,
        runtime: Arc<dyn ContainerRuntime>,
        audit: Arc<Audit>,
        watch: Watch,
        ttl: Duration,
    ) -> Self {
        Self {
            store,
            runtime,
            audit,
            watch,
            ttl,
            previews: Mutex::new(HashMap::new()),
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn deny(&self, who: &Who, stack: &str, why: String) -> Refusal {
        let _ = self
            .audit
            .record(&audit::denied(who.actor.clone(), who.ip, Some(stack), why));
        Refusal::Denied(denial_reason(&who.access))
    }

    fn busy(&self, stack: &str) -> Option<Refusal> {
        self.running
            .lock()
            .unwrap()
            .get(stack)
            .map(|r| Refusal::Busy {
                stack: stack.to_owned(),
                operation_id: r.operation_id,
                actor: r.actor.clone(),
            })
    }

    /// `PreviewOp`.
    pub async fn preview(
        &self,
        who: &Who,
        stack: &depths_protocol::StackName,
        op: &StackOp,
    ) -> Result<Preview, Refusal> {
        let kind = op.kind();
        let name = StackName::parse(stack.as_str()).map_err(|e| {
            let _ = self.audit.record(&audit::denied(
                who.actor.clone(),
                who.ip,
                Some(stack.as_str()),
                "invalid stack name",
            ));
            Refusal::Invalid(format!("invalid stack name: {e}"))
        })?;
        let core_op = to_core(op)?;
        if !role::permits(&who.access, Some(&name), core_op.required_action()) {
            return Err(self.deny(who, name.as_str(), format!("{kind:?} is not permitted")));
        }
        let (store, n) = (self.store.clone(), name.clone());
        let entry = blocking(move || store.entry(&n))
            .await?
            .map_err(|e| store_refusal(&name, e))?;
        if !entry.actionable() {
            let why = crate::inventory::invalid_reason(&entry)
                .or(entry.read_error.clone())
                .unwrap_or_else(|| "the stack cannot be acted on".into());
            return Err(Refusal::Invalid(why));
        }
        if let Some(b) = self.busy(name.as_str()) {
            return Err(b);
        }
        let runtime = self.runtime.clone();
        let snap = blocking(move || runtime.containers_by_project())
            .await?
            .map_err(|e| {
                let why = match e {
                    RuntimeError::Unavailable(r) | RuntimeError::Failed(r) => r,
                    RuntimeError::Unsupported => "the container runtime cannot list".into(),
                };
                Refusal::Invalid(format!("the container runtime is unavailable: {why}"))
            })?;

        let ctx = StackContext {
            stacks_root: self.store.root(),
            stack: &name,
            global_env: self.store.global_env(),
            has_env: entry.has_env,
        };
        let plan = plan::plan(&ctx, &core_op);
        let steps = wire_steps(&plan, self.store.root(), &name);
        let argv = steps.iter().map(|s| s.argv.clone()).collect();
        let only = service_of(&core_op);
        let containers = snap
            .projects
            .get(name.as_str())
            .map(|p| {
                p.containers
                    .iter()
                    .filter(|c| only.is_none_or(|s| c.service.as_deref() == Some(s)))
                    .map(|c| c.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let stack_hash = entry
            .content_hash
            .as_ref()
            .map(|h| h.as_str().to_owned())
            .unwrap_or_default();
        let preview_id = PreviewId(uuid::Uuid::new_v4());
        let expires_at = (chrono::Utc::now()
            + chrono::Duration::from_std(self.ttl).unwrap_or_default())
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let preview = Preview {
            preview_id,
            stack: name.as_str().to_owned(),
            kind,
            steps,
            file_diff: None,
            reaches: Reach {
                containers,
                ..Reach::default()
            },
            destructive: plan.destructive,
            stack_hash: stack_hash.clone(),
            expires_at,
        };

        let mut previews = self.previews.lock().unwrap();
        let now = Instant::now();
        previews.retain(|_, b| b.expires > now && !b.used);
        if previews.len() >= MAX_PREVIEWS {
            return Err(Refusal::Invalid(
                "too many unconfirmed previews; try again in a few minutes".into(),
            ));
        }
        previews.insert(
            preview_id,
            Bound {
                session: who.session.clone(),
                stack: name,
                kind: kind.into(),
                op: core_op,
                plan,
                argv,
                stack_hash,
                expires: now + self.ttl,
                used: false,
            },
        );
        Ok(preview)
    }

    fn unclaim(&self, id: PreviewId) {
        if let Some(b) = self.previews.lock().unwrap().get_mut(&id) {
            b.used = false;
        }
    }

    /// `Confirm`: claims the preview and reserves the stack. Run the returned job after sending
    /// `OpStarted`.
    pub async fn confirm(&self, who: &Who, preview_id: PreviewId) -> Result<Job, Refusal> {
        let (stack, kind, op, plan, argv, stack_hash) = {
            let mut previews = self.previews.lock().unwrap();
            let Some(b) = previews.get_mut(&preview_id) else {
                return Err(Refusal::Void(PreviewVoidReason::Unknown));
            };
            if b.session != who.session {
                return Err(Refusal::Void(PreviewVoidReason::Unknown));
            }
            if b.used {
                return Err(Refusal::Void(PreviewVoidReason::Used));
            }
            if b.expires <= Instant::now() {
                previews.remove(&preview_id);
                return Err(Refusal::Void(PreviewVoidReason::Expired));
            }
            b.used = true;
            (
                b.stack.clone(),
                b.kind,
                b.op.clone(),
                b.plan.clone(),
                b.argv.clone(),
                b.stack_hash.clone(),
            )
        };
        if !role::permits(&who.access, Some(&stack), op.required_action()) {
            return Err(self.deny(
                who,
                stack.as_str(),
                format!("{kind:?} is no longer permitted"),
            ));
        }
        let (store, n) = (self.store.clone(), stack.clone());
        let entry = match blocking(move || store.entry(&n)).await? {
            Ok(e) => e,
            Err(_) => return Err(Refusal::Void(PreviewVoidReason::StackChanged)),
        };
        let now_hash = entry
            .content_hash
            .as_ref()
            .map(|h| h.as_str().to_owned())
            .unwrap_or_default();
        if now_hash != stack_hash {
            return Err(Refusal::Void(PreviewVoidReason::StackChanged));
        }
        let operation_id = OperationId(uuid::Uuid::new_v4());
        let cancel = CancelToken::new();
        {
            let mut running = self.running.lock().unwrap();
            if let Some(r) = running.get(stack.as_str()) {
                let busy = Refusal::Busy {
                    stack: stack.as_str().to_owned(),
                    operation_id: r.operation_id,
                    actor: r.actor.clone(),
                };
                drop(running);
                self.unclaim(preview_id);
                return Err(busy);
            }
            running.insert(
                stack.as_str().to_owned(),
                RunningOp {
                    operation_id,
                    actor: who.actor.clone(),
                    user: who.user.clone(),
                    cancel: cancel.clone(),
                },
            );
        }
        self.previews.lock().unwrap().remove(&preview_id);
        Ok(Job {
            operation_id,
            record: OpRecord {
                actor: who.actor.clone(),
                ip: who.ip,
                action: kind,
                stack: stack.as_str().to_owned(),
                preview_id,
                operation_id,
                argv,
            },
            stack,
            plan,
            cancel,
            store: self.store.clone(),
            runtime: self.runtime.clone(),
            audit: self.audit.clone(),
            watch: self.watch.clone(),
            running: self.running.clone(),
        })
    }

    /// `Cancel`: the person who started it, or an admin.
    pub fn cancel(&self, who: &Who, operation_id: OperationId) -> Result<(), Refusal> {
        let running = self.running.lock().unwrap();
        let Some((stack, r)) = running.iter().find(|(_, r)| r.operation_id == operation_id) else {
            return Err(Refusal::Invalid("no such running operation".into()));
        };
        if r.user != who.user && who.access.role != Role::Admin {
            let stack = stack.clone();
            drop(running);
            return Err(self.deny(who, &stack, "cancel of another person's operation".into()));
        }
        r.cancel.cancel();
        Ok(())
    }

    /// Whether an operation is running on `stack`.
    pub fn is_busy(&self, stack: &str) -> bool {
        self.running.lock().unwrap().contains_key(stack)
    }
}

struct Sink {
    operation_id: OperationId,
    tx: mpsc::Sender<EventEnvelope>,
}

impl OutputSink for Sink {
    fn emit(&self, chunk: OutputChunk) -> bool {
        self.tx
            .blocking_send(EventEnvelope {
                reply_to: None,
                event: Event::OpOutput {
                    operation_id: self.operation_id,
                    chunk: chunk.text,
                },
            })
            .is_ok()
    }
}

impl Job {
    /// Runs the plan to the end, whether or not anyone still listens on `tx`, then audits and
    /// sends `OpFinished`. Blocking: call from `spawn_blocking`.
    pub fn run(self, tx: mpsc::Sender<EventEnvelope>) -> OpOutcome {
        let sink = Sink {
            operation_id: self.operation_id,
            tx: tx.clone(),
        };
        let result = depths_core::ports::execute(
            &self.plan,
            None,
            &self.stack,
            self.store.as_ref(),
            self.runtime.as_ref(),
            &sink,
            &self.cancel,
        );
        let (outcome, audited) = match result {
            Ok(StepOutcome::Succeeded) => (OpOutcome::Succeeded, AuditOutcome::Succeeded),
            Ok(StepOutcome::Failed {
                exit_code,
                stderr_tail,
            }) => (
                OpOutcome::Failed {
                    exit_code,
                    stderr: stderr_tail.clone(),
                },
                AuditOutcome::Failed {
                    exit_code,
                    stderr_tail,
                },
            ),
            Ok(StepOutcome::Cancelled) => (
                OpOutcome::Cancelled,
                AuditOutcome::Failed {
                    exit_code: None,
                    stderr_tail: "cancelled".into(),
                },
            ),
            Err(e) => {
                let why = match e {
                    ExecError::Runtime(RuntimeError::Unavailable(r) | RuntimeError::Failed(r)) => r,
                    e => e.to_string(),
                };
                (
                    OpOutcome::Failed {
                        exit_code: None,
                        stderr: why.clone(),
                    },
                    AuditOutcome::Failed {
                        exit_code: None,
                        stderr_tail: why,
                    },
                )
            }
        };
        let _ = self.audit.record(&self.record.finished(audited));
        self.running.lock().unwrap().remove(self.stack.as_str());
        self.watch.kick();
        let _ = tx.blocking_send(EventEnvelope {
            reply_to: None,
            event: Event::OpFinished {
                operation_id: self.operation_id,
                outcome: outcome.clone(),
            },
        });
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use depths_core::plan::RunStep;
    use depths_core::ports::{OutputStream, RuntimeSnapshot, Validation};
    use depths_protocol::{AuditFilter, Provider};

    #[derive(Default)]
    struct Mock {
        ran: Mutex<Vec<Vec<String>>>,
        fail_with: Mutex<Option<String>>,
        /// Blocks `run_step` until cleared, or until cancelled.
        hold: std::sync::atomic::AtomicBool,
    }

    impl ContainerRuntime for Mock {
        fn containers_by_project(&self) -> Result<RuntimeSnapshot, RuntimeError> {
            Ok(RuntimeSnapshot::default())
        }
        fn run_step(
            &self,
            step: &RunStep,
            _: Option<&[u8]>,
            sink: &dyn OutputSink,
            cancel: &CancelToken,
        ) -> Result<StepOutcome, RuntimeError> {
            self.ran.lock().unwrap().push(step.argv.clone());
            while self.hold.load(std::sync::atomic::Ordering::SeqCst) {
                if cancel.is_cancelled() {
                    return Ok(StepOutcome::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            sink.emit(OutputChunk {
                stream: OutputStream::Stderr,
                text: "Container web-app-1  Started\n".into(),
            });
            Ok(match self.fail_with.lock().unwrap().clone() {
                Some(stderr_tail) => StepOutcome::Failed {
                    exit_code: Some(1),
                    stderr_tail,
                },
                None => StepOutcome::Succeeded,
            })
        }
        fn validate(&self, _: &RunStep, _: &[u8]) -> Result<Validation, RuntimeError> {
            Err(RuntimeError::Unsupported)
        }
        fn follow_events(
            &self,
            _: &(dyn Fn() + Sync),
            _: &CancelToken,
        ) -> Result<(), RuntimeError> {
            Err(RuntimeError::Unsupported)
        }
    }

    struct Fx {
        dir: tempfile::TempDir,
        mock: Arc<Mock>,
        audit: Arc<Audit>,
        ops: Ops,
    }

    fn fx(ttl: Duration) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let stacks = dir.path().join("stacks");
        for s in ["web", "db"] {
            std::fs::create_dir_all(stacks.join(s)).unwrap();
            std::fs::write(
                stacks.join(s).join("compose.yaml"),
                "services:\n  app:\n    image: nginx\n",
            )
            .unwrap();
        }
        let store: Arc<dyn StackStore> =
            Arc::new(depths_stacks_fs::FsStackStore::open(&stacks).unwrap());
        let mock = Arc::new(Mock::default());
        let audit = Arc::new(Audit::open(dir.path().join("audit"), 1 << 20).unwrap());
        let watch = Watch::new(store.clone(), mock.clone());
        let ops = Ops::with_ttl(store, mock.clone(), audit.clone(), watch, ttl);
        Fx {
            dir,
            mock,
            audit,
            ops,
        }
    }

    fn who(session: &str, sub: &str, role: Role, stacks: &[&str]) -> Who {
        Who {
            session: session.into(),
            user: format!("discord:{sub}"),
            access: Access {
                role,
                stacks: stacks
                    .iter()
                    .map(|s| StackName::parse(s).unwrap())
                    .collect::<BTreeSet<_>>(),
            },
            actor: Actor {
                provider: Provider::Discord,
                subject: sub.into(),
                username: format!("u{sub}"),
            },
            ip: "203.0.113.9".parse().unwrap(),
        }
    }

    fn name(s: &str) -> depths_protocol::StackName {
        depths_protocol::StackName::new(s).unwrap()
    }

    fn entries(a: &Audit) -> Vec<depths_protocol::AuditEntry> {
        a.page(&AuditFilter::default(), None).unwrap().0
    }

    async fn run(job: Job) -> (OpOutcome, Vec<Event>) {
        let (tx, mut rx) = mpsc::channel(64);
        let out = tokio::task::spawn_blocking(move || job.run(tx))
            .await
            .unwrap();
        let mut evs = Vec::new();
        while let Ok(e) = rx.try_recv() {
            evs.push(e.event);
        }
        (out, evs)
    }

    #[tokio::test]
    async fn a_user_cannot_preview_an_unassigned_stack_and_it_is_audited() {
        let f = fx(PREVIEW_TTL);
        let u = who("s1", "7", Role::User, &["web"]);
        let r = f.ops.preview(&u, &name("db"), &StackOp::Restart).await;
        assert_eq!(r.unwrap_err(), Refusal::Denied(DenyReason::NotPermitted));
        let e = entries(&f.audit);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].action, AuditAction::Denied);
        assert_eq!(e[0].stack.as_deref(), Some("db"));
        assert!(f.mock.ran.lock().unwrap().is_empty());

        // Destructive down is admin-only even on an assigned stack.
        let down = StackOp::Down {
            remove_volumes: true,
            remove_images: false,
        };
        let r = f.ops.preview(&u, &name("web"), &down).await;
        assert_eq!(r.unwrap_err(), Refusal::Denied(DenyReason::NotPermitted));
    }

    #[tokio::test]
    async fn the_confirm_runs_exactly_the_previewed_argv_and_audits_it() {
        let f = fx(PREVIEW_TTL);
        let u = who("s1", "7", Role::User, &["web"]);
        let p = f
            .ops
            .preview(&u, &name("web"), &StackOp::Restart)
            .await
            .unwrap();
        assert_eq!(p.steps.len(), 1);
        assert_eq!(p.steps[0].argv[..2], ["docker", "compose"]);
        assert!(!p.destructive);
        assert_eq!(p.stack_hash.len(), 64);

        let job = f.ops.confirm(&u, p.preview_id).await.unwrap();
        assert!(f.ops.is_busy("web"));
        let id = job.operation_id;
        let (out, evs) = run(job).await;
        assert_eq!(out, OpOutcome::Succeeded);
        assert_eq!(*f.mock.ran.lock().unwrap(), vec![p.steps[0].argv.clone()]);
        assert!(matches!(&evs[0], Event::OpOutput { chunk, .. } if chunk.contains("Started")));
        assert!(matches!(
            evs.last(),
            Some(Event::OpFinished { operation_id, outcome: OpOutcome::Succeeded }) if *operation_id == id
        ));
        assert!(!f.ops.is_busy("web"));

        let e = entries(&f.audit);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].action, AuditAction::Restart);
        assert_eq!(e[0].operation_id, Some(id));
        assert_eq!(e[0].preview_id, Some(p.preview_id));
        assert_eq!(e[0].argv, vec![p.steps[0].argv.clone()]);

        // Used once.
        let again = f.ops.confirm(&u, p.preview_id).await;
        assert!(matches!(
            again,
            Err(Refusal::Void(
                PreviewVoidReason::Unknown | PreviewVoidReason::Used
            ))
        ));
    }

    #[tokio::test]
    async fn compose_stderr_comes_back_verbatim() {
        let f = fx(PREVIEW_TTL);
        let stderr = "Error response from daemon: driver failed programming external connectivity on endpoint web-app-1: Bind for 0.0.0.0:80 failed: port is already allocated\n";
        *f.mock.fail_with.lock().unwrap() = Some(stderr.into());
        let a = who("s1", "1", Role::Admin, &[]);
        let p = f.ops.preview(&a, &name("web"), &StackOp::Up).await.unwrap();
        let (out, _) = run(f.ops.confirm(&a, p.preview_id).await.unwrap()).await;
        assert_eq!(
            out,
            OpOutcome::Failed {
                exit_code: Some(1),
                stderr: stderr.into()
            }
        );
        let e = entries(&f.audit);
        assert_eq!(
            e[0].outcome,
            AuditOutcome::Failed {
                exit_code: Some(1),
                stderr_tail: stderr.into()
            }
        );
    }

    #[tokio::test]
    async fn previews_are_bound_to_session_hash_and_time() {
        let f = fx(PREVIEW_TTL);
        let a = who("s1", "1", Role::Admin, &[]);
        let other = who("s2", "1", Role::Admin, &[]);
        let p = f
            .ops
            .preview(&a, &name("web"), &StackOp::Pull)
            .await
            .unwrap();
        assert_eq!(
            f.ops.confirm(&other, p.preview_id).await.err(),
            Some(Refusal::Void(PreviewVoidReason::Unknown)),
            "another session's id is unknown"
        );
        std::fs::write(
            f.dir.path().join("stacks/web/compose.yaml"),
            "services:\n  app:\n    image: caddy\n",
        )
        .unwrap();
        assert_eq!(
            f.ops.confirm(&a, p.preview_id).await.err(),
            Some(Refusal::Void(PreviewVoidReason::StackChanged))
        );

        let f = fx(Duration::ZERO);
        let p = f
            .ops
            .preview(&a, &name("web"), &StackOp::Pull)
            .await
            .unwrap();
        assert_eq!(
            f.ops.confirm(&a, p.preview_id).await.err(),
            Some(Refusal::Void(PreviewVoidReason::Expired))
        );
        assert!(f.mock.ran.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_revoked_role_cannot_confirm() {
        let f = fx(PREVIEW_TTL);
        let u = who("s1", "7", Role::User, &["web"]);
        let p = f
            .ops
            .preview(&u, &name("web"), &StackOp::Stop)
            .await
            .unwrap();
        let revoked = who("s1", "7", Role::User, &[]);
        assert_eq!(
            f.ops.confirm(&revoked, p.preview_id).await.err(),
            Some(Refusal::Denied(DenyReason::NotPermitted))
        );
        assert_eq!(entries(&f.audit)[0].action, AuditAction::Denied);
        assert!(f.mock.ran.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_operation_per_stack_and_cancel_rules() {
        let f = fx(PREVIEW_TTL);
        f.mock.hold.store(true, std::sync::atomic::Ordering::SeqCst);
        let u = who("s1", "7", Role::User, &["web"]);
        let p = f.ops.preview(&u, &name("web"), &StackOp::Up).await.unwrap();
        let job = f.ops.confirm(&u, p.preview_id).await.unwrap();
        let id = job.operation_id;
        let (tx, _rx) = mpsc::channel(64);
        let h = tokio::task::spawn_blocking(move || job.run(tx));

        let b = who("s2", "8", Role::User, &["web"]);
        match f.ops.preview(&b, &name("web"), &StackOp::Stop).await {
            Err(Refusal::Busy {
                operation_id,
                actor,
                ..
            }) => {
                assert_eq!(operation_id, id);
                assert_eq!(actor.subject, "7");
            }
            r => panic!("{r:?}"),
        }
        assert_eq!(
            f.ops.cancel(&b, id),
            Err(Refusal::Denied(DenyReason::NotPermitted)),
            "not someone else's"
        );
        f.ops.cancel(&u, id).unwrap();
        assert_eq!(h.await.unwrap(), OpOutcome::Cancelled);
        assert!(!f.ops.is_busy("web"));
    }

    #[tokio::test]
    async fn a_missing_stack_is_invalid() {
        let f = fx(PREVIEW_TTL);
        let a = who("s1", "1", Role::Admin, &[]);
        let r = f.ops.preview(&a, &name("missing"), &StackOp::Up).await;
        assert!(matches!(r, Err(Refusal::Invalid(why)) if why.contains("missing")));
    }
}
