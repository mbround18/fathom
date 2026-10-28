//! Frames of the server ↔ runner channel (contracts/runner.md).
//!
//! One enum covers both SSH subsystems (`keel-enroll-v1`, `keel-runner-v1`) and the local
//! executor socket. On the wire a frame is JSON tagged by `"frame"`, e.g.
//! `{"frame":"Started","run_id":"…"}`; binary fields are standard base64. Unknown frames and
//! unknown fields are refused (fail closed).
//!
//! Secret material (`Secrets` values, the enrollment token) is held in [`SecretString`], whose
//! `Debug` is redacted, and `Frame`'s own `Debug` summarises binary payloads by length.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::events::HostStats;

/// The SSH subsystems the listener offers. Nothing else is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subsystem {
    /// `keel-enroll-v1`: `Enroll` → `Enrolled`, for a key Keel does not know yet.
    Enroll,
    /// `keel-runner-v1`: every other channel frame.
    Runner,
}

impl Subsystem {
    pub const ENROLL_NAME: &'static str = "keel-enroll-v1";
    pub const RUNNER_NAME: &'static str = "keel-runner-v1";

    /// The subsystem name as requested over SSH.
    pub const fn name(self) -> &'static str {
        match self {
            Subsystem::Enroll => Self::ENROLL_NAME,
            Subsystem::Runner => Self::RUNNER_NAME,
        }
    }

    /// Exact match only: no prefixes, no case folding.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            Self::ENROLL_NAME => Some(Subsystem::Enroll),
            Self::RUNNER_NAME => Some(Subsystem::Runner),
            _ => None,
        }
    }
}

/// Which way a frame travels. For the local socket, "server" is the network side (`keel-runner`)
/// and "runner" is the executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    RunnerToServer,
    ServerToRunner,
    Both,
}

/// Who is sending, for [`Frame::may_be_sent_by`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peer {
    Runner,
    Server,
}

/// Why a host refused a run: the `run_hosts.refusal` values of data-model.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    Unsigned,
    BadOverride,
    BaseMismatch,
    ProfileNotAllowed,
    Downgrade,
    Replay,
    LedgerCorrupt,
    VersionTooOld,
}

impl Refusal {
    pub const fn as_str(self) -> &'static str {
        match self {
            Refusal::Unsigned => "unsigned",
            Refusal::BadOverride => "bad_override",
            Refusal::BaseMismatch => "base_mismatch",
            Refusal::ProfileNotAllowed => "profile_not_allowed",
            Refusal::Downgrade => "downgrade",
            Refusal::Replay => "replay",
            Refusal::LedgerCorrupt => "ledger_corrupt",
            Refusal::VersionTooOld => "version_too_old",
        }
    }
}

/// `stdout` or `stderr` of `ansible-playbook`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// A string that must never be logged: a credential value or an enrollment token.
///
/// `Debug` prints `<redacted>`; there is deliberately no `Display`. Read it with
/// [`SecretString::expose`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself. Callers must not log it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// A job's last applied commit on the host, from the executor's ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Base {
    pub job: String,
    pub commit: String,
}

/// One credential sent with a run. `value` is redacted in `Debug`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretItem {
    pub name: String,
    pub kind: String,
    pub value: SecretString,
}

/// An override or baseline approval (research R20). The host verifies the passkey assertion
/// itself; these fields are base64url as WebAuthn produced them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    pub kind: String,
    pub approver: String,
    pub credential_id: String,
    pub authenticator_data: String,
    pub client_data_json: String,
    pub signature: String,
    /// Unix seconds.
    pub expires_at: u64,
    pub comment: String,
    /// Inventory names the approval covers, as in the challenge: non-empty, sorted, no
    /// duplicates.
    pub hosts: Vec<String>,
}

/// The most entries one `OutsideRuns` frame may carry (spec 058 amendment A2).
pub const MAX_OUTSIDE_RUNS: usize = 100;
/// The longest `user` of an [`OutsideRun`], in bytes.
pub const MAX_OUTSIDE_USER: usize = 64;
/// The longest `playbook` of an [`OutsideRun`], in bytes.
pub const MAX_OUTSIDE_PLAYBOOK: usize = 256;
/// The longest `at` of an [`OutsideRun`], in bytes (an RFC 3339 time is far shorter).
pub const MAX_OUTSIDE_AT: usize = 64;

