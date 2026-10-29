//! Spec 062 US1 over the socket: `ListUpdates`, `CheckUpdatesNow`, `PreviewUpdateChange(Settings)`
//! and its `Confirm`, the `UpdatesChanged` feed and the `UpdatesBadge`.
//!
//! Every command is admin only (`Action::ManageUpdates`), then gated on the network: the page is a
//! view (`ViewUpdates`, allowed on a restricted network), Check now makes outbound requests and a
//! settings change is a change (both need a trusted network).
//!
//! A socket that may manage updates subscribes to the service's change signal when it opens.
//! Changes are coalesced: at most one `UpdatesChanged` (once the socket asked for the list) and
//! one `UpdatesBadge` (when the count moved) per [`COALESCE`].

use std::time::Duration;

use depths_core::role::{self, Access, Action, CommandKind};
use depths_protocol::{
    AuditAction, AuditChange, AuditOutcome, DenyReason, Event, EventEnvelope, OperationKind,
    PasskeyAssertion, Preview, PreviewId, Reach, RequestId, StackName, UpdateChange,
};
use tokio::sync::broadcast;
use tokio::time::Instant;

use super::{Conn, invalid, reply};
use crate::audit;
use crate::http::AppState;
use crate::updates::scheduler::CheckNowRefused;
use crate::updates::{change, view};

/// At most one `UpdatesChanged` and one `UpdatesBadge` per socket in this time.
pub const COALESCE: Duration = Duration::from_secs(1);

/// One socket's subscription to update changes.
pub struct Feed {
    rx: Option<broadcast::Receiver<()>>,
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
        let rx = app
            .updates
            .as_ref()
            .filter(|_| role::permits(access, None, Action::ManageUpdates))
            .map(|u| u.subscribe_changes());
        Self {
            rx,
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

/// The next change signal; never resolves without a subscription. `false` when the service is
/// gone (the subscription is dropped).
pub async fn recv(feed: &mut Feed) -> bool {
    match feed.rx.as_mut() {
        Some(rx) => match rx.recv().await {
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => true,
            Err(broadcast::error::RecvError::Closed) => {
                feed.rx = None;
                false
            }
        },
        None => std::future::pending().await,
    }
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

/// `PreviewUpdateChange`: US1 serves `Settings` (checks on or off, the interval).
pub fn preview_change(
    conn: &Conn,
    access: &Access,
    id: Option<RequestId>,
    change: UpdateChange,
) -> EventEnvelope {
    let Some(u) = conn.app.updates.clone() else {
        return off(conn, id);
    };
    let UpdateChange::Settings(after) = change else {
        return conn.deny_reason(
            id,
            None,
            DenyReason::FeatureDisabled,
            "this update change is not available yet (spec 062)".into(),
        );
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
    if let depths_protocol::UpdatesAvailability::Unreadable { path, error } = u.availability() {
        return invalid(
            id,
            format!("{path} could not be read ({error}); fix or remove it first"),
        );
    }
    let before = u.store().settings();
    if let Err(why) = change::check(&before, &after) {
        return invalid(id, why);
    }
    let summary = change::summary(&before, &after);
    let expires = u.clock().now() + chrono::Duration::seconds(change::PREVIEW_TTL_SECS);
    let preview_id = u.hold_change(change::PendingChange {
        session: conn.session.clone(),
        before,
        after,
        expires,
    });
    reply(
        id,
        Event::Preview(Preview {
            preview_id,
            stack: String::new(),
            kind: OperationKind::Update,
            steps: Vec::new(),
            file_diff: Some(summary),
            reaches: Reach::default(),
            destructive: false,
            stack_hash: String::new(),
            expires_at: expires.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            warnings: Vec::new(),
            ack_required: false,
            // Spec 054: a settings change always needs a fresh passkey check.
            step_up: crate::step_up::preview_request(
                &conn.app,
                conn.caller(access.role),
                CommandKind::ManageUpdates,
                false,
            ),
        }),
    )
}

/// Whether `Confirm{preview_id}` is a settings change of this socket's session.
pub fn owns(conn: &Conn, preview_id: PreviewId) -> bool {
    conn.app
        .updates
        .as_ref()
        .is_some_and(|u| u.has_change(&conn.session, preview_id))
}

/// `Confirm` of a settings change: applied, audited `update_settings` with before and after, the
/// scheduler woken, and `UpdatesChanged`.
///
/// Spec 054: `ManageUpdates` always steps up. The check runs after the role and network gate and
/// before the change is claimed, so a refused check changes nothing.
pub async fn confirm(
    conn: &Conn,
    access: &Access,
    id: Option<RequestId>,
    preview_id: PreviewId,
    assertion: Option<&PasskeyAssertion>,
) -> EventEnvelope {
    let Some(u) = conn.app.updates.clone() else {
        return off(conn, id);
    };
    // Authorised when previewed; the role or the network may have changed since.
    if let Some(refused) = gate(conn, access, id, CommandKind::ManageUpdates, "Confirm") {
        return refused;
    }
    if let Some(refused) = conn
        .step_up(
            id,
            access.role,
            CommandKind::ManageUpdates,
            false,
            assertion,
            "Confirm",
        )
        .await
    {
        return refused;
    }
    let pending = match u.take_change(&conn.session, preview_id) {
        Ok(c) => c,
        Err(reason) => return reply(id, Event::PreviewVoid { reason }),
    };
    let before = u.store().settings();
    let at = u
        .clock()
        .now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let actor = conn.actor.clone();
    let result = u.store().update_decisions(|d| {
        let mut s = d.settings.settings();
        s.checks_enabled = pending.after.checks_enabled;
        s.interval_secs = pending.after.interval_secs;
        d.settings.set(s, actor, at);
    });
    let outcome = match &result {
        Ok(_) => AuditOutcome::Succeeded,
        Err(e) => AuditOutcome::Failed {
            exit_code: None,
            stderr_tail: e.to_string(),
        },
    };
    let mut entry = audit::entry(
        conn.actor.clone(),
        AuditAction::UpdateSettings,
        conn.ip,
        outcome,
    );
    entry.preview_id = Some(preview_id);
    let after = result
        .as_ref()
        .map_or_else(|_| before.clone(), |d| d.settings.settings());
    entry.change = Some(AuditChange {
        before: serde_json::to_value(&before).unwrap_or_default(),
        after: serde_json::to_value(&after).unwrap_or_default(),
    });
    let _ = conn.app.audit.record(&entry);
    match result {
        Ok(_) => {
            u.wake();
            u.notify_changed();
            reply(id, Event::UpdatesChanged(view::updates_view(&u)))
        }
        Err(e) => invalid(id, format!("the settings were not saved: {e}")),
    }
}
