//! T039, T054: lifecycle operations, preview then confirm.
//!
//! `PreviewOp` checks the permission table, plans the operation ([`depths_core::plan`]) and
//! hands back exactly the argv the confirm will run, bound to the session, the stack's
//! `content_hash` and a five-minute expiry. `Confirm` claims the preview once, re-checks the
//! permission (the role may have changed in between) and the hash, refuses while another
//! operation runs on the stack, and runs the plan's steps unchanged.
//!
//! `PreviewSave` (T060) does the same for an edit: it checks the draft against the hash the
//! editor loaded (`Conflict`), validates it with Compose (`ValidationFailed`), and previews the
//! write with a diff. The confirm writes exactly the previewed draft.
//!
//! 051 FR-015 to FR-019: the previews of what brings containers up from a compose file (`Up`,
//! `Update`, `ServiceUp`, `Save`, `Create`) carry that file's lint. Under
//! `DEPTHS_COMPOSE_POLICY=enforce` an escape-level finding refuses a user's preview
//! (`compose_blocked`) and makes an admin's `ack_required`: its confirm runs only with
//! `acknowledge_findings`, and the audit entry records the findings and the acknowledgement.
//! Root-level findings never block, and nothing here rewrites a file.
//!
//! Every refusal of an action is audited as `denied`; every confirmed operation is audited
//! once it finishes, with its argv and outcome, before `OpFinished` goes out.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use depths_core::compose_lint::ComposePolicy;
use depths_core::plan::{self, Operation, Plan, StackContext, Step as PlanStep};
use depths_core::ports::{
    CancelToken, ContainerRuntime, ExecError, OutputChunk, OutputSink, RuntimeError, StackDraft,
    StackStore, StepOutcome, StoreError, Validation,
};
use depths_core::role::{self, Access, Action, Role};
use depths_core::{ContentHash, StackName};
use depths_protocol::{
    Actor, AuditOutcome, AuditSecurity, ComposeWarning, DenyReason, DraftFile, EnvText, Event,
    EventEnvelope, OpOutcome, OperationId, OperationKind, Preview, PreviewId, PreviewVoidReason,
    Reach, SecurityLevel, SecuritySeverity, StackOp, Step, StepStdin, ValidationError,
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
        kind: OperationKind,
        actor: Actor,
    },
    Void(PreviewVoidReason),
    /// 051 FR-018: escape-level compose findings under `enforce`; the detail names them.
    ComposeBlocked(String),
    ValidationFailed(Vec<ValidationError>),
    /// The stack changed since the editor loaded it.
    Conflict {
        current_text: String,
        current_env: Option<EnvText>,
        current_hash: String,
    },
}

impl From<Refusal> for Event {
    fn from(r: Refusal) -> Self {
        match r {
            Refusal::Denied(reason) => Event::Denied {
                reason,
                detail: None,
            },
            Refusal::Invalid(reason) => Event::Invalid { reason },
            Refusal::Busy {
                stack,
                operation_id,
                kind,
                actor,
            } => Event::Busy {
                stack,
                operation_id,
                kind,
                actor,
            },
            Refusal::Void(reason) => Event::PreviewVoid { reason },
            Refusal::ComposeBlocked(detail) => Event::Denied {
                reason: DenyReason::ComposeBlocked,
                detail: Some(detail),
            },
            Refusal::ValidationFailed(errors) => Event::ValidationFailed { errors },
            Refusal::Conflict {
                current_text,
                current_env,
                current_hash,
            } => Event::Conflict {
                current_text,
                current_env,
                current_hash,
            },
        }
    }
}

struct Bound {
    session: String,
    stack: StackName,
    kind: OperationKind,
    op: Operation,
    plan: Plan,
    /// What a save or create writes, exactly as previewed.
    draft: Option<StackDraft>,
    argv: Vec<Vec<String>>,
    /// Saves and creates: what the audit entry records of the change.
    diff_summary: Option<String>,
    /// Empty for a create: the stack must still not exist at the confirm.
    stack_hash: String,
    expires: Instant,
    used: bool,
    /// 051: what the audit entry records of the compose findings, and whether only an
    /// admin's acknowledgement may confirm.
    security: Option<AuditSecurity>,
    ack_required: bool,
}

/// 051 FR-015 to FR-018: a preview's compose lint and what it means for the confirm.
#[derive(Default)]
struct Findings {
    warnings: Vec<ComposeWarning>,
    /// Anything above informational: what the audit entry records.
    security: Option<AuditSecurity>,
    ack_required: bool,
}

/// The operations that bring containers up from the compose file, so its lint applies.
fn linted(kind: OperationKind) -> bool {
    matches!(
        kind,
        OperationKind::Up
            | OperationKind::Update
            | OperationKind::ServiceUp
            | OperationKind::Save
            | OperationKind::Create
    )
}

fn blocked_detail(warnings: &[ComposeWarning]) -> String {
    let named: Vec<String> = warnings
        .iter()
        .filter(|w| w.severity == SecuritySeverity::Escape)
        .map(|w| format!("{}: {}", w.service, w.message))
        .collect();
    format!(
        "the compose file can break containment ({}); DEPTHS_COMPOSE_POLICY=enforce lets only \
         an admin deploy it, acknowledging the findings",
        named.join("; ")
    )
}

