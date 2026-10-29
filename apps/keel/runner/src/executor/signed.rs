//! Signed mode on the host (spec 055 T098, contracts/signed-mode.md, research R19 and R20).
//!
//! The executor decides alone. The base comes from its own ledger, never from `Assign`; the
//! signers, approvers, origin, RP id, quorum and maximum approval age come from its trust files.
//! Nothing Keel sends can lower the quorum (`max(policy.override_quorum, Keel's)`) or add a signer
//! or approver.
//!
//! The ledger is append-only and crash-safe: every line is written with `O_APPEND` and `fsync`ed
//! before the executor acts on it, and the directory is `fsync`ed when the file is created. An
//! exclusive `flock` on `ledger.lock` serializes concurrent executor instances while they read,
//! verify and append. A ledger that does not verify refuses every run (`ledger_corrupt`); the
//! executor never truncates or rewrites it.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write as _;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use keel_exec::frames;
use keel_trust::challenge::format_utc;
use keel_trust::ledger::{Entry, Outcome};
use keel_trust::override_verify::{RunContext, parse_approvers};
use keel_trust::{Approval, Approver, Ledger, OverridePolicy, Refusal, Signers};

use crate::policy::{Policy, read_trust_file};

/// Trusted signers under the trust directory.
pub const SIGNERS_FILE: &str = "signers";
/// Approver passkeys under the trust directory.
pub const APPROVERS_FILE: &str = "approvers.json";
/// The lock serializing ledger reads and appends, under the state directory.
pub const LOCK_FILE: &str = "ledger.lock";

/// The host's trust, as signed mode needs it.
pub struct Trust {
    pub signers: Signers,
    pub approvers: Vec<Approver>,
    pub policy: OverridePolicy,
}

/// Reads `signers` and `approvers.json`. Both must exist and parse: a host that requires signed
/// work and cannot say whom it trusts runs nothing.
pub fn load_trust(trust_dir: &Path, policy: &Policy) -> Result<Trust, String> {
    let signers = read_trust_file(&trust_dir.join(SIGNERS_FILE)).map_err(|e| e.to_string())?;
    let signers = Signers::parse(&signers).map_err(|e| e.to_string())?;
    let approvers = read_trust_file(&trust_dir.join(APPROVERS_FILE)).map_err(|e| e.to_string())?;
    let approvers = parse_approvers(&approvers).map_err(|e| e.to_string())?;
    let max_age = i64::try_from(policy.max_override_age().as_secs()).unwrap_or(i64::MAX);
    Ok(Trust {
        signers,
        approvers,
        policy: OverridePolicy {
            origin: policy.origin.clone(),
            rp_id: policy.rp_id.clone(),
            override_quorum: policy.override_quorum,
            max_override_age_secs: max_age,
            inventory_name: policy.inventory_name.clone(),
        },
    })
}

/// The run as keel-trust sees it, from `Assign`. A missing quorum counts as 0, so the host's own
/// quorum applies; keel-trust takes the larger of the two. The assigned params (spec 061) join
/// the rebuilt challenge, so an approval made without them does not verify.
pub fn run_context(
    workspace: &str,
    binding: &str,
    run_id: &str,
    job: &str,
    quorum: Option<u32>,
    params: &BTreeMap<String, String>,
) -> RunContext {
    RunContext {
        workspace: workspace.to_owned(),
        binding: binding.to_owned(),
        run: run_id.to_owned(),
        job: job.to_owned(),
        assign_quorum: quorum.unwrap_or(0),
        params: params.clone(),
    }
}

/// Whether a wire approval carries a passkey assertion at all.
fn has_assertion(a: &frames::Approval) -> bool {
    !a.authenticator_data.is_empty() && !a.client_data_json.is_empty() && !a.signature.is_empty()
}

/// Turns the `Approvals` items into keel-trust approvals.
///
/// Items without a passkey assertion are dropped: they are not approvals the host can verify,
/// and with nothing left the run is `unsigned` (or `downgrade`), exactly as with no approvals.
/// Any other item that does not decode is `bad_override`.
pub fn wire_approvals(items: &[frames::Approval]) -> Result<Vec<Approval>, String> {
    let mut out = Vec::new();
    for (index, a) in items.iter().filter(|a| has_assertion(a)).enumerate() {
        let expires_at = i64::try_from(a.expires_at)
            .ok()
            .and_then(format_utc)
            .ok_or_else(|| format!("approval {index}: expires_at is out of range"))?;
        // keel-trust's own decoding (kind, base64url fields), so both sides read them alike.
        let value = serde_json::json!({
            "kind": a.kind,
            "approver": a.approver,
            "credential_id": a.credential_id,
            "authenticator_data": a.authenticator_data,
            "client_data_json": a.client_data_json,
            "signature": a.signature,
            "expires_at": expires_at,
            "comment": a.comment,
            "hosts": a.hosts,
        });
        let approval: Approval = serde_json::from_value(value).map_err(|_| {
            format!("approval {index}: the kind or an assertion field does not decode")
        })?;
        out.push(approval);
    }
    Ok(out)
}

