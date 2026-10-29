//! Spec 062 US3: the Auto queue (FR-013 to FR-016, FR-025).
//!
//! Each tick reads the stored policies, works out whether Auto is paused, and runs the stacks that
//! are due, one at a time in stack-name order. A stack is due when its policy is Auto, its window
//! (the stack's, or else the host's) is open and has not run this occurrence, and at least one of
//! its services has an update available whose effective policy is still Auto. Before each stack
//! the clock is read again: when the window has closed, the stack in progress has finished and the
//! rest wait for the next opening.
//!
//! A scheduled run acts as "Depths (scheduled)" on behalf of the policy's `set_by`, who must still
//! be an administrator (FR-025); otherwise the stack is listed in `paused_policies` until another
//! administrator saves the policy.
//!
//! The occurrence each stack last ran is kept in memory: a restart inside an open window runs a
//! due stack again at most once (misfit noted in the report).

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Local, LocalResult, NaiveDateTime, TimeZone, Utc};
use depths_core::updates::window::{LocalZone, Occurrence, OccurrenceKey, Window};
use depths_protocol::{
    Actor, AutoPause, EventEnvelope, MaintenanceWindow, RunId, RunTrigger, UpdatePolicy,
    UpdateState, UpdatesAvailability,
};

use super::Updates;
use super::inputs::Inputs;
use super::view;
use super::watchtower;

/// The host's zone (`chrono::Local`, which reads `TZ` and `/etc/localtime`).
pub struct HostZone;

impl LocalZone for HostZone {
    fn to_local(&self, utc: DateTime<Utc>) -> NaiveDateTime {
        utc.with_timezone(&Local).naive_local()
    }

    fn from_local(&self, local: NaiveDateTime) -> LocalResult<DateTime<Utc>> {
        Local
            .from_local_datetime(&local)
            .map(|t| t.with_timezone(&Utc))
    }
}

/// What the Updates page shows about the Auto queue.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub paused: Option<AutoPause>,
    /// Auto stacks whose policy author is no longer an administrator.
    pub paused_policies: Vec<String>,
    /// When the next window opens for an Auto stack (RFC 3339), or the one open now.
    pub next_window: Option<String>,
}

/// One stack the queue would run now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub stack: String,
    pub set_by: Actor,
    pub occurrence: Occurrence,
}

/// A tick's view: the status, and the due stacks in name order (empty while paused).
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub status: Status,
    pub due: Vec<Due>,
}

/// Whether `actor` is an administrator now (`users.json`).
pub type IsAdmin = dyn Fn(&Actor) -> bool + Send + Sync;

/// Administrators as `users.json` says now.
pub fn admins(users: Arc<crate::users::Registry>) -> Arc<IsAdmin> {
    Arc::new(move |a: &Actor| {
        let id = format!("{}:{}", crate::users::provider_str(a.provider), a.subject);
        depths_core::role::permits(
            &users.access(&id),
            None,
            depths_core::role::Action::ManageUpdates,
        )
    })
}

