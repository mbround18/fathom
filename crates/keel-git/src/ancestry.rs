//! Ancestry of a fetched head against the binding's previous one (spec 055 T045, research R5,
//! data-model `deliveries.sync_state`).

use keel_core::ports::CommitId;

use crate::mirror::{Mirror, MirrorError, object_id};

/// How a new head relates to the previous one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ancestry {
    /// No previous head: the first sync.
    First,
    /// The previous head is an ancestor of the new one.
    FastForward,
    /// The same commit.
    Same,
    /// The new head is an ancestor of the previous one (an older delivery arrived late).
    Behind,
    /// Neither descends from the other: history was rewritten (force-push).
    Rewritten,
}

impl Ancestry {
    /// The `deliveries.sync_state` this maps to.
    #[must_use]
    pub const fn sync_state(self) -> &'static str {
        match self {
            Ancestry::First | Ancestry::FastForward => "synced",
            Ancestry::Same => "duplicate",
            Ancestry::Behind => "stale",
            Ancestry::Rewritten => "history_rewritten",
        }
    }
}

impl Mirror {
    /// Classifies `new` against `previous`. A previous head no longer in the mirror counts as
    /// rewritten.
    ///
    /// # Errors
    ///
    /// `new` is not in the mirror, or the graph cannot be walked.
    pub fn classify(
        &self,
        previous: Option<&CommitId>,
        new: &CommitId,
    ) -> Result<Ancestry, MirrorError> {
        let Some(previous) = previous else {
            return Ok(Ancestry::First);
        };
        if previous == new {
            return Ok(Ancestry::Same);
        }
        let repo = self.repo(&[])?;
        let new_id = object_id(new)?;
        repo.find_commit(new_id)
            .map_err(|_| MirrorError::NotFound(new.0.clone()))?;
        let Ok(prev_id) = object_id(previous) else {
            return Ok(Ancestry::Rewritten);
        };
        if repo.find_commit(prev_id).is_err() {
            return Ok(Ancestry::Rewritten);
        }
        let base = repo
            .merge_base(prev_id, new_id)
            .map_err(|e| MirrorError::Repo(e.to_string()))?
            .map(gix::Id::detach);
        Ok(match base {
            Some(b) if b == prev_id => Ancestry::FastForward,
            Some(b) if b == new_id => Ancestry::Behind,
            _ => Ancestry::Rewritten,
        })
    }
}
