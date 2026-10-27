//! Packs for host runners (spec 055 T070, contracts/runner.md "Pack"): every object reachable
//! from the run's commit that is not reachable from the host's base, as a git pack (version 2,
//! undeltified, zlib-compressed objects, SHA-1 trailer). With no base, the whole history: the
//! host verifies the chain itself.

use std::collections::HashSet;
use std::io::Write as _;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use gix::ObjectId;
use keel_core::ports::CommitId;
use sha1::{Digest, Sha1};

use crate::mirror::{Mirror, MirrorError, object_id};

/// A built pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    pub bytes: Vec<u8>,
    pub objects: u32,
}

fn repo_err(e: impl std::fmt::Display) -> MirrorError {
    MirrorError::Repo(e.to_string())
}

/// Adds `tree` and everything under it to `out`, skipping what `skip` or `out` has.
fn add_tree(
    repo: &gix::Repository,
    tree: ObjectId,
    skip: &HashSet<ObjectId>,
    out: &mut Vec<ObjectId>,
    seen: &mut HashSet<ObjectId>,
) -> Result<(), MirrorError> {
    let mut stack = vec![tree];
    while let Some(id) = stack.pop() {
        if skip.contains(&id) || !seen.insert(id) {
            continue;
        }
        out.push(id);
        let obj = repo.find_object(id).map_err(repo_err)?;
        if obj.kind != gix::object::Kind::Tree {
            continue;
        }
        let tree = obj.into_tree();
        for entry in tree.iter() {
            let entry = entry.map_err(repo_err)?;
            let mode = entry.mode();
            // Submodule commits live in another repository.
            if mode.is_commit() {
                continue;
            }
            let oid = entry.oid().to_owned();
            if mode.is_tree() {
                stack.push(oid);
            } else if !skip.contains(&oid) && seen.insert(oid) {
                out.push(oid);
            }
        }
    }
    Ok(())
}

/// Every commit, tree and blob reachable from `tips`, minus `skip`.
fn reachable(
    repo: &gix::Repository,
    tips: &[ObjectId],
    skip: &HashSet<ObjectId>,
) -> Result<Vec<ObjectId>, MirrorError> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if tips.is_empty() {
        return Ok(out);
    }
    let walk = repo
        .rev_walk(tips.iter().copied())
        .all()
        .map_err(repo_err)?;
    for info in walk {
        let info = info.map_err(repo_err)?;
        if skip.contains(&info.id) || !seen.insert(info.id) {
            continue;
        }
        out.push(info.id);
        let commit = repo.find_commit(info.id).map_err(repo_err)?;
        let tree = commit.tree_id().map_err(repo_err)?.detach();
        add_tree(repo, tree, skip, &mut out, &mut seen)?;
    }
    Ok(out)
}

fn type_code(kind: gix::object::Kind) -> u8 {
    match kind {
        gix::object::Kind::Commit => 1,
        gix::object::Kind::Tree => 2,
        gix::object::Kind::Blob => 3,
        gix::object::Kind::Tag => 4,
    }
}

fn object_header(kind: u8, mut size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(10);
    // The low four bits of the size, then seven at a time, most-significant bit continues.
    let mut byte = (kind << 4) | (size & 0x0f) as u8;
    size >>= 4;
    while size > 0 {
        out.push(byte | 0x80);
        byte = (size & 0x7f) as u8;
        size >>= 7;
    }
    out.push(byte);
    out
}

impl Mirror {
    /// Builds the pack of `base..target` (or all of `target`'s history when `base` is `None`).
    ///
    /// # Errors
    ///
    /// A commit is not in the mirror, an object cannot be read, or the pack would hold more
    /// objects than a pack can count.
    pub fn build_pack(
        &self,
        base: Option<&CommitId>,
        target: &CommitId,
    ) -> Result<Pack, MirrorError> {
        let repo = self.repo(&[])?;
        let target_id = object_id(target)?;
        repo.find_commit(target_id)
            .map_err(|_| MirrorError::NotFound(target.0.clone()))?;
        let skip: HashSet<ObjectId> = match base {
            Some(b) => {
                let b = object_id(b)?;
                repo.find_commit(b)
                    .map_err(|_| MirrorError::NotFound(b.to_string()))?;
                reachable(&repo, &[b], &HashSet::new())?
                    .into_iter()
                    .collect()
            }
            None => HashSet::new(),
        };
        let ids = reachable(&repo, &[target_id], &skip)?;
        let count = u32::try_from(ids.len()).map_err(|_| repo_err("too many objects"))?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"PACK");
        bytes.extend_from_slice(&2u32.to_be_bytes());
        bytes.extend_from_slice(&count.to_be_bytes());
        for id in ids {
            let obj = repo.find_object(id).map_err(repo_err)?;
            bytes.extend_from_slice(&object_header(type_code(obj.kind), obj.data.len()));
            let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
            z.write_all(&obj.data).map_err(repo_err)?;
            bytes.extend_from_slice(&z.finish().map_err(repo_err)?);
        }
        let trailer = Sha1::digest(&bytes);
        bytes.extend_from_slice(&trailer);
        Ok(Pack {
            bytes,
            objects: count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::object_header;

    #[test]
    fn object_headers_encode_type_and_size() {
        assert_eq!(object_header(3, 5), vec![0x35]);
        assert_eq!(object_header(1, 16), vec![0x90, 0x01]);
        assert_eq!(object_header(2, 0x1234), vec![0xa4, 0xa3, 0x02]);
    }
}
