//! Payload types shared by commands and events.
//!
//! Every string a client can send is a validated newtype: it fails to *parse*, not just to
//! authorise, when it is out of bounds. That is defence in depth, not the check itself: the server
//! still resolves names through the fs adapter and authorises every command (contracts/protocol.md,
//! "Rules the server enforces"). Doc comments here are exported into the TypeScript bindings.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::wire::Container;

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

/// Longest stack, service or container name accepted: one path component (`NAME_MAX`).
pub const MAX_NAME_BYTES: usize = 255;
/// Longest subject in a user id. OIDC caps `sub` at 255 ASCII characters.
pub const MAX_SUBJECT_BYTES: usize = 255;
/// Longest CSRF token accepted. `web-auth` issues 43 (32 bytes, URL-safe base64).
pub const MAX_CSRF_TOKEN_BYTES: usize = 128;
/// Longest opaque audit cursor accepted.
pub const MAX_CURSOR_BYTES: usize = 256;
/// Longest RFC 3339 timestamp accepted.
pub const MAX_TIMESTAMP_BYTES: usize = 64;
/// Most log lines a client may ask to back-fill. The web keeps rings of 10,000 lines.
pub const MAX_LOG_TAIL: u32 = 10_000;
/// Largest base64 payload in one `ShellInput` (48 KiB of keystrokes or paste).
pub const MAX_SHELL_INPUT_B64_BYTES: usize = 64 * 1024;
/// Most targets in one `WatchStats`.
pub const MAX_STATS_TARGETS: usize = 64;
/// Most stacks in one `PutUser`.
pub const MAX_USER_STACKS: usize = 1024;
/// Most open streams (log follows, shells and the stats watch) on one socket (T090).
pub const MAX_STREAMS_PER_SOCKET: usize = 32;

// ---------------------------------------------------------------------------
// Validated strings
// ---------------------------------------------------------------------------

/// Why a value from the client was refused by the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidValue {
    pub what: &'static str,
    pub reason: &'static str,
}

impl fmt::Display for InvalidValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {}: {}", self.what, self.reason)
    }
}

impl std::error::Error for InvalidValue {}

/// A string newtype that serializes as a plain string and checks itself on the way in.
macro_rules! checked_string {
    ($(#[$meta:meta])* $name:ident, $what:literal, $check:expr) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Checks `value` against this type's rule.
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
                let value = value.into();
                let check: fn(&str) -> Result<(), &'static str> = $check;
                check(&value).map_err(|reason| InvalidValue { what: $what, reason })?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = InvalidValue;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

fn bounded(value: &str, max: usize) -> Result<(), &'static str> {
    if value.is_empty() {
        Err("empty")
    } else if value.len() > max {
        Err("too long")
    } else {
        Ok(())
    }
}

/// `first` for the first byte, `rest` for the others. A leading alphanumeric also keeps a name
/// from ever reading as an argv flag.
fn charset(value: &str, first: fn(u8) -> bool, rest: fn(u8) -> bool) -> Result<(), &'static str> {
    let mut bytes = value.bytes();
    match bytes.next() {
        Some(b) if first(b) => {}
        _ => return Err("bad first character"),
    }
    if bytes.all(rest) {
        Ok(())
    } else {
        Err("bad character")
    }
}

checked_string!(
    /// A stack (compose project) name: `^[a-z0-9][a-z0-9_-]*$`, at most 255 bytes.
    StackName,
    "stack name",
    |v| {
        bounded(v, MAX_NAME_BYTES)?;
        charset(
            v,
            |b| b.is_ascii_lowercase() || b.is_ascii_digit(),
            |b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-',
        )
    }
);

checked_string!(
    /// A compose service name: `^[a-zA-Z0-9][a-zA-Z0-9._-]*$`, at most 255 bytes.
    ServiceName,
    "service name",
    |v| {
        bounded(v, MAX_NAME_BYTES)?;
        charset(v, |b| b.is_ascii_alphanumeric(), |b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
        })
    }
);

checked_string!(
    /// A container id or name as docker accepts it: `^[a-zA-Z0-9][a-zA-Z0-9._-]*$`, at most 255
    /// bytes.
    ContainerRef,
    "container",
    |v| {
        bounded(v, MAX_NAME_BYTES)?;
        charset(v, |b| b.is_ascii_alphanumeric(), |b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
        })
    }
);

checked_string!(
    /// A users-registry key, `<provider>:<subject>`: `discord:<snowflake>`, `github:<numeric id>`
    /// or `oidc:<sub>`. Never a display name.
    UserId,
    "user id",
    |v| {
        let (provider, subject) = v.split_once(':').ok_or("missing provider")?;
        bounded(subject, MAX_SUBJECT_BYTES)?;
        match provider {
            "discord" | "github" if subject.bytes().all(|b| b.is_ascii_digit()) => Ok(()),
            "discord" | "github" => Err("subject must be a numeric id"),
            "oidc" if subject.bytes().all(|b| b.is_ascii_graphic()) => Ok(()),
            "oidc" => Err("bad character"),
            _ => Err("unknown provider"),
        }
    }
);

