//! Spec 062 over the socket: `ListUpdates`, `CheckUpdatesNow`, `PreviewUpdateBatch`,
//! `PreviewUpdateChange` and their `Confirm`, the `UpdatesChanged` feed and the `UpdatesBadge`.
//!
//! FR-026: a user's 050 `PreviewOp { Update }` comes here too while checks are on
//! ([`preview_op`]); it keeps 050's gates for `Update`, and its `Confirm` runs a one-stack manual
//! run whose reply is the stack's `OpStarted`, as for 050's operations.
//!
//! Every other command is admin only (`Action::ManageUpdates`), then gated on the network: the page is a
//! view (`ViewUpdates`, allowed on a restricted network), Check now makes outbound requests, a
//! batch is an `Update` and a change is `ManageUpdates` (all three need a trusted network).
//!
//! A socket that may manage updates subscribes to the service's change signal when it opens.
//! Changes are coalesced: at most one `UpdatesChanged` (once the socket asked for the list) and
//! one `UpdatesBadge` (when the count moved) per [`COALESCE`].

use std::sync::Arc;
use std::time::Duration;

use depths_core::ports::CancelToken;
use depths_core::role::{self, Access, Action, CommandKind};
use depths_protocol::{
    Actor, AuditAction, AuditChange, AuditOutcome, AuditUpdate, DenyReason, Event, EventEnvelope,
    MaintenanceWindow, OperationKind, PasskeyAssertion, Preview, PreviewId, Reach, RequestId,
    RunId, RunTrigger, StackName, StackUpdatePlan, Step, StepStdin, UpdateChange, UpdatePolicy,
    UpdateState, WatchtowerAnswer,
};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::time::Instant;

use super::{Conn, invalid, reply};
use crate::audit;
use crate::http::AppState;
use crate::updates::Updates;
use crate::updates::batch::{self, Only, StackPreview};
use crate::updates::change::{self, Held, ManualUpdate};
use crate::updates::inputs::RuntimeInputs;
use crate::updates::run::{Batch, Runner, StackJob};
use crate::updates::scheduler::CheckNowRefused;
use crate::updates::store::{self, StoredPolicy, WatchtowerAck};
use crate::updates::view;

/// At most one `UpdatesChanged` and one `UpdatesBadge` per socket in this time.
pub const COALESCE: Duration = Duration::from_secs(1);

/// One socket's subscription to update changes.
pub struct Feed {
    rx: Option<broadcast::Receiver<()>>,
    /// Scheduled runs' events (US3), relayed as they come.
    runs: Option<broadcast::Receiver<EventEnvelope>>,
    /// The socket asked for `ListUpdates`, so it gets `UpdatesChanged`.
    pub list: bool,
    /// The badge count last sent.
    badge: Option<u32>,
    /// A change arrived and has not been sent.
    pub pending: bool,
    /// Not before this.
    pub next: Instant,
}

impl Feed {
    /// Subscribed when the person may manage updates and the feature is on.
    pub fn open(app: &AppState, access: &Access) -> Self {
        let u = app
            .updates
            .as_ref()
            .filter(|_| role::permits(access, None, Action::ManageUpdates));
        Self {
            rx: u.map(|u| u.subscribe_changes()),
            runs: u.map(|u| u.subscribe_runs()),
            list: false,
            badge: None,
            pending: false,
            next: Instant::now(),
        }
    }

    /// The badge to send on sign-in: admins with the feature on.
    pub fn initial(&mut self, app: &AppState) -> Option<EventEnvelope> {
        self.rx.as_ref()?;
        let u = app.updates.as_ref()?;
        let count = view::badge(&view::services(u));
        self.badge = Some(count);
        Some(reply(
            None,
            Event::UpdatesBadge {
                stacks_with_updates: count,
            },
        ))
    }

    /// What a change owes this socket, at most once per [`COALESCE`]. Someone who lost the right
    /// to manage updates is unsubscribed instead.
    pub fn flush(&mut self, app: &AppState, access: &Access) -> Vec<EventEnvelope> {
        self.pending = false;
        self.next = Instant::now() + COALESCE;
        let Some(u) = app.updates.as_ref() else {
            return Vec::new();
        };
        if !role::permits(access, None, Action::ManageUpdates) {
            self.rx = None;
            self.runs = None;
            self.list = false;
            return Vec::new();
        }
        let mut evs = Vec::new();
        let services = view::services(u);
        let count = view::badge(&services);
        if self.list {
            evs.push(reply(None, Event::UpdatesChanged(view::updates_view(u))));
        }
        if self.badge != Some(count) {
            self.badge = Some(count);
            evs.push(reply(
                None,
                Event::UpdatesBadge {
                    stacks_with_updates: count,
                },
            ));
        }
        evs
    }
}

/// What a socket's update subscriptions produced.
pub enum Signal {
    /// Something `ListUpdates` shows changed.
    Changed,
    /// The service is gone; the change subscription is dropped.
    Gone,
    /// A scheduled run's event (US3), relayed as it comes.
    Run(Box<EventEnvelope>),
}

