//! Approving and denying a run held by an approval policy (spec 064, research R4).
//!
//! The rules are pure: the server reads the facts (the run's state and held terms, the caller,
//! who started the run, whether the caller already answered it) and asks these functions what
//! to do. A held run keeps the terms it was held with ([`HeldTerms`]), never the live policy's
//! (FR-009).

use uuid::Uuid;

use crate::run::{RunEvent, RunState};

/// The approval terms copied onto a run when it starts waiting for approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeldTerms {
    /// Distinct approvals the run needs before it is queued; at least 1.
    pub needed: u16,
    /// The person who started the run may not approve it.
    pub four_eyes: bool,
    /// When the approval window closes, in Unix seconds.
    pub expires_at: i64,
}

/// Why an approval or a denial is refused. Each is an `OpError` code with a message for people.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The run is no longer waiting for approval.
    NotPending(RunState),
    /// The approval window has closed.
    Expired,
    /// Four-eyes: the caller started the run.
    FourEyesRequester,
    /// The caller already approved or denied the run.
    AlreadyAnswered,
}

impl Refusal {
    /// The stable `OpError` code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Refusal::NotPending(_) => "run_answered",
            Refusal::Expired => "approval_expired",
            Refusal::FourEyesRequester => "four_eyes_requester",
            Refusal::AlreadyAnswered => "already_answered",
        }
    }

    /// What the person is told.
    #[must_use]
    pub fn message(self) -> String {
        match self {
            Refusal::NotPending(state) => {
                format!("This run has already been answered; it is now {state}.")
            }
            Refusal::Expired => "This run's approval window has closed.".to_owned(),
            Refusal::FourEyesRequester => {
                "You started this run, so someone else must approve it.".to_owned()
            }
            Refusal::AlreadyAnswered => "You have already answered this run.".to_owned(),
        }
    }
}

fn check_open(state: RunState, terms: &HeldTerms, now: i64) -> Result<(), Refusal> {
    if state != RunState::PendingApproval {
        return Err(Refusal::NotPending(state));
    }
    if now >= terms.expires_at {
        return Err(Refusal::Expired);
    }
    Ok(())
}

/// May `actor` approve a run in `state` held under `terms`, at `now` (Unix seconds)?
/// `requester` is who started the run (`None` for a push), and `already_answered` whether the
/// actor already approved or denied it.
///
/// # Errors
///
/// The first rule that refuses, in the order: not pending, expired, four-eyes, already answered.
pub fn check_approve(
    state: RunState,
    terms: &HeldTerms,
    now: i64,
    actor: Uuid,
    requester: Option<Uuid>,
    already_answered: bool,
) -> Result<(), Refusal> {
    check_open(state, terms, now)?;
    if terms.four_eyes && requester == Some(actor) {
        return Err(Refusal::FourEyesRequester);
    }
    if already_answered {
        return Err(Refusal::AlreadyAnswered);
    }
    Ok(())
}

/// May someone deny a run in `state` held under `terms`, at `now`? Four-eyes guards approving,
/// not withdrawing, so the person who started the run may deny it.
///
/// # Errors
///
/// The first rule that refuses: not pending, expired, already answered.
pub fn check_deny(
    state: RunState,
    terms: &HeldTerms,
    now: i64,
    already_answered: bool,
) -> Result<(), Refusal> {
    check_open(state, terms, now)?;
    if already_answered {
        return Err(Refusal::AlreadyAnswered);
    }
    Ok(())
}