checked_string!(
    /// The session's CSRF token (URL-safe base64), at most 128 bytes.
    CsrfToken,
    "csrf token",
    |v| {
        bounded(v, MAX_CSRF_TOKEN_BYTES)?;
        charset(v, is_b64url, is_b64url)
    }
);

checked_string!(
    /// A stack's `content_hash`: lowercase sha256 hex.
    ContentHash,
    "content hash",
    |v| {
        if v.len() != 64 {
            return Err("not 64 hex digits");
        }
        charset(v, is_lower_hex, is_lower_hex)
    }
);

checked_string!(
    /// An RFC 3339 timestamp.
    Timestamp,
    "timestamp",
    |v| {
        bounded(v, MAX_TIMESTAMP_BYTES)?;
        chrono::DateTime::parse_from_rfc3339(v)
            .map(|_| ())
            .map_err(|_| "not RFC 3339")
    }
);

checked_string!(
    /// Where the next audit page starts. Opaque: pass back the server's `next_cursor`.
    AuditCursor,
    "audit cursor",
    |v| {
        bounded(v, MAX_CURSOR_BYTES)?;
        charset(v, is_b64url, is_b64url)
    }
);

checked_string!(
    /// Terminal bytes, standard base64 with padding, at most 64 KiB encoded.
    ShellBytes,
    "shell bytes",
    |v| {
        if v.len() > MAX_SHELL_INPUT_B64_BYTES {
            return Err("too long");
        }
        if v.len() % 4 != 0 {
            return Err("not padded base64");
        }
        let body = v.trim_end_matches('=');
        if v.len() - body.len() > 2 {
            return Err("not padded base64");
        }
        if body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
        {
            Ok(())
        } else {
            Err("bad character")
        }
    }
);

fn is_b64url(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn is_lower_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
}

/// How many lines of history a log stream starts with, at most 10,000.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(try_from = "u32", into = "u32")]
pub struct LogTail(u32);

impl LogTail {
    pub fn new(lines: u32) -> Result<Self, InvalidValue> {
        if lines > MAX_LOG_TAIL {
            return Err(InvalidValue {
                what: "log tail",
                reason: "more than 10000 lines",
            });
        }
        Ok(Self(lines))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for LogTail {
    type Error = InvalidValue;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<LogTail> for u32 {
    fn from(value: LogTail) -> Self {
        value.0
    }
}

/// Deserializes a list of at most `MAX` items.
pub(crate) fn at_most<'de, D, T, const MAX: usize>(de: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let items = Vec::<T>::deserialize(de)?;
    if items.len() > MAX {
        return Err(serde::de::Error::invalid_length(
            items.len(),
            &format!("at most {MAX} items").as_str(),
        ));
    }
    Ok(items)
}

// ---------------------------------------------------------------------------
// Ids
// ---------------------------------------------------------------------------

/// Client-chosen id of one command, echoed as `reply_to` on the events it produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct RequestId(#[ts(type = "number")] pub u64);

/// Single-use id of a preview (data-model "Preview").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct PreviewId(#[ts(type = "string")] pub Uuid);

/// Server-generated id of a running or finished operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct OperationId(#[ts(type = "string")] pub Uuid);

// ---------------------------------------------------------------------------
// People
// ---------------------------------------------------------------------------

/// Sign-in provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Oidc,
    Github,
    Discord,
}

/// Access level in the users registry. `pending` = signed in, no access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Pending,
    User,
    Admin,
}

/// Who did something, as the audit log and `Busy` name them. No tokens, no email.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Actor {
    pub provider: Provider,
    pub subject: String,
    pub username: String,
}

/// One users-registry entry (data-model "User").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct User {
    /// `<provider>:<subject>`.
    pub id: String,
    pub provider: Provider,
    /// Set for `oidc:` entries: a `sub` from another issuer never matches.
    pub issuer: Option<String>,
    pub subject: String,
    /// Refreshed at each sign-in; display only.
    pub username: Option<String>,
    pub email: Option<String>,
    pub role: Role,
    /// Assigned stacks; ignored for `admin`. A name with no directory is kept and shown missing.
    pub stacks: Vec<String>,
    /// From `DEPTHS_OWNER_USER_ID`; owners are always `admin` and cannot be demoted or removed.
    pub owner: bool,
    /// The `github:<login>` an owner entry was pinned from.
    pub pinned_from: Option<String>,
    /// `None` for an invited id that has not signed in yet.
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
}

/// The users list: assigned people and those waiting for access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UserList {
    pub users: Vec<User>,
    pub pending: Vec<User>,
}