/// The next signal; never resolves without a subscription.
pub async fn recv(feed: &mut Feed) -> Signal {
    let Feed {
        rx, runs, pending, ..
    } = feed;
    tokio::select! {
        s = changed(rx) => s,
        s = run_event(runs, pending) => s,
    }
}

async fn changed(rx: &mut Option<broadcast::Receiver<()>>) -> Signal {
    match rx.as_mut() {
        Some(r) => match r.recv().await {
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => Signal::Changed,
            Err(broadcast::error::RecvError::Closed) => {
                *rx = None;
                Signal::Gone
            }
        },
        None => std::future::pending().await,
    }
}

/// A socket too slow to keep up misses the events it lagged behind and gets `UpdatesChanged`.
async fn run_event(
    runs: &mut Option<broadcast::Receiver<EventEnvelope>>,
    pending: &mut bool,
) -> Signal {
    loop {
        let Some(r) = runs.as_mut() else {
            return std::future::pending().await;
        };
        match r.recv().await {
            Ok(ev) => return Signal::Run(Box::new(ev)),
            Err(broadcast::error::RecvError::Lagged(_)) => *pending = true,
            Err(broadcast::error::RecvError::Closed) => *runs = None,
        }
    }
}

/// Stops relaying scheduled runs to someone who lost the right to manage updates.
pub fn may_watch_runs(feed: &mut Feed, access: &Access) -> bool {
    let ok = role::permits(access, None, Action::ManageUpdates);
    if !ok {
        feed.runs = None;
    }
    ok
}

fn off(conn: &Conn, id: Option<RequestId>) -> EventEnvelope {
    conn.deny_reason(
        id,
        None,
        DenyReason::FeatureDisabled,
        "image updates are off (DEPTHS_UPDATES)".into(),
    )
}

/// Admin, then the network for `kind`. `Some` is the refusal to send.
fn gate(
    conn: &Conn,
    access: &Access,
    id: Option<RequestId>,
    kind: CommandKind,
    cmd: &str,
) -> Option<EventEnvelope> {
    if !role::permits(access, None, Action::ManageUpdates) {
        return Some(conn.deny(id, None, format!("{cmd} needs manage_updates")));
    }
    conn.untrusted(id, kind, cmd)
}

/// `ListUpdates`: the page, then `UpdatesChanged` as things change.
pub fn list(conn: &mut Conn, access: &Access, id: Option<RequestId>) -> EventEnvelope {
    let Some(u) = conn.app.updates.clone() else {
        return off(conn, id);
    };
    if let Some(refused) = gate(conn, access, id, CommandKind::ViewUpdates, "ListUpdates") {
        return refused;
    }
    conn.updates.list = true;
    reply(id, Event::Updates(view::updates_view(&u)))
}

/// `CheckUpdatesNow`: `CheckStarted`, `CheckBusy`, or `Invalid` while checks are off.
pub fn check_now(
    conn: &Conn,
    access: &Access,
    id: Option<RequestId>,
    stacks: Option<Vec<StackName>>,
) -> EventEnvelope {
    let Some(u) = conn.app.updates.clone() else {
        return off(conn, id);
    };
    if let Some(refused) = gate(
        conn,
        access,
        id,
        CommandKind::CheckUpdates,
        "CheckUpdatesNow",
    ) {
        return refused;
    }
    let stacks = stacks.map(|v| v.iter().map(|s| s.as_str().to_owned()).collect());
    match u.request_check_now(stacks) {
        Ok(()) => reply(id, Event::CheckStarted),
        Err(CheckNowRefused::Off) => invalid(
            id,
            "update checks are off: turn them on in the Updates settings first",
        ),
        Err(CheckNowRefused::Busy { retry_after_secs }) => {
            reply(id, Event::CheckBusy { retry_after_secs })
        }
    }
}

/// The previews of `stacks`, on the blocking pool (they read the stack store and run
/// `compose config`). `held` limits each to that held service and digest (`ApplyHeld`).
async fn previews(
    conn: &Conn,
    u: &Arc<Updates>,
    stacks: Vec<String>,
    held: Option<(String, String)>,
) -> Result<Vec<StackPreview>, String> {
    let u = u.clone();
    let store = conn.app.store.clone();
    let runtime = conn.app.runtime.clone();
    let policy = conn.app.ops.compose_policy();
    tokio::task::spawn_blocking(move || {
        let inputs = RuntimeInputs {
            store: store.clone(),
            runtime,
            hostname: std::env::var("HOSTNAME").ok(),
        };
        stacks
            .iter()
            .map(|s| {
                let only = match &held {
                    Some((service, digest)) => Only::Held { service, digest },
                    None => Only::Available,
                };
                batch::preview_stack(&u, &*store, &inputs, policy, s, only)
            })
            .collect()
    })
    .await
    .map_err(|e| format!("the preview failed: {e}"))?
}

