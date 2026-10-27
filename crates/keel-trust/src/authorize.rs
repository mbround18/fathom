//! The executor's decision for one signed-mode run, checks in a fixed order:
//!
//! 1. `ledger_corrupt`: the ledger chain does not verify.
//! 2. `replay`: the run already has a start line.
//! 3. `base_mismatch`: Keel's assumed base (from `Assign`) is not the host's ledger base.
//! 4. `pack_incomplete`: the target or a commit on the walk is missing or malformed.
//! 5. The commit check. `signed` authorizes. Otherwise, with no approvals: `unsigned`, or
//!    `downgrade` when the target is not a descendant of the base. With approvals: they must
//!    verify ([`crate::override_verify`]) or the run is refused `bad_override`.
//!
//! Whether signed mode applies at all (`policy.require_signed`) is the executor's choice; this
//! function is only called when it does.

use serde::Serialize;

use crate::challenge::ApprovalKind;
use crate::commit_check::{self, CheckError, CommitFailure, OverrideReason, Verdict};
use crate::ledger::{Auth, Ledger, LedgerError};
use crate::object::{CommitId, ObjectStore};
use crate::override_verify::{self, Approval, Approver, OverrideError, OverridePolicy, RunContext};
use crate::signers::Signers;

/// Everything the executor knows about a run when it decides.
pub struct RunRequest<'a> {
    /// The ledger file's text.
    pub ledger: &'a str,
    /// The executor's object store.
    pub store: &'a dyn ObjectStore,
    /// The host's trusted signers.
    pub signers: &'a Signers,
    /// The host's approver passkeys.
    pub approvers: &'a [Approver],
    /// The host's override policy.
    pub policy: &'a OverridePolicy,
    /// The run (workspace, binding, run id, job, quorum).
    pub run: &'a RunContext,
    /// Keel's belief about the base, from `Assign`.
    pub assumed_base: Option<&'a CommitId>,
    /// The commit to run.
    pub target: &'a CommitId,
    /// The `Approvals` items (empty when Keel saw every commit signed).
    pub approvals: &'a [Approval],
    /// Unix seconds.
    pub now: i64,
}

/// The run may start.
pub struct Authorized {
    /// The verified ledger, to append the start line to.
    pub ledger: Ledger,
    /// The host's base for the job.
    pub base: Option<CommitId>,
    /// What to record on the start line.
    pub auth: Auth,
}

/// Why the host refuses the run. [`Refusal::code`] is the `run_hosts.refusal` value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "refusal", rename_all = "snake_case")]
pub enum Refusal {
    /// The ledger chain is broken; the executor refuses all work.
    LedgerCorrupt {
        /// What is broken.
        detail: String,
    },
    /// The run already started on this host.
    Replay,
    /// Keel assumed a different base than the host's ledger has.
    BaseMismatch {
        /// The host's base.
        host_base: Option<CommitId>,
    },
    /// Objects are missing or malformed.
    PackIncomplete {
        /// What is missing.
        detail: String,
    },
    /// Commits are not signed and no approval was given.
    Unsigned {
        /// The failing commits.
        failures: Vec<CommitFailure>,
    },
    /// The target is not a descendant of the base and no approval was given.
    Downgrade,
    /// The approvals do not hold.
    BadOverride {
        /// The first failing approval and step, or a short quorum.
        error: OverrideError,
    },
}

impl Refusal {
    /// The `run_hosts.refusal` value.
    pub fn code(&self) -> &'static str {
        match self {
            Self::LedgerCorrupt { .. } => "ledger_corrupt",
            Self::Replay => "replay",
            Self::BaseMismatch { .. } => "base_mismatch",
            Self::PackIncomplete { .. } => "pack_incomplete",
            Self::Unsigned { .. } => "unsigned",
            Self::Downgrade => "downgrade",
            Self::BadOverride { .. } => "bad_override",
        }
    }
}

/// Decides whether the host may run `req.target`.
pub fn authorize(req: &RunRequest<'_>) -> Result<Authorized, Refusal> {
    let ledger = Ledger::parse(req.ledger).map_err(|e| match e {
        LedgerError::Corrupt { .. } => Refusal::LedgerCorrupt {
            detail: e.to_string(),
        },
        other => Refusal::LedgerCorrupt {
            detail: other.to_string(),
        },
    })?;
    if ledger.has_start(&req.run.run) {
        return Err(Refusal::Replay);
    }
    let base = ledger.base(&req.run.job);
    if base.as_ref() != req.assumed_base {
        return Err(Refusal::BaseMismatch { host_base: base });
    }
    let verdict = commit_check::check(req.store, req.signers, base.as_ref(), req.target).map_err(
        |e| match e {
            CheckError::PackIncomplete { detail } => Refusal::PackIncomplete { detail },
            CheckError::Store(e) => Refusal::PackIncomplete {
                detail: e.to_string(),
            },
        },
    )?;
    let auth = match verdict {
        Verdict::Signed { signers } => Auth::Signed { signers },
        Verdict::NeedsOverride { reason, failures } => {
            if req.approvals.is_empty() {
                return Err(match reason {
                    OverrideReason::Downgrade => Refusal::Downgrade,
                    OverrideReason::Unsigned | OverrideReason::Baseline => {
                        Refusal::Unsigned { failures }
                    }
                });
            }
            let approved = override_verify::verify_override(
                req.approvals,
                req.approvers,
                req.policy,
                req.run,
                base.as_ref(),
                req.target,
                req.now,
            )
            .map_err(|error| Refusal::BadOverride { error })?;
            match approved.kind {
                ApprovalKind::Override => Auth::Override {
                    approvals: approved.credentials,
                    approvers: approved.labels,
                },
                ApprovalKind::Baseline => Auth::Baseline {
                    approvals: approved.credentials,
                    approvers: approved.labels,
                },
            }
        }
    };
    Ok(Authorized { ledger, base, auth })
}
