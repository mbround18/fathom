//! Changed paths between two commits (spec 055 T069): which files a push touched, so a job whose
//! `paths` globs match one of them runs. Renames count as both their old and new paths.

use std::collections::BTreeSet;

use gix::bstr::ByteSlice;
use keel_core::ports::CommitId;

use crate::mirror::{Mirror, MirrorError, object_id};

impl Mirror {
    /// The files that differ between `from` and `to` (added, removed or modified), as
    /// repository-relative paths. Directories are not listed, only the files in them. `from`
    /// `None` compares against the empty tree: every file in `to`.
    ///
    /// # Errors
    ///
    /// A commit is not in the mirror, or the trees cannot be read.
    pub fn changed_paths(
        &self,
        from: Option<&CommitId>,
        to: &CommitId,
    ) -> Result<BTreeSet<String>, MirrorError> {
        let repo = self.repo(&[])?;
        let tree = |c: &CommitId| -> Result<gix::Tree<'_>, MirrorError> {
            repo.find_commit(object_id(c)?)
                .map_err(|_| MirrorError::NotFound(c.0.clone()))?
                .tree()
                .map_err(|e| MirrorError::Repo(e.to_string()))
        };
        let new = tree(to)?;
        let old = from.map(tree).transpose()?;
        // Path tracking on, rename tracking off: a rename is a deletion and an addition.
        let changes = repo
            .diff_tree_to_tree(old.as_ref(), Some(&new), gix::diff::Options::default())
            .map_err(|e| MirrorError::Repo(e.to_string()))?;
        let mut out = BTreeSet::new();
        for change in &changes {
            if change.entry_mode().is_tree() {
                continue;
            }
            out.insert(change.location().to_str_lossy().into_owned());
            let source = change.source_location();
            if source != change.location() {
                out.insert(source.to_str_lossy().into_owned());
            }
        }
        Ok(out)
    }
}
