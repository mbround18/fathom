//! The passkey gates (spec 054 US4 and US5).
//!
//! - **Verification** ([`verified`], T063): an admin or owner whose session has not proved it is
//!   them gets only the restricted set ([`depths_core::role::restricted_ok`], the same set an
//!   untrusted network gets), and one with no passkey is told to enrol. Called right after the
//!   052 trust gate.
//! - **Step-up** ([`step_up`], T071): the risky actions of
//!   [`depths_core::role::step_up_needed`] need a check within `DEPTHS_STEP_UP_WINDOW`. A stale
//!   session answers with the command's `assertion`, or gets `StepUpRequired { request }` and
//!   nothing changes. Called after role and stack access, and before anything is claimed or run.
//!
//! The order for one command: authenticate, role and stack (the existing 050 checks), trust
//! (052), verification, step-up, handler.

use std::net::IpAddr;

use chrono::{DateTime, Utc};
use depths_core::role::{CommandKind, restricted_ok, step_up_needed};
use depths_protocol::{
    Actor, AuditAction, AuditOutcome, AuditPasskey, DenyReason, Event, PasskeyAssertion,
    PasskeyRefusedReason, PasskeyRequest, Role, Verification,
};

use crate::http::AppState;
use crate::passkeys::BeginError;
use crate::trust::rfc3339;

/// Whose command it is.
#[derive(Debug, Clone, Copy)]
pub struct Caller<'a> {
    /// The users-registry id.
    pub user: &'a str,
    /// The session handle.
    pub session: &'a str,
    pub actor: &'a Actor,
    pub ip: IpAddr,
    pub role: Role,
}

/// Where `caller`'s session stands on passkeys.
pub fn verification(app: &AppState, caller: Caller<'_>) -> Verification {
    let (at, recovering) = app.trust.sessions.verification(caller.session);
    Verification {
        verified: at.is_some(),
        at: at.map(rfc3339),
        window_secs: u32::try_from(app.passkeys.window().num_seconds()).unwrap_or(u32::MAX),
        enrolled: app.passkeys.enrolled(caller.user),
        required: caller.role == Role::Admin,
        recovering,
    }
}

/// Why the verification gate refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unverified {
    /// No passkey yet: enrol first.
    Enrol,
    /// A passkey, but this session has not checked it.
    Verify,
}

impl Unverified {
    /// The socket reply.
    pub fn event(self) -> Event {
        match self {
            Unverified::Enrol => Event::PasskeyEnrolmentRequired,
            Unverified::Verify => Event::Denied {
                reason: DenyReason::NotPermitted,
                detail: Some("Confirm it's you with a passkey first.".into()),
            },
        }
    }

    /// The HTTP 403 body's `error`.
    pub fn code(self) -> &'static str {
        match self {
            Unverified::Enrol => "passkey_enrolment_required",
            Unverified::Verify => "passkey_verification_required",
        }
    }
}

/// T063: an admin may use more than the restricted set only once this session has verified
/// with a passkey. Everyone else, and the restricted set, pass.
pub fn verified(
    app: &AppState,
    user: &str,
    session: &str,
    role: Role,
    kind: CommandKind,
) -> Result<(), Unverified> {
    if role != Role::Admin || restricted_ok(kind) {
        return Ok(());
    }
    if app.trust.sessions.verification(session).0.is_some() {
        return Ok(());
    }
    if app.passkeys.enrolled(user) {
        Err(Unverified::Verify)
    } else {
        Err(Unverified::Enrol)
    }
}

/// The session checked within the window.
pub fn fresh(app: &AppState, session: &str) -> bool {
    app.passkeys
        .fresh(app.trust.sessions.verification(session).0)
}

fn write(app: &AppState, e: &depths_protocol::AuditEntry) {
    if let Err(err) = app.audit.record(e) {
        tracing::error!(error = %err, "audit write failed");
    }
}

/// Audits a passkey event for `caller`.
pub fn audit(
    app: &AppState,
    caller: Caller<'_>,
    action: AuditAction,
    outcome: AuditOutcome,
    detail: AuditPasskey,
) {
    write(
        app,
        &crate::audit::passkey(caller.actor.clone(), action, caller.ip, outcome, detail),
    );
}

/// Audits a refused passkey answer (`passkey_failed`) for `cmd`.
pub fn audit_failed(app: &AppState, caller: Caller<'_>, cmd: &str, reason: PasskeyRefusedReason) {
    audit(
        app,
        caller,
        AuditAction::PasskeyFailed,
        AuditOutcome::Denied {
            reason: format!("{cmd}: passkey refused"),
        },
        AuditPasskey {
            reason: Some(reason),
            cmd: Some(cmd.chars().take(64).collect()),
            ..AuditPasskey::default()
        },
    );
}