/// Runtime feature flags (R12), as `GET /api/v1/me` reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Features {
    pub shell: bool,
    pub edit: bool,
    /// `WatchHost` is available: the server reads the host's `/proc` (`DEPTHS_HOST_PROC`) and
    /// this person is an admin. Always `false` for anyone else.
    pub host_metrics: bool,
    /// `DEPTHS_SHELL_POLICY`: who may open a shell by a container's security level.
    #[serde(default)]
    #[ts(as = "Option<ShellPolicy>", optional)]
    pub shell_policy: ShellPolicy,
    /// `DEPTHS_COMPOSE_POLICY`: whether escape-level compose findings block `up` and saves.
    #[serde(default)]
    #[ts(as = "Option<ComposePolicy>", optional)]
    pub compose_policy: ComposePolicy,
    /// How the docker daemon runs, for admins only; absent for anyone else, or when it could
    /// not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub daemon: Option<DaemonMode>,
}

// ---------------------------------------------------------------------------
// Container security
// ---------------------------------------------------------------------------

/// How contained a container is (`docs/guides/depths/security.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SecurityLevel {
    /// Not root on the host, nothing that reaches the host.
    Ok,
    /// uid 0 on a rootful daemon, nothing worse.
    Root,
    /// The container can reach the host: a shell in it is a shell on the host.
    Escape,
}

/// How much one finding weighs. `info` never raises the level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SecuritySeverity {
    Info,
    Root,
    Escape,
}

/// One finding: a machine-readable `code` (`privileged`, `runtime_socket`, `root_user`, ...)
/// and one sentence for a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SecurityReason {
    pub code: String,
    pub severity: SecuritySeverity,
    pub message: String,
}

/// Who may open a shell in a container under the server's policy, past the role check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ShellRule {
    /// Whoever the role table lets in.
    PerRole,
    AdminOnly,
    /// Nobody, whatever their role.
    Refused,
}

/// A container's containment, from `docker inspect` (cached by container id) and the daemon's
/// mode. The server re-inspects at shell open; this is for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ContainerSecurity {
    pub level: SecurityLevel,
    /// Worst first.
    pub reasons: Vec<SecurityReason>,
    pub shell: ShellRule,
}

/// A container in a stack, with its containment when it is known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StackContainer {
    #[serde(flatten)]
    #[ts(flatten)]
    pub container: Container,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub security: Option<ContainerSecurity>,
}

impl From<Container> for StackContainer {
    fn from(container: Container) -> Self {
        Self {
            container,
            security: None,
        }
    }
}

/// `DEPTHS_SHELL_POLICY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ShellPolicy {
    /// Escape: nobody. Root on a rootful daemon: admins. Otherwise per role.
    #[default]
    Strict,
    /// Escape: admins. Otherwise per role.
    Relaxed,
}

/// `DEPTHS_COMPOSE_POLICY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ComposePolicy {
    /// Findings are shown; nothing is blocked.
    #[default]
    Warn,
    /// `up` and saves with an escape-level finding need an admin's audited acknowledgement.
    Enforce,
}

/// How the docker daemon runs (`docker info` `SecurityOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DaemonMode {
    Rootless,
    /// `userns-remap` is on.
    Userns,
    Rootful,
}

/// One compose lint finding: what a service asks for that weakens containment. Warnings; the
/// file is never rewritten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ComposeWarning {
    pub service: String,
    pub code: String,
    pub severity: SecuritySeverity,
    pub message: String,
    /// 1-based line in the compose file, when found.
    pub line: Option<u32>,
}

/// The signed-in person as the UI shows them. No tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MeIdentity {
    pub provider: Provider,
    pub subject: String,
    pub username: String,
    pub email: Option<String>,
}

/// `GET /api/v1/me`. `role: pending` drives the no-access page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Me {
    pub identity: MeIdentity,
    pub csrf_token: String,
    pub role: Role,
    pub owner: bool,
    pub stacks: Vec<String>,
    pub features: Features,
    /// Whether the network of this request is trusted (spec 052).
    pub network: TrustDecision,
    /// Ask "home or public" for this network.
    pub needs_answer: bool,
    /// Admins only: every request so far came from a private address and no trusted proxy is
    /// configured, so the real client addresses are probably hidden (R7). Always `false` for
    /// anyone else.
    pub proxy_warning: bool,
}

// ---------------------------------------------------------------------------
// Stacks
// ---------------------------------------------------------------------------

/// The compose file a stack uses: the first present, in Compose's own order (R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ComposeFileName {
    #[serde(rename = "compose.yaml")]
    ComposeYaml,
    #[serde(rename = "compose.yml")]
    ComposeYml,
    #[serde(rename = "docker-compose.yml")]
    DockerComposeYml,
    #[serde(rename = "docker-compose.yaml")]
    DockerComposeYaml,
}

/// Derived stack status (R2). A container that exited 0 with restart policy `no` is completed
/// and never makes a stack partial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum StackStatus {
    Running,
    Partial,
    Stopped,
    NotDeployed,
    Invalid,
}

/// One service from the compose file, joined with its containers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ServiceSummary {
    pub name: String,
    pub image: Option<String>,
    pub containers: Vec<StackContainer>,
}

