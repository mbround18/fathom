//! Suggested/Executed action state machine (T016). Pure functions only — no I/O, no
//! kube client, no cluster-mutating command construction anywhere in this module. See
//! `specs/013-agent-sdk-ops/data-model.md`'s state diagram:
//!
//! ```text
//! Proposed --review()--> Reviewed --supersede()--> Superseded
//!    |                       |
//!    +--decline()--> Declined <---------------------+ (also reachable from Reviewed)
//! ```
//!
//! Invariant 4 (data-model.md): an `ExecutedAction` can only ever be constructed from a
//! `SuggestedAction` whose status is `Reviewed`. `build_executed_action` is the single
//! place that enforces this — it is the structural backing for "confirming a suggestion
//! recomputes the diff fresh and never bypasses the existing confirmation path"
//! (Principle I): this module never itself performs or triggers that confirmation, it
//! only records that the *frontend* already did, via the same command dispatch a
//! manual click would use.

use crate::protocol::v1::{ActionOutcome, ExecutedAction, SuggestedAction, SuggestedActionStatus};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StateError {
    Terminal {
        id: String,
        status: SuggestedActionStatus,
    },
    NotReviewed {
        id: String,
        status: SuggestedActionStatus,
    },
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::Terminal { id, status } => write!(
                f,
                "suggested action {id} is {status:?}, a terminal state; no further transition is possible"
            ),
            StateError::NotReviewed { id, status } => write!(
                f,
                "suggested action {id} must be Reviewed before an ExecutedAction can be recorded for it, but it is {status:?}"
            ),
        }
    }
}

impl std::error::Error for StateError {}

/// `Proposed -> Reviewed`. A no-op (returns `Ok(())` without changing anything) when
/// already `Reviewed`, since re-opening the confirmation dialog on an already-reviewed
/// suggestion is a normal, harmless UI action (each open recomputes the diff fresh
/// client-side regardless of this status). Errors on `Declined`/`Superseded` (terminal).
pub(crate) fn review(suggestion: &mut SuggestedAction) -> Result<(), StateError> {
    match suggestion.status {
        SuggestedActionStatus::Proposed => {
            suggestion.status = SuggestedActionStatus::Reviewed;
            Ok(())
        }
        SuggestedActionStatus::Reviewed => Ok(()),
        terminal @ (SuggestedActionStatus::Declined | SuggestedActionStatus::Superseded) => {
            Err(StateError::Terminal {
                id: suggestion.id.clone(),
                status: terminal,
            })
        }
    }
}

/// `Proposed|Reviewed -> Declined`. Terminal once reached; erroring (not silently
/// succeeding) when called again from `Declined`/`Superseded` (US3 AS4: no silent
/// no-op that could be mistaken for a fresh decline).
pub(crate) fn decline(suggestion: &mut SuggestedAction) -> Result<(), StateError> {
    match suggestion.status {
        SuggestedActionStatus::Proposed | SuggestedActionStatus::Reviewed => {
            suggestion.status = SuggestedActionStatus::Declined;
            Ok(())
        }
        terminal @ (SuggestedActionStatus::Declined | SuggestedActionStatus::Superseded) => {
            Err(StateError::Terminal {
                id: suggestion.id.clone(),
                status: terminal,
            })
        }
    }
}

/// `Proposed|Reviewed -> Superseded`, used when a newer suggestion for the same target
/// replaces this one. Terminal once reached.
///
/// Not yet called from `src/backend/cmd/agent.rs`: wiring `AgentAsk` to supersede a
/// target's outstanding suggestions when it proposes a new one for the same target is
/// Phase 5 (US3) work, once tool-call parsing (`TurnResult::suggested_actions`) is
/// implemented. Kept here now, fully tested, so Phase 5 only has to wire the call.
#[allow(dead_code)]
pub(crate) fn supersede(suggestion: &mut SuggestedAction) -> Result<(), StateError> {
    match suggestion.status {
        SuggestedActionStatus::Proposed | SuggestedActionStatus::Reviewed => {
            suggestion.status = SuggestedActionStatus::Superseded;
            Ok(())
        }
        terminal @ (SuggestedActionStatus::Declined | SuggestedActionStatus::Superseded) => {
            Err(StateError::Terminal {
                id: suggestion.id.clone(),
                status: terminal,
            })
        }
    }
}

