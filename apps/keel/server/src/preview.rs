//! Preview/Confirm (spec 055 T027, contracts/protocol.md): every destructive or privileged
//! command first returns a [`Preview`]; nothing happens until the same session sends `Confirm`.
//!
//! - Previews live in memory, keyed by [`PreviewId`], and are bound to the session that asked.
//! - A preview is single-use: a successful confirm removes it.
//! - It expires [`TTL`] (5 minutes) after it was made.
//! - A `Confirm` from another session is refused as `preview_unknown`, so it learns nothing, and
//!   the preview stays usable by its owner.
//! - When `step_up` is set, the session must have verified a passkey within the step-up window,
//!   or the `Confirm` must carry an assertion that the [`StepUpVerifier`] accepts. A missing or
//!   failed step-up does not consume the preview.
//!
//! The pending operation `T` is opaque here; the dispatcher stores what it will run.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use keel_core::ports::Clock;
use keel_protocol::{PasskeyAssertion, Preview, PreviewId};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// How long a preview stays confirmable.
pub const TTL: Duration = Duration::minutes(5);

/// The most previews one session may hold; making another drops its oldest.
pub const MAX_PER_SESSION: usize = 32;

/// What to show for a new preview.
#[derive(Debug, Clone)]
pub struct Draft {
    pub action: String,
    pub summary: String,
    pub affected: Vec<String>,
    pub step_up: bool,
}

/// Why a `Confirm` was refused. [`ConfirmError::code`] is the `OpError` code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmError {
    /// No such preview for this session (never made, already used, or another session's).
    Unknown,
    /// The preview expired; it is gone.
    Expired,
    /// Step-up is required and the session has not verified recently; send an assertion.
    StepUpRequired,
    /// The assertion did not verify.
    StepUpFailed,
}

impl ConfirmError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            ConfirmError::Unknown => "preview_unknown",
            ConfirmError::Expired => "preview_expired",
            ConfirmError::StepUpRequired => "step_up_required",
            ConfirmError::StepUpFailed => "step_up_failed",
        }
    }

    #[must_use]
    pub const fn message(&self) -> &'static str {
        match self {
            ConfirmError::Unknown => "That preview does not exist or was already used.",
            ConfirmError::Expired => "That preview expired. Start the action again.",
            ConfirmError::StepUpRequired => "Confirm with your passkey to continue.",
            ConfirmError::StepUpFailed => "The passkey check failed.",
        }
    }
}

/// Checks a passkey assertion for a session (T028 supplies the real one).
pub trait StepUpVerifier: Send + Sync {
    /// `Ok(())` only when the assertion is valid for this session's user.
    fn verify<'a>(
        &'a self,
        session: &'a str,
        assertion: &'a PasskeyAssertion,
    ) -> Pin<Box<dyn Future<Output = Result<(), ()>> + Send + 'a>>;
}

/// Refuses every assertion: step-up fails closed until passkeys are wired.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoStepUp;

impl StepUpVerifier for NoStepUp {
    fn verify<'a>(
        &'a self,
        _session: &'a str,
        _assertion: &'a PasskeyAssertion,
    ) -> Pin<Box<dyn Future<Output = Result<(), ()>> + Send + 'a>> {
        Box::pin(async { Err(()) })
    }
}

struct Pending<T> {
    session: String,
    created: OffsetDateTime,
    /// Creation order, for evicting a session's oldest preview.
    order: u64,
    step_up: bool,
    op: T,
}

/// The confirmation context a `Confirm` arrives with.
#[derive(Clone, Copy)]
pub struct Confirmation<'a> {
    /// The session's id (web-auth's hashed id).
    pub session: &'a str,
    pub preview_id: PreviewId,
    pub step_up: Option<&'a PasskeyAssertion>,
    /// When this session last verified a passkey.
    pub verified_at: Option<OffsetDateTime>,
    /// `--step-up-window`.
    pub window: Duration,
}

/// The in-memory preview store.
pub struct Previews<T> {
    clock: Arc<dyn Clock>,
    next: AtomicU64,
    pending: Mutex<HashMap<PreviewId, Pending<T>>>,
}

impl<T> Previews<T> {
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            next: AtomicU64::new(0),
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PreviewId, Pending<T>>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stores `op` for `session` and returns the preview to send.
    pub fn create(&self, session: &str, draft: Draft, op: T) -> Preview {
        let now = self.clock.now();
        let id = PreviewId(Uuid::now_v7());
        let mut map = self.lock();
        map.retain(|_, p| now - p.created < TTL);
        let mut mine: Vec<(PreviewId, u64)> = map
            .iter()
            .filter(|(_, p)| p.session == session)
            .map(|(k, p)| (*k, p.order))
            .collect();
        if mine.len() >= MAX_PER_SESSION {
            mine.sort_by_key(|(_, order)| *order);
            for (old, _) in mine.iter().take(mine.len() + 1 - MAX_PER_SESSION) {
                map.remove(old);
            }
        }
        map.insert(
            id,
            Pending {
                session: session.to_owned(),
                created: now,
                order: self.next.fetch_add(1, Ordering::Relaxed),
                step_up: draft.step_up,
                op,
            },
        );
        Preview {
            preview_id: id,
            action: draft.action,
            summary: draft.summary,
            affected: draft.affected,
            step_up: draft.step_up,
            expires_at: (now + TTL).format(&Rfc3339).unwrap_or_default(),
        }
    }

    /// Consumes the preview and returns its operation, or says why not.
    pub async fn confirm(
        &self,
        c: Confirmation<'_>,
        verifier: &dyn StepUpVerifier,
    ) -> Result<T, ConfirmError> {
        let step_up = self.check(&c)?;
        if step_up {
            let now = self.clock.now();
            let recent = c
                .verified_at
                .is_some_and(|at| at <= now && now - at <= c.window);
            if !recent {
                let Some(assertion) = c.step_up else {
                    return Err(ConfirmError::StepUpRequired);
                };
                verifier
                    .verify(c.session, assertion)
                    .await
                    .map_err(|()| ConfirmError::StepUpFailed)?;
            }
        }
        // Re-check after the await: another confirm may have won, or the TTL may have passed.
        self.check(&c)?;
        self.lock()
            .remove(&c.preview_id)
            .map(|p| p.op)
            .ok_or(ConfirmError::Unknown)
    }

    /// Whether the preview exists, is this session's and is live; returns its step-up flag.
    fn check(&self, c: &Confirmation<'_>) -> Result<bool, ConfirmError> {
        let now = self.clock.now();
        let mut map = self.lock();
        let Some(p) = map.get(&c.preview_id) else {
            return Err(ConfirmError::Unknown);
        };
        if p.session != c.session {
            return Err(ConfirmError::Unknown);
        }
        if now - p.created >= TTL {
            map.remove(&c.preview_id);
            return Err(ConfirmError::Expired);
        }
        Ok(p.step_up)
    }

    /// Drops every preview of `session` (sign-out, revocation).
    pub fn forget_session(&self, session: &str) {
        self.lock().retain(|_, p| p.session != session);
    }

    /// Previews held (tests and metrics).
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