/// One stack (an immediate sub-directory of the stacks root holding a compose file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Stack {
    pub name: String,
    /// `false` when the directory name is not a valid compose project name: listed, never acted on.
    pub valid_name: bool,
    pub compose_file: ComposeFileName,
    /// Other compose file names present; non-empty means a warning (FR-004).
    pub shadowed_files: Vec<ComposeFileName>,
    pub has_env: bool,
    pub content_hash: String,
    pub status: StackStatus,
    /// Any container's health is `unhealthy`.
    pub unhealthy: bool,
    pub services: Vec<ServiceSummary>,
    pub running_services: u32,
    pub total_services: u32,
    /// Latest mtime of the compose file and `.env`, RFC 3339.
    pub modified_at: Option<String>,
    /// The I/O error when the directory or compose file cannot be read; never edited.
    pub read_error: Option<String>,
    /// Why the stack is `invalid`: the compose file's parse error, or the directory name not
    /// being a valid project name. `None` for a valid stack.
    pub invalid_reason: Option<String>,
    /// Set when Depths cannot write the files, or the stack directory is a symlink.
    pub read_only_reason: Option<String>,
    /// A compose or `.env` file exceeds `DEPTHS_MAX_FILE_BYTES`: shown, not loaded for editing.
    pub too_large: bool,
    /// The worst level among its containers whose containment is known; absent when none is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub security: Option<SecurityLevel>,
}

/// A compose project with no matching stack directory. Read-only; admins only (FR-008).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ForeignProject {
    pub name: String,
    pub working_dir: Option<String>,
    pub containers: Vec<Container>,
    /// The stack whose directory this project runs from under another name (`-p`).
    pub elsewhere_of: Option<String>,
}

/// What `ListStacks` returns, and every live `StacksChanged` after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StackList {
    pub root: String,
    pub stacks: Vec<Stack>,
    /// Empty for a `user`.
    pub foreign: Vec<ForeignProject>,
    /// Containers with no project label. Empty for a `user`.
    pub standalone: Vec<Container>,
    pub warnings: Vec<String>,
    /// The runtime stopped answering: this is the last known state (FR-010).
    pub stale: bool,
    pub as_of: String,
}

/// A stack's `.env` as sent to the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "text", rename_all = "snake_case")]
pub enum EnvText {
    /// Values of secret-looking keys (`PASS`, `SECRET`, `TOKEN`, `KEY`, `PRIVATE`, `CREDENTIAL`,
    /// case-insensitive) replaced. `RevealEnv` for the real text.
    Masked(String),
    /// The exact text (no secret-looking keys, or after `RevealEnv`).
    Plain(String),
}

/// A stack's files, for the editor (`GetStack`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StackDetail {
    pub stack: Stack,
    /// `None` when the stack is `too_large` or unreadable.
    pub compose_text: Option<String>,
    /// `None` when there is no `.env`, or it is too large to load.
    pub env: Option<EnvText>,
    pub content_hash: String,
    /// Compose lint of `compose_text`, worst first.
    #[serde(default)]
    #[ts(as = "Option<Vec<ComposeWarning>>", optional)]
    pub compose_warnings: Vec<ComposeWarning>,
}

/// An edit to save, or a stack to create (data-model "StackDraft").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StackDraft {
    pub stack: StackName,
    /// The `content_hash` the editor loaded; absent for create.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub base_hash: Option<ContentHash>,
    /// Exact text, written byte for byte.
    pub compose: String,
    /// Exact text. Absent leaves `.env` as it is; empty with no existing file creates none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub env: Option<String>,
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// Every operation kind (data-model "Operation").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Up,
    Stop,
    Restart,
    Down,
    Pull,
    Update,
    Save,
    Create,
    Delete,
    ServiceUp,
    ServiceStop,
    ServiceRestart,
}

/// What `PreviewOp` asks for: an operation kind with its options. Save and create go through
/// `PreviewSave` instead, because they carry a draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    content = "options",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum StackOp {
    Up,
    Stop,
    Restart,
    /// Both off by default; named in the preview when on (FR-015). Either one is admin-only.
    Down {
        #[serde(default)]
        remove_volumes: bool,
        #[serde(default)]
        remove_images: bool,
    },
    Pull,
    /// Pull, then up: one preview, two commands.
    Update,
    /// `down --remove-orphans`, then the directory only when `remove_directory`. Admin-only.
    Delete {
        #[serde(default)]
        remove_directory: bool,
    },
    ServiceUp {
        service: ServiceName,
    },
    ServiceStop {
        service: ServiceName,
    },
    ServiceRestart {
        service: ServiceName,
    },
}

impl StackOp {
    pub fn kind(&self) -> OperationKind {
        match self {
            Self::Up => OperationKind::Up,
            Self::Stop => OperationKind::Stop,
            Self::Restart => OperationKind::Restart,
            Self::Down { .. } => OperationKind::Down,
            Self::Pull => OperationKind::Pull,
            Self::Update => OperationKind::Update,
            Self::Delete { .. } => OperationKind::Delete,
            Self::ServiceUp { .. } => OperationKind::ServiceUp,
            Self::ServiceStop { .. } => OperationKind::ServiceStop,
            Self::ServiceRestart { .. } => OperationKind::ServiceRestart,
        }
    }
}