/// A run of `ansible-playbook` on the host that Keel did not start, as the host recorded it in
/// `/var/lib/keel-outside/runs.jsonl` (spec 058 amendment A2). These are the host's claims, not
/// facts Keel checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutsideRun {
    /// RFC 3339, as the host's clock had it.
    pub at: String,
    pub user: String,
    pub playbook: String,
    /// 40 lowercase hex characters, or `None` when the checkout had no commit.
    pub commit: Option<String>,
    pub dirty: bool,
    /// `--check`: a dry run, which changed nothing.
    pub check: bool,
}

impl OutsideRun {
    /// The record's bounds: `at` non-empty and at most 64 bytes, `user` at most 64 bytes,
    /// `playbook` at most 256, `commit` 40 lowercase hex characters when present.
    pub fn check(&self) -> Result<(), &'static str> {
        if self.at.is_empty() || self.at.len() > MAX_OUTSIDE_AT {
            return Err("at is empty or too long");
        }
        if self.user.len() > MAX_OUTSIDE_USER {
            return Err("user is over 64 bytes");
        }
        if self.playbook.len() > MAX_OUTSIDE_PLAYBOOK {
            return Err("playbook is over 256 bytes");
        }
        if let Some(c) = &self.commit
            && !(c.len() == 40 && c.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        {
            return Err("commit is not 40 lowercase hex characters");
        }
        Ok(())
    }
}

/// Every frame of contracts/runner.md, plus the local-socket `ReadLedger`/`Ledger` pair.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "frame", deny_unknown_fields)]
pub enum Frame {
    // ---- keel-enroll-v1 ----
    /// R → S, the only frame an unknown key may send.
    Enroll {
        token: SecretString,
        inventory_name: String,
        allowed_profiles: Vec<String>,
        require_signed: bool,
    },
    /// S → R. `cert` is the OpenSSH one-line certificate.
    Enrolled {
        host_id: String,
        cert: String,
    },

    // ---- keel-runner-v1 ----
    /// R → S, first frame of a runner session.
    Hello {
        host_id: String,
        version: String,
        ansible_core: String,
        python: String,
        allowed_profiles: Vec<String>,
        require_signed: bool,
        override_quorum: u32,
        ledger_head: Option<String>,
        bases: Vec<Base>,
    },
    /// Either side may start a renewal; the server answers with `Cert`.
    RenewCert {},
    /// S → R. `cert` is the OpenSSH one-line certificate.
    Cert {
        cert: String,
    },
    /// S → R. An OpenSSH KRL (see `krl`).
    Krl {
        #[serde(with = "b64")]
        bytes: Vec<u8>,
    },
    /// S → R. `base` is Keel's belief (`None` for a baseline); `deadline` is Unix seconds.
    /// `workspace` and `binding` are lowercase hyphenated UUIDs and, with `run_id`, `job`, `base`
    /// and `target`, rebuild the override challenge (contracts/signed-mode.md). `quorum` is the
    /// binding's override quorum; the host takes the larger of it and its own.
    Assign {
        run_id: String,
        workspace: String,
        binding: String,
        job: String,
        base: Option<String>,
        target: String,
        deadline: u64,
        pack_size: u64,
        quorum: Option<u32>,
    },
    /// S → R. A slice of the Git pack; the last chunk carries the whole pack's SHA-256 (hex).
    Pack {
        run_id: String,
        #[serde(with = "b64")]
        chunk: Vec<u8>,
        last: bool,
        sha256: Option<String>,
    },
    /// S → R. Values are redacted in `Debug`.
    Secrets {
        run_id: String,
        items: Vec<SecretItem>,
    },
    /// S → R.
    Approvals {
        run_id: String,
        items: Vec<Approval>,
    },
    /// R → S.
    Started {
        run_id: String,
    },
    /// R → S.
    Refused {
        run_id: String,
        reason: Refusal,
        detail: String,
    },
    /// R → S. `json` is one `keel_events` line, verbatim (see `events`).
    Event {
        run_id: String,
        seq: u64,
        json: String,
    },
    /// R → S.
    Log {
        run_id: String,
        seq: u64,
        stream: LogStream,
        #[serde(with = "b64")]
        bytes: Vec<u8>,
    },
    /// S → R.
    Cancel {
        run_id: String,
    },
    /// R → S. `stats` is Ansible's per-host summary; `ledger_line` the executor's appended line.
    Finished {
        run_id: String,
        exit_code: i32,
        stats: BTreeMap<String, HostStats>,
        ledger_line: String,
    },
    /// R → S. New lines of the host's outside-runs record: at most [`MAX_OUTSIDE_RUNS`] entries;
    /// `skipped` counts lines the runner dropped as malformed or oversized; `through` is the byte
    /// offset in the record just past this batch, which the server echoes in `OutsideRunsAck`.
    OutsideRuns {
        #[serde(deserialize_with = "bounded_outside_runs")]
        entries: Vec<OutsideRun>,
        skipped: u32,
        through: u64,
    },
    /// S → R. The server has stored every entry up to byte `through`; only now does the runner
    /// move its offset.
    OutsideRunsAck {
        through: u64,
    },
    Ping,
    Pong,

