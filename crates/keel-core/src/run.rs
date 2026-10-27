//! The run state machine (spec 055, T019), exactly as the diagram in data-model.md:
//!
//! ```text
//! needs_review ──override──▶ pending_approval | queued      ──deny / ttl──▶ denied | expired
//! pending_approval ──approve──▶ queued                       ──deny / ttl──▶ denied | expired
//! queued ──dispatch──▶ running ──▶ succeeded | failed | refused
//! queued, running ──cancel──▶ cancelled; needs_review, pending_approval, queued ──binding disabled──▶ cancelled
//! ```
//!
//! A queued run that cannot start (bad inventory, missing artifact, unavailable credential) fails;
//! a running run whose runner is lost fails with `runner_lost`. Terminal states never move.

use serde::{Deserialize, Serialize};

use crate::domain::str_enum;

str_enum!(
    /// A run's state.
    RunState, "run state" {
        NeedsReview => "needs_review",
        PendingApproval => "pending_approval",
        Queued => "queued",
        Running => "running",
        Succeeded => "succeeded",
        Failed => "failed",
        Refused => "refused",
        Cancelled => "cancelled",
        Denied => "denied",
        Expired => "expired",
    }
);

impl RunState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            RunState::Succeeded
                | RunState::Failed
                | RunState::Refused
                | RunState::Cancelled
                | RunState::Denied
                | RunState::Expired
        )
    }
}

str_enum!(
    /// Why a run reached its state.
    StateReason, "state reason" {
        ApprovalDenied => "approval_denied",
        ApprovalExpired => "approval_expired",
        BindingDisabled => "binding_disabled",
        CancelledByUser => "cancelled_by_user",
        RunnerLost => "runner_lost",
        InventoryInvalid => "inventory_invalid",
        ArtifactMissing => "artifact_missing",
        CredentialUnavailable => "credential_unavailable",
        HostRefused => "host_refused",
    }
);

str_enum!(
    /// One host's state within a run (`run_hosts.state`).
    HostState, "host state" {
        Queued => "queued",
        Sent => "sent",
        Running => "running",
        Succeeded => "succeeded",
        Failed => "failed",
        Refused => "refused",
        Lost => "lost",
        Cancelled => "cancelled",
    }
);

impl HostState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !matches!(
            self,
            HostState::Queued | HostState::Sent | HostState::Running
        )
    }
}

/// A run's state and why it got there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStatus {
    pub state: RunState,
    pub reason: Option<StateReason>,
}

impl RunStatus {
    const fn of(state: RunState) -> Self {
        Self {
            state,
            reason: None,
        }
    }

    const fn because(state: RunState, reason: StateReason) -> Self {
        Self {
            state,
            reason: Some(reason),
        }
    }

    /// The state a new run starts in. Signed mode puts a run whose ranges fail the signature check
    /// in `needs_review`; otherwise the policy decides between approval and the queue.
    #[must_use]
    pub const fn initial(needs_review: bool, requires_approval: bool) -> Self {
        if needs_review {
            Self::of(RunState::NeedsReview)
        } else if requires_approval {
            Self::of(RunState::PendingApproval)
        } else {
            Self::of(RunState::Queued)
        }
    }
}

/// Why a queued run could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unrunnable {
    InventoryInvalid,
    ArtifactMissing,
    CredentialUnavailable,
}

impl From<Unrunnable> for StateReason {
    fn from(u: Unrunnable) -> Self {
        match u {
            Unrunnable::InventoryInvalid => StateReason::InventoryInvalid,
            Unrunnable::ArtifactMissing => StateReason::ArtifactMissing,
            Unrunnable::CredentialUnavailable => StateReason::CredentialUnavailable,
        }
    }
}

/// Something that happens to a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEvent {
    /// A review quorum overrode the signature check. `approval_satisfied` is true when the same
    /// people also satisfy the approval policy (or none is required).
    Overridden { approval_satisfied: bool },
    /// The approval policy is satisfied.
    Approved,
    /// A reviewer or approver denied the run.
    Denied,
    /// The approval or review TTL passed.
    Expired,
    /// A runner took the run.
    Dispatched,
    /// A user cancelled the run.
    CancelledByUser,
    /// The run's binding was disabled.
    BindingDisabled,
    /// The run could not start.
    Unrunnable(Unrunnable),
    /// The runner disappeared mid-run.
    RunnerLost,
    /// Every host reached a terminal state.
    HostsFinished(Vec<HostState>),
}

/// A transition the diagram does not allow.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("illegal transition from {from} on {event}")]
pub struct IllegalTransition {
    pub from: RunState,
    pub event: &'static str,
}

