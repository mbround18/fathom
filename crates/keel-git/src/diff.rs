//! Changed paths between two commits (spec 055 T069): which files a push touched, so a job whose
//! `paths` globs match one of them runs. Renames count as both their old and new paths.
//!
//! Signed-mode review (spec 055 T100) reads the same mirror: raw commit objects for the
//! signature check, each commit's people and summary, the changed files with how they changed,
//! and one file's unified diff.

use std::collections::BTreeSet;

use gix::bstr::ByteSlice;
use gix::diff::blob::unified_diff::{ConsumeBinaryHunk, ContextSize};
use gix::diff::blob::{Algorithm, InternedInput, UnifiedDiff, diff_with_slider_heuristics};
use keel_core::ports::CommitId;

use crate::mirror::{Mirror, MirrorError, object_id};

/// The largest blob a review diffs; a bigger file is shown as binary.
pub const MAX_DIFF_BLOB_BYTES: usize = 1024 * 1024;

/// How a file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
    Deleted,
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub change: Change,
}

/// One commit's people and summary, as a review lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMeta {
    /// `Name <email>`.
    pub author: String,
    pub author_email: String,
    /// `Name <email>`.
    pub committer: String,
    pub summary: String,
}

/// One file's unified diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// Either side is binary (has a NUL byte) or too large; `lines` is empty.
    pub binary: bool,
    /// `@@` hunk headers and `+`, `-` or ` ` prefixed lines, without newlines.
    pub lines: Vec<String>,
}