struct RunningOp {
    operation_id: OperationId,
    kind: OperationKind,
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
    /// `DEPTHS_COMPOSE_POLICY=enforce` (051 FR-017).
    enforce: AtomicBool,
}

/// A confirmed operation, ready to run. Send `OpStarted` first, then [`Job::run`].
pub struct Job {
    pub operation_id: OperationId,
    stack: StackName,
    plan: Plan,
    draft: Option<StackDraft>,
    record: OpRecord,
    cancel: CancelToken,
    store: Arc<dyn StackStore>,
    runtime: Arc<dyn ContainerRuntime>,
    audit: Arc<Audit>,
    watch: Watch,
    running: Arc<Mutex<HashMap<String, RunningOp>>>,
}

/// The permission row `op` needs, for callers that check the role before another check (the
/// network gate, spec 052). `None` for an op that does not parse; `preview` refuses it.
pub fn required_action(op: &StackOp) -> Option<depths_core::role::Action> {
    to_core(op).ok().map(|o| o.required_action())
}

/// What the network gate (spec 052) calls an operation kind.
pub fn command_kind(kind: OperationKind) -> depths_core::role::CommandKind {
    use depths_core::role::CommandKind as K;
    match kind {
        OperationKind::Up | OperationKind::ServiceUp => K::Start,
        OperationKind::Stop | OperationKind::ServiceStop => K::Stop,
        OperationKind::Restart | OperationKind::ServiceRestart => K::Restart,
        OperationKind::Down => K::Down,
        OperationKind::Pull => K::Pull,
        OperationKind::Update => K::Update,
        OperationKind::Delete => K::Delete,
        OperationKind::Save | OperationKind::Create => K::Edit,
    }
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

/// The compose file's name in the stack, for the diff header.
fn files_name(entry: &depths_core::ports::StackEntry) -> &'static str {
    entry
        .compose_file
        .map(|f| f.file_name())
        .unwrap_or("compose.yaml")
}

