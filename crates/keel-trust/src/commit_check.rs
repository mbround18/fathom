//! The commit check (contract signed-mode.md, T096): every commit reachable from `target` and not
//! from `base`, merges included, must be signed by a trusted signer.
//!
//! OpenPGP signatures are verified with rpgp over the commit without its signature header. The
//! issuing key or subkey must be in the signers file, be signing-capable (a subkey needs the sign
//! flag on its binding and a valid primary-key back-signature), and be valid, not expired and not
//! revoked both at the committer time and at the signature's creation time. Validity at a time
//! comes from the latest verified self-signature or binding created at or before it. A revocation
//! with no reason or "key compromised" (or any unknown reason) revokes the key for all time; a
//! "superseded" or "retired" revocation revokes it from its creation time on.
//!
//! SSH signatures must use namespace `git` and a key in the signers file.
//!
//! Every undecidable case is a failure, never a pass.

use std::collections::{BTreeSet, HashSet, VecDeque};

use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey};
use pgp::crypto::hash::HashAlgorithm;
use pgp::packet::{RevocationCode, Signature, SignatureType};
use pgp::types::{KeyDetails, Tag};
use serde::Serialize;
use ssh_key::SshSig;

use crate::object::{
    CommitId, ObjectStore, ParsedCommit, SignatureBlock, StoreError, parse_commit,
};
use crate::signers::Signers;

/// The namespace Git uses for SSH commit signatures.
pub const SSH_NAMESPACE: &str = "git";

/// Why a commit does not count as signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Problem {
    /// No signature header.
    Missing,
    /// The signature cannot be parsed, or is not OpenPGP or SSH.
    Unparseable,
    /// No trusted key matches the signature's issuer.
    UnknownKey,
    /// The signature does not verify, or is not a binary-document signature.
    BadSignature,
    /// The key or subkey had expired at the committer or signature time.
    Expired,
    /// The key, its primary or its subkey is revoked.
    Revoked,
    /// The key has no sign flag, or a signing subkey has no valid back-signature.
    NotSigningCapable,
    /// The key has no verified self-signature or binding at that time, or the time is before the
    /// key was created.
    KeyInvalid,
    /// An SSH signature with a namespace other than `git`.
    WrongNamespace,
    /// MD5, SHA-1 or RIPEMD-160.
    WeakHash,
}

/// One commit that failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitFailure {
    /// The commit.
    pub commit: CommitId,
    /// Why.
    pub problem: Problem,
}

/// Why an override is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideReason {
    /// At least one new commit is not signed by a trusted signer.
    Unsigned,
    /// `target` is neither `base` nor a descendant of it.
    Downgrade,
    /// No base: a first run, which only a baseline approval can authorize.
    Baseline,
}

/// The result of the check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Every new commit is signed. `signers` is sorted and unique (empty when `target == base`).
    Signed {
        /// Signer ids.
        signers: Vec<String>,
    },
    /// Only an override (or, for [`OverrideReason::Baseline`], a baseline approval) can authorize
    /// this target.
    NeedsOverride {
        /// The most serious reason: baseline, then downgrade, then unsigned.
        reason: OverrideReason,
        /// Every new commit that failed, in walk order.
        failures: Vec<CommitFailure>,
    },
}

/// The check could not run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    /// A commit or the target's tree is missing or malformed (`pack_incomplete`).
    #[error("pack incomplete: {detail}")]
    PackIncomplete {
        /// What is missing.
        detail: String,
    },
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

fn incomplete(detail: impl Into<String>) -> CheckError {
    CheckError::PackIncomplete {
        detail: detail.into(),
    }
}

fn load<S: ObjectStore + ?Sized>(store: &S, id: &CommitId) -> Result<ParsedCommit, CheckError> {
    let raw = store
        .commit_bytes(id)?
        .ok_or_else(|| incomplete(format!("commit {id} missing")))?;
    parse_commit(&raw, id.is_sha256()).map_err(|e| incomplete(format!("commit {id}: {e}")))
}

