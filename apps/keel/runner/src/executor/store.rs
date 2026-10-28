//! The executor's own Git object store (`<state>/git`, bare) and the checkout of one commit.
//!
//! The store is opened isolated (no system, global or repository config is trusted, no hooks
//! exist or run), and a pack only adds objects: no refs, no config and nothing else is taken
//! from it. The checkout is written by hand from the commit's tree, so no Git machinery runs
//! over repository content: entry names `.`, `..`, `.git` or containing `/` are refused,
//! symbolic links are written as links and never followed, a name that appears twice in one
//! tree fails the checkout, and submodules are skipped.

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use gix::ObjectId;
use keel_core::manifest::{PathKind, RepoTree};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("git store: {0}")]
    Repo(String),
    #[error("the pack does not contain commit {0}")]
    MissingCommit(String),
    #[error("checkout: {0}")]
    Checkout(String),
}

fn repo_err(e: impl std::fmt::Display) -> StoreError {
    StoreError::Repo(e.to_string())
}

/// The bare object store.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    /// Opens `<dir>`, creating an empty bare repository there on first use.
    pub fn open(dir: &Path) -> Result<Self, StoreError> {
        if !dir.join("HEAD").is_file() {
            std::fs::create_dir_all(dir).map_err(repo_err)?;
            gix::init_bare(dir).map_err(repo_err)?;
            // Templates may add sample hooks; nothing runs them, but keep the store clean.
            let _ = std::fs::remove_dir_all(dir.join("hooks"));
        }
        let store = Self {
            path: dir.to_owned(),
        };
        store.repo()?;
        Ok(store)
    }

    fn repo(&self) -> Result<gix::Repository, StoreError> {
        gix::open_opts(&self.path, gix::open::Options::isolated()).map_err(repo_err)
    }

    /// Indexes `pack` into `objects/pack`. Thin packs are refused (no base lookup is offered).
    pub fn unpack(&self, pack: &[u8]) -> Result<(), StoreError> {
        let dir = self.path.join("objects").join("pack");
        std::fs::create_dir_all(&dir).map_err(repo_err)?;
        let mut reader = std::io::BufReader::new(pack);
        gix_pack::Bundle::write_to_directory(
            &mut reader,
            Some(&dir),
            &mut gix::progress::Discard,
            &AtomicBool::new(false),
            None::<gix::objs::find::Never>,
            gix::hash::Kind::Sha1,
            gix_pack::bundle::write::Options {
                thread_limit: Some(1),
                // No single object or delta buffer may exceed this.
                alloc_limit_bytes: Some(512 * 1024 * 1024),
                ..Default::default()
            },
        )
        .map_err(|e| StoreError::Repo(format!("the pack does not unpack: {e}")))?;
        Ok(())
    }

    /// The tree of `commit`, which must be a commit in the store.
    fn commit_tree(&self, repo: &gix::Repository, commit: &str) -> Result<ObjectId, StoreError> {
        let id = ObjectId::from_hex(commit.as_bytes())
            .map_err(|_| StoreError::MissingCommit(commit.to_owned()))?;
        let commit_obj = repo
            .find_commit(id)
            .map_err(|_| StoreError::MissingCommit(commit.to_owned()))?;
        commit_obj.tree_id().map(|t| t.detach()).map_err(repo_err)
    }

    /// A read-only view of `commit`'s tree, for manifest validation.
    pub fn tree(&self, commit: &str) -> Result<GitTree, StoreError> {
        let repo = self.repo()?;
        let root = self.commit_tree(&repo, commit)?;
        Ok(GitTree { repo, root })
    }

    /// Writes `commit`'s tree into `dest`, which must not exist yet.
    pub fn checkout(&self, commit: &str, dest: &Path) -> Result<(), StoreError> {
        let repo = self.repo()?;
        let root = self.commit_tree(&repo, commit)?;
        std::fs::create_dir(dest).map_err(|e| StoreError::Checkout(e.to_string()))?;
        let mut stack = vec![(root, dest.to_owned())];
        while let Some((tree_id, dir)) = stack.pop() {
            let obj = repo.find_object(tree_id).map_err(repo_err)?;
            if obj.kind != gix::object::Kind::Tree {
                return Err(StoreError::Checkout(
                    "a tree entry is not a tree".to_owned(),
                ));
            }
            let tree = obj.into_tree();
            for entry in tree.iter() {
                let entry = entry.map_err(repo_err)?;
                let name = std::str::from_utf8(entry.filename())
                    .map_err(|_| StoreError::Checkout("an entry name is not UTF-8".to_owned()))?;
                if matches!(name, "" | "." | ".." | ".git") || name.contains(['/', '\0']) {
                    return Err(StoreError::Checkout(format!("refused entry name {name:?}")));
                }
                let path = dir.join(name);
                let mode = entry.mode();
                let oid = entry.oid().to_owned();
                if mode.is_commit() {
                    continue;
                }
                if mode.is_tree() {
                    std::fs::create_dir(&path)
                        .map_err(|e| StoreError::Checkout(format!("{name}: {e}")))?;
                    stack.push((oid, path));
                    continue;
                }
                let blob = repo.find_object(oid).map_err(repo_err)?;
                if blob.kind != gix::object::Kind::Blob {
                    return Err(StoreError::Checkout(format!("{name}: not a blob")));
                }
                if mode.is_link() {
                    let target = std::str::from_utf8(&blob.data).map_err(|_| {
                        StoreError::Checkout(format!("{name}: link target is not UTF-8"))
                    })?;
                    std::os::unix::fs::symlink(target, &path)
                        .map_err(|e| StoreError::Checkout(format!("{name}: {e}")))?;
                } else {
                    let file_mode = if mode.is_executable() { 0o700 } else { 0o600 };
                    let mut f = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(file_mode)
                        .open(&path)
                        .map_err(|e| StoreError::Checkout(format!("{name}: {e}")))?;
                    f.write_all(&blob.data)
                        .map_err(|e| StoreError::Checkout(format!("{name}: {e}")))?;
                }
            }
        }
        Ok(())
    }
}