/// Checks a draft `.env` for what Compose would misread: a line that is not `KEY=value`, `KEY`,
/// a comment or blank. A secret still showing the mask is not an error: the draft has been
/// through [`depths_core::secrets::unmask_env`], which put the real value back. The inner lines
/// of a quoted multi-line value (a PEM key) are part of that value, not lines of their own.
fn lint_env(draft: &str) -> Vec<ValidationError> {
    use depths_core::secrets::closes;
    let key_of = |line: &str| -> Option<(String, String)> {
        let t = line.trim_start();
        if t.is_empty() || t.starts_with('#') {
            return None;
        }
        let (k, v) = t.split_once('=').unwrap_or((t, ""));
        Some((
            k.trim().trim_start_matches("export ").trim().to_owned(),
            v.trim_start().to_owned(),
        ))
    };
    let mut errors = Vec::new();
    let mut inside: Option<char> = None;
    for (i, line) in draft.lines().enumerate() {
        if let Some(q) = inside {
            if closes(line, q) {
                inside = None;
            }
            continue;
        }
        let Some((key, value)) = key_of(line) else {
            continue;
        };
        if let Some(q @ ('"' | '\'')) = value.chars().next()
            && !closes(&value[1..], q)
        {
            inside = Some(q);
        }
        let at = Some(u32::try_from(i + 1).unwrap_or(u32::MAX));
        let valid_key = key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
        if !valid_key {
            errors.push(ValidationError {
                file: DraftFile::Env,
                line: at,
                message: "expected KEY=value".into(),
            });
        }
    }
    errors
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
            enforce: AtomicBool::new(false),
        }
    }

    /// `DEPTHS_COMPOSE_POLICY` (051 FR-017); `warn` until set.
    pub fn set_compose_policy(&self, p: ComposePolicy) {
        self.enforce
            .store(p == ComposePolicy::Enforce, Ordering::SeqCst);
    }

    /// Lints `compose` for a preview of `kind` (051 FR-015 to FR-018). Refuses a non-admin
    /// under `enforce` when there is an escape-level finding, audited with the findings.
    fn lint(
        &self,
        who: &Who,
        stack: &str,
        kind: OperationKind,
        compose: Option<&str>,
    ) -> Result<Findings, Refusal> {
        let Some(text) = compose.filter(|_| linted(kind)) else {
            return Ok(Findings::default());
        };
        let warnings = crate::security::warnings(text);
        let serious: Vec<&ComposeWarning> = warnings
            .iter()
            .filter(|w| w.severity != SecuritySeverity::Info)
            .collect();
        let escape = serious
            .iter()
            .any(|w| w.severity == SecuritySeverity::Escape);
        let security = (!serious.is_empty()).then(|| AuditSecurity {
            level: if escape {
                SecurityLevel::Escape
            } else {
                SecurityLevel::Root
            },
            reasons: serious
                .iter()
                .map(|w| format!("{}:{}", w.service, w.code))
                .collect(),
            acknowledged: false,
        });
        let enforced = escape && self.enforce.load(Ordering::SeqCst);
        if enforced && who.access.role != Role::Admin {
            let detail = blocked_detail(&warnings);
            let mut e = audit::denied(
                who.actor.clone(),
                who.ip,
                Some(stack),
                format!("{kind:?} refused: {detail}"),
            );
            e.security = security;
            let _ = self.audit.record(&e);
            return Err(Refusal::ComposeBlocked(detail));
        }
        Ok(Findings {
            warnings,
            security,
            ack_required: enforced,
        })
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
                kind: r.kind,
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
        let name = self.parse_name(who, stack.as_str())?;
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
        let compose = if linted(kind) {
            let (store, n) = (self.store.clone(), name.clone());
            let files = blocking(move || store.read(&n))
                .await?
                .map_err(|e| store_refusal(&name, e))?;
            Some(files.compose)
        } else {
            None
        };
        let findings = self.lint(who, name.as_str(), kind, compose.as_deref())?;
        let preview_id = PreviewId(uuid::Uuid::new_v4());
        let expires_at = self.expires_at();
        let preview = Preview {
            preview_id,
            stack: name.as_str().to_owned(),
            kind,
            steps,
            step_up: None,
            file_diff: self.removal_diff(&name, &core_op).await,
            reaches: Reach {
                containers,
                ..Reach::default()
            },
            destructive: plan.destructive,
            stack_hash: stack_hash.clone(),
            expires_at,
            warnings: findings.warnings,
            ack_required: findings.ack_required,
        };

        self.bind(
            preview_id,
            Bound {
                session: who.session.clone(),
                stack: name,
                kind,
                op: core_op,
                plan,
                draft: None,
                argv,
                diff_summary: None,
                stack_hash,
                expires: Instant::now() + self.ttl,
                used: false,
                security: findings.security,
                ack_required: findings.ack_required,
            },
        )?;
        Ok(preview)
    }

    /// Keeps a preview for its confirm, dropping the spent and expired ones.
    fn bind(&self, id: PreviewId, bound: Bound) -> Result<(), Refusal> {
        let mut previews = self.previews.lock().unwrap();
        let now = Instant::now();
        previews.retain(|_, b| b.expires > now && !b.used);
        if previews.len() >= MAX_PREVIEWS {
            return Err(Refusal::Invalid(
                "too many unconfirmed previews; try again in a few minutes".into(),
            ));
        }
        previews.insert(id, bound);
        Ok(())
    }

    fn expires_at(&self) -> String {
        (chrono::Utc::now() + chrono::Duration::from_std(self.ttl).unwrap_or_default())
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    /// A stack name from the wire; an invalid one is audited and refused.
    fn parse_name(&self, who: &Who, stack: &str) -> Result<StackName, Refusal> {
        StackName::parse(stack).map_err(|e| {
            let _ = self.audit.record(&audit::denied(
                who.actor.clone(),
                who.ip,
                Some(stack),
                "invalid stack name",
            ));
            Refusal::Invalid(format!("invalid stack name: {e}"))
        })
    }

    /// The removal diff a `Delete{remove_directory}` preview shows. Best effort: a stack whose
    /// files cannot be read still previews, without a diff.
    async fn removal_diff(&self, name: &StackName, op: &Operation) -> Option<String> {
        if !matches!(
            op,
            Operation::Delete {
                remove_directory: true
            }
        ) {
            return None;
        }
        let (store, n) = (self.store.clone(), name.clone());
        let files = blocking(move || store.read(&n)).await.ok()?.ok()?;
        let mut diff =
            crate::diff::unified(files.compose_file.file_name(), Some(&files.compose), None);
        if let Some(env) = files.env.as_deref() {
            let masked = depths_core::secrets::mask_env(env).unwrap_or_else(|| env.to_owned());
            diff.push_str(&crate::diff::unified(".env", Some(&masked), None));
        }
        Some(diff)
    }

    /// `PreviewSave`: a save when the draft carries `base_hash` (the stack's editors), a create
    /// when it does not (admins). Refuses with `Conflict` when the stack changed since the
    /// editor loaded it and with `ValidationFailed` when the draft does not validate; otherwise
    /// previews the write, then `up` when `deploy`.
    pub async fn preview_save(
        &self,
        who: &Who,
        draft: &depths_protocol::StackDraft,
        deploy: bool,
    ) -> Result<Preview, Refusal> {
        let name = self.parse_name(who, draft.stack.as_str())?;
        let base_hash = match &draft.base_hash {
            Some(h) => Some(
                ContentHash::parse(h.as_str())
                    .ok_or_else(|| Refusal::Invalid("invalid base hash".into()))?,
            ),
            None => None,
        };
        let (kind, action) = match base_hash {
            Some(_) => (OperationKind::Save, Action::Edit),
            None => (OperationKind::Create, Action::CreateStack),
        };
        if !role::permits(&who.access, Some(&name), action) {
            return Err(self.deny(who, name.as_str(), format!("{kind:?} is not permitted")));
        }

        // What is on disk now: the base of the diff, and whether `.env` exists.
        let (store, n) = (self.store.clone(), name.clone());
        let found = blocking(move || store.entry(&n)).await?;
        let (current, compose_file, has_env) = match (&base_hash, found) {
            (None, Err(StoreError::NotFound)) => (None, "compose.yaml", false),
            (None, Ok(_) | Err(StoreError::NotAStack)) => {
                return Err(Refusal::Invalid(format!(
                    "the name {} is taken: the stacks directory already has it",
                    name.as_str()
                )));
            }
            (_, Err(e)) => return Err(store_refusal(&name, e)),
            (Some(base), Ok(entry)) => {
                if !entry.editable() {
                    let why = entry
                        .read_only_reason
                        .clone()
                        .or(crate::inventory::invalid_reason(&entry))
                        .or(entry.read_error.clone())
                        .unwrap_or_else(|| "the stack cannot be edited".into());
                    return Err(Refusal::Invalid(why));
                }
                let (store, n) = (self.store.clone(), name.clone());
                let files = blocking(move || store.read(&n))
                    .await?
                    .map_err(|e| store_refusal(&name, e))?;
                if &files.content_hash != base {
                    return Err(Refusal::Conflict {
                        current_env: files.env.map(|t| match depths_core::secrets::mask_env(&t) {
                            Some(m) => EnvText::Masked(m),
                            None => EnvText::Plain(t),
                        }),
                        current_text: files.compose,
                        current_hash: files.content_hash.as_str().to_owned(),
                    });
                }
                (Some(files), files_name(&entry), entry.has_env)
            }
        };
        if let Some(b) = self.busy(name.as_str()) {
            return Err(b);
        }

        // `.env` came out masked; every line that still shows the mask for a secret the file
        // has gets its real value back, so saving without a reveal (users cannot reveal) keeps
        // the secrets instead of writing asterisks over them.
        let current_env = current.as_ref().and_then(|f| f.env.as_deref());
        let env = draft
            .env
            .as_deref()
            .map(|e| depths_core::secrets::unmask_env(e, current_env));
        let core_draft = StackDraft {
            stack: name.clone(),
            base_hash: base_hash.clone(),
            compose: draft.compose.clone(),
            env: env.clone(),
        };
        let env_after_write = core_draft.env_exists_after(has_env);
        let core_op = match base_hash {
            Some(_) => Operation::Save {
                deploy,
                env_after_write,
            },
            None => Operation::Create {
                deploy,
                env_after_write,
            },
        };
        let ctx = StackContext {
            stacks_root: self.store.root(),
            stack: &name,
            global_env: self.store.global_env(),
            has_env,
        };
        let plan = plan::plan(&ctx, &core_op);

        // Validate: `.env` by the lint here, the compose file by Compose itself.
        let mut errors = env.as_deref().map(lint_env).unwrap_or_default();
        let Some(step) = plan
            .commands()
            .find(|s| s.stdin == plan::StepStdin::Draft)
            .cloned()
        else {
            return Err(Refusal::Invalid("the save has no validate step".into()));
        };
        let (runtime, text) = (self.runtime.clone(), draft.compose.clone());
        match blocking(move || runtime.validate(&step, text.as_bytes())).await? {
            Ok(Validation::Valid) => {}
            Ok(Validation::Invalid(issues)) => {
                errors.extend(issues.into_iter().map(|i| ValidationError {
                    file: DraftFile::Compose,
                    line: i.line,
                    message: i.message,
                }));
            }
            Err(e) => {
                let why = match e {
                    RuntimeError::Unavailable(r) | RuntimeError::Failed(r) => r,
                    RuntimeError::Unsupported => "it cannot validate".into(),
                };
                return Err(Refusal::Invalid(format!(
                    "the draft could not be validated: the container runtime is unavailable: {why}"
                )));
            }
        }
        if !errors.is_empty() {
            return Err(Refusal::ValidationFailed(errors));
        }

        // The diff: the compose file, then `.env` masked on both sides.
        let mut file_diff = crate::diff::unified(
            compose_file,
            current.as_ref().map(|f| f.compose.as_str()),
            Some(&draft.compose),
        );
        if let Some(new_env) = env.as_deref() {
            let mask = |t: &str| depths_core::secrets::mask_env(t).unwrap_or_else(|| t.to_owned());
            let old = current_env.map(mask);
            let new = (!new_env.is_empty() || old.is_some()).then(|| mask(new_env));
            file_diff.push_str(&crate::diff::unified(
                ".env",
                old.as_deref(),
                new.as_deref(),
            ));
        }

        let diff_summary = crate::diff::summary(
            compose_file,
            current.as_ref().map(|f| f.compose.as_str()),
            &draft.compose,
            current_env,
            env.as_deref(),
            base_hash.as_ref().map(ContentHash::as_str),
        );

        let containers = if deploy {
            let runtime = self.runtime.clone();
            let project = name.as_str().to_owned();
            blocking(move || runtime.containers_by_project())
                .await?
                .ok()
                .and_then(|mut snap| snap.projects.remove(&project))
                .map(|p| p.containers.into_iter().map(|c| c.name).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let steps = wire_steps(&plan, self.store.root(), &name);
        let argv = steps.iter().map(|s| s.argv.clone()).collect();
        let stack_hash = base_hash
            .as_ref()
            .map(|h| h.as_str().to_owned())
            .unwrap_or_default();
        let findings = self.lint(who, name.as_str(), kind, Some(&draft.compose))?;
        let preview_id = PreviewId(uuid::Uuid::new_v4());
        let preview = Preview {
            preview_id,
            stack: name.as_str().to_owned(),
            kind,
            steps,
            step_up: None,
            file_diff: Some(file_diff),
            reaches: Reach {
                containers,
                ..Reach::default()
            },
            destructive: plan.destructive,
            stack_hash: stack_hash.clone(),
            expires_at: self.expires_at(),
            warnings: findings.warnings,
            ack_required: findings.ack_required,
        };
        self.bind(
            preview_id,
            Bound {
                session: who.session.clone(),
                stack: name,
                kind,
                op: core_op,
                plan,
                draft: Some(core_draft),
                argv,
                diff_summary: Some(diff_summary),
                stack_hash,
                expires: Instant::now() + self.ttl,
                used: false,
                security: findings.security,
                ack_required: findings.ack_required,
            },
        )?;
        Ok(preview)
    }

    /// The kind of `session`'s unconfirmed preview `id`, for the network gate before `Confirm`.
    pub fn preview_kind(&self, session: &str, id: PreviewId) -> Option<OperationKind> {
        self.previews
            .lock()
            .unwrap()
            .get(&id)
            .filter(|b| b.session == session)
            .map(|b| b.kind)
    }

    fn unclaim(&self, id: PreviewId) {
        if let Some(b) = self.previews.lock().unwrap().get_mut(&id) {
            b.used = false;
        }
    }

    /// `Confirm`: claims the preview and reserves the stack. Run the returned job after sending
    /// `OpStarted`. A preview with `ack_required` confirms only for an admin who gives
    /// `acknowledge_findings` (051 FR-018); otherwise it is refused, audited, and stays
    /// unclaimed so it can be confirmed again.
    pub async fn confirm(
        &self,
        who: &Who,
        preview_id: PreviewId,
        acknowledge_findings: bool,
    ) -> Result<Job, Refusal> {
        let (stack, kind, op, plan, draft, argv, diff_summary, stack_hash, security) = {
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
            if b.ack_required && !(acknowledge_findings && who.access.role == Role::Admin) {
                let (stack, kind) = (b.stack.as_str().to_owned(), b.kind);
                let mut e = audit::denied(
                    who.actor.clone(),
                    who.ip,
                    Some(&stack),
                    format!("{kind:?} confirm refused: escape-level findings not acknowledged"),
                );
                e.security.clone_from(&b.security);
                drop(previews);
                let _ = self.audit.record(&e);
                let why = if who.access.role == Role::Admin {
                    "this preview has escape-level compose findings; deploy anyway only by \
                     acknowledging them"
                } else {
                    "this preview has escape-level compose findings that only an admin may \
                     acknowledge"
                };
                return Err(Refusal::ComposeBlocked(why.into()));
            }
            b.used = true;
            let mut security = b.security.clone();
            if let Some(s) = security.as_mut() {
                s.acknowledged = b.ack_required;
            }
            (
                b.stack.clone(),
                b.kind,
                b.op.clone(),
                b.plan.clone(),
                b.draft.clone(),
                b.argv.clone(),
                b.diff_summary.clone(),
                b.stack_hash.clone(),
                security,
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
        let unchanged = match (&op, blocking(move || store.entry(&n)).await?) {
            // A create's name must still be free.
            (Operation::Create { .. }, Err(StoreError::NotFound)) => true,
            (Operation::Create { .. }, _) | (_, Err(_)) => false,
            (_, Ok(entry)) => {
                entry
                    .content_hash
                    .as_ref()
                    .map(|h| h.as_str())
                    .unwrap_or_default()
                    == stack_hash
            }
        };
        if !unchanged {
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
                    kind: r.kind,
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
                    kind,
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
                action: kind.into(),
                stack: stack.as_str().to_owned(),
                preview_id,
                operation_id,
                argv,
                diff_summary,
                security,
            },
            stack,
            plan,
            draft,
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
            self.draft.as_ref(),
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
                guard: None,
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
    use depths_protocol::{AuditAction, AuditFilter, Provider};

    #[derive(Default)]
    struct Mock {
        ran: Mutex<Vec<Vec<String>>>,
        fail_with: Mutex<Option<String>>,
        /// Blocks `run_step` until cleared, or until cancelled.
        hold: std::sync::atomic::AtomicBool,
        /// What `validate` answers; `None` is valid.
        validation: Mutex<Option<Validation>>,
        /// Every validate step and the draft it was given.
        validated: Mutex<Vec<(RunStep, String)>>,
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
        fn validate(&self, step: &RunStep, draft: &[u8]) -> Result<Validation, RuntimeError> {
            self.validated
                .lock()
                .unwrap()
                .push((step.clone(), String::from_utf8(draft.to_vec()).unwrap()));
            Ok(self
                .validation
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(Validation::Valid))
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

        let job = f.ops.confirm(&u, p.preview_id, false).await.unwrap();
        assert!(f.ops.is_busy("web"));
        let id = job.operation_id;
        let (out, evs) = run(job).await;
        assert_eq!(out, OpOutcome::Succeeded);
        assert_eq!(*f.mock.ran.lock().unwrap(), vec![p.steps[0].argv.clone()]);
        assert!(matches!(&evs[0], Event::OpOutput { chunk, .. } if chunk.contains("Started")));
        assert!(matches!(
            evs.last(),
            Some(Event::OpFinished { operation_id, outcome: OpOutcome::Succeeded, .. }) if *operation_id == id
        ));
        assert!(!f.ops.is_busy("web"));

        let e = entries(&f.audit);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].action, AuditAction::Restart);
        assert_eq!(e[0].operation_id, Some(id));
        assert_eq!(e[0].preview_id, Some(p.preview_id));
        assert_eq!(e[0].argv, vec![p.steps[0].argv.clone()]);

        // Used once.
        let again = f.ops.confirm(&u, p.preview_id, false).await;
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
        let (out, _) = run(f.ops.confirm(&a, p.preview_id, false).await.unwrap()).await;
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
            f.ops.confirm(&other, p.preview_id, false).await.err(),
            Some(Refusal::Void(PreviewVoidReason::Unknown)),
            "another session's id is unknown"
        );
        std::fs::write(
            f.dir.path().join("stacks/web/compose.yaml"),
            "services:\n  app:\n    image: caddy\n",
        )
        .unwrap();
        assert_eq!(
            f.ops.confirm(&a, p.preview_id, false).await.err(),
            Some(Refusal::Void(PreviewVoidReason::StackChanged))
        );

        let f = fx(Duration::ZERO);
        let p = f
            .ops
            .preview(&a, &name("web"), &StackOp::Pull)
            .await
            .unwrap();
        assert_eq!(
            f.ops.confirm(&a, p.preview_id, false).await.err(),
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
            f.ops.confirm(&revoked, p.preview_id, false).await.err(),
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
        let job = f.ops.confirm(&u, p.preview_id, false).await.unwrap();
        let id = job.operation_id;
        let (tx, _rx) = mpsc::channel(64);
        let h = tokio::task::spawn_blocking(move || job.run(tx));

        let b = who("s2", "8", Role::User, &["web"]);
        match f.ops.preview(&b, &name("web"), &StackOp::Stop).await {
            Err(Refusal::Busy {
                operation_id,
                kind,
                actor,
                ..
            }) => {
                assert_eq!(operation_id, id);
                assert_eq!(kind, OperationKind::Up);
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

    // -- T060: save, create, delete --------------------------------------------------------

    const CADDY: &str = "services:\n  app:\n    image: caddy\n";

    fn hash_of(f: &Fx, stack: &str) -> String {
        f.ops
            .store
            .read(&StackName::parse(stack).unwrap())
            .unwrap()
            .content_hash
            .as_str()
            .to_owned()
    }

    fn draft(
        stack: &str,
        base: Option<&str>,
        compose: &str,
        env: Option<&str>,
    ) -> depths_protocol::StackDraft {
        depths_protocol::StackDraft {
            stack: name(stack),
            base_hash: base.map(|h| depths_protocol::ContentHash::new(h).unwrap()),
            compose: compose.into(),
            env: env.map(Into::into),
        }
    }

    #[tokio::test]
    async fn an_assigned_user_saves_exactly_the_previewed_draft() {
        let f = fx(PREVIEW_TTL);
        let u = who("s1", "7", Role::User, &["web"]);
        let base = hash_of(&f, "web");
        let p = f
            .ops
            .preview_save(&u, &draft("web", Some(&base), CADDY, None), true)
            .await
            .unwrap();
        assert_eq!(p.kind, OperationKind::Save);
        assert_eq!(p.stack_hash, base);
        // validate (reads the draft), write, up.
        assert_eq!(p.steps.len(), 3);
        assert_eq!(p.steps[0].stdin, StepStdin::Draft);
        assert_eq!(
            p.steps[0].argv,
            ["docker", "compose", "-f", "-", "config", "--quiet"]
        );
        assert_eq!(p.steps[1].argv[0], "depths:write-draft");
        assert_eq!(p.steps[2].argv[2..], ["up", "-d", "--remove-orphans"]);
        let diff = p.file_diff.clone().unwrap();
        assert!(
            diff.contains("-    image: nginx\n+    image: caddy\n"),
            "{diff}"
        );
        // Compose saw the draft, not the file on disk.
        assert_eq!(f.mock.validated.lock().unwrap()[0].1, CADDY);

        let (out, _) = run(f.ops.confirm(&u, p.preview_id, false).await.unwrap()).await;
        assert_eq!(out, OpOutcome::Succeeded);
        let web = f.dir.path().join("stacks/web");
        assert_eq!(
            std::fs::read_to_string(web.join("compose.yaml")).unwrap(),
            CADDY
        );
        assert!(
            !web.join(".env").exists(),
            "an absent env leaves .env alone"
        );
        let ran = f.mock.ran.lock().unwrap().clone();
        assert_eq!(ran, vec![p.steps[0].argv.clone(), p.steps[2].argv.clone()]);
        assert_eq!(entries(&f.audit)[0].action, AuditAction::Save);
    }

    #[tokio::test]
    async fn a_save_is_audited_with_its_diff_summary_and_no_values() {
        // T099 F9 (FR-035): a save records what changed, `.env` reduced to its keys.
        let f = fx(PREVIEW_TTL);
        std::fs::write(
            f.dir.path().join("stacks/web/.env"),
            "DB_PASSWORD=hunter2\nGONE=1\nKEEP=same\n",
        )
        .unwrap();
        let u = who("s1", "7", Role::User, &["web"]);
        let base = hash_of(&f, "web");
        let env = "DB_PASSWORD=correct-horse\nKEEP=same\nNEW_KEY=fresh-value\n";
        let p = f
            .ops
            .preview_save(&u, &draft("web", Some(&base), CADDY, Some(env)), false)
            .await
            .unwrap();
        let (out, _) = run(f.ops.confirm(&u, p.preview_id, false).await.unwrap()).await;
        assert_eq!(out, OpOutcome::Succeeded);
        let e = &entries(&f.audit)[0];
        assert_eq!(e.action, AuditAction::Save);
        let s = e.diff_summary.clone().expect("a save has a diff summary");
        assert!(s.contains("compose.yaml: +1 -1 lines"), "{s}");
        assert!(s.contains(&format!("base {base}")), "{s}");
        assert!(s.contains("sha256 "), "{s}");
        for key in ["NEW_KEY", "DB_PASSWORD", "GONE"] {
            assert!(s.contains(key), "{key} missing from {s}");
        }
        assert!(!s.contains("KEEP"), "an unchanged key is not listed: {s}");
        for value in ["hunter2", "correct-horse", "fresh-value", "caddy", "nginx"] {
            assert!(!s.contains(value), "{value} leaked into {s}");
        }

        // A create has a summary too, with no base.
        let a = who("s2", "1", Role::Admin, &[]);
        let p = f
            .ops
            .preview_save(&a, &draft("fresh", None, CADDY, None), false)
            .await
            .unwrap();
        run(f.ops.confirm(&a, p.preview_id, false).await.unwrap()).await;
        let e = &entries(&f.audit)[0];
        assert_eq!(e.action, AuditAction::Create);
        let s = e.diff_summary.clone().expect("a create has a diff summary");
        assert!(
            s.contains("compose.yaml: +3 -0 lines") && s.contains("base none"),
            "{s}"
        );
    }

    #[tokio::test]
    async fn saves_and_creates_need_their_permission() {
        let f = fx(PREVIEW_TTL);
        let u = who("s1", "7", Role::User, &["web"]);
        let base = hash_of(&f, "db");
        let r = f
            .ops
            .preview_save(&u, &draft("db", Some(&base), CADDY, None), false)
            .await;
        assert_eq!(r.unwrap_err(), Refusal::Denied(DenyReason::NotPermitted));
        let r = f
            .ops
            .preview_save(&u, &draft("new", None, CADDY, None), false)
            .await;
        assert_eq!(r.unwrap_err(), Refusal::Denied(DenyReason::NotPermitted));
        let e = entries(&f.audit);
        assert_eq!(e.len(), 2);
        assert!(e.iter().all(|e| e.action == AuditAction::Denied));
        assert!(f.mock.validated.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stale_base_hash_is_a_conflict_with_the_current_files_masked() {
        let f = fx(PREVIEW_TTL);
        let a = who("s1", "1", Role::Admin, &[]);
        let base = hash_of(&f, "web");
        let web = f.dir.path().join("stacks/web");
        std::fs::write(web.join(".env"), "DB_PASSWORD=hunter2\nPORT=80\n").unwrap();
        match f
            .ops
            .preview_save(&a, &draft("web", Some(&base), CADDY, None), false)
            .await
        {
            Err(Refusal::Conflict {
                current_text,
                current_env,
                current_hash,
            }) => {
                assert_eq!(current_text, "services:\n  app:\n    image: nginx\n");
                assert_eq!(
                    current_env,
                    Some(EnvText::Masked("DB_PASSWORD=********\nPORT=80\n".into()))
                );
                assert_eq!(current_hash, hash_of(&f, "web"));
                assert_ne!(current_hash, base);
            }
            r => panic!("{r:?}"),
        }
    }

    #[tokio::test]
    async fn validation_errors_name_their_file_and_line() {
        let f = fx(PREVIEW_TTL);
        let a = who("s1", "1", Role::Admin, &[]);
        let web = f.dir.path().join("stacks/web");
        std::fs::write(web.join(".env"), "DB_PASSWORD=hunter2\n").unwrap();
        let base = hash_of(&f, "web");
        *f.mock.validation.lock().unwrap() = Some(Validation::Invalid(vec![
            depths_core::ports::ValidationIssue {
                line: Some(3),
                message: "services.app.image must be a string".into(),
            },
        ]));
        let env = "DB_PASSWORD=********\nnot a key\n";
        match f
            .ops
            .preview_save(&a, &draft("web", Some(&base), CADDY, Some(env)), false)
            .await
        {
            Err(Refusal::ValidationFailed(errors)) => {
                assert_eq!(
                    errors.iter().map(|e| (e.file, e.line)).collect::<Vec<_>>(),
                    // The masked line is not an error: its value is kept (see below).
                    vec![(DraftFile::Env, Some(2)), (DraftFile::Compose, Some(3))]
                );
                assert_eq!(errors[0].message, "expected KEY=value");
                assert!(!errors.iter().any(|e| e.message.contains("hunter2")));
            }
            r => panic!("{r:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(web.join(".env")).unwrap(),
            "DB_PASSWORD=hunter2\n"
        );
    }

    #[tokio::test]
    async fn a_user_saving_the_masked_env_keeps_the_real_secrets() {
        // Users cannot reveal `.env`, so they edit it masked. The save must not write the mask
        // over the secrets: a line still showing it, for a key the file has, keeps its value.
        let f = fx(PREVIEW_TTL);
        let web = f.dir.path().join("stacks/web");
        let on_disk = "DB_PASSWORD=hunter2\nAPI_TOKEN=t0k\nPRIVATE_KEY=\"-----BEGIN KEY-----\nMIIE\n-----END KEY-----\"\nPORT=80\n";
        std::fs::write(web.join(".env"), on_disk).unwrap();
        let u = who("s1", "7", Role::User, &["web"]);
        let base = hash_of(&f, "web");
        // What GetStack sent, with PORT and API_TOKEN edited, and a key renamed that keeps the
        // mask as its literal value.
        let masked = depths_core::secrets::mask_env(on_disk).unwrap();
        assert_eq!(
            masked,
            "DB_PASSWORD=********\nAPI_TOKEN=********\nPRIVATE_KEY=********\nPORT=80\n"
        );
        let edited = "DB_PASSWORD=********\nAPI_TOKEN=new-token\nPRIVATE_KEY=********\nPORT=8080\nOLD_PASSWORD=********\n";
        let p = f
            .ops
            .preview_save(&u, &draft("web", Some(&base), CADDY, Some(edited)), false)
            .await
            .unwrap();
        let diff = p.file_diff.clone().unwrap();
        for leak in ["hunter2", "MIIE", "new-token", "t0k"] {
            assert!(!diff.contains(leak), "{leak} leaked into the diff: {diff}");
        }
        assert!(diff.contains("-PORT=80\n+PORT=8080\n"), "{diff}");
        assert!(
            !diff.contains("-DB_PASSWORD"),
            "an unchanged secret is not in the diff: {diff}"
        );
        let (out, _) = run(f.ops.confirm(&u, p.preview_id, false).await.unwrap()).await;
        assert_eq!(out, OpOutcome::Succeeded);
        assert_eq!(
            std::fs::read_to_string(web.join(".env")).unwrap(),
            "DB_PASSWORD=hunter2\nAPI_TOKEN=new-token\nPRIVATE_KEY=\"-----BEGIN KEY-----\nMIIE\n-----END KEY-----\"\nPORT=8080\nOLD_PASSWORD=********\n"
        );
        let s = entries(&f.audit)[0].diff_summary.clone().unwrap();
        assert!(
            !s.contains("DB_PASSWORD") && !s.contains("PRIVATE_KEY"),
            "kept keys are unchanged: {s}"
        );
        assert!(s.contains("API_TOKEN") && s.contains("PORT"), "{s}");
    }

    #[tokio::test]
    async fn an_admin_creates_a_stack_whose_name_is_free() {
        let f = fx(PREVIEW_TTL);
        let a = who("s1", "1", Role::Admin, &[]);
        let r = f
            .ops
            .preview_save(&a, &draft("web", None, CADDY, None), false)
            .await;
        assert!(matches!(r, Err(Refusal::Invalid(why)) if why.contains("taken")));

        let p = f
            .ops
            .preview_save(&a, &draft("cache", None, CADDY, Some("PORT=6379\n")), false)
            .await
            .unwrap();
        assert_eq!(p.kind, OperationKind::Create);
        assert_eq!(p.stack_hash, "");
        assert_eq!(p.steps.len(), 2);
        assert_eq!(p.steps[0].stdin, StepStdin::Draft);
        let diff = p.file_diff.clone().unwrap();
        assert!(
            diff.starts_with("--- /dev/null\n+++ b/compose.yaml\n"),
            "{diff}"
        );
        assert!(
            diff.contains("+++ b/.env\n@@ -0,0 +1,1 @@\n+PORT=6379\n"),
            "{diff}"
        );

        let (out, _) = run(f.ops.confirm(&a, p.preview_id, false).await.unwrap()).await;
        assert_eq!(out, OpOutcome::Succeeded);
        let cache = f.dir.path().join("stacks/cache");
        assert_eq!(
            std::fs::read_to_string(cache.join("compose.yaml")).unwrap(),
            CADDY
        );
        assert_eq!(
            std::fs::read_to_string(cache.join(".env")).unwrap(),
            "PORT=6379\n"
        );

        // The name was taken between the preview and the confirm.
        let p = f
            .ops
            .preview_save(&a, &draft("late", None, CADDY, None), false)
            .await
            .unwrap();
        std::fs::create_dir(f.dir.path().join("stacks/late")).unwrap();
        assert_eq!(
            f.ops.confirm(&a, p.preview_id, false).await.err(),
            Some(Refusal::Void(PreviewVoidReason::StackChanged))
        );
    }

    #[tokio::test]
    async fn delete_is_admin_only_and_keeps_the_directory_unless_asked() {
        let f = fx(PREVIEW_TTL);
        let u = who("s1", "7", Role::User, &["web"]);
        let keep = StackOp::Delete {
            remove_directory: false,
        };
        assert_eq!(
            f.ops.preview(&u, &name("web"), &keep).await.unwrap_err(),
            Refusal::Denied(DenyReason::NotPermitted)
        );
        let a = who("s2", "1", Role::Admin, &[]);
        let p = f.ops.preview(&a, &name("web"), &keep).await.unwrap();
        assert_eq!(p.steps.len(), 1);
        assert!(p.file_diff.is_none());
        assert!(!p.destructive);
        run(f.ops.confirm(&a, p.preview_id, false).await.unwrap()).await;
        assert!(f.dir.path().join("stacks/web/compose.yaml").exists());

        let gone = StackOp::Delete {
            remove_directory: true,
        };
        let p = f.ops.preview(&a, &name("web"), &gone).await.unwrap();
        assert!(p.destructive);
        assert!(
            p.file_diff
                .as_deref()
                .unwrap()
                .contains("+++ /dev/null\n@@ -1,3 +0,0 @@\n-services:")
        );
        let (out, _) = run(f.ops.confirm(&a, p.preview_id, false).await.unwrap()).await;
        assert_eq!(out, OpOutcome::Succeeded);
        assert!(!f.dir.path().join("stacks/web").exists());
    }
}