/// The only constructor for `ExecutedAction`. Requires `suggestion.status ==
/// Reviewed`; any other status is an error, never a silently-accepted no-op — this is
/// what makes it structurally impossible to record an executed action for a suggestion
/// the operator never reviewed (data-model.md invariant 4).
pub(crate) fn build_executed_action(
    id: String,
    suggestion: &SuggestedAction,
    command_summary: String,
    outcome: ActionOutcome,
    executed_at: String,
) -> Result<ExecutedAction, StateError> {
    if suggestion.status != SuggestedActionStatus::Reviewed {
        return Err(StateError::NotReviewed {
            id: suggestion.id.clone(),
            status: suggestion.status,
        });
    }
    Ok(ExecutedAction {
        id,
        suggested_action_id: suggestion.id.clone(),
        command_summary,
        outcome,
        executed_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::v1::AgentObjectRef;

    fn fixture(status: SuggestedActionStatus) -> SuggestedAction {
        SuggestedAction {
            id: "sugg-1".to_owned(),
            action_kind: "restart".to_owned(),
            target: AgentObjectRef {
                kind: "Pod".to_owned(),
                namespace: "default".to_owned(),
                name: "api-0".to_owned(),
            },
            params: serde_json::json!({}),
            rationale: "Container is crash-looping.".to_owned(),
            status,
        }
    }

    #[test]
    fn review_from_proposed_transitions_to_reviewed() {
        let mut s = fixture(SuggestedActionStatus::Proposed);
        review(&mut s).expect("review succeeds");
        assert_eq!(s.status, SuggestedActionStatus::Reviewed);
    }

    #[test]
    fn review_is_idempotent_from_reviewed() {
        let mut s = fixture(SuggestedActionStatus::Reviewed);
        review(&mut s).expect("review succeeds");
        assert_eq!(s.status, SuggestedActionStatus::Reviewed);
    }

    #[test]
    fn review_from_declined_errors() {
        let mut s = fixture(SuggestedActionStatus::Declined);
        let err = review(&mut s).expect_err("terminal state rejects review");
        assert!(matches!(err, StateError::Terminal { .. }));
        // Status must not have changed.
        assert_eq!(s.status, SuggestedActionStatus::Declined);
    }

    #[test]
    fn review_from_superseded_errors() {
        let mut s = fixture(SuggestedActionStatus::Superseded);
        let err = review(&mut s).expect_err("terminal state rejects review");
        assert!(matches!(err, StateError::Terminal { .. }));
        assert_eq!(s.status, SuggestedActionStatus::Superseded);
    }

    #[test]
    fn decline_from_proposed_and_reviewed_succeeds() {
        let mut s = fixture(SuggestedActionStatus::Proposed);
        decline(&mut s).expect("decline succeeds");
        assert_eq!(s.status, SuggestedActionStatus::Declined);

        let mut s = fixture(SuggestedActionStatus::Reviewed);
        decline(&mut s).expect("decline succeeds");
        assert_eq!(s.status, SuggestedActionStatus::Declined);
    }

    #[test]
    fn decline_is_terminal_no_double_decline() {
        let mut s = fixture(SuggestedActionStatus::Declined);
        let err = decline(&mut s).expect_err("already declined");
        assert!(matches!(err, StateError::Terminal { .. }));
    }

    #[test]
    fn decline_from_superseded_errors() {
        let mut s = fixture(SuggestedActionStatus::Superseded);
        let err = decline(&mut s).expect_err("superseded is terminal");
        assert!(matches!(err, StateError::Terminal { .. }));
    }

    #[test]
    fn supersede_from_proposed_and_reviewed_succeeds() {
        let mut s = fixture(SuggestedActionStatus::Proposed);
        supersede(&mut s).expect("supersede succeeds");
        assert_eq!(s.status, SuggestedActionStatus::Superseded);

        let mut s = fixture(SuggestedActionStatus::Reviewed);
        supersede(&mut s).expect("supersede succeeds");
        assert_eq!(s.status, SuggestedActionStatus::Superseded);
    }

    #[test]
    fn supersede_is_terminal() {
        let mut s = fixture(SuggestedActionStatus::Superseded);
        let err = supersede(&mut s).expect_err("already terminal");
        assert!(matches!(err, StateError::Terminal { .. }));

        let mut s = fixture(SuggestedActionStatus::Declined);
        let err = supersede(&mut s).expect_err("already terminal");
        assert!(matches!(err, StateError::Terminal { .. }));
    }

    /// The safety-critical property (Principle I / data-model.md invariant 4): an
    /// `ExecutedAction` can only ever be built from a `Reviewed` suggestion.
    #[test]
    fn build_executed_action_requires_reviewed_status() {
        for status in [
            SuggestedActionStatus::Proposed,
            SuggestedActionStatus::Declined,
            SuggestedActionStatus::Superseded,
        ] {
            let s = fixture(status);
            let err = build_executed_action(
                "exec-1".to_owned(),
                &s,
                "RestartDeployment default/api".to_owned(),
                ActionOutcome::Success,
                "2026-09-22T00:00:00Z".to_owned(),
            )
            .expect_err("non-Reviewed status must be rejected");
            assert!(matches!(err, StateError::NotReviewed { .. }));
        }
    }

    #[test]
    fn build_executed_action_succeeds_from_reviewed() {
        let s = fixture(SuggestedActionStatus::Reviewed);
        let executed = build_executed_action(
            "exec-1".to_owned(),
            &s,
            "RestartDeployment default/api".to_owned(),
            ActionOutcome::Success,
            "2026-09-22T00:00:00Z".to_owned(),
        )
        .expect("Reviewed status is accepted");
        assert_eq!(executed.suggested_action_id, "sugg-1");
        assert_eq!(executed.outcome, ActionOutcome::Success);
    }

    #[test]
    fn build_executed_action_preserves_verbatim_error_outcome() {
        let s = fixture(SuggestedActionStatus::Reviewed);
        let executed = build_executed_action(
            "exec-2".to_owned(),
            &s,
            "DeletePod default/api-0".to_owned(),
            ActionOutcome::Error {
                message: "pods \"api-0\" is forbidden: User cannot delete pods".to_owned(),
            },
            "2026-09-22T00:00:00Z".to_owned(),
        )
        .expect("Reviewed status is accepted");
        assert_eq!(
            executed.outcome,
            ActionOutcome::Error {
                message: "pods \"api-0\" is forbidden: User cannot delete pods".to_owned(),
            }
        );
    }
}