/// Runs the commit check. `base` is the host's base for the job (from its ledger), or `None` on
/// a first run.
pub fn check<S: ObjectStore + ?Sized>(
    store: &S,
    signers: &Signers,
    base: Option<&CommitId>,
    target: &CommitId,
) -> Result<Verdict, CheckError> {
    if base.is_some_and(|b| b.is_sha256() != target.is_sha256()) {
        return Err(incomplete("base and target use different hashes"));
    }
    let tip = load(store, target)?;
    if !store.tree_complete(&tip.tree)? {
        return Err(incomplete(format!("tree of {target} incomplete")));
    }
    if base == Some(target) {
        return Ok(Verdict::Signed {
            signers: Vec::new(),
        });
    }

    // Everything reachable from base.
    let mut excluded = HashSet::new();
    if let Some(base) = base {
        let mut queue = VecDeque::from([base.clone()]);
        while let Some(id) = queue.pop_front() {
            if excluded.insert(id.clone()) {
                queue.extend(load(store, &id)?.parents);
            }
        }
    }

    // Everything reachable from target and not from base.
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([(target.clone(), tip)]);
    seen.insert(target.clone());
    let mut signer_ids = BTreeSet::new();
    let mut failures = Vec::new();
    let mut reached_base = false;
    while let Some((id, commit)) = queue.pop_front() {
        match verify_commit(signers, &commit) {
            Ok(signer) => {
                signer_ids.insert(signer);
            }
            Err(problem) => failures.push(CommitFailure {
                commit: id.clone(),
                problem,
            }),
        }
        for parent in &commit.parents {
            if excluded.contains(parent) {
                reached_base |= Some(parent) == base;
                continue;
            }
            if seen.insert(parent.clone()) {
                let c = load(store, parent)?;
                queue.push_back((parent.clone(), c));
            }
        }
    }

    let reason = if base.is_none() {
        Some(OverrideReason::Baseline)
    } else if !reached_base {
        Some(OverrideReason::Downgrade)
    } else if !failures.is_empty() {
        Some(OverrideReason::Unsigned)
    } else {
        None
    };
    Ok(match reason {
        Some(reason) => Verdict::NeedsOverride { reason, failures },
        None => Verdict::Signed {
            signers: signer_ids.into_iter().collect(),
        },
    })
}

/// Verifies one commit's signature and returns the signer id.
pub fn verify_commit(signers: &Signers, commit: &ParsedCommit) -> Result<String, Problem> {
    match &commit.signature {
        None => Err(Problem::Missing),
        Some(SignatureBlock::Other) => Err(Problem::Unparseable),
        Some(SignatureBlock::Ssh(text)) => verify_ssh(signers, text, &commit.payload),
        Some(SignatureBlock::OpenPgp(text)) => {
            verify_pgp(signers, text, &commit.payload, commit.committer_time)
        }
    }
}

fn verify_ssh(signers: &Signers, text: &str, payload: &[u8]) -> Result<String, Problem> {
    let sig = SshSig::from_pem(text.trim_end()).map_err(|_| Problem::Unparseable)?;
    if sig.namespace() != SSH_NAMESPACE {
        return Err(Problem::WrongNamespace);
    }
    let signer = signers
        .ssh
        .iter()
        .find(|s| s.key.key_data() == sig.public_key())
        .ok_or(Problem::UnknownKey)?;
    signer
        .key
        .verify(SSH_NAMESPACE, payload, &sig)
        .map_err(|_| Problem::BadSignature)?;
    Ok(signer.id.clone())
}

fn weak(hash: HashAlgorithm) -> bool {
    matches!(
        hash,
        HashAlgorithm::None | HashAlgorithm::Md5 | HashAlgorithm::Sha1 | HashAlgorithm::Ripemd160
    )
}

fn verify_pgp(
    signers: &Signers,
    text: &str,
    payload: &[u8],
    committer_time: i64,
) -> Result<String, Problem> {
    let (detached, _) = DetachedSignature::from_string(text).map_err(|_| Problem::Unparseable)?;
    let sig = detached.signature;
    let hash = sig.hash_alg().ok_or(Problem::Unparseable)?;
    if weak(hash) {
        return Err(Problem::WeakHash);
    }
    if sig.typ() != Some(SignatureType::Binary) {
        return Err(Problem::BadSignature);
    }
    let created = i64::from(sig.created().ok_or(Problem::Unparseable)?.as_secs());
    let fprs = sig.issuer_fingerprint();
    let kids = sig.issuer_key_id();
    let issued_by = |k: &dyn KeyDetailsDyn| {
        fprs.iter().any(|f| **f == k.fpr()) || kids.iter().any(|id| **id == k.kid())
    };

    // The most specific problem across matching keys; a pass anywhere wins.
    let mut problem = Problem::UnknownKey;
    for signer in &signers.pgp {
        let cert = &signer.cert;
        if issued_by(&cert.primary_key) {
            let result = sig
                .verify(&cert.primary_key, payload)
                .map_err(|_| Problem::BadSignature)
                .and_then(|()| {
                    primary_can_sign(cert, committer_time).and(primary_can_sign(cert, created))
                });
            match result {
                Ok(()) => return Ok(signer.id.clone()),
                Err(p) => problem = worse(problem, p),
            }
        }
        for sub in &cert.public_subkeys {
            if issued_by(&sub.key) {
                let result = sig
                    .verify(&sub.key, payload)
                    .map_err(|_| Problem::BadSignature)
                    .and_then(|()| {
                        subkey_can_sign(cert, sub, committer_time)
                            .and(subkey_can_sign(cert, sub, created))
                    });
                match result {
                    Ok(()) => return Ok(signer.id.clone()),
                    Err(p) => problem = worse(problem, p),
                }
            }
        }
    }
    Err(problem)
}

/// Picks the problem to report when several keys match: anything beats `UnknownKey`, and a
/// verified-but-invalid key beats a bad signature.
fn worse(current: Problem, new: Problem) -> Problem {
    match (current, new) {
        (Problem::UnknownKey, p) => p,
        (Problem::BadSignature, p) => p,
        (c, _) => c,
    }
}