impl RunEvent {
    fn name(&self) -> &'static str {
        match self {
            RunEvent::Overridden { .. } => "override",
            RunEvent::Approved => "approve",
            RunEvent::Denied => "deny",
            RunEvent::Expired => "expire",
            RunEvent::Dispatched => "dispatch",
            RunEvent::CancelledByUser => "cancel",
            RunEvent::BindingDisabled => "binding_disabled",
            RunEvent::Unrunnable(_) => "unrunnable",
            RunEvent::RunnerLost => "runner_lost",
            RunEvent::HostsFinished(_) => "hosts_finished",
        }
    }
}

/// Applies `event` to a run in `from`. Everything the diagram does not list is refused.
pub fn transition(from: RunState, event: &RunEvent) -> Result<RunStatus, IllegalTransition> {
    use RunState as S;
    use StateReason as R;
    let next = match (from, event) {
        (
            S::NeedsReview,
            RunEvent::Overridden {
                approval_satisfied: true,
            },
        ) => RunStatus::of(S::Queued),
        (
            S::NeedsReview,
            RunEvent::Overridden {
                approval_satisfied: false,
            },
        ) => RunStatus::of(S::PendingApproval),
        (S::PendingApproval, RunEvent::Approved) => RunStatus::of(S::Queued),
        (S::NeedsReview | S::PendingApproval, RunEvent::Denied) => {
            RunStatus::because(S::Denied, R::ApprovalDenied)
        }
        (S::NeedsReview | S::PendingApproval, RunEvent::Expired) => {
            RunStatus::because(S::Expired, R::ApprovalExpired)
        }
        (S::NeedsReview | S::PendingApproval | S::Queued, RunEvent::BindingDisabled) => {
            RunStatus::because(S::Cancelled, R::BindingDisabled)
        }
        (S::Queued | S::Running, RunEvent::CancelledByUser) => {
            RunStatus::because(S::Cancelled, R::CancelledByUser)
        }
        (S::Queued, RunEvent::Dispatched) => RunStatus::of(S::Running),
        (S::Queued, RunEvent::Unrunnable(u)) => RunStatus::because(S::Failed, (*u).into()),
        (S::Running, RunEvent::RunnerLost) => RunStatus::because(S::Failed, R::RunnerLost),
        (S::Running, RunEvent::HostsFinished(hosts)) => match summarize(hosts) {
            Some(status) => status,
            None => {
                return Err(IllegalTransition {
                    from,
                    event: event.name(),
                });
            }
        },
        _ => {
            return Err(IllegalTransition {
                from,
                event: event.name(),
            });
        }
    };
    Ok(next)
}