/// Marks the session verified now and audits `passkey_verified` for `cmd`.
pub fn mark_verified(app: &AppState, caller: Caller<'_>, cmd: &str, name: String) {
    app.trust
        .sessions
        .set_verified(caller.session, app.passkeys.now());
    audit(
        app,
        caller,
        AuditAction::PasskeyVerified,
        AuditOutcome::Succeeded,
        AuditPasskey {
            name: Some(name),
            cmd: Some(cmd.chars().take(64).collect()),
            ..AuditPasskey::default()
        },
    );
}

/// The reply when passkeys cannot run at all: fail closed.
pub fn unavailable() -> Event {
    Event::Denied {
        reason: DenyReason::FeatureDisabled,
        detail: Some("Passkeys need an https public URL (DEPTHS_PUBLIC_URL).".into()),
    }
}

fn internal() -> Event {
    Event::Invalid {
        reason: "Depths could not complete that. Try again.".into(),
    }
}

/// A verify ceremony to put on a `Preview` (`step_up`) when confirming it will need a check:
/// the action needs step-up, the session is stale and the person has a passkey. `None`
/// otherwise; a person with no passkey learns on `Confirm` (`PasskeyEnrolmentRequired`).
pub fn preview_request(
    app: &AppState,
    caller: Caller<'_>,
    kind: CommandKind,
    escape_findings: bool,
) -> Option<PasskeyRequest> {
    if !step_up_needed(kind, escape_findings) || fresh(app, caller.session) {
        return None;
    }
    app.passkeys.begin_verify(caller.session, caller.user).ok()
}

/// T071: the step-up gate. `Ok` when `kind` (with `escape_findings`) needs no check, the
/// session checked within the window, or `assertion` verifies (the session is then fresh
/// again). Otherwise the reply to send, and nothing may change. Audits `step_up_required`,
/// `passkey_verified` and `passkey_failed`.
pub async fn step_up(
    app: &AppState,
    caller: Caller<'_>,
    kind: CommandKind,
    escape_findings: bool,
    assertion: Option<&PasskeyAssertion>,
    cmd: &str,
) -> Result<(), Event> {
    if !step_up_needed(kind, escape_findings) || fresh(app, caller.session) {
        return Ok(());
    }
    if !app.passkeys.enrolled(caller.user) {
        audit(
            app,
            caller,
            AuditAction::StepUpRequired,
            AuditOutcome::Denied {
                reason: format!("{cmd}: no passkey enrolled"),
            },
            AuditPasskey {
                cmd: Some(cmd.chars().take(64).collect()),
                ..AuditPasskey::default()
            },
        );
        return Err(Event::PasskeyEnrolmentRequired);
    }
    if let Some(a) = assertion {
        return match app
            .passkeys
            .verify_assertion(caller.session, caller.user, a)
            .await
        {
            Ok(name) => {
                mark_verified(app, caller, cmd, name);
                Ok(())
            }
            Err(f) => {
                let reason = f.reason();
                audit_failed(app, caller, cmd, reason);
                Err(Event::PasskeyRefused { reason })
            }
        };
    }
    let request = match app.passkeys.begin_verify(caller.session, caller.user) {
        Ok(r) => r,
        Err(BeginError::Unavailable) => return Err(unavailable()),
        Err(BeginError::NotEnrolled) => return Err(Event::PasskeyEnrolmentRequired),
        Err(BeginError::StepUpFirst | BeginError::Internal) => return Err(internal()),
    };
    audit(
        app,
        caller,
        AuditAction::StepUpRequired,
        AuditOutcome::Denied {
            reason: format!("{cmd}: a fresh passkey check is needed"),
        },
        AuditPasskey {
            cmd: Some(cmd.chars().take(64).collect()),
            ..AuditPasskey::default()
        },
    );
    Err(Event::StepUpRequired { request })
}

/// For HTTP routes, which carry no assertion: `Some(code)` for a 403 when `kind` needs a check
/// the session does not have (`step_up_required`, or `passkey_enrolment_required`). The client
/// verifies with `PasskeyBegin { verify }` / `PasskeyFinish` and retries.
pub fn http_step_up(app: &AppState, caller: Caller<'_>, kind: CommandKind) -> Option<&'static str> {
    if !step_up_needed(kind, false) || fresh(app, caller.session) {
        return None;
    }
    let code = if app.passkeys.enrolled(caller.user) {
        "step_up_required"
    } else {
        "passkey_enrolment_required"
    };
    audit(
        app,
        caller,
        AuditAction::StepUpRequired,
        AuditOutcome::Denied {
            reason: format!("{kind:?}: a fresh passkey check is needed"),
        },
        AuditPasskey {
            cmd: Some(format!("{kind:?}")),
            ..AuditPasskey::default()
        },
    );
    Some(code)
}

/// `verified_at` as a time, for tests and the sessions page.
pub fn verified_at(app: &AppState, session: &str) -> Option<DateTime<Utc>> {
    app.trust.sessions.verification(session).0
}