fn repo_err(e: impl std::fmt::Display) -> MirrorError {
    MirrorError::Repo(e.to_string())
}

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
        Ok(self
            .changed_files(from, to)?
            .into_iter()
            .map(|f| f.path)
            .collect())
    }

    /// The files that differ between `from` and `to`, with how they changed, sorted by path. A
    /// rename is a deletion and an addition. `from` `None` lists every file of `to` as added.
    ///
    /// # Errors
    ///
    /// A commit is not in the mirror, or the trees cannot be read.
    pub fn changed_files(
        &self,
        from: Option<&CommitId>,
        to: &CommitId,
    ) -> Result<Vec<ChangedFile>, MirrorError> {
        let repo = self.repo(&[])?;
        let tree = |c: &CommitId| -> Result<gix::Tree<'_>, MirrorError> {
            repo.find_commit(object_id(c)?)
                .map_err(|_| MirrorError::NotFound(c.0.clone()))?
                .tree()
                .map_err(repo_err)
        };
        let new = tree(to)?;
        let old = from.map(tree).transpose()?;
        // Path tracking on, rename tracking off: a rename is a deletion and an addition.
        let changes = repo
            .diff_tree_to_tree(old.as_ref(), Some(&new), gix::diff::Options::default())
            .map_err(repo_err)?;
        let mut out = Vec::new();
        for change in &changes {
            use gix::object::tree::diff::ChangeDetached as C;
            if change.entry_mode().is_tree() {
                continue;
            }
            let path = change.location().to_str_lossy().into_owned();
            match change {
                C::Addition { .. } => out.push(ChangedFile {
                    path,
                    change: Change::Added,
                }),
                C::Deletion { .. } => out.push(ChangedFile {
                    path,
                    change: Change::Deleted,
                }),
                C::Modification { .. } => out.push(ChangedFile {
                    path,
                    change: Change::Modified,
                }),
                C::Rewrite {
                    source_location, ..
                } => {
                    out.push(ChangedFile {
                        path: source_location.to_str_lossy().into_owned(),
                        change: Change::Deleted,
                    });
                    out.push(ChangedFile {
                        path,
                        change: Change::Added,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out.dedup_by(|a, b| a.path == b.path);
        Ok(out)
    }

    /// The raw commit object (headers and message), or `None` when the mirror lacks it.
    ///
    /// # Errors
    ///
    /// The mirror cannot be read, or the object is not a commit.
    pub fn commit_bytes(&self, commit: &CommitId) -> Result<Option<Vec<u8>>, MirrorError> {
        let repo = self.repo(&[])?;
        let Ok(id) = object_id(commit) else {
            return Ok(None);
        };
        let Some(obj) = repo.try_find_object(id).map_err(repo_err)? else {
            return Ok(None);
        };
        if obj.kind != gix::object::Kind::Commit {
            return Err(MirrorError::Repo(format!("{} is not a commit", commit.0)));
        }
        Ok(Some(obj.data.clone()))
    }

    /// Whether every object under `tree` (a tree id) is in the mirror. Submodule commits are
    /// not followed.
    ///
    /// # Errors
    ///
    /// The mirror cannot be read.
    pub fn tree_complete(&self, tree: &CommitId) -> Result<bool, MirrorError> {
        let repo = self.repo(&[])?;
        let Ok(root) = object_id(tree) else {
            return Ok(false);
        };
        let mut todo = vec![root];
        let mut seen = BTreeSet::new();
        while let Some(id) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(obj) = repo.try_find_object(id).map_err(repo_err)? else {
                return Ok(false);
            };
            if obj.kind != gix::object::Kind::Tree {
                return Ok(false);
            }
            let tree = obj.into_tree();
            let decoded = tree.decode().map_err(repo_err)?;
            for entry in &decoded.entries {
                let oid = entry.oid.to_owned();
                if entry.mode.is_tree() {
                    todo.push(oid);
                } else if entry.mode.is_commit() {
                    continue;
                } else if !repo.has_object(oid) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// The commit's author, committer and summary.
    ///
    /// # Errors
    ///
    /// The commit is not in the mirror.
    pub fn commit_meta(&self, commit: &CommitId) -> Result<CommitMeta, MirrorError> {
        let repo = self.repo(&[])?;
        let c = repo
            .find_commit(object_id(commit)?)
            .map_err(|_| MirrorError::NotFound(commit.0.clone()))?;
        let d = c.decode().map_err(repo_err)?;
        let author = d.author().map_err(repo_err)?;
        let committer = d.committer().map_err(repo_err)?;
        let person = |name: &gix::bstr::BStr, email: &gix::bstr::BStr| {
            format!("{} <{}>", name.to_str_lossy(), email.to_str_lossy())
        };
        Ok(CommitMeta {
            author: person(author.name, author.email),
            author_email: author.email.to_str_lossy().into_owned(),
            committer: person(committer.name, committer.email),
            summary: d.message_summary().to_str_lossy().into_owned(),
        })
    }

    /// The blob at `path` in `commit`, or `None` when there is no file there.
    fn blob_at(&self, commit: &CommitId, path: &str) -> Result<Option<Vec<u8>>, MirrorError> {
        let repo = self.repo(&[])?;
        let tree = repo
            .find_commit(object_id(commit)?)
            .map_err(|_| MirrorError::NotFound(commit.0.clone()))?
            .tree()
            .map_err(repo_err)?;
        let Some(entry) = tree.lookup_entry_by_path(path).map_err(repo_err)? else {
            return Ok(None);
        };
        if !entry.mode().is_blob() {
            return Ok(None);
        }
        Ok(Some(entry.object().map_err(repo_err)?.data.clone()))
    }

    /// The unified diff (3 lines of context) of `path` between `from` and `to`. A side without
    /// the file is empty.
    ///
    /// # Errors
    ///
    /// A commit is not in the mirror.
    pub fn file_diff(
        &self,
        from: Option<&CommitId>,
        to: &CommitId,
        path: &str,
    ) -> Result<FileDiff, MirrorError> {
        let old = match from {
            Some(f) => self.blob_at(f, path)?.unwrap_or_default(),
            None => Vec::new(),
        };
        let new = self.blob_at(to, path)?.unwrap_or_default();
        let binary = |b: &[u8]| b.len() > MAX_DIFF_BLOB_BYTES || b.contains(&0);
        if binary(&old) || binary(&new) {
            return Ok(FileDiff {
                binary: true,
                lines: Vec::new(),
            });
        }
        let input = InternedInput::new(old.as_slice(), new.as_slice());
        let diff = diff_with_slider_heuristics(Algorithm::Histogram, &input);
        let out = UnifiedDiff::new(
            &diff,
            &input,
            ConsumeBinaryHunk::new(Vec::<u8>::new(), "\n"),
            ContextSize::symmetrical(3),
        )
        .consume()
        .map_err(repo_err)?;
        let lines = out.to_str_lossy().lines().map(str::to_owned).collect();
        Ok(FileDiff {
            binary: false,
            lines,
        })
    }
}