fn readable(u: &Updates, id: Option<RequestId>) -> Option<EventEnvelope> {
    match u.availability() {
        depths_protocol::UpdatesAvailability::Unreadable { path, error } => Some(invalid(
            id,
            format!("{path} could not be read ({error}); fix or remove it first"),
        )),
        _ => None,
    }
}

fn expiry(u: &Updates) -> chrono::DateTime<chrono::Utc> {
    u.clock().now() + chrono::Duration::seconds(change::PREVIEW_TTL_SECS)
}

fn rfc3339(t: chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// `PreviewUpdateBatch`: per stack, what would be recreated, what is left and why, the steps
/// and the compose findings. Its `Confirm` runs the stacks in this order.
pub async fn preview_batch(
    conn: &Conn,
    access: &Access,
    id: Option<RequestId>,
    stacks: Vec<StackName>,
) -> EventEnvelope {
    let Some(u) = conn.app.updates.clone() else {
        return off(conn, id);
    };
    if let Some(refused) = gate(conn, access, id, CommandKind::Update, "PreviewUpdateBatch") {
        return refused;
    }
    if let Some(refused) = readable(&u, id) {
        return refused;
    }
    let names: Vec<String> = stacks.iter().map(|s| s.as_str().to_owned()).collect();
    if names.is_empty() {
        return invalid(id, "choose at least one stack");
    }
    if let Some(twice) = names
        .iter()
        .enumerate()
        .find(|(i, n)| names[..*i].contains(n))
    {
        return invalid(id, format!("{} is listed twice", twice.1));
    }
    let previews = match previews(conn, &u, names, None).await {
        Ok(p) => p,
        Err(why) => return invalid(id, why),
    };
    let escape = previews.iter().any(|p| p.escape);
    let plans = previews.iter().map(|p| p.plan.clone()).collect();
    let jobs = previews.into_iter().filter_map(|p| p.job).collect();
    let preview_id = u.hold_change(change::PendingChange {
        session: conn.session.clone(),
        held: Held::Batch { jobs, escape },
        expires: expiry(&u),
    });
    reply(
        id,
        Event::UpdatePreview {
            preview_id,
            stacks: plans,
            // Spec 054: `Update` steps up with an escape-level finding.
            step_up: crate::step_up::preview_request(
                &conn.app,
                conn.caller(access.role),
                CommandKind::Update,
                escape,
            ),
        },
    )
}

/// FR-026: a user's 050 `PreviewOp { Update }`. While checks are on and the stack has update
/// states, the preview is this feature's image-update plan for the services with a newer image,
/// held for a one-stack manual run. `None` when 050's pull-then-up still applies: the feature is
/// off, checks are off, or no service of the stack has been checked yet.
///
/// The caller has already passed 050's gates for `Update` (role on the stack, then the network);
/// nothing is added here. The step-up is 054's rule for `Update`, as 050's previews carry it.
pub async fn preview_op(
    conn: &Conn,
    access: &Access,
    id: Option<RequestId>,
    stack: &StackName,
) -> Option<EventEnvelope> {
    let u = conn.app.updates.clone()?;
    let name = stack.as_str().to_owned();
    let known = view::stack_services(&u, &name)
        .iter()
        .any(|s| s.state != UpdateState::NotChecked);
    if !image_update(u.checks_enabled(), known) {
        return None;
    }
    if let Some(busy) = conn.app.ops.busy(&name) {
        return Some(reply(id, busy.into()));
    }
    let p = match previews(conn, &u, vec![name.clone()], None).await {
        Ok(mut v) => v.pop()?,
        Err(why) => return Some(invalid(id, why)),
    };
    let Some(job) = p.job else {
        return Some(invalid(id, nothing_to_update(&p.plan)));
    };
    let shown: Vec<String> = job.pins.iter().map(|p| p.service.clone()).collect();
    let runtime = conn.app.runtime.clone();
    let project = name.clone();
    let containers = tokio::task::spawn_blocking(move || {
        runtime
            .containers_by_project()
            .ok()
            .and_then(|snap| snap.projects.get(&project).cloned())
            .map(|p| {
                p.containers
                    .into_iter()
                    .filter(|c| c.service.as_ref().is_some_and(|s| shown.contains(s)))
                    .map(|c| c.name)
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    let cwd = conn.app.store.root().join(&name).display().to_string();
    let expires = expiry(&u);
    let stack_hash = job.stack_hash.clone();
    let preview_id = u.hold_change(change::PendingChange {
        session: conn.session.clone(),
        held: Held::Manual(ManualUpdate {
            job,
            escape: p.escape,
            ack_required: p.plan.ack_required,
        }),
        expires,
    });
    Some(reply(
        id,
        Event::Preview(Preview {
            preview_id,
            stack: name,
            kind: OperationKind::Update,
            steps: p
                .plan
                .argv
                .iter()
                .map(|argv| Step {
                    argv: argv.clone(),
                    cwd: cwd.clone(),
                    stdin: StepStdin::None,
                })
                .collect(),
            step_up: crate::step_up::preview_request(
                &conn.app,
                conn.caller(access.role),
                CommandKind::Update,
                p.escape,
            ),
            file_diff: Some(manual_summary(&p.plan)),
            reaches: Reach {
                containers,
                ..Reach::default()
            },
            destructive: false,
            stack_hash,
            expires_at: rfc3339(expires),
            warnings: p.plan.findings,
            ack_required: p.plan.ack_required,
        }),
    ))
}

/// FR-026's switch, through core's [`depths_core::plan::update_operation`]: whether 050's
/// `Update` becomes the image-update plan.
fn image_update(checks_enabled: bool, states_known: bool) -> bool {
    matches!(
        depths_core::plan::update_operation(checks_enabled, states_known, Vec::new(), Vec::new()),
        depths_core::plan::Operation::ImageUpdate { .. }
    )
}

/// Why a user's Update has nothing to install: each service and why it is left.
fn nothing_to_update(plan: &StackUpdatePlan) -> String {
    let why: Vec<String> = plan
        .skipped
        .iter()
        .map(|(service, why)| format!("{service}: {why}"))
        .collect();
    if why.is_empty() {
        format!("nothing to update on {}", plan.stack)
    } else {
        format!("nothing to update on {} ({})", plan.stack, why.join("; "))
    }
}

/// What a user's image update changes, as diff lines: per service the running image (`-`) and
/// the one installed (`+`), then the services left as they are.
fn manual_summary(plan: &StackUpdatePlan) -> String {
    let mut lines = Vec::new();
    for s in &plan.services {
        let from = s
            .running
            .as_ref()
            .map(|r| r.repo_digest.clone().unwrap_or_else(|| r.image_id.clone()))
            .unwrap_or_default();
        lines.push(format!("-{}: {} {from}", s.service, s.reference));
        if let Some(a) = &s.available {
            let version = a
                .version
                .as_ref()
                .map(|v| format!(" ({v})"))
                .unwrap_or_default();
            lines.push(format!(
                "+{}: {} {}{version}",
                s.service, s.reference, a.digest
            ));
        }
    }
    for (service, why) in &plan.skipped {
        lines.push(format!(" {service}: left as it is ({why})"));
    }
    lines.push(" only services with a newer image are recreated".into());
    lines.join("\n")
}

/// What a change preview shows besides its summary.
#[derive(Default)]
struct Shown {
    stack: String,
    steps: Vec<Step>,
    warnings: Vec<depths_protocol::ComposeWarning>,
    ack_required: bool,
}

/// `PreviewUpdateChange`: settings, a stack's policy, forgetting a rejected digest, applying a
/// held image or answering about a Watchtower container. Every one steps up (FR-017).
pub async fn preview_change(
    conn: &Conn,
    access: &Access,
    id: Option<RequestId>,
    change: UpdateChange,
) -> EventEnvelope {
    let Some(u) = conn.app.updates.clone() else {
        return off(conn, id);
    };
    if let Some(refused) = gate(
        conn,
        access,
        id,
        CommandKind::ManageUpdates,
        "PreviewUpdateChange",
    ) {
        return refused;
    }
    if let Some(refused) = readable(&u, id) {
        return refused;
    }
    let made = match change {
        UpdateChange::Settings(after) => {
            let before = u.store().settings();
            change::check(&before, &after).map(|()| {
                (
                    change::summary(&before, &after),
                    Held::Settings { before, after },
                    Shown::default(),
                )
            })
        }
        UpdateChange::Policy {
            stack,
            policy,
            window,
        } => policy_change(conn, &u, stack.as_str(), policy, window),
        UpdateChange::Retry {
            stack,
            service,
            digest,
        } => retry_change(&u, stack.as_str(), service.as_str(), digest.as_str()),
        UpdateChange::ApplyHeld {
            stack,
            service,
            digest,
        } => apply_held_change(conn, &u, stack.as_str(), service.as_str(), digest.as_str()).await,
        UpdateChange::AckWatchtower {
            container_id,
            answer,
        } => watchtower_change(conn, &u, container_id.as_str(), answer).await,
    };
    let (summary, held, shown) = match made {
        Ok(m) => m,
        Err(why) => return invalid(id, why),
    };
    let expires = expiry(&u);
    let preview_id = u.hold_change(change::PendingChange {
        session: conn.session.clone(),
        held,
        expires,
    });
    reply(
        id,
        Event::Preview(Preview {
            preview_id,
            stack: shown.stack,
            kind: OperationKind::Update,
            steps: shown.steps,
            file_diff: Some(summary),
            reaches: Reach::default(),
            destructive: false,
            stack_hash: String::new(),
            expires_at: rfc3339(expires),
            warnings: shown.warnings,
            ack_required: shown.ack_required,
            // Spec 054: an update change always needs a fresh passkey check.
            step_up: crate::step_up::preview_request(
                &conn.app,
                conn.caller(access.role),
                CommandKind::ManageUpdates,
                false,
            ),
        }),
    )
}

type Made = Result<(String, Held, Shown), String>;

fn word<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// A stack's policy (US3). Saving the same policy again is allowed when someone else set it: that
/// is how a paused Auto policy is resumed under a current administrator.
fn policy_change(
    conn: &Conn,
    u: &Updates,
    stack: &str,
    policy: UpdatePolicy,
    window: Option<MaintenanceWindow>,
) -> Made {
    let name = depths_core::StackName::parse(stack).map_err(|e| format!("{stack}: {e}"))?;
    conn.app
        .store
        .entry(&name)
        .map_err(|e| format!("{stack}: {e}"))?;
    if let Some(w) = &window {
        store::validate_window("window", w)?;
    }
    let before = u
        .store()
        .decisions()
        .map_err(|e| e.to_string())?
        .policies
        .get(stack)
        .cloned();
    if let Some(b) = &before
        && b.policy == policy
        && b.window == window
        && Actor::from(b.set_by.clone()) == conn.actor
    {
        return Err("nothing to change".into());
    }
    let mut lines = vec![format!(
        "update policy of {stack}: {} -> {}",
        before
            .as_ref()
            .map_or_else(|| "notify (default)".to_owned(), |b| word(&b.policy)),
        word(&policy)
    )];
    let old_window = before.as_ref().and_then(|b| b.window.clone());
    if old_window != window {
        let show = |w: &Option<MaintenanceWindow>| {
            w.as_ref()
                .map_or_else(|| "the host window".to_owned(), change::window)
        };
        lines.push(format!(
            "maintenance window: {} -> {}",
            show(&old_window),
            show(&window)
        ));
    }
    if policy == UpdatePolicy::Auto {
        lines.push(format!(
            "Depths will install new images for {stack} in the maintenance window, on your behalf"
        ));
    }
    Ok((
        lines.join("\n"),
        Held::Policy {
            stack: stack.to_owned(),
            before,
            policy,
            window,
        },
        Shown {
            stack: stack.to_owned(),
            ..Shown::default()
        },
    ))
}

fn retry_change(u: &Updates, stack: &str, service: &str, digest: &str) -> Made {
    let rejected = u.store().decisions().map_err(|e| e.to_string())?.rejected;
    if !rejected
        .iter()
        .any(|r| r.stack == stack && r.service == service && r.digest == digest)
    {
        return Err(format!("{stack}/{service} has no rejected image {digest}"));
    }
    Ok((
        format!(
            "forget that {digest} failed for {stack}/{service}: the next check offers it again"
        ),
        Held::Retry {
            stack: stack.to_owned(),
            service: service.to_owned(),
            digest: digest.to_owned(),
        },
        Shown {
            stack: stack.to_owned(),
            ..Shown::default()
        },
    ))
}

async fn apply_held_change(
    conn: &Conn,
    u: &Arc<Updates>,
    stack: &str,
    service: &str,
    digest: &str,
) -> Made {
    let held = Some((service.to_owned(), digest.to_owned()));
    let p = previews(conn, u, vec![stack.to_owned()], held)
        .await?
        .pop()
        .ok_or("nothing to preview")?;
    let Some(job) = p.job else {
        let why = p
            .plan
            .skipped
            .first()
            .map_or_else(|| "it is not held".to_owned(), |(_, why)| why.clone());
        return Err(format!("{stack}/{service}: {why}"));
    };
    let cwd = conn.app.store.root().join(stack).display().to_string();
    Ok((
        format!(
            "install {digest} for {stack}/{service} although it was held; the hold is overridden \
             for this run only and audited"
        ),
        Held::ApplyHeld {
            digest: digest.to_owned(),
            job,
        },
        Shown {
            stack: stack.to_owned(),
            steps: p
                .plan
                .argv
                .into_iter()
                .map(|argv| Step {
                    argv,
                    cwd: cwd.clone(),
                    stdin: StepStdin::None,
                })
                .collect(),
            warnings: p.plan.findings,
            ack_required: p.plan.ack_required,
        },
    ))
}

async fn watchtower_change(
    conn: &Conn,
    u: &Arc<Updates>,
    container_id: &str,
    answer: WatchtowerAnswer,
) -> Made {
    let runtime = conn.app.runtime.clone();
    let found = tokio::task::spawn_blocking(move || crate::updates::watchtower::running(&*runtime))
        .await
        .map_err(|e| e.to_string())?;
    let w = found
        .into_iter()
        .find(|w| {
            w.container_id.starts_with(container_id) || container_id.starts_with(&w.container_id)
        })
        .ok_or_else(|| format!("no running Watchtower container {container_id}"))?;
    let before = u
        .store()
        .decisions()
        .map_err(|e| e.to_string())?
        .watchtower_acks
        .into_iter()
        .find(|a| a.container_id == w.container_id && a.image_id == w.image_id);
    if before.as_ref().is_some_and(|a| a.answer == answer) {
        return Err("nothing to change".into());
    }
    let says = match answer {
        WatchtowerAnswer::Stopped => "you stopped it, or will stop it",
        WatchtowerAnswer::MonitorOnly => {
            "it only monitors (WATCHTOWER_MONITOR_ONLY), so it changes nothing"
        }
    };
    Ok((
        format!(
            "Watchtower container {} ({}): {says}; the warning is no longer shown for it",
            w.name, w.container_id
        ),
        Held::Watchtower {
            container_id: w.container_id,
            image_id: w.image_id,
            name: w.name,
            answer,
        },
        Shown::default(),
    ))
}

/// Whether `Confirm{preview_id}` is an update change or batch of this socket's session.
pub fn owns(conn: &Conn, preview_id: PreviewId) -> bool {
    conn.app
        .updates
        .as_ref()
        .is_some_and(|u| u.has_change(&conn.session, preview_id))
}

/// `Confirm` of an update change or batch.
///
/// A change is applied, audited with before and after, and answered with `UpdatesChanged`. A
/// batch starts a run: `UpdateRunStarted` (the reply), then the run's events.
///
/// Spec 054: a change (`ManageUpdates`) always steps up; a batch (`Update`) steps up with an
/// escape-level finding. The check runs after the role and network gate and before the preview
/// is claimed, so a refused check changes nothing.
pub async fn confirm(
    conn: &Conn,
    access: &Access,
    id: Option<RequestId>,
    preview_id: PreviewId,
    acknowledged: bool,
    assertion: Option<&PasskeyAssertion>,
) -> Vec<EventEnvelope> {
    let Some(u) = conn.app.updates.clone() else {
        return vec![off(conn, id)];
    };
    if let Some(m) = u.manual_update(&conn.session, preview_id) {
        return confirm_manual(conn, &u, access, id, preview_id, acknowledged, assertion, m).await;
    }
    let (kind, escape) = match u.batch_escape(&conn.session, preview_id) {
        Some(escape) => (CommandKind::Update, escape),
        None => (CommandKind::ManageUpdates, false),
    };
    // Authorised when previewed; the role or the network may have changed since.
    if let Some(refused) = gate(conn, access, id, kind, "Confirm") {
        return vec![refused];
    }
    if let Some(refused) = conn
        .step_up(id, access.role, kind, escape, assertion, "Confirm")
        .await
    {
        return vec![refused];
    }
    let pending = match u.take_change(&conn.session, preview_id) {
        Ok(c) => c,
        Err(reason) => return vec![reply(id, Event::PreviewVoid { reason })],
    };
    let at = rfc3339(u.clock().now());
    let actor = conn.actor.clone();
    match pending.held {
        Held::Settings { after, .. } => {
            let mut before = None;
            let result = u.store().update_decisions(|d| {
                before = Some(d.settings.settings());
                d.settings.set(after, actor, at);
            });
            vec![saved(
                conn,
                &u,
                id,
                preview_id,
                AuditAction::UpdateSettings,
                None,
                result
                    .map(|d| (json(&before), json(&d.settings.settings())))
                    .map_err(|e| e.to_string()),
            )]
        }
        Held::Policy {
            stack,
            policy,
            window,
            ..
        } => {
            let mut before = None;
            let result = u.store().update_decisions(|d| {
                before = d.policies.get(&stack).map(StoredPolicy::to_wire);
                d.policies.insert(
                    stack.clone(),
                    StoredPolicy {
                        policy,
                        window,
                        set_by: actor.into(),
                        set_at: at,
                    },
                );
            });
            let result = result
                .map(|d| {
                    (
                        json(&before),
                        json(&d.policies.get(&stack).map(StoredPolicy::to_wire)),
                    )
                })
                .map_err(|e| e.to_string());
            vec![saved(
                conn,
                &u,
                id,
                preview_id,
                AuditAction::UpdatePolicy,
                Some(stack),
                result,
            )]
        }
        Held::Retry {
            stack,
            service,
            digest,
        } => {
            let mut forgot = Vec::new();
            let result = u.store().update_decisions(|d| {
                d.rejected.retain(|r| {
                    let it = r.stack == stack && r.service == service && r.digest == digest;
                    if it {
                        forgot.push(r.clone());
                    }
                    !it
                });
            });
            let result = result.map_err(|e| e.to_string()).and_then(|_| {
                let r = forgot.first().ok_or("it was already forgotten")?;
                Ok((
                    serde_json::json!({
                        "service": r.service, "digest": r.digest, "reason": r.reason, "at": r.at,
                    }),
                    Value::Null,
                ))
            });
            vec![saved(
                conn,
                &u,
                id,
                preview_id,
                AuditAction::UpdateRetry,
                Some(stack),
                result,
            )]
        }
        Held::Watchtower {
            container_id,
            image_id,
            name,
            answer,
        } => {
            let mut before = None;
            let result = u.store().update_decisions(|d| {
                before = d
                    .watchtower_acks
                    .iter()
                    .find(|a| a.container_id == container_id)
                    .map(|a| a.answer);
                d.watchtower_acks.retain(|a| a.container_id != container_id);
                d.watchtower_acks.push(WatchtowerAck {
                    container_id: container_id.clone(),
                    image_id,
                    name: name.clone(),
                    answer,
                    by: actor.into(),
                    at,
                });
            });
            let result = result
                .map(|_| {
                    (
                        serde_json::json!({ "container": name, "answer": before }),
                        serde_json::json!({ "container": name, "answer": answer }),
                    )
                })
                .map_err(|e| e.to_string());
            vec![saved(
                conn,
                &u,
                id,
                preview_id,
                AuditAction::WatchtowerAck,
                None,
                result,
            )]
        }
        Held::ApplyHeld { digest, mut job } => {
            let run_id = RunId(uuid::Uuid::new_v4());
            let mut entry = audit::entry(
                conn.actor.clone(),
                AuditAction::ImageHoldApply,
                conn.ip,
                AuditOutcome::Succeeded,
            );
            entry.preview_id = Some(preview_id);
            entry.stack = Some(job.stack.clone());
            entry.update = Some(AuditUpdate {
                run_id,
                services: Vec::new(),
                rejected_digest: None,
                hold: Some(format!("overridden for {digest}")),
            });
            let _ = conn.app.audit.record(&entry);
            job.acknowledged = acknowledged;
            run(conn, u, id, run_id, vec![job], false)
        }
        // `confirm_manual` takes these before they get here.
        Held::Manual(_) => vec![reply(
            id,
            Event::PreviewVoid {
                reason: depths_protocol::PreviewVoidReason::Unknown,
            },
        )],
        Held::Batch { mut jobs, .. } => {
            if jobs.is_empty() {
                return vec![invalid(
                    id,
                    "nothing to update: every service shown is up to date or left as it is",
                )];
            }
            for j in &mut jobs {
                j.acknowledged = acknowledged;
            }
            run(conn, u, id, RunId(uuid::Uuid::new_v4()), jobs, false)
        }
    }
}

/// FR-026: `Confirm` of a user's image update, with 050's gates for `Update` on the stack (role,
/// network, 054's step-up rule, 051's acknowledgement), then its busy and stale refusals, all
/// before the preview is claimed so a refusal changes nothing. The run answers with the stack's
/// `OpStarted`, and its events go to this socket.
#[allow(clippy::too_many_arguments)]
async fn confirm_manual(
    conn: &Conn,
    u: &Arc<Updates>,
    access: &Access,
    id: Option<RequestId>,
    preview_id: PreviewId,
    acknowledged: bool,
    assertion: Option<&PasskeyAssertion>,
    m: ManualUpdate,
) -> Vec<EventEnvelope> {
    let stack = m.job.stack.clone();
    let permitted = depths_core::StackName::parse(&stack).is_ok_and(|s| {
        role::permits(
            access,
            Some(&s),
            depths_core::plan::Operation::Update.required_action(),
        )
    });
    if !permitted {
        return vec![conn.deny(id, Some(&stack), "Update is no longer permitted".into())];
    }
    if let Some(refused) = conn.untrusted(id, CommandKind::Update, "Confirm") {
        return vec![refused];
    }
    if let Some(refused) = conn
        .step_up(
            id,
            access.role,
            CommandKind::Update,
            m.escape,
            assertion,
            "Confirm",
        )
        .await
    {
        return vec![refused];
    }
    let admin = access.role == role::Role::Admin;
    if m.ack_required && !(acknowledged && admin) {
        let _ = conn.app.audit.record(&audit::denied(
            conn.actor.clone(),
            conn.ip,
            Some(&stack),
            "Update confirm refused: escape-level findings not acknowledged",
        ));
        let why = if admin {
            "this preview has escape-level compose findings; deploy anyway only by acknowledging \
             them"
        } else {
            "this preview has escape-level compose findings that only an admin may acknowledge"
        };
        return vec![reply(
            id,
            crate::ops::Refusal::ComposeBlocked(why.into()).into(),
        )];
    }
    if let Some(busy) = conn.app.ops.busy(&stack) {
        return vec![reply(id, busy.into())];
    }
    let store = conn.app.store.clone();
    let name = stack.clone();
    let hash_now = tokio::task::spawn_blocking(move || {
        depths_core::StackName::parse(&name)
            .ok()
            .and_then(|n| store.entry(&n).ok())
            .and_then(|e| e.content_hash)
            .map(|h| h.as_str().to_owned())
    })
    .await
    .ok()
    .flatten();
    if hash_now.as_deref() != Some(m.job.stack_hash.as_str()) {
        let _ = u.take_change(&conn.session, preview_id);
        return vec![reply(
            id,
            Event::PreviewVoid {
                reason: depths_protocol::PreviewVoidReason::StackChanged,
            },
        )];
    }
    let mut job = match u.take_change(&conn.session, preview_id) {
        Ok(change::PendingChange {
            held: Held::Manual(m),
            ..
        }) => m.job,
        Ok(_) => {
            return vec![reply(
                id,
                Event::PreviewVoid {
                    reason: depths_protocol::PreviewVoidReason::Unknown,
                },
            )];
        }
        Err(reason) => return vec![reply(id, Event::PreviewVoid { reason })],
    };
    job.acknowledged = acknowledged && admin;
    run(
        conn,
        u.clone(),
        id,
        RunId(uuid::Uuid::new_v4()),
        vec![job],
        true,
    )
}

fn json<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or_default()
}

/// Audits a saved change and answers it.
fn saved(
    conn: &Conn,
    u: &Updates,
    id: Option<RequestId>,
    preview_id: PreviewId,
    action: AuditAction,
    stack: Option<String>,
    result: Result<(Value, Value), String>,
) -> EventEnvelope {
    let outcome = match &result {
        Ok(_) => AuditOutcome::Succeeded,
        Err(e) => AuditOutcome::Failed {
            exit_code: None,
            stderr_tail: e.clone(),
        },
    };
    let mut entry = audit::entry(conn.actor.clone(), action, conn.ip, outcome);
    entry.preview_id = Some(preview_id);
    entry.stack = stack;
    if let Ok((before, after)) = &result {
        entry.change = Some(AuditChange {
            before: before.clone(),
            after: after.clone(),
        });
    }
    let _ = conn.app.audit.record(&entry);
    match result {
        Ok(_) => {
            u.wake();
            u.notify_changed();
            reply(id, Event::UpdatesChanged(view::updates_view(u)))
        }
        Err(e) => invalid(id, format!("the change was not saved: {e}")),
    }
}

/// Starts a confirmed run on the blocking pool. `UpdateRunStarted` is its reply (with
/// `reply_on_op`, a user's FR-026 Update: the stack's `OpStarted`), so nothing is returned here.
fn run(
    conn: &Conn,
    u: Arc<Updates>,
    id: Option<RequestId>,
    run_id: RunId,
    jobs: Vec<StackJob>,
    reply_on_op: bool,
) -> Vec<EventEnvelope> {
    let runner = Runner {
        ops: conn.app.ops.clone(),
        updates: u,
        sleep: Arc::new(std::thread::sleep),
    };
    let batch = Batch {
        run_id,
        trigger: RunTrigger::Manual {
            actor: conn.actor.clone(),
        },
        ip: conn.ip,
        jobs,
        cancel: CancelToken::new(),
        reply_to: id,
        reply_on_op,
    };
    let tx = conn.out.clone();
    tokio::task::spawn_blocking(move || {
        runner.run(batch, tx);
    });
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(skipped: &[(&str, &str)]) -> StackUpdatePlan {
        StackUpdatePlan {
            stack: "web".into(),
            services: Vec::new(),
            skipped: skipped
                .iter()
                .map(|(s, w)| ((*s).to_owned(), (*w).to_owned()))
                .collect(),
            argv: Vec::new(),
            findings: Vec::new(),
            ack_required: false,
        }
    }

    #[test]
    fn the_image_update_needs_checks_on_and_a_known_state() {
        assert!(image_update(true, true));
        assert!(!image_update(false, true), "checks off: 050's Update");
        assert!(
            !image_update(true, false),
            "nothing checked yet: 050's Update"
        );
        assert!(!image_update(false, false));
    }

    #[test]
    fn nothing_to_update_names_each_service_and_why() {
        assert_eq!(nothing_to_update(&plan(&[])), "nothing to update on web");
        assert_eq!(
            nothing_to_update(&plan(&[("app", "up to date"), ("db", "excluded")])),
            "nothing to update on web (app: up to date; db: excluded)"
        );
    }

    #[test]
    fn the_summary_is_diff_lines_from_running_to_available() {
        let mut p = plan(&[("db", "up to date")]);
        p.services.push(depths_protocol::ServiceUpdate {
            stack: "web".into(),
            service: "app".into(),
            reference: "nginx:1".into(),
            running: Some(depths_protocol::RunningImage {
                image_id: "sha256:old".into(),
                repo_digest: Some("nginx@sha256:was".into()),
                version: None,
                revision: None,
            }),
            available: Some(depths_protocol::AvailableImage {
                digest: "sha256:new".into(),
                platform_digest: String::new(),
                config_digest: String::new(),
                version: Some("1.2".into()),
                revision: None,
                created: None,
                user: None,
            }),
            state: UpdateState::Available,
            reason: None,
            effective: depths_protocol::EffectivePolicy {
                policy: UpdatePolicy::Notify,
                source: depths_protocol::PolicySource::Default,
                excluded: None,
                labels: Vec::new(),
            },
            last_checked: None,
            last_outcome: None,
            replicas: 1,
        });
        assert_eq!(
            manual_summary(&p),
            "-app: nginx:1 nginx@sha256:was\n\
             +app: nginx:1 sha256:new (1.2)\n \
             db: left as it is (up to date)\n \
             only services with a newer image are recreated"
        );
    }
}