/// What a step reads on stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum StepStdin {
    /// The draft's compose text (`config --quiet -f -`).
    Draft,
    None,
}

/// One command a preview will run, exactly as it will run it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Step {
    /// The literal argv, program first.
    pub argv: Vec<String>,
    pub cwd: String,
    pub stdin: StepStdin,
}

/// What an operation affects. Volumes only when chosen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Reach {
    pub containers: Vec<String>,
    pub networks: Vec<String>,
    pub volumes: Vec<String>,
}

/// What an operation will do, before it does it. `Confirm` with `preview_id` to run it.
///
/// The server binds the preview to the session, the stack's `content_hash` and `expires_at`
/// (5 min); the session id stays on the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Preview {
    pub preview_id: PreviewId,
    pub stack: String,
    pub kind: OperationKind,
    pub steps: Vec<Step>,
    /// Unified diff, for save, create and delete.
    pub file_diff: Option<String>,
    pub reaches: Reach,
    pub destructive: bool,
    pub stack_hash: String,
    pub expires_at: String,
    /// Compose lint of what this operation deploys or writes, worst first.
    #[serde(default)]
    #[ts(as = "Option<Vec<ComposeWarning>>", optional)]
    pub warnings: Vec<ComposeWarning>,
    /// `DEPTHS_COMPOSE_POLICY=enforce` and an escape-level warning: `Confirm` needs
    /// `acknowledge_findings`, which only an admin may give, and it is audited.
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub ack_required: bool,
}

/// Which file of a draft a validation error is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DraftFile {
    Compose,
    Env,
}

/// One error from validating a draft. `line` when Compose gives one; `file` says which of the
/// draft's files it is in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ValidationError {
    pub file: DraftFile,
    pub line: Option<u32>,
    pub message: String,
}

/// Why a `Confirm` found its preview void.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PreviewVoidReason {
    /// No such preview for this session. Also what another session's id gets: existence is not
    /// disclosed.
    Unknown,
    /// Already confirmed once.
    Used,
    Expired,
    /// The stack's files changed since the preview.
    StackChanged,
}

/// How an operation ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpOutcome {
    Succeeded,
    /// Docker's stderr, verbatim (FR-013). `exit_code` is absent when the process was killed.
    Failed {
        exit_code: Option<i32>,
        stderr: String,
    },
    /// `Cancel` sent SIGINT.
    Cancelled,
}

/// Why a command was refused before anything ran. Audited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    /// The person's role or stack list does not cover this command.
    NotPermitted,
    /// Signed in, not yet given access.
    Pending,
    /// The feature flag for this command is off (`DEPTHS_FEATURE_SHELL`, `DEPTHS_FEATURE_EDIT`).
    FeatureDisabled,
    /// Owners cannot be demoted or removed.
    Owner,
    /// `PutUser` may only assign `user` or `admin`.
    RoleNotAssignable,
    /// The request came from a network not trusted for this person (spec 052): only the
    /// restricted view is allowed. `GET /api/v1/me` `network` says why and when it may mature.
    UntrustedNetwork,
    /// 051: the container can reach the host and the shell policy refuses it to this person
    /// (strict: everyone; relaxed: everyone but admins). `detail` names the findings.
    ShellEscape,
    /// 051: the container runs as root on a rootful daemon and strict keeps it to admins.
    ShellRootAdminOnly,
    /// 051: the container's configuration could not be read at the moment of the request, so the
    /// shell is refused (fail closed).
    ShellUnverifiable,
    /// 051: `DEPTHS_COMPOSE_POLICY=enforce` and an escape-level finding: only an admin with
    /// `acknowledge_findings` may save or deploy it. `detail` names the findings.
    ComposeBlocked,
}

// ---------------------------------------------------------------------------
// Streams
// ---------------------------------------------------------------------------

/// What a log stream follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LogTarget {
    /// `docker compose logs`, of one service or the whole stack.
    Stack {
        stack: StackName,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        service: Option<ServiceName>,
    },
    /// `docker logs` of one container. Admin-only for a container outside every stack.
    Container { container: ContainerRef },
}

/// What `WatchStats` samples.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StatsTarget {
    /// Every container of the stack.
    Stack {
        stack: StackName,
    },
    Container {
        container: ContainerRef,
    },
}

/// The shell `OpenShell` runs in the container. Absent: `/bin/sh`, which every image with a shell has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Shell {
    Sh,
    Bash,
    Ash,
}

/// Why a stream closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum StreamCloseReason {
    /// The process ended (container stopped, shell exited).
    Ended,
    /// `CloseStream`.
    Closed,
    /// The connection dropped (proxy idle cut, R8): reopen with backoff.
    Disconnected,
    /// The stream could not continue. Carries the runtime's message.
    Failed(String),
    /// 051 FR-014: the shell's container was recreated (a new container under its name).
    ContainerRecreated,
    /// 051 FR-014: the shell's container is gone.
    ContainerRemoved,
}