/// Small object-safe view so the issuer match works for primaries and subkeys alike.
trait KeyDetailsDyn {
    fn fpr(&self) -> pgp::types::Fingerprint;
    fn kid(&self) -> pgp::types::KeyId;
}

impl<T: KeyDetails> KeyDetailsDyn for T {
    fn fpr(&self) -> pgp::types::Fingerprint {
        self.fingerprint()
    }
    fn kid(&self) -> pgp::types::KeyId {
        self.legacy_key_id()
    }
}

fn to_u32(t: i64) -> Option<u32> {
    u32::try_from(t).ok()
}

fn sig_created(sig: &Signature) -> Option<i64> {
    sig.created().map(|t| i64::from(t.as_secs()))
}

/// Revocation rule: hard reasons revoke for all time, soft ones from their creation on.
fn revoked_at(rev: &Signature, t: i64) -> bool {
    match rev.revocation_reason_code() {
        Some(RevocationCode::KeySuperseded) | Some(RevocationCode::KeyRetired) => {
            sig_created(rev).is_none_or(|c| c <= t)
        }
        _ => true,
    }
}

/// `Ok(expiry offset)` check: `created_at + key_expiration_time` must be after `t`.
fn check_expiry(key_created: i64, binding: &Signature, t: i64) -> Result<(), Problem> {
    match binding
        .key_expiration_time()
        .map(|d| i64::from(d.as_secs()))
    {
        Some(secs) if secs > 0 && t >= key_created + secs => Err(Problem::Expired),
        _ => Ok(()),
    }
}

/// The primary key is valid at `t`: created, not revoked, not expired, with a verified
/// self-signature. Returns that latest self-signature.
fn primary_valid(cert: &SignedPublicKey, t: i64) -> Result<&Signature, Problem> {
    let primary = &cert.primary_key;
    to_u32(t).ok_or(Problem::KeyInvalid)?;
    let key_created = i64::from(primary.created_at().as_secs());
    if t < key_created {
        return Err(Problem::KeyInvalid);
    }
    for rev in &cert.details.revocation_signatures {
        if rev.typ() == Some(SignatureType::KeyRevocation)
            && rev.verify_key(primary).is_ok()
            && revoked_at(rev, t)
        {
            return Err(Problem::Revoked);
        }
    }
    let direct = cert
        .details
        .direct_signatures
        .iter()
        .filter(|s| s.typ() == Some(SignatureType::Key) && s.verify_key(primary).is_ok());
    let certs = cert.details.users.iter().flat_map(|u| {
        u.signatures.iter().filter(move |s| {
            matches!(
                s.typ(),
                Some(
                    SignatureType::CertGeneric
                        | SignatureType::CertPersona
                        | SignatureType::CertCasual
                        | SignatureType::CertPositive
                )
            ) && s.verify_certification(primary, Tag::UserId, &u.id).is_ok()
        })
    });
    let latest = direct
        .chain(certs)
        .filter(|s| sig_created(s).is_some_and(|c| c <= t))
        .max_by_key(|s| sig_created(s))
        .ok_or(Problem::KeyInvalid)?;
    check_expiry(key_created, latest, t)?;
    Ok(latest)
}

fn primary_can_sign(cert: &SignedPublicKey, t: i64) -> Result<(), Problem> {
    let selfsig = primary_valid(cert, t)?;
    if selfsig.key_flags().sign() {
        Ok(())
    } else {
        Err(Problem::NotSigningCapable)
    }
}

fn subkey_can_sign(
    cert: &SignedPublicKey,
    sub: &pgp::composed::SignedPublicSubKey,
    t: i64,
) -> Result<(), Problem> {
    primary_valid(cert, t)?;
    let primary = &cert.primary_key;
    let key_created = i64::from(sub.key.created_at().as_secs());
    if t < key_created {
        return Err(Problem::KeyInvalid);
    }
    for rev in &sub.signatures {
        if rev.typ() == Some(SignatureType::SubkeyRevocation)
            && rev.verify_subkey_binding(primary, &sub.key).is_ok()
            && revoked_at(rev, t)
        {
            return Err(Problem::Revoked);
        }
    }
    let binding = sub
        .signatures
        .iter()
        .filter(|s| s.typ() == Some(SignatureType::SubkeyBinding))
        .filter(|s| sig_created(s).is_some_and(|c| c <= t))
        .filter(|s| s.verify_subkey_binding(primary, &sub.key).is_ok())
        .max_by_key(|s| sig_created(s))
        .ok_or(Problem::KeyInvalid)?;
    check_expiry(key_created, binding, t)?;
    if !binding.key_flags().sign() {
        return Err(Problem::NotSigningCapable);
    }
    let backsig_ok = binding.embedded_signature().is_some_and(|back| {
        back.typ() == Some(SignatureType::KeyBinding)
            && back.verify_primary_key_binding(&sub.key, primary).is_ok()
    });
    if backsig_ok {
        Ok(())
    } else {
        Err(Problem::NotSigningCapable)
    }
}
