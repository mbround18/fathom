//! Ports: the interfaces Keel's core needs from the outside world (spec 055, T017). Adapters live
//! in the server (git, secrets, the database); the core never does I/O itself.

use std::sync::Mutex;

use time::OffsetDateTime;

use crate::domain::{Origin, WorkspaceId};

/// The current time. Inject [`SystemClock`] in production and [`TestClock`] in tests, so TTLs and
/// expiry are deterministic.
pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}

/// The wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// A clock that only moves when told to.
#[derive(Debug)]
pub struct TestClock(Mutex<OffsetDateTime>);

impl TestClock {
    #[must_use]
    pub fn new(at: OffsetDateTime) -> Self {
        Self(Mutex::new(at))
    }

    pub fn advance(&self, by: time::Duration) {
        let mut now = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *now += by;
    }

    pub fn set(&self, at: OffsetDateTime) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = at;
    }
}

impl Clock for TestClock {
    fn now(&self) -> OffsetDateTime {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// A git commit id (hex SHA-1 or SHA-256).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CommitId(pub String);

/// Errors a [`GitSource`] reports.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("ref not found: {0}")]
    RefNotFound(String),
    #[error("path not found at {commit}: {path}")]
    PathNotFound { commit: String, path: String },
    #[error("git source unavailable: {0}")]
    Unavailable(String),
}

/// Read access to a bound repository's mirror.
pub trait GitSource: Send + Sync {
    /// Resolves a branch to its current commit.
    fn resolve(&self, branch: &str) -> Result<CommitId, GitError>;
    /// Reads one file at a commit.
    fn read_file(&self, commit: &CommitId, path: &str) -> Result<Vec<u8>, GitError>;
}

/// Errors a [`SecretResolver`] reports. Never carries the secret value.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("credential not found")]
    NotFound,
    #[error("credential not granted to this workspace")]
    NotGranted,
    #[error("credential unavailable: {0}")]
    Unavailable(String),
}

/// Resolves a credential name to its plaintext for one run. The caller holds the value only for
/// as long as the run needs it and never logs it.
pub trait SecretResolver: Send + Sync {
    fn resolve(&self, workspace: WorkspaceId, name: &str) -> Result<Vec<u8>, SecretError>;
}

/// Where suite-level policy comes from. Standalone Keel owns all of it locally.
pub trait SuitePolicy: Send + Sync {
    /// The origin new teams, memberships and grants get.
    fn origin(&self) -> Origin;
    /// Whether local edits to identity and grants are allowed (always, when standalone).
    fn local_edits_allowed(&self) -> bool;
}

/// The standalone adapter: everything is local.
#[derive(Debug, Default, Clone, Copy)]
pub struct Standalone;

impl SuitePolicy for Standalone {
    fn origin(&self) -> Origin {
        Origin::Local
    }

    fn local_edits_allowed(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clock_moves_only_when_told() {
        let start = OffsetDateTime::UNIX_EPOCH;
        let c = TestClock::new(start);
        assert_eq!(c.now(), start);
        c.advance(time::Duration::minutes(5));
        assert_eq!(c.now(), start + time::Duration::minutes(5));
    }

    #[test]
    fn standalone_is_local() {
        assert_eq!(Standalone.origin(), Origin::Local);
        assert!(Standalone.local_edits_allowed());
    }
}

/// A boxed future, so [`Forge`] stays object-safe without an async-trait dependency.
pub type ForgeFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, ForgeError>> + Send + 'a>>;

/// One file a [`ChangeRequest`] writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEdit {
    /// Repository-relative path.
    pub path: String,
    /// The blob the edit was computed against; `None` creates the file. The forge refuses the
    /// write when the file has changed since, so Keel never overwrites what it has not seen.
    pub expected_blob_sha: Option<String>,
    pub new_content: Vec<u8>,
}

/// A change proposed as a pull request on a new branch (spec 066 R8). Never a push to an existing
/// branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeRequest {
    pub base_branch: String,
    pub new_branch: String,
    pub title: String,
    pub body: String,
    pub files: Vec<FileEdit>,
}

/// The pull request a [`Forge`] opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedChange {
    /// The pull request's web URL.
    pub url: String,
    pub branch: String,
    pub number: u64,
}

/// What a [`Forge::probe`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeAccess {
    /// The token may write to the repository (`permissions.push`).
    pub can_push: bool,
    pub default_branch: String,
}

/// Errors a [`Forge`] reports. Never carries the token.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ForgeError {
    /// The binding URL names no owner and repository the forge can be asked about.
    #[error("cannot tell the repository from {0:?}")]
    Url(String),
    #[error("the forge rejected the token")]
    Unauthorized,
    #[error("the forge has no such repository, branch or file")]
    NotFound,
    /// A file changed since the edit was computed (the `sha` precondition failed), or the branch
    /// already exists.
    #[error("a file changed on the forge since the edit was made: {0}")]
    Conflict(String),
    /// Any other refusal.
    #[error("the forge refused ({status}): {message}")]
    Refused { status: u16, message: String },
    #[error("could not reach the forge: {0}")]
    Unavailable(String),
    #[error("unexpected forge response: {0}")]
    Response(String),
}

impl ForgeError {
    /// The protocol refusal code: every forge failure is `forge_refused`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        "forge_refused"
    }
}

/// A code forge's API: Forgejo/Gitea or GitHub (adapters in `keel-forge`).
pub trait Forge: Send + Sync {
    /// Reads the repository and the token's permissions on it.
    fn probe(&self) -> ForgeFuture<'_, ForgeAccess>;
    /// Creates `new_branch` from `base_branch`, writes the files with their `sha` preconditions,
    /// and opens a pull request.
    fn open_change<'a>(&'a self, change: &'a ChangeRequest) -> ForgeFuture<'a, OpenedChange>;
}