// ---------------------------------------------------------------------------
// Audit
// ---------------------------------------------------------------------------

/// What an audit entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    Up,
    Stop,
    Restart,
    Down,
    Pull,
    Update,
    Save,
    Create,
    Delete,
    ServiceUp,
    ServiceStop,
    ServiceRestart,
    SignIn,
    SignOut,
    Denied,
    ShellOpen,
    RevealEnv,
    UserAdded,
    UserChanged,
    UserRemoved,
    /// A person's first network, trusted (spec 052).
    NetworkFirst,
    /// A network not seen before for this person.
    NetworkSeen,
    /// The person answered "home or public".
    NetworkAnswer,
    NetworkWhitelistAdd,
    NetworkWhitelistRemove,
    NetworkForget,
    /// A session was ended from the sessions page or by an admin.
    SessionEnd,
    /// A command refused because the network is untrusted.
    NetworkRefused,
}

impl From<OperationKind> for AuditAction {
    fn from(kind: OperationKind) -> Self {
        match kind {
            OperationKind::Up => Self::Up,
            OperationKind::Stop => Self::Stop,
            OperationKind::Restart => Self::Restart,
            OperationKind::Down => Self::Down,
            OperationKind::Pull => Self::Pull,
            OperationKind::Update => Self::Update,
            OperationKind::Save => Self::Save,
            OperationKind::Create => Self::Create,
            OperationKind::Delete => Self::Delete,
            OperationKind::ServiceUp => Self::ServiceUp,
            OperationKind::ServiceStop => Self::ServiceStop,
            OperationKind::ServiceRestart => Self::ServiceRestart,
        }
    }
}

/// A role and stack list, before or after a users change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Grant {
    pub role: Role,
    pub stacks: Vec<String>,
}

/// The target of a `user_added`, `user_changed` or `user_removed` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UserChange {
    pub id: String,
    pub before: Option<Grant>,
    pub after: Option<Grant>,
}

/// How an audited action ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuditOutcome {
    Succeeded,
    Failed {
        exit_code: Option<i32>,
        stderr_tail: String,
    },
    Denied {
        reason: String,
    },
}

/// One audit-log line. Never holds `.env` values or file contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AuditEntry {
    /// RFC 3339 UTC.
    pub at: String,
    pub actor: Actor,
    pub action: AuditAction,
    pub stack: Option<String>,
    pub preview_id: Option<PreviewId>,
    pub operation_id: Option<OperationId>,
    /// The steps run, each an argv.
    pub argv: Vec<Vec<String>>,
    /// For users changes.
    pub target: Option<UserChange>,
    /// For saves: files and lines changed, `.env` lines reduced to their keys.
    pub diff_summary: Option<String>,
    pub outcome: AuditOutcome,
    pub source_ip: String,
    /// For network and session entries (spec 052). Never a location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub network: Option<AuditNetwork>,
    /// Shell opens and refusals, and enforced deploys: the level and reason codes that decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub security: Option<AuditSecurity>,
}

/// What an audit entry records of a container's or compose file's containment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AuditSecurity {
    pub level: SecurityLevel,
    pub reasons: Vec<String>,
    /// An admin acknowledged the risk ("deploy anyway").
    #[serde(default)]
    pub acknowledged: bool,
}

/// `GetAudit` filter. Every field narrows; absent means any.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AuditFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stack: Option<StackName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub actor: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub action: Option<AuditAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub to: Option<Timestamp>,
}

// ---------------------------------------------------------------------------
// Host metrics
// ---------------------------------------------------------------------------

/// One host metrics tick (`WatchHost`, admin only). Every figure is `None` when the server could
/// not read it, rather than a misleading zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct HostSample {
    /// RFC 3339.
    pub at: String,
    /// Busy share of all CPUs since the previous tick, 0 to 100.
    pub cpu_percent: Option<f32>,
    /// Logical CPUs.
    pub cpu_cores: Option<u32>,
    pub load: Option<HostLoad>,
    pub memory: Option<HostMemory>,
    pub swap: Option<HostSwap>,
    #[ts(type = "number | null")]
    pub uptime_secs: Option<u64>,
    /// The filesystem holding the stacks directory.
    pub disk: Option<HostDisk>,
    /// From the stack inventory: every container Docker reports, running or not.
    pub containers_running: u32,
    pub containers_stopped: u32,
}