/// What a run's `approvals` distinct approvals do: `Approved` once they reach `needed`.
#[must_use]
pub fn after_approve(terms: &HeldTerms, approvals: u32) -> Option<RunEvent> {
    (approvals >= u32::from(terms.needed.max(1))).then_some(RunEvent::Approved)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    fn terms(needed: u16, four_eyes: bool) -> HeldTerms {
        HeldTerms {
            needed,
            four_eyes,
            expires_at: NOW + 60,
        }
    }

    fn user(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    const ALL: [RunState; 10] = [
        RunState::NeedsReview,
        RunState::PendingApproval,
        RunState::Queued,
        RunState::Running,
        RunState::Succeeded,
        RunState::Failed,
        RunState::Refused,
        RunState::Cancelled,
        RunState::Denied,
        RunState::Expired,
    ];

    #[test]
    fn approval_refuses_every_state_but_pending_approval() {
        for s in ALL {
            let r = check_approve(s, &terms(1, false), NOW, user(1), None, false);
            if s == RunState::PendingApproval {
                assert_eq!(r, Ok(()));
            } else {
                assert_eq!(r, Err(Refusal::NotPending(s)), "{s}");
            }
            let d = check_deny(s, &terms(1, false), NOW, false);
            if s == RunState::PendingApproval {
                assert_eq!(d, Ok(()));
            } else {
                assert_eq!(d, Err(Refusal::NotPending(s)), "{s}");
            }
        }
    }

    #[test]
    fn approval_refuses_once_the_window_closes() {
        let t = terms(1, false);
        let s = RunState::PendingApproval;
        assert_eq!(
            check_approve(s, &t, t.expires_at - 1, user(1), None, false),
            Ok(())
        );
        assert_eq!(
            check_approve(s, &t, t.expires_at, user(1), None, false),
            Err(Refusal::Expired)
        );
        assert_eq!(
            check_deny(s, &t, t.expires_at + 5, false),
            Err(Refusal::Expired)
        );
    }

    #[test]
    fn four_eyes_refuses_only_the_requester_and_only_when_set() {
        let s = RunState::PendingApproval;
        assert_eq!(
            check_approve(s, &terms(1, true), NOW, user(1), Some(user(1)), false),
            Err(Refusal::FourEyesRequester)
        );
        assert_eq!(
            check_approve(s, &terms(1, true), NOW, user(2), Some(user(1)), false),
            Ok(())
        );
        assert_eq!(
            check_approve(s, &terms(1, false), NOW, user(1), Some(user(1)), false),
            Ok(())
        );
        // A push run has no requester to exclude.
        assert_eq!(
            check_approve(s, &terms(1, true), NOW, user(1), None, false),
            Ok(())
        );
        // The requester may still deny.
        assert_eq!(check_deny(s, &terms(1, true), NOW, false), Ok(()));
    }

    #[test]
    fn a_second_answer_is_refused() {
        let s = RunState::PendingApproval;
        assert_eq!(
            check_approve(s, &terms(2, false), NOW, user(1), None, true),
            Err(Refusal::AlreadyAnswered)
        );
        assert_eq!(
            check_deny(s, &terms(2, false), NOW, true),
            Err(Refusal::AlreadyAnswered)
        );
    }

    #[test]
    fn approved_exactly_when_the_count_reaches_needed() {
        for needed in 1..=3u16 {
            for approvals in 0..=4u32 {
                let ev = after_approve(&terms(needed, false), approvals);
                if approvals >= u32::from(needed) {
                    assert_eq!(ev, Some(RunEvent::Approved));
                } else {
                    assert_eq!(ev, None);
                }
            }
        }
    }

    #[test]
    fn codes_and_messages_match_the_data_model() {
        assert_eq!(Refusal::NotPending(RunState::Queued).code(), "run_answered");
        assert_eq!(
            Refusal::NotPending(RunState::Queued).message(),
            "This run has already been answered; it is now queued."
        );
        assert_eq!(Refusal::Expired.code(), "approval_expired");
        assert_eq!(
            Refusal::Expired.message(),
            "This run's approval window has closed."
        );
        assert_eq!(Refusal::FourEyesRequester.code(), "four_eyes_requester");
        assert_eq!(
            Refusal::FourEyesRequester.message(),
            "You started this run, so someone else must approve it."
        );
        assert_eq!(Refusal::AlreadyAnswered.code(), "already_answered");
        assert_eq!(
            Refusal::AlreadyAnswered.message(),
            "You have already answered this run."
        );
    }
}