/// Summarizes a run's hosts. `None` while any host is still in flight.
///
/// `refused` if any host refused; `succeeded` only if every host succeeded; otherwise `failed`
/// (with `runner_lost` when a host was lost). A run with no hosts fails: there is nothing that
/// could have succeeded.
#[must_use]
pub fn summarize(hosts: &[HostState]) -> Option<RunStatus> {
    if hosts.iter().any(|h| !h.is_terminal()) {
        return None;
    }
    if hosts.contains(&HostState::Refused) {
        return Some(RunStatus::because(
            RunState::Refused,
            StateReason::HostRefused,
        ));
    }
    if !hosts.is_empty() && hosts.iter().all(|h| *h == HostState::Succeeded) {
        return Some(RunStatus::of(RunState::Succeeded));
    }
    let reason = hosts
        .contains(&HostState::Lost)
        .then_some(StateReason::RunnerLost);
    Some(RunStatus {
        state: RunState::Failed,
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use RunState as S;
    use StateReason as R;

    fn events() -> Vec<RunEvent> {
        vec![
            RunEvent::Overridden {
                approval_satisfied: true,
            },
            RunEvent::Overridden {
                approval_satisfied: false,
            },
            RunEvent::Approved,
            RunEvent::Denied,
            RunEvent::Expired,
            RunEvent::Dispatched,
            RunEvent::CancelledByUser,
            RunEvent::BindingDisabled,
            RunEvent::Unrunnable(Unrunnable::InventoryInvalid),
            RunEvent::Unrunnable(Unrunnable::ArtifactMissing),
            RunEvent::Unrunnable(Unrunnable::CredentialUnavailable),
            RunEvent::RunnerLost,
            RunEvent::HostsFinished(vec![HostState::Succeeded]),
        ]
    }

    /// Every allowed (from, event) -> status, from the data-model diagram.
    fn allowed(from: S, ev: &RunEvent) -> Option<RunStatus> {
        let st = |s| RunStatus {
            state: s,
            reason: None,
        };
        let because = |s, r| RunStatus {
            state: s,
            reason: Some(r),
        };
        Some(match (from, ev) {
            (
                S::NeedsReview,
                RunEvent::Overridden {
                    approval_satisfied: true,
                },
            ) => st(S::Queued),
            (
                S::NeedsReview,
                RunEvent::Overridden {
                    approval_satisfied: false,
                },
            ) => st(S::PendingApproval),
            (S::NeedsReview | S::PendingApproval, RunEvent::Denied) => {
                because(S::Denied, R::ApprovalDenied)
            }
            (S::NeedsReview | S::PendingApproval, RunEvent::Expired) => {
                because(S::Expired, R::ApprovalExpired)
            }
            (S::NeedsReview | S::PendingApproval | S::Queued, RunEvent::BindingDisabled) => {
                because(S::Cancelled, R::BindingDisabled)
            }
            (S::PendingApproval, RunEvent::Approved) => st(S::Queued),
            (S::Queued, RunEvent::Dispatched) => st(S::Running),
            (S::Queued | S::Running, RunEvent::CancelledByUser) => {
                because(S::Cancelled, R::CancelledByUser)
            }
            (S::Queued, RunEvent::Unrunnable(Unrunnable::InventoryInvalid)) => {
                because(S::Failed, R::InventoryInvalid)
            }
            (S::Queued, RunEvent::Unrunnable(Unrunnable::ArtifactMissing)) => {
                because(S::Failed, R::ArtifactMissing)
            }
            (S::Queued, RunEvent::Unrunnable(Unrunnable::CredentialUnavailable)) => {
                because(S::Failed, R::CredentialUnavailable)
            }
            (S::Running, RunEvent::RunnerLost) => because(S::Failed, R::RunnerLost),
            (S::Running, RunEvent::HostsFinished(_)) => st(S::Succeeded),
            _ => return None,
        })
    }

    #[test]
    fn every_pair_is_allowed_or_refused_as_the_diagram_says() {
        let mut allowed_count = 0;
        for from in S::ALL {
            for ev in events() {
                let got = transition(*from, &ev);
                match allowed(*from, &ev) {
                    Some(want) => {
                        allowed_count += 1;
                        assert_eq!(got, Ok(want), "{from} {ev:?}");
                    }
                    None => assert!(got.is_err(), "{from} {ev:?} should be refused, got {got:?}"),
                }
            }
        }
        assert_eq!(allowed_count, 18);
    }

    #[test]
    fn terminal_states_never_move() {
        for from in S::ALL.iter().filter(|s| s.is_terminal()) {
            for ev in events() {
                assert!(transition(*from, &ev).is_err(), "{from} {ev:?}");
            }
        }
    }

    #[test]
    fn initial_state() {
        assert_eq!(RunStatus::initial(true, true).state, S::NeedsReview);
        assert_eq!(RunStatus::initial(true, false).state, S::NeedsReview);
        assert_eq!(RunStatus::initial(false, true).state, S::PendingApproval);
        assert_eq!(RunStatus::initial(false, false).state, S::Queued);
    }

    #[test]
    fn summary_of_hosts() {
        use HostState as H;
        let s = |h: &[H]| summarize(h);
        assert_eq!(
            s(&[H::Succeeded, H::Succeeded]).unwrap().state,
            S::Succeeded
        );
        assert_eq!(
            s(&[H::Succeeded, H::Failed]).unwrap(),
            RunStatus {
                state: S::Failed,
                reason: None
            }
        );
        assert_eq!(
            s(&[H::Succeeded, H::Refused, H::Failed]).unwrap(),
            RunStatus {
                state: S::Refused,
                reason: Some(R::HostRefused)
            }
        );
        assert_eq!(
            s(&[H::Succeeded, H::Lost]).unwrap(),
            RunStatus {
                state: S::Failed,
                reason: Some(R::RunnerLost)
            }
        );
        assert_eq!(s(&[H::Succeeded, H::Cancelled]).unwrap().state, S::Failed);
        assert_eq!(s(&[]).unwrap().state, S::Failed, "no hosts fails closed");
        for in_flight in [H::Queued, H::Sent, H::Running] {
            assert_eq!(s(&[H::Succeeded, in_flight]), None);
            assert!(transition(S::Running, &RunEvent::HostsFinished(vec![in_flight])).is_err());
        }
        assert_eq!(
            transition(S::Running, &RunEvent::HostsFinished(vec![H::Refused]))
                .unwrap()
                .state,
            S::Refused
        );
    }

    #[test]
    fn spellings() {
        assert_eq!(S::PendingApproval.as_str(), "pending_approval");
        assert_eq!(
            "credential_unavailable".parse::<R>().unwrap(),
            R::CredentialUnavailable
        );
        assert_eq!(R::ALL.len(), 9);
        assert_eq!(S::ALL.len(), 10);
    }
}