/// Load average over 1, 5 and 15 minutes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
pub struct HostLoad {
    pub one: f32,
    pub five: f32,
    pub fifteen: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostMemory {
    #[ts(type = "number")]
    pub total_bytes: u64,
    #[ts(type = "number")]
    pub available_bytes: u64,
    /// `total - available`.
    #[ts(type = "number")]
    pub used_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostSwap {
    #[ts(type = "number")]
    pub total_bytes: u64,
    #[ts(type = "number")]
    pub free_bytes: u64,
    #[ts(type = "number")]
    pub used_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HostDisk {
    #[ts(type = "number")]
    pub total_bytes: u64,
    #[ts(type = "number")]
    pub used_bytes: u64,
    /// Free to an unprivileged writer (`f_bavail`).
    #[ts(type = "number")]
    pub free_bytes: u64,
}

// ---------------------------------------------------------------------------
// Network trust and sessions (spec 052)
// ---------------------------------------------------------------------------

/// Why a network is or is not trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TrustReason {
    /// In the operator's `DEPTHS_TRUSTED_NETWORKS`.
    Always,
    /// The person's first network.
    First,
    /// An admin whitelisted it.
    Whitelist,
    /// Answered "home" and used long enough, on at least two days.
    Matured,
    /// `DEPTHS_NETWORK_GATE=false`.
    GateOff,
    /// Not answered yet.
    Unknown,
    /// Answered "public": never matures.
    Public,
    /// Answered "home", not matured yet.
    Maturing,
}

/// The gate's decision for one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TrustDecision {
    pub trusted: bool,
    pub reason: TrustReason,
    /// RFC 3339: when a maturing "home" network's window ends (two distinct days are needed too).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub matures_at: Option<String>,
    /// The network, as a CIDR like `203.0.113.7/32`.
    pub network: String,
}

/// "Home or public".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAnswer {
    Home,
    Public,
    None,
}

/// Where a remembered network came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NetworkSource {
    First,
    Answer,
    Whitelist,
    Seen,
}

/// One remembered network, as `GET /api/v1/networks` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NetworkView {
    /// CIDR.
    pub key: String,
    /// RFC 3339.
    pub first_used: String,
    pub last_used: String,
    /// Distinct UTC days of use, `YYYY-MM-DD`, latest 8.
    pub days: Vec<String>,
    pub source: NetworkSource,
    pub answer: NetworkAnswer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub whitelisted_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub at: Option<String>,
    /// Admin label, plain text, at most 64 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub label: Option<String>,
    pub trusted: bool,
    pub reason: TrustReason,
    /// City, region, country from the optional offline database; `private network`, or
    /// `unknown`. Display only.
    pub location: String,
    /// The network of this request.
    pub current: bool,
}

/// One live session, as `GET /api/v1/sessions` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SessionView {
    /// A short non-reversible hash of the session handle; never the cookie.
    pub id_hint: String,
    /// RFC 3339.
    pub created: String,
    pub last_used: String,
    pub address: String,
    /// CIDR of `address`.
    pub network: String,
    pub trusted: bool,
    pub location: String,
    /// Plain text, like `Firefox on Linux`.
    pub browser: String,
    /// The session making this request.
    pub current: bool,
}

/// `POST /api/v1/networks/answer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AnswerNetwork {
    /// CIDR of a network the person used.
    pub network: String,
    pub answer: NetworkAnswer,
}

/// `POST /api/v1/users/{id}/networks/whitelist`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WhitelistNetwork {
    /// A CIDR like `198.51.100.0/24`, or one address.
    pub cidr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub label: Option<String>,
}

/// `POST /api/v1/sessions/end-others` and the admin `DELETE /api/v1/users/{id}/sessions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SessionsEnded {
    pub ended: u32,
}

/// The HTTP body of a request refused for its network: status 403.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UntrustedNetworkRefusal {
    /// Always `untrusted_network`.
    pub error: String,
    pub network: String,
    pub reason: TrustReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub matures_at: Option<String>,
}

/// The network part of an audit entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AuditNetwork {
    /// The network involved, as a CIDR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub network: Option<String>,
    /// `network_refused`: the command or route refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cmd: Option<String>,
    /// Whose network or session, when not the actor's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub person: Option<String>,
    /// `session_end`: the ended session's `id_hint`, or `others` / `all`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session: Option<String>,
    /// `network_answer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub answer: Option<NetworkAnswer>,
}

// ---------------------------------------------------------------------------
// Files (spec 053)
// ---------------------------------------------------------------------------

pub use depths_volumes::mount::{
    Browsable, Entry, EntryKind, LinkInfo, Mount, MountId, MountKind, MountService, MountState,
    NotBrowsable, NotBrowsableCode, Owner, WriteBlocked,
};
pub use depths_volumes::path::{Name, RawName, RelPath};
pub use depths_volumes::text::{BinaryReason, Eol, FileContent, Fingerprint, LineEnding};

/// Most files in one `Upload` preview.
pub const MAX_UPLOAD_FILES: usize = 1000;
/// Longest history version id accepted.
pub const MAX_VERSION_ID_BYTES: usize = 96;

checked_string!(
    /// A history version: the blob's sha256 hex, `-`, and a sequence number. Opaque: pass back
    /// the server's `FileVersion::id`.
    VersionId,
    "version id",
    |v| {
        bounded(v, MAX_VERSION_ID_BYTES)?;
        let (blob, seq) = v.split_once('-').ok_or("missing sequence")?;
        if blob.len() != 64 || !blob.bytes().all(is_lower_hex) {
            return Err("not 64 hex digits");
        }
        if seq.is_empty() || !seq.bytes().all(|b| b.is_ascii_digit()) {
            return Err("bad sequence");
        }
        Ok(())
    }
);

