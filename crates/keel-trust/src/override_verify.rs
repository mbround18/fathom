//! Host-side verification of override and baseline approvals (contract signed-mode.md, T097).
//!
//! For each approval, in order:
//!
//! 1. The kind matches the run (`baseline` exactly when the host has no base), `expires_at`
//!    parses, and the host's `inventory_name` is in the approval's `hosts`. The challenge is
//!    rebuilt from the host's own base, the target it verified and the run it was assigned.
//! 2. `credential_id` is in `approvers.json`.
//! 3. Client data: `type == "webauthn.get"`, the rebuilt challenge, `origin == policy.origin`.
//! 4. Authenticator data: `rpIdHash == SHA-256(rp_id)`, UP and UV set.
//! 5. The ES256 or EdDSA signature over `authenticator_data ‖ SHA-256(client_data_json)`.
//! 6. `now < expires_at <= now + max_override_age`.
//!
//! Steps 3 to 5 are [`web_passkeys::verify_assertion`], which is pure. The override holds when the
//! number of distinct approvers (by `approvers.json` label) that pass is at least
//! `max(policy.override_quorum, the run's quorum, 1)`. Otherwise the result is `bad_override`
//! with the first failing approval and step, or [`OverrideError::QuorumShort`] when every
//! approval passed but there are too few.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use web_passkeys::{CoseAlg, VerifyError, b64, verify_assertion};

use crate::challenge::{ApprovalKind, OverrideScope, ScopeError, normalize_hosts, parse_utc};
use crate::object::CommitId;

/// One approver passkey from `approvers.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approver {
    /// Who this is (counted for quorum).
    pub label: String,
    /// The credential id, base64url without padding.
    pub credential_id: String,
    /// The COSE_Key, base64url without padding.
    pub public_key_cose: String,
    /// `ES256` or `EdDSA`.
    pub alg: CoseAlg,
}

/// `approvers.json` is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApproversError {
    /// Not a JSON array of approvers with exactly the known fields.
    #[error("approvers.json is not a list of approvers")]
    Json,
    /// An empty label, a credential id that is not base64url, or a key that does not decode.
    #[error("approver {0}: invalid entry")]
    Entry(usize),
    /// The same credential id twice.
    #[error("approver {0}: duplicate credential id")]
    Duplicate(usize),
}

/// Parses and checks `approvers.json`.
pub fn parse_approvers(text: &str) -> Result<Vec<Approver>, ApproversError> {
    let list: Vec<Approver> = serde_json::from_str(text).map_err(|_| ApproversError::Json)?;
    let mut ids = BTreeSet::new();
    for (i, a) in list.iter().enumerate() {
        let key_ok = b64::decode(&a.public_key_cose)
            .and_then(|k| web_passkeys::CoseKey::decode(&k).ok())
            .is_some_and(|k| k.alg() == a.alg);
        if a.label.trim().is_empty()
            || b64::decode(&a.credential_id).is_none_or(|c| c.is_empty())
            || !key_ok
        {
            return Err(ApproversError::Entry(i));
        }
        if !ids.insert(a.credential_id.as_str()) {
            return Err(ApproversError::Duplicate(i));
        }
    }
    Ok(list)
}

/// One item of the `Approvals` frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    /// `override` or `baseline`.
    pub kind: ApprovalKind,
    /// Who approved, as Keel names them (informational; quorum counts `approvers.json` labels).
    pub approver: String,
    /// The passkey's credential id, base64url.
    pub credential_id: String,
    /// WebAuthn authenticator data.
    #[serde(with = "web_passkeys::b64::serde_bytes")]
    pub authenticator_data: Vec<u8>,
    /// WebAuthn client data JSON, as signed.
    #[serde(with = "web_passkeys::b64::serde_bytes")]
    pub client_data_json: Vec<u8>,
    /// The assertion signature.
    #[serde(with = "web_passkeys::b64::serde_bytes")]
    pub signature: Vec<u8>,
    /// `YYYY-MM-DDTHH:MM:SSZ`, part of the challenge.
    pub expires_at: String,
    /// The approver's reason.
    pub comment: String,
    /// Inventory names the approval covers, part of the challenge.
    pub hosts: Vec<String>,
}

/// The host's `policy.toml`, as far as overrides go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverridePolicy {
    /// Keel's web origin, e.g. `https://keel.example.com`.
    pub origin: String,
    /// The WebAuthn RP id.
    pub rp_id: String,
    /// The host's minimum number of distinct approvers.
    pub override_quorum: u32,
    /// Upper bound on `expires_at - now`, in seconds (default 24 h).
    pub max_override_age_secs: i64,
    /// This host's inventory name.
    pub inventory_name: String,
}

/// The run being authorized, from `Assign` and the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunContext {
    /// Workspace id.
    pub workspace: String,
    /// Binding id.
    pub binding: String,
    /// Run id.
    pub run: String,
    /// Job name.
    pub job: String,
    /// The quorum Keel asked for (0 when Keel did not say).
    pub assign_quorum: u32,
}

/// Which step of the verification an approval failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalProblem {
    /// Step 1: the kind does not match the run.
    KindMismatch,
    /// Step 1: this host is not in the approval's hosts.
    HostNotCovered,
    /// Step 1: the scope is invalid (bad timestamp, hosts, ids).
    #[serde(serialize_with = "ser_display")]
    Scope(ScopeError),
    /// Step 2: the credential is not in `approvers.json`, or its pinned key does not decode.
    UnknownCredential,
    /// Steps 3 to 5: the assertion did not verify.
    #[serde(serialize_with = "ser_display")]
    Assertion(VerifyError),
    /// Step 6: `expires_at` is not in the future.
    Expired,
    /// Step 6: `expires_at` is further ahead than `max_override_age`.
    TooFarAhead,
}