/// keel-trust's refusal as the wire's, with a detail that names no secret and no assertion.
/// `pack_incomplete` has no wire refusal: it is `Err`, and the run fails to set up.
pub fn wire_refusal(
    r: &Refusal,
    assumed_base: Option<&str>,
) -> Result<(frames::Refusal, String), String> {
    Ok(match r {
        Refusal::LedgerCorrupt { detail } => (frames::Refusal::LedgerCorrupt, detail.clone()),
        Refusal::Replay => (
            frames::Refusal::Replay,
            "this run already started on this host".to_owned(),
        ),
        Refusal::BaseMismatch { host_base } => (
            frames::Refusal::BaseMismatch,
            format!(
                "Keel's base is {}, this host's is {}",
                assumed_base.unwrap_or("none"),
                host_base.as_ref().map_or("none", |b| b.as_str())
            ),
        ),
        Refusal::PackIncomplete { detail } => return Err(format!("pack_incomplete: {detail}")),
        Refusal::Unsigned { failures } => (
            frames::Refusal::Unsigned,
            match failures.first() {
                None => "this host has no base for the job: a first run needs a baseline approval"
                    .to_owned(),
                Some(f) => format!(
                    "{} commit(s) not signed by a trusted signer, first {} ({})",
                    failures.len(),
                    f.commit.as_str(),
                    serde_json::to_string(&f.problem).unwrap_or_default()
                ),
            },
        ),
        Refusal::Downgrade => (
            frames::Refusal::Downgrade,
            "the target is not this host's base or a descendant of it".to_owned(),
        ),
        Refusal::BadOverride { error } => (frames::Refusal::BadOverride, error.to_string()),
    })
}

/// Holds the exclusive ledger lock until dropped (closing the descriptor releases it).
pub struct LedgerLock(#[allow(dead_code)] File);

impl LedgerLock {
    /// Blocks until this process holds `<state_dir>/ledger.lock`.
    pub fn acquire(state_dir: &Path) -> std::io::Result<Self> {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(state_dir.join(LOCK_FILE))?;
        loop {
            // SAFETY: flock on a descriptor this process owns.
            if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(Self(f));
            }
            let e = std::io::Error::last_os_error();
            if e.kind() != std::io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
    }
}

/// The ledger file and its text. A missing file is an empty ledger; anything unreadable is an
/// error, which callers treat as corrupt.
pub fn read_ledger_text(path: &Path) -> Result<String, String> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|_| "the ledger is not UTF-8".to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.kind().to_string()),
    }
}

/// Appends one line (ending in `\n`) and `fsync`s it, and the directory when the file is new.
pub fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    debug_assert!(line.ends_with('\n'));
    let created = !path.exists();
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(line.as_bytes())?;
    f.sync_all()?;
    if created && let Some(dir) = path.parent() {
        File::open(dir)?.sync_all()?;
    }
    Ok(())
}

/// The current time as a ledger timestamp.
pub fn ledger_now() -> String {
    let now = i64::try_from(crate::runner::unix_now()).unwrap_or(i64::MAX);
    format_utc(now).unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned())
}

/// A verified ledger and where it lives, held under the lock while a line is appended.
pub struct OpenLedger {
    pub path: PathBuf,
    pub ledger: Ledger,
}

impl OpenLedger {
    /// Appends `entry`: built and checked against the verified chain in memory, then written
    /// and `fsync`ed. Returns the line without its `\n`.
    pub fn append(&mut self, entry: Entry) -> Result<String, String> {
        let line = self.ledger.append(entry).map_err(|e| e.to_string())?;
        append_line(&self.path, &line).map_err(|e| format!("ledger write: {e}"))?;
        Ok(line.trim_end_matches('\n').to_owned())
    }
}

/// Appends the `end` line for a started run, under the lock and against a freshly verified
/// chain. Returns the line, or why it could not be written.
pub fn record_end(
    state_dir: &Path,
    ledger_path: &Path,
    run: &str,
    job: &str,
    outcome: Outcome,
    exit: Option<i64>,
) -> Result<String, String> {
    let _lock = LedgerLock::acquire(state_dir).map_err(|e| format!("ledger lock: {e}"))?;
    let text = read_ledger_text(ledger_path)?;
    let ledger = Ledger::parse(&text).map_err(|e| e.to_string())?;
    let mut open = OpenLedger {
        path: ledger_path.to_owned(),
        ledger,
    };
    open.append(Entry::End {
        at: ledger_now(),
        run: run.to_owned(),
        job: job.to_owned(),
        outcome,
        exit,
    })
}