/// One commit's tree, as keel-core's manifest validation sees it.
pub struct GitTree {
    repo: gix::Repository,
    root: ObjectId,
}

impl GitTree {
    fn entry(&self, path: &str) -> Option<(gix::objs::tree::EntryMode, ObjectId)> {
        let path = path.trim_end_matches('/');
        let tree = self.repo.find_object(self.root).ok()?.into_tree();
        let entry = tree.lookup_entry_by_path(path).ok()??;
        Some((entry.mode(), entry.oid().to_owned()))
    }

    /// The bytes of the regular file at `path`; `None` when absent or not a regular file.
    pub fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let (mode, oid) = self.entry(path)?;
        if !mode.is_blob() || mode.is_link() {
            return None;
        }
        let obj = self.repo.find_object(oid).ok()?;
        (obj.kind == gix::object::Kind::Blob).then(|| obj.data.clone())
    }
}

impl RepoTree for GitTree {
    fn kind(&self, path: &str) -> Option<PathKind> {
        if path.trim_end_matches('/').is_empty() {
            return Some(PathKind::Dir);
        }
        let (mode, oid) = self.entry(path)?;
        Some(if mode.is_tree() {
            PathKind::Dir
        } else if mode.is_link() {
            let obj = self.repo.find_object(oid).ok()?;
            PathKind::Symlink(String::from_utf8_lossy(&obj.data).into_owned())
        } else if mode.is_blob() {
            PathKind::File
        } else {
            PathKind::Other
        })
    }
}

impl Store {
    /// The store as keel-trust's [`keel_trust::ObjectStore`], for signed mode's commit check.
    pub fn objects(&self) -> Result<Objects, StoreError> {
        Ok(Objects(self.repo()?))
    }
}

/// An opened store, read by keel-trust's commit check. Holds a `gix::Repository`, so it must
/// not live across an await.
pub struct Objects(gix::Repository);

fn trust_err(e: impl std::fmt::Display) -> keel_trust::object::StoreError {
    keel_trust::object::StoreError(e.to_string())
}

fn object_id(id: &keel_trust::CommitId) -> Result<ObjectId, keel_trust::object::StoreError> {
    ObjectId::from_hex(id.as_str().as_bytes()).map_err(trust_err)
}

impl keel_trust::ObjectStore for Objects {
    fn commit_bytes(
        &self,
        id: &keel_trust::CommitId,
    ) -> Result<Option<Vec<u8>>, keel_trust::object::StoreError> {
        let Some(obj) = self.0.try_find_object(object_id(id)?).map_err(trust_err)? else {
            return Ok(None);
        };
        if obj.kind != gix::object::Kind::Commit {
            return Err(trust_err(format!("{} is not a commit", id.as_str())));
        }
        Ok(Some(obj.data.clone()))
    }

    /// Every tree and blob under `tree` is present with the right kind. Submodule entries
    /// (commits) are not followed: the checkout skips them too.
    fn tree_complete(
        &self,
        tree: &keel_trust::CommitId,
    ) -> Result<bool, keel_trust::object::StoreError> {
        let mut stack = vec![object_id(tree)?];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(obj) = self.0.try_find_object(id).map_err(trust_err)? else {
                return Ok(false);
            };
            if obj.kind != gix::object::Kind::Tree {
                return Ok(false);
            }
            for entry in obj.into_tree().iter() {
                let Ok(entry) = entry else { return Ok(false) };
                let mode = entry.mode();
                let oid = entry.oid().to_owned();
                if mode.is_commit() {
                    continue;
                }
                if mode.is_tree() {
                    stack.push(oid);
                    continue;
                }
                match self.0.try_find_header(oid).map_err(trust_err)? {
                    Some(h) if h.kind() == gix::object::Kind::Blob => {}
                    _ => return Ok(false),
                }
            }
        }
        Ok(true)
    }
}