fn ser_display<T: std::fmt::Display, S: serde::Serializer>(v: &T, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(v)
}

impl ApprovalProblem {
    /// The contract's step number (1 to 6).
    pub fn step(&self) -> u8 {
        match self {
            Self::KindMismatch | Self::HostNotCovered | Self::Scope(_) => 1,
            Self::UnknownCredential => 2,
            Self::Assertion(e) => match e {
                VerifyError::ClientDataMalformed
                | VerifyError::WrongType
                | VerifyError::WrongChallenge
                | VerifyError::WrongOrigin
                | VerifyError::CrossOrigin => 3,
                VerifyError::AuthenticatorDataMalformed
                | VerifyError::WrongRpId
                | VerifyError::UserNotPresent
                | VerifyError::UserNotVerified => 4,
                _ => 5,
            },
            Self::Expired | Self::TooFarAhead => 6,
        }
    }
}

/// The override does not hold (`bad_override`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum OverrideError {
    /// The first approval that failed, and why.
    #[error("approval {index} failed at step {step}: {problem:?}")]
    BadApproval {
        /// Index in the `Approvals` items.
        index: usize,
        /// Contract step, 1 to 6.
        step: u8,
        /// What failed.
        problem: ApprovalProblem,
    },
    /// Every approval passed, but too few distinct approvers.
    #[error("quorum short: {have} of {need}")]
    QuorumShort {
        /// Distinct approvers that passed.
        have: usize,
        /// Required.
        need: usize,
    },
}

/// The approvals that carried the override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approved {
    /// `override` or `baseline`.
    pub kind: ApprovalKind,
    /// Credential ids of the approvals that passed, in order.
    pub credentials: Vec<String>,
    /// Their approver labels, in the same order.
    pub labels: Vec<String>,
}

/// Verifies `approvals` for a run from `base` (the host's own) to `target` at time `now`
/// (Unix seconds).
pub fn verify_override(
    approvals: &[Approval],
    approvers: &[Approver],
    policy: &OverridePolicy,
    run: &RunContext,
    base: Option<&CommitId>,
    target: &CommitId,
    now: i64,
) -> Result<Approved, OverrideError> {
    let kind = if base.is_some() {
        ApprovalKind::Override
    } else {
        ApprovalKind::Baseline
    };
    let need = policy.override_quorum.max(run.assign_quorum).max(1) as usize;
    let mut first_failure = None;
    let mut approved = Approved {
        kind,
        credentials: Vec::new(),
        labels: Vec::new(),
    };
    let mut distinct = BTreeSet::new();
    for (index, approval) in approvals.iter().enumerate() {
        match verify_one(approval, approvers, policy, run, kind, base, target, now) {
            Ok(approver) => {
                approved.credentials.push(approval.credential_id.clone());
                approved.labels.push(approver.label.clone());
                distinct.insert(approver.label.as_str());
            }
            Err(problem) => {
                if first_failure.is_none() {
                    first_failure = Some(OverrideError::BadApproval {
                        index,
                        step: problem.step(),
                        problem,
                    });
                }
            }
        }
    }
    if distinct.len() >= need {
        return Ok(approved);
    }
    Err(first_failure.unwrap_or(OverrideError::QuorumShort {
        have: distinct.len(),
        need,
    }))
}

#[allow(clippy::too_many_arguments)]
fn verify_one<'a>(
    approval: &Approval,
    approvers: &'a [Approver],
    policy: &OverridePolicy,
    run: &RunContext,
    kind: ApprovalKind,
    base: Option<&CommitId>,
    target: &CommitId,
    now: i64,
) -> Result<&'a Approver, ApprovalProblem> {
    // 1. Scope.
    if approval.kind != kind {
        return Err(ApprovalProblem::KindMismatch);
    }
    let expires =
        parse_utc(&approval.expires_at).ok_or(ApprovalProblem::Scope(ScopeError::ExpiresAt))?;
    if !approval.hosts.contains(&policy.inventory_name) {
        return Err(ApprovalProblem::HostNotCovered);
    }
    // The approval must state its hosts in canonical form; the host does not normalize them, so a
    // differently ordered list cannot produce a second valid challenge.
    if normalize_hosts(approval.hosts.iter().cloned()) != approval.hosts {
        return Err(ApprovalProblem::Scope(ScopeError::Hosts));
    }
    let scope = OverrideScope {
        kind,
        workspace: run.workspace.clone(),
        binding: run.binding.clone(),
        run: run.run.clone(),
        job: run.job.clone(),
        base: base.cloned(),
        target: target.clone(),
        hosts: approval.hosts.clone(),
        expires_at: approval.expires_at.clone(),
    };
    let challenge = scope.challenge().map_err(ApprovalProblem::Scope)?;

    // 2. Pinned approver.
    let approver = approvers
        .iter()
        .find(|a| a.credential_id == approval.credential_id)
        .ok_or(ApprovalProblem::UnknownCredential)?;
    let key = b64::decode(&approver.public_key_cose).ok_or(ApprovalProblem::UnknownCredential)?;

    // 3 to 5. The assertion.
    verify_assertion(
        &key,
        approver.alg,
        &approval.authenticator_data,
        &approval.client_data_json,
        &approval.signature,
        &challenge,
        &policy.origin,
        &policy.rp_id,
    )
    .map_err(ApprovalProblem::Assertion)?;

    // 6. Expiry window.
    if expires <= now {
        return Err(ApprovalProblem::Expired);
    }
    if expires > now.saturating_add(policy.max_override_age_secs) {
        return Err(ApprovalProblem::TooFarAhead);
    }
    Ok(approver)
}
