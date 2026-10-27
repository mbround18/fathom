//! Run pins (spec 055 T045, data-model `runs`): creating a run writes `refs/keel/runs/<run id>`
//! in the binding's mirror, so the commit it ran survives a force-push upstream and `git gc`.

use gix::refs::transaction::PreviousValue;
use keel_core::ports::CommitId;
use uuid::Uuid;

use crate::mirror::{Mirror, MirrorError, object_id};

impl Mirror {
    /// The pin ref for a run.
    #[must_use]
    pub fn pin_ref(run: Uuid) -> String {
        format!("refs/keel/runs/{run}")
    }

    /// Pins `commit` for `run`. Pinning the same commit again is a no-op; repinning a run to a
    /// different commit is refused (a run's commit never changes).
    ///
    /// # Errors
    ///
    /// The commit is not in the mirror, or the run is already pinned elsewhere.
    pub fn pin(&self, run: Uuid, commit: &CommitId) -> Result<(), MirrorError> {
        let id = object_id(commit)?;
        let repo = self.repo(&[])?;
        repo.find_commit(id)
            .map_err(|_| MirrorError::NotFound(commit.0.clone()))?;
        if let Some(existing) = self.pinned(run)? {
            return if existing == *commit {
                Ok(())
            } else {
                Err(MirrorError::Repo(format!("run {run} is already pinned")))
            };
        }
        repo.reference(
            Self::pin_ref(run).as_str(),
            id,
            PreviousValue::MustNotExist,
            "keel: pin run",
        )
        .map_err(|e| MirrorError::Repo(e.to_string()))?;
        Ok(())
    }

    /// The commit `run` is pinned to, if any.
    ///
    /// # Errors
    ///
    /// The mirror cannot be read.
    pub fn pinned(&self, run: Uuid) -> Result<Option<CommitId>, MirrorError> {
        match self.resolve_ref(&Self::pin_ref(run)) {
            Ok(c) => Ok(Some(c)),
            Err(MirrorError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Removes `run`'s pin (run retention). Absent pins are fine.
    ///
    /// # Errors
    ///
    /// The ref exists and cannot be deleted.
    pub fn unpin(&self, run: Uuid) -> Result<(), MirrorError> {
        let repo = self.repo(&[])?;
        let Some(r) = repo
            .try_find_reference(Self::pin_ref(run).as_str())
            .map_err(|e| MirrorError::Repo(e.to_string()))?
        else {
            return Ok(());
        };
        r.delete().map_err(|e| MirrorError::Repo(e.to_string()))
    }
}