    // ---- local executor socket only ----
    /// Network side → executor: report the ledger head and per-job bases (for `Hello`).
    ReadLedger,
    /// Executor → network side: the answer to `ReadLedger`.
    Ledger {
        ledger_head: Option<String>,
        bases: Vec<Base>,
    },
}

impl Frame {
    /// The frame's tag, as on the wire.
    pub const fn name(&self) -> &'static str {
        match self {
            Frame::Enroll { .. } => "Enroll",
            Frame::Enrolled { .. } => "Enrolled",
            Frame::Hello { .. } => "Hello",
            Frame::RenewCert {} => "RenewCert",
            Frame::Cert { .. } => "Cert",
            Frame::Krl { .. } => "Krl",
            Frame::Assign { .. } => "Assign",
            Frame::Pack { .. } => "Pack",
            Frame::Secrets { .. } => "Secrets",
            Frame::Approvals { .. } => "Approvals",
            Frame::Started { .. } => "Started",
            Frame::Refused { .. } => "Refused",
            Frame::Event { .. } => "Event",
            Frame::Log { .. } => "Log",
            Frame::Cancel { .. } => "Cancel",
            Frame::Finished { .. } => "Finished",
            Frame::OutsideRuns { .. } => "OutsideRuns",
            Frame::OutsideRunsAck { .. } => "OutsideRunsAck",
            Frame::Ping => "Ping",
            Frame::Pong => "Pong",
            Frame::ReadLedger => "ReadLedger",
            Frame::Ledger { .. } => "Ledger",
        }
    }

    /// The SSH subsystem that carries this frame; `None` for local-socket-only frames.
    pub const fn subsystem(&self) -> Option<Subsystem> {
        match self {
            Frame::Enroll { .. } | Frame::Enrolled { .. } => Some(Subsystem::Enroll),
            Frame::ReadLedger | Frame::Ledger { .. } => None,
            _ => Some(Subsystem::Runner),
        }
    }

    pub const fn direction(&self) -> Direction {
        match self {
            Frame::Enroll { .. }
            | Frame::Hello { .. }
            | Frame::Started { .. }
            | Frame::Refused { .. }
            | Frame::Event { .. }
            | Frame::Log { .. }
            | Frame::Finished { .. }
            | Frame::OutsideRuns { .. }
            | Frame::ReadLedger => Direction::RunnerToServer,
            Frame::Enrolled { .. }
            | Frame::Cert { .. }
            | Frame::Krl { .. }
            | Frame::Assign { .. }
            | Frame::Pack { .. }
            | Frame::Secrets { .. }
            | Frame::Approvals { .. }
            | Frame::Cancel { .. }
            | Frame::OutsideRunsAck { .. }
            | Frame::Ledger { .. } => Direction::ServerToRunner,
            Frame::RenewCert {} | Frame::Ping | Frame::Pong => Direction::Both,
        }
    }

    /// Whether `peer` may send this frame at all. A receiver refuses anything else.
    pub const fn may_be_sent_by(&self, peer: Peer) -> bool {
        matches!(
            (self.direction(), peer),
            (Direction::Both, _)
                | (Direction::RunnerToServer, Peer::Runner)
                | (Direction::ServerToRunner, Peer::Server)
        )
    }

    /// The run this frame belongs to, if any.
    pub fn run_id(&self) -> Option<&str> {
        match self {
            Frame::Assign { run_id, .. }
            | Frame::Pack { run_id, .. }
            | Frame::Secrets { run_id, .. }
            | Frame::Approvals { run_id, .. }
            | Frame::Started { run_id }
            | Frame::Refused { run_id, .. }
            | Frame::Event { run_id, .. }
            | Frame::Log { run_id, .. }
            | Frame::Cancel { run_id }
            | Frame::Finished { run_id, .. } => Some(run_id),
            _ => None,
        }
    }
}