/// The protocol window as depths-core's.
pub fn to_core(w: &MaintenanceWindow) -> Option<Window> {
    serde_json::to_value(w)
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Why Auto runs nothing at all, in the order the page names them.
pub fn paused(u: &Updates) -> Option<AutoPause> {
    if !u.config().auto {
        return Some(AutoPause::Disabled);
    }
    if matches!(u.availability(), UpdatesAvailability::Unreadable { .. }) {
        return Some(AutoPause::SettingsUnreadable);
    }
    let acks = u
        .store()
        .decisions()
        .map(|d| d.watchtower_acks)
        .unwrap_or_default();
    watchtower::pause(&u.watchtower(), &acks)
}

/// Whether `stack` has an update the queue may install: available, not held or rejected, and
/// still Auto after labels.
fn has_auto_update(u: &Updates, stack: &str) -> bool {
    view::stack_services(u, stack)
        .iter()
        .any(|s| s.state == UpdateState::Available && s.effective.policy == UpdatePolicy::Auto)
}

/// The queue at `now`. `last_run` holds the occurrence each stack last ran.
pub fn plan(
    u: &Updates,
    now: DateTime<Utc>,
    zone: &dyn LocalZone,
    is_admin: &IsAdmin,
    last_run: &BTreeMap<String, OccurrenceKey>,
) -> Plan {
    let mut out = Plan {
        status: Status {
            paused: paused(u),
            ..Status::default()
        },
        due: Vec::new(),
    };
    let Ok(decisions) = u.store().decisions() else {
        return out;
    };
    let host = to_core(&decisions.settings.window);
    let mut next: Option<DateTime<Utc>> = None;
    // BTreeMap: stack-name order.
    for (stack, p) in &decisions.policies {
        if p.policy != UpdatePolicy::Auto {
            continue;
        }
        let set_by = Actor::from(p.set_by.clone());
        if !is_admin(&set_by) {
            out.status.paused_policies.push(stack.clone());
            continue;
        }
        let Some(window) = p.window.as_ref().map_or_else(|| host.clone(), to_core) else {
            continue;
        };
        if let Some(o) = window.next_occurrence(now, zone) {
            next = Some(next.map_or(o.start, |n| n.min(o.start)));
        }
        if out.status.paused.is_some() {
            continue;
        }
        if let Some(occurrence) = window.due(now, zone, last_run.get(stack).copied())
            && has_auto_update(u, stack)
        {
            out.due.push(Due {
                stack: stack.clone(),
                set_by,
                occurrence,
            });
        }
    }
    out.status.next_window = next.map(rfc3339);
    out
}

/// Records the status and runs the due stacks, one at a time. `run_stack` runs one stack
/// (blocking) as "Depths (scheduled)" on behalf of `set_by`. Before each stack the window and the
/// author are checked again.
pub fn tick(
    u: &Updates,
    zone: &dyn LocalZone,
    is_admin: &IsAdmin,
    last_run: &mut BTreeMap<String, OccurrenceKey>,
    run_stack: &mut dyn FnMut(&str, &Actor),
) {
    let p = plan(u, u.clock().now(), zone, is_admin, last_run);
    if u.set_auto_status(p.status) {
        u.notify_changed();
    }
    for d in p.due {
        if !d.occurrence.contains(u.clock().now()) {
            // The window closed while an earlier stack ran (FR-013).
            break;
        }
        if paused(u).is_some() {
            break;
        }
        if !is_admin(&d.set_by) {
            continue;
        }
        // Marked before it runs: a failed or held stack waits for the next occurrence.
        last_run.insert(d.stack.clone(), d.occurrence.key);
        run_stack(&d.stack, &d.set_by);
    }
}

/// What a scheduled stack needs to run: the stack store and runtime behind [`Inputs`], and
/// [`crate::ops::Ops`] and the health-wait `sleep` for the runner.
pub struct Deps {
    pub ops: Arc<crate::ops::Ops>,
    pub inputs: Arc<dyn Inputs>,
    pub zone: Arc<dyn LocalZone>,
    pub is_admin: Arc<IsAdmin>,
    /// How the runner waits between health reads: `std::thread::sleep` in production.
    pub sleep: super::run::Sleep,
}

/// Runs one stack through the US2 runner (blocking), forwarding its events to connected
/// administrators.
fn run_one(u: &Arc<Updates>, deps: &Deps, stack: &str, set_by: &Actor) {
    let store = deps.ops.stack_store().clone();
    let preview = super::batch::preview_stack(
        u,
        &*store,
        &*deps.inputs,
        deps.ops.compose_policy(),
        stack,
        super::batch::Only::Auto,
    );
    let job = match preview {
        Ok(p) => p.job,
        Err(e) => {
            tracing::info!(stack, "scheduled update skipped: {e}");
            None
        }
    };
    let Some(job) = job else {
        return;
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<EventEnvelope>(64);
    let forward = {
        let u = u.clone();
        std::thread::spawn(move || {
            while let Some(ev) = rx.blocking_recv() {
                u.send_run_event(ev);
            }
        })
    };
    let runner = super::run::Runner {
        ops: deps.ops.clone(),
        updates: u.clone(),
        sleep: deps.sleep.clone(),
    };
    let run = runner.run(
        super::run::Batch {
            run_id: RunId(uuid::Uuid::new_v4()),
            trigger: RunTrigger::Scheduled {
                on_behalf_of: [(stack.to_owned(), set_by.clone())].into(),
            },
            ip: std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED),
            jobs: vec![job],
            cancel: depths_core::ports::CancelToken::new(),
            reply_to: None,
            reply_on_op: false,
        },
        tx,
    );
    let _ = forward.join();
    tracing::info!(stack, run = %run.run_id.0, "scheduled update finished");
}

/// The Auto task: a tick, then sleep until the next window opens or something changes.
pub async fn run(u: Arc<Updates>, deps: Arc<Deps>) {
    let mut changes = u.subscribe_changes();
    let mut last_run: BTreeMap<String, OccurrenceKey> = BTreeMap::new();
    loop {
        let ticked = {
            let (u, deps) = (u.clone(), deps.clone());
            let mut last = std::mem::take(&mut last_run);
            tokio::task::spawn_blocking(move || {
                tick(&u, &*deps.zone, &*deps.is_admin, &mut last, &mut |s, by| {
                    run_one(&u, &deps, s, by);
                });
                last
            })
            .await
        };
        match ticked {
            Ok(l) => last_run = l,
            Err(e) => tracing::warn!("auto update tick panicked: {e}"),
        }
        // Wake at the next opening; a check pass (new update), a policy change or an
        // acknowledgement wakes it sooner. At most fifteen minutes, so a removed admin is noticed.
        let now = u.clock().now();
        let wait = u
            .auto_status()
            .next_window
            .and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
            .map(|t| t.with_timezone(&Utc))
            .filter(|t| *t > now)
            .and_then(|t| (t - now).to_std().ok())
            .unwrap_or(std::time::Duration::from_secs(900))
            .clamp(
                std::time::Duration::from_secs(1),
                std::time::Duration::from_secs(900),
            );
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            r = changes.recv() => {
                if matches!(r, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                    return;
                }
                // Coalesce a burst (the runner and a pass both signal).
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                changes = changes.resubscribe();
            }
        }
    }
}

#[cfg(test)]
mod tests;