/// What `Create` makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CreateKind {
    File,
    Dir,
}

/// One file of an `Upload` preview, as the browser sees it before sending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct UploadFile {
    pub name: Name,
    pub size: u64,
}

/// A change to a mount's files, previewed with `PreviewFileOp` and applied by `Confirm`
/// (constitution I). The mount is always one the server listed; a rename cannot cross mounts.
///
/// Tagged like [`StackOp`]: `{ "kind": "save", "options": { ... } }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    content = "options",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FileOp {
    /// `text` is LF text from the editor; the server re-applies `bom` and `eol`. `base` is the
    /// fingerprint `ReadFile` returned: a different file on disk is `changed_on_disk`.
    Save {
        mount: MountId,
        path: RelPath,
        text: String,
        base: Fingerprint,
        bom: bool,
        eol: LineEnding,
    },
    Create {
        mount: MountId,
        parent: RelPath,
        name: Name,
        kind: CreateKind,
    },
    Rename {
        mount: MountId,
        from: RelPath,
        to: RelPath,
    },
    /// A folder is deleted with everything in it. `raw_name` addresses a last component that is
    /// not UTF-8 (`path` is then its parent).
    Delete {
        mount: MountId,
        path: RelPath,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        raw_name: Option<RawName>,
    },
    Restore {
        mount: MountId,
        path: RelPath,
        version: VersionId,
    },
    /// The bytes follow over HTTP, one POST per file carrying the confirmed preview's id.
    Upload {
        mount: MountId,
        dir: RelPath,
        #[serde(deserialize_with = "at_most::<_, _, MAX_UPLOAD_FILES>")]
        files: Vec<UploadFile>,
    },
}

/// One file an `Upload` preview would write: `exists` names a file it would replace, with the
/// fingerprint the POST must still find.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UploadTarget {
    pub name: Name,
    pub size: u64,
    pub exists: bool,
    pub fingerprint: Option<Fingerprint>,
}

/// What a file change would do. `Confirm` with `preview_id` applies it; it expires at
/// `expires_at` or when the session ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FilePreview {
    pub preview_id: PreviewId,
    pub stack: String,
    pub mount: MountId,
    /// One line: what changes, where.
    pub summary: String,
    /// Unified diff, for `Save` and `Restore`.
    pub diff: Option<String>,
    /// `Delete` of a folder: how many entries go with it, counted up to a cap.
    pub entries: Option<u64>,
    /// The count stopped at the cap ("10,000+").
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub entries_capped: bool,
    /// `Upload`: every file, and whether it replaces one.
    #[serde(default)]
    #[ts(as = "Option<Vec<UploadTarget>>", optional)]
    pub uploads: Vec<UploadTarget>,
    /// Services with a running container that mounts this mount (FR-010).
    pub running: Vec<String>,
    pub expires_at: String,
}

/// Why a version was kept: the operation that replaced its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum VersionReason {
    Save,
    Upload,
    Delete,
    Restore,
}

/// A kept earlier version of a file (FR-015).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FileVersion {
    pub id: VersionId,
    pub at: String,
    /// The user id of whoever replaced it.
    pub author: String,
    pub size: u64,
    pub reason: VersionReason,
}

/// A file command refused for a reason of its own. Role, trust and a feature that is off are
/// `Denied` (`not_permitted`, `untrusted_network`, `feature_disabled`), as for every other command.
/// Audited as `files.refused`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileRefusal {
    /// `DEPTHS_FEATURE_FILES=read_only`: nothing may change.
    FeatureReadOnly,
    /// The helper cannot run on this server; the reason is shown to admins.
    HelperUnavailable {
        reason: String,
    },
    /// tmpfs, a root or escape bind, or a single-file bind.
    NotBrowsable {
        reason: NotBrowsable,
    },
    /// Every service mounts it read-only.
    ReadOnlyMount,
    /// A named volume that does not exist yet (before the first deploy).
    NotCreated,
    /// The volume or path is gone.
    Missing,
    /// The path leaves the mount (a link out, or a magic link).
    OutsideMount,
    WriteBlocked {
        reason: WriteBlocked,
    },
    /// The file changed since it was read; `current` is what is there now.
    ChangedOnDisk {
        current: Fingerprint,
    },
    /// The owner changed since the listing; reload.
    OwnerChanged,
    /// Over the editor, download or upload limit.
    TooLarge {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        bytes: Option<u64>,
    },
    NotText,
    Exists,
    NotFound,
    InvalidName,
    /// A rename between folders with different owners: no single uid can do it.
    MixedOwners,
    /// An upload without a confirmed preview, or a name or size the preview did not name.
    NotConfirmed,
    /// A folder delete stopped part way: what was removed, and the system error verbatim.
    Partial {
        removed: u64,
        failed_at: String,
        errno: String,
        message: String,
    },
    /// The system error verbatim (`EACCES`, `ENOSPC`, ...).
    OsError {
        errno: String,
        message: String,
    },
}