/// Binary payloads are shown by length only; secrets through their redacting `Debug`.
impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Len(usize);
        impl fmt::Debug for Len {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "<{} bytes>", self.0)
            }
        }
        let mut d = f.debug_struct(self.name());
        match self {
            Frame::Enroll {
                token,
                inventory_name,
                allowed_profiles,
                require_signed,
            } => d
                .field("token", token)
                .field("inventory_name", inventory_name)
                .field("allowed_profiles", allowed_profiles)
                .field("require_signed", require_signed),
            Frame::Enrolled { host_id, cert } => d.field("host_id", host_id).field("cert", cert),
            Frame::Hello {
                host_id,
                version,
                ansible_core,
                python,
                allowed_profiles,
                require_signed,
                override_quorum,
                ledger_head,
                bases,
            } => d
                .field("host_id", host_id)
                .field("version", version)
                .field("ansible_core", ansible_core)
                .field("python", python)
                .field("allowed_profiles", allowed_profiles)
                .field("require_signed", require_signed)
                .field("override_quorum", override_quorum)
                .field("ledger_head", ledger_head)
                .field("bases", bases),
            Frame::RenewCert {} | Frame::Ping | Frame::Pong | Frame::ReadLedger => &mut d,
            Frame::Cert { cert } => d.field("cert", cert),
            Frame::Krl { bytes } => d.field("bytes", &Len(bytes.len())),
            Frame::Assign {
                run_id,
                workspace,
                binding,
                job,
                base,
                target,
                deadline,
                pack_size,
                quorum,
            } => d
                .field("run_id", run_id)
                .field("workspace", workspace)
                .field("binding", binding)
                .field("job", job)
                .field("base", base)
                .field("target", target)
                .field("deadline", deadline)
                .field("pack_size", pack_size)
                .field("quorum", quorum),
            Frame::Pack {
                run_id,
                chunk,
                last,
                sha256,
            } => d
                .field("run_id", run_id)
                .field("chunk", &Len(chunk.len()))
                .field("last", last)
                .field("sha256", sha256),
            Frame::Secrets { run_id, items } => d.field("run_id", run_id).field("items", items),
            Frame::Approvals { run_id, items } => d.field("run_id", run_id).field("items", items),
            Frame::Started { run_id } | Frame::Cancel { run_id } => d.field("run_id", run_id),
            Frame::Refused {
                run_id,
                reason,
                detail,
            } => d
                .field("run_id", run_id)
                .field("reason", reason)
                .field("detail", detail),
            Frame::Event { run_id, seq, json } => d
                .field("run_id", run_id)
                .field("seq", seq)
                .field("json", &Len(json.len())),
            Frame::Log {
                run_id,
                seq,
                stream,
                bytes,
            } => d
                .field("run_id", run_id)
                .field("seq", seq)
                .field("stream", stream)
                .field("bytes", &Len(bytes.len())),
            Frame::Finished {
                run_id,
                exit_code,
                stats,
                ledger_line,
            } => d
                .field("run_id", run_id)
                .field("exit_code", exit_code)
                .field("stats", stats)
                .field("ledger_line", ledger_line),
            Frame::OutsideRuns {
                entries,
                skipped,
                through,
            } => d
                .field("entries", entries)
                .field("skipped", skipped)
                .field("through", through),
            Frame::OutsideRunsAck { through } => d.field("through", through),
            Frame::Ledger { ledger_head, bases } => {
                d.field("ledger_head", ledger_head).field("bases", bases)
            }
        };
        d.finish()
    }
}

/// `OutsideRuns.entries`, refused past [`MAX_OUTSIDE_RUNS`] while decoding.
fn bounded_outside_runs<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<OutsideRun>, D::Error> {
    struct Bounded;
    impl<'de> serde::de::Visitor<'de> for Bounded {
        type Value = Vec<OutsideRun>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "at most {MAX_OUTSIDE_RUNS} outside runs")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(run) = seq.next_element::<OutsideRun>()? {
                if out.len() == MAX_OUTSIDE_RUNS {
                    return Err(serde::de::Error::invalid_length(out.len() + 1, &self));
                }
                out.push(run);
            }
            Ok(out)
        }
    }
    d.deserialize_seq(Bounded)
}

/// Standard base64 for `Vec<u8>` fields.
mod b64 {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        STANDARD
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}
