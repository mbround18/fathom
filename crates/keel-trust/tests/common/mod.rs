//! Shared by the vector generator (tests/gen) and the vector runner (tests/vectors.rs): the file
//! formats under tests/vectors/, an in-memory [`ObjectStore`], and the summary of an
//! `authorize` result that `case.json` records.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use keel_trust::authorize::{Authorized, Refusal};
use keel_trust::object::{CommitId, ObjectStore, StoreError};
use keel_trust::override_verify::{Approval, OverrideError, OverridePolicy, RunContext};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Every case directory the generator writes, in order. The runner refuses a missing or extra one.
pub const CASES: &[&str] = &[
    "signed_openpgp",
    "signed_ssh",
    "unsigned_tip",
    "unsigned_under_signed_merge",
    "unknown_key",
    "expired_pgp_subkey",
    "revoked_key",
    "downgrade",
    "reapply_base",
    "baseline",
    "baseline_without_approval",
    "override_ok",
    "override_other_host",
    "override_wrong_origin",
    "override_uv_missing",
    "override_expired",
    "override_quorum_short",
    "replay",
    "ledger_corrupt",
    "base_mismatch",
    "pack_incomplete",
];

pub fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vectors")
}

/// `repo.json`: raw commits by id (SHA-1 of `commit <len>\0<raw>`), and the trees present.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoFile {
    pub commits: BTreeMap<String, String>,
    pub trees: BTreeSet<String>,
}

/// An in-memory store. Refuses a commit whose bytes do not hash to its id.
pub struct MemStore(pub RepoFile);

pub fn git_sha1(raw: &[u8]) -> String {
    use sha1::Digest as _;
    let mut h = sha1::Sha1::new();
    h.update(format!("commit {}\0", raw.len()).as_bytes());
    h.update(raw);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

impl ObjectStore for MemStore {
    fn commit_bytes(&self, id: &CommitId) -> Result<Option<Vec<u8>>, StoreError> {
        match self.0.commits.get(id.as_str()) {
            None => Ok(None),
            Some(raw) if git_sha1(raw.as_bytes()) == id.as_str() => {
                Ok(Some(raw.clone().into_bytes()))
            }
            Some(_) => Err(StoreError(format!("object {id} does not match its id"))),
        }
    }

    fn tree_complete(&self, tree: &CommitId) -> Result<bool, StoreError> {
        Ok(self.0.trees.contains(tree.as_str()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFile {
    pub workspace: String,
    pub binding: String,
    pub run: String,
    pub job: String,
    pub assign_quorum: u32,
}

impl RunFile {
    pub fn context(&self) -> RunContext {
        RunContext {
            workspace: self.workspace.clone(),
            binding: self.binding.clone(),
            run: self.run.clone(),
            job: self.job.clone(),
            assign_quorum: self.assign_quorum,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyFile {
    pub origin: String,
    pub rp_id: String,
    pub override_quorum: u32,
    pub max_override_age_secs: i64,
    pub inventory_name: String,
}

impl PolicyFile {
    pub fn policy(&self) -> OverridePolicy {
        OverridePolicy {
            origin: self.origin.clone(),
            rp_id: self.rp_id.clone(),
            override_quorum: self.override_quorum,
            max_override_age_secs: self.max_override_age_secs,
            inventory_name: self.inventory_name.clone(),
        }
    }
}

/// `case.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseFile {
    pub description: String,
    /// The host's ledger text.
    pub ledger: String,
    /// Keel's assumed base, from `Assign`.
    pub assumed_base: Option<String>,
    pub target: String,
    pub run: RunFile,
    pub policy: PolicyFile,
    pub approvals: Vec<Approval>,
    /// Unix seconds.
    pub now: i64,
    /// The commit check's result from the host's ledger base (`null` when the ledger is corrupt):
    /// a serialized `Verdict`, or `{"error": "pack_incomplete"}`.
    pub expect_check: Value,
    /// The authorize result, see [`summarize`].
    pub expect: Value,
}

/// `{"authorized": <Auth>}` or `{"refused": "<code>", ...}` with the refusal's checkable detail.
pub fn summarize(result: &Result<Authorized, Refusal>) -> Value {
    match result {
        Ok(a) => json!({ "authorized": a.auth }),
        Err(r) => {
            let mut v = json!({ "refused": r.code() });
            match r {
                Refusal::Unsigned { failures } => v["failures"] = json!(failures),
                Refusal::BaseMismatch { host_base } => v["host_base"] = json!(host_base),
                Refusal::BadOverride {
                    error: OverrideError::BadApproval { index, step, .. },
                } => {
                    v["index"] = json!(index);
                    v["step"] = json!(step);
                }
                Refusal::BadOverride {
                    error: OverrideError::QuorumShort { have, need },
                } => {
                    v["have"] = json!(have);
                    v["need"] = json!(need);
                }
                _ => {}
            }
            v
        }
    }
}