/// `keel-executor ledger verify`: checks the whole chain. `Ok` says how many lines and the head.
pub fn verify_ledger_file(path: &Path) -> Result<String, String> {
    let text = read_ledger_text(path)?;
    let ledger = Ledger::parse(&text).map_err(|e| e.to_string())?;
    let n = ledger.lines().len();
    if n == 0 {
        return Ok("the ledger is empty".to_owned());
    }
    let head: String = ledger.head().iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("the ledger verifies: {n} lines, head {head}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use keel_trust::ApprovalKind;

    fn wire(kind: &str) -> frames::Approval {
        frames::Approval {
            kind: kind.to_owned(),
            approver: "alice".to_owned(),
            credential_id: "Y3JlZA".to_owned(),
            authenticator_data: "AAEC".to_owned(),
            client_data_json: "e30".to_owned(),
            signature: "AwQ".to_owned(),
            expires_at: 1_800_000_000,
            comment: "ok".to_owned(),
            hosts: vec!["web-1".to_owned()],
        }
    }

    #[test]
    fn passkey_less_items_are_dropped() {
        let mut empty = wire("override");
        empty.signature.clear();
        let mut no_data = wire("override");
        no_data.authenticator_data.clear();
        assert!(wire_approvals(&[empty, no_data]).unwrap().is_empty());
    }

    #[test]
    fn a_wire_approval_decodes_with_its_hosts() {
        let got = wire_approvals(&[wire("baseline")]).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, ApprovalKind::Baseline);
        assert_eq!(got[0].authenticator_data, vec![0, 1, 2]);
        assert_eq!(got[0].expires_at, "2027-01-15T08:00:00Z");
        assert_eq!(got[0].hosts, vec!["web-1".to_owned()]);
    }

    #[test]
    fn a_bad_item_is_an_error_not_dropped() {
        assert!(wire_approvals(&[wire("sudo")]).is_err());
        let mut bad = wire("override");
        bad.signature = "!!".to_owned();
        assert!(wire_approvals(&[bad]).is_err());
    }

    #[test]
    fn append_is_fsynced_and_append_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let mut open = OpenLedger {
            path: path.clone(),
            ledger: Ledger::empty(),
        };
        let target = keel_trust::CommitId::parse(&"a".repeat(40)).unwrap();
        let start = open
            .append(Entry::Start {
                at: "2026-10-27T10:00:00Z".to_owned(),
                run: "r1".to_owned(),
                job: "site".to_owned(),
                base: None,
                target: target.clone(),
                auth: keel_trust::Auth::Signed {
                    signers: vec!["SHA256:x".to_owned()],
                },
            })
            .unwrap();
        assert!(!start.ends_with('\n'));
        let end = record_end(dir.path(), &path, "r1", "site", Outcome::Succeeded, Some(0)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, format!("{start}\n{end}\n"));
        let ledger = Ledger::parse(&text).unwrap();
        assert_eq!(ledger.base("site"), Some(target));
        assert!(verify_ledger_file(&path).unwrap().contains("2 lines"));
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_broken_chain_does_not_verify_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        std::fs::write(&path, "{\"seq\":1}\n").unwrap();
        assert!(verify_ledger_file(&path).is_err());
        assert!(record_end(dir.path(), &path, "r", "j", Outcome::Failed, Some(2)).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"seq\":1}\n");
    }

    // ---- spec 061 T064: params in the rebuilt override challenge -------------------------------

    const WS: &str = "0b7f1c2e-3d4a-4b5c-8d6e-7f8091a2b3c4";
    const BINDING: &str = "1c2d3e4f-5a6b-4c7d-8e9f-a0b1c2d3e4f5";
    const RUN: &str = "2d3e4f50-6a7b-4c8d-9eaf-b0c1d2e3f405";
    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn b64url(bytes: &[u8]) -> String {
        const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
            for i in 0..=chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]));
            }
        }
        out
    }

    /// A deterministic software passkey (EdDSA) that signs WebAuthn assertions.
    struct Passkey(russh::keys::ssh_key::private::Ed25519Keypair);

    impl Passkey {
        fn new() -> Self {
            Self(russh::keys::ssh_key::private::Ed25519Keypair::from_seed(
                &[7; 32],
            ))
        }

        fn approver(&self) -> Approver {
            // COSE_Key {1: 1 (OKP), 3: -8 (EdDSA), -1: 6 (Ed25519), -2: x}.
            let mut cose = vec![0xa4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21, 0x58, 0x20];
            cose.extend_from_slice(&self.0.public.0);
            let text = serde_json::json!([{
                "label": "alice",
                "credential_id": b64url(b"cred-1"),
                "public_key_cose": b64url(&cose),
                "alg": "EdDSA",
            }])
            .to_string();
            parse_approvers(&text).unwrap().remove(0)
        }

        /// An `override` approval for `web-1`, signed over the challenge of `params`.
        fn approve(
            &self,
            base: &keel_trust::CommitId,
            target: &keel_trust::CommitId,
            expires_at: &str,
            params: &BTreeMap<String, String>,
        ) -> Approval {
            use russh::keys::signature::Signer as _;
            use sha2::{Digest as _, Sha256};
            let scope = keel_trust::OverrideScope {
                kind: ApprovalKind::Override,
                workspace: WS.to_owned(),
                binding: BINDING.to_owned(),
                run: RUN.to_owned(),
                job: "keel-server".to_owned(),
                base: Some(base.clone()),
                target: target.clone(),
                hosts: vec!["web-1".to_owned()],
                expires_at: expires_at.to_owned(),
                params: params.clone(),
            };
            let challenge = scope.challenge().unwrap();
            let client_data_json = serde_json::json!({
                "type": "webauthn.get",
                "challenge": b64url(&challenge),
                "origin": "https://keel.example",
            })
            .to_string()
            .into_bytes();
            let mut authenticator_data = Sha256::digest(b"keel.example").to_vec();
            authenticator_data.extend_from_slice(&[0x05, 0, 0, 0, 1]); // UP | UV, counter 1
            let mut signed = authenticator_data.clone();
            signed.extend_from_slice(&Sha256::digest(&client_data_json));
            let signature: russh::keys::ssh_key::Signature = self.0.try_sign(&signed).unwrap();
            Approval {
                kind: ApprovalKind::Override,
                approver: "alice".to_owned(),
                credential_id: b64url(b"cred-1"),
                authenticator_data,
                client_data_json,
                signature: signature.as_bytes().to_vec(),
                expires_at: expires_at.to_owned(),
                comment: "update Keel".to_owned(),
                hosts: vec!["web-1".to_owned()],
            }
        }
    }

    #[test]
    fn the_rebuilt_challenge_carries_the_assigned_params() {
        use keel_trust::override_verify::{ApprovalProblem, OverrideError, verify_override};
        let key = Passkey::new();
        let approvers = vec![key.approver()];
        let policy = OverridePolicy {
            origin: "https://keel.example".to_owned(),
            rp_id: "keel.example".to_owned(),
            override_quorum: 1,
            max_override_age_secs: 86_400,
            inventory_name: "web-1".to_owned(),
        };
        let base = keel_trust::CommitId::parse(&"1".repeat(40)).unwrap();
        let target = keel_trust::CommitId::parse(&"2".repeat(40)).unwrap();
        let now = 1_800_000_000;
        let expires_at = format_utc(now + 3_600).unwrap();
        let params = BTreeMap::from([(
            "keel_server_image_digest".to_owned(),
            DIGEST.to_owned(),
        )]);
        let none = BTreeMap::new();
        let verify = |approval: &Approval, assigned: &BTreeMap<String, String>| {
            let run = run_context(WS, BINDING, RUN, "keel-server", None, assigned);
            verify_override(
                std::slice::from_ref(approval),
                &approvers,
                &policy,
                &run,
                Some(&base),
                &target,
                now,
            )
        };

        // Approved with the params the run was assigned: verifies.
        let with = key.approve(&base, &target, &expires_at, &params);
        assert!(verify(&with, &params).is_ok(), "{:?}", verify(&with, &params));

        // Approved without them (or for another value): the challenge differs, so it doesn't.
        let without = key.approve(&base, &target, &expires_at, &none);
        let other = key.approve(
            &base,
            &target,
            &expires_at,
            &BTreeMap::from([(
                "keel_server_image_digest".to_owned(),
                format!("sha256:{}", "f".repeat(64)),
            )]),
        );
        for bad in [&without, &other] {
            let err = verify(bad, &params).unwrap_err();
            // Step 3 is the client data; its type and origin are right, so it is the challenge.
            assert!(
                matches!(
                    err,
                    OverrideError::BadApproval {
                        step: 3,
                        problem: ApprovalProblem::Assertion(_),
                        ..
                    }
                ),
                "{err:?}"
            );
        }

        // A run without params keeps verifying an approval made without them, as before 061,
        // and refuses one bound to params it wasn't assigned.
        assert!(verify(&without, &none).is_ok());
        assert!(verify(&with, &none).is_err());
    }
}
