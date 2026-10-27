//! Pure trust checks shared by the Keel server and the host executor (spec 055, contract
//! signed-mode.md). No file system or network I/O: callers feed objects, trust files and the
//! ledger text in, and get verdicts out, so the server's early check and the executor's enforcing
//! check reach the same verdict from the same inputs.
//!
//! - [`object`]: commit ids, the [`object::ObjectStore`] trait and the commit parse.
//! - [`signers`]: the trusted signers file (SSH `allowed_signers` lines, then OpenPGP keys).
//! - [`commit_check`]: the signature walk from `base` to `target` (T096).
//! - [`challenge`]: the override challenge, `SHA-256(JCS(scope))` (T090).
//! - [`override_verify`]: host-side verification of override and baseline approvals (T097).
//! - [`ledger`]: the hash-chained host ledger (T091).
//! - [`authorize`]: the executor's decision for one run, in refusal order.

pub mod authorize;
pub mod challenge;
pub mod commit_check;
pub mod ledger;
pub mod object;
pub mod override_verify;
pub mod signers;

pub use authorize::{Authorized, Refusal, RunRequest, authorize};
pub use challenge::{ApprovalKind, OverrideScope};
pub use commit_check::{Verdict, check};
pub use ledger::{Auth, Ledger};
pub use object::{CommitId, ObjectStore};
pub use override_verify::{Approval, Approver, OverridePolicy};
pub use signers::Signers;
