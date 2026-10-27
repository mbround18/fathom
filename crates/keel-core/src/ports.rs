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
