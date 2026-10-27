//! Git commits as keel-trust sees them: ids, the object store it reads them from, and the parse
//! of a raw commit into parents, committer time and signature.
//!
//! keel-trust does no I/O. The server feeds it from its gix mirror and the executor from its own
//! gix store, both through [`ObjectStore`]; tests use an in-memory map.

use std::fmt;

/// A Git object id: 40 (SHA-1) or 64 (SHA-256) lowercase hex characters.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct CommitId(String);

/// The text is not a lowercase hex object id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a Git object id")]
pub struct InvalidCommitId;

impl CommitId {
    /// Parses a lowercase hex object id of 40 or 64 characters. Upper case is refused, so every
    /// id has exactly one spelling (it is hashed into challenges and the ledger).
    pub fn parse(text: &str) -> Result<Self, InvalidCommitId> {
        let ok = matches!(text.len(), 40 | 64)
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if ok {
            Ok(Self(text.to_owned()))
        } else {
            Err(InvalidCommitId)
        }
    }

    /// The hex text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is a SHA-256 object id (64 characters).
    pub fn is_sha256(&self) -> bool {
        self.0.len() == 64
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for CommitId {
    type Error = InvalidCommitId;
    fn try_from(text: String) -> Result<Self, InvalidCommitId> {
        Self::parse(&text)
    }
}

impl From<CommitId> for String {
    fn from(id: CommitId) -> String {
        id.0
    }
}

/// The object store failed (I/O, corruption). Distinct from "not present", which is `Ok(None)`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("object store: {0}")]
pub struct StoreError(pub String);

/// Read access to a Git object store.
///
/// Implementations must only return bytes whose object id is `id` (gix verifies this when it
/// reads loose or packed objects); keel-trust takes the parents from the signed bytes, so a store
/// cannot change the graph without breaking a signature.
pub trait ObjectStore {
    /// The raw commit object (the content after the `commit <len>\0` header), or `None` if the
    /// object is absent. An object that is present but not a commit is an error.
    fn commit_bytes(&self, id: &CommitId) -> Result<Option<Vec<u8>>, StoreError>;

    /// Whether the tree and everything it references (subtrees, blobs) are present.
    fn tree_complete(&self, tree: &CommitId) -> Result<bool, StoreError>;
}

/// A commit's signature header, as found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureBlock {
    /// An ASCII-armored OpenPGP signature.
    OpenPgp(String),
    /// An SSH signature (`-----BEGIN SSH SIGNATURE-----`).
    Ssh(String),
    /// Some other signature format (X.509, unknown armor).
    Other,
}

/// The parts of a commit the trust check uses. The author and committer identities are never
/// read: only the committer time is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommit {
    /// The root tree.
    pub tree: CommitId,
    /// Parents, in order.
    pub parents: Vec<CommitId>,
    /// Committer time, Unix seconds.
    pub committer_time: i64,
    /// The signature header for this repository's hash (`gpgsig` for SHA-1 ids, `gpgsig-sha256`
    /// for SHA-256 ids), if any.
    pub signature: Option<SignatureBlock>,
    /// The bytes that were signed: the commit with every `gpgsig` and `gpgsig-sha256` header
    /// removed, as `git verify-commit` reconstructs them.
    pub payload: Vec<u8>,
}

/// The raw commit is not a well-formed Git commit.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("malformed commit: {0}")]
pub struct MalformedCommit(pub &'static str);

/// Parses a raw commit object. `sha256` selects the signature header (`gpgsig-sha256` when the
/// repository uses SHA-256 ids). Duplicate signature headers, a missing or repeated `tree` or
/// `committer`, and ids of the wrong length are refused.
pub fn parse_commit(raw: &[u8], sha256: bool) -> Result<ParsedCommit, MalformedCommit> {
    let header_end = find_header_end(raw).ok_or(MalformedCommit("no end of headers"))?;
    let header = std::str::from_utf8(&raw[..header_end])
        .map_err(|_| MalformedCommit("headers are not UTF-8"))?;

    let sig_name = if sha256 { "gpgsig-sha256" } else { "gpgsig" };
    let id_len = if sha256 { 64 } else { 40 };

    let mut tree = None;
    let mut parents = Vec::new();
    let mut committer_time = None;
    let mut signature: Option<String> = None;
    let mut payload = Vec::with_capacity(raw.len());

    // Headers: `name value` lines; a line starting with a space continues the previous one.
    let mut lines = header.split_inclusive('\n').peekable();
    let mut seen_parent_end = false;
    while let Some(line) = lines.next() {
        let mut continuation = Vec::new();
        while let Some(next) = lines.peek() {
            if next.starts_with(' ') {
                continuation.push(*next);
                lines.next();
            } else {
                break;
            }
        }
        let body = line
            .strip_suffix('\n')
            .ok_or(MalformedCommit("unterminated header"))?;
        let (name, value) = body
            .split_once(' ')
            .ok_or(MalformedCommit("header without value"))?;
        let is_sig = name == "gpgsig" || name == "gpgsig-sha256";
        if !is_sig {
            payload.extend_from_slice(line.as_bytes());
            for c in &continuation {
                payload.extend_from_slice(c.as_bytes());
            }
        }
        match name {
            "tree" => {
                if tree.is_some() || !parents.is_empty() || !continuation.is_empty() {
                    return Err(MalformedCommit("tree header"));
                }
                tree = Some(object_id(value, id_len)?);
            }
            "parent" => {
                if tree.is_none() || seen_parent_end || !continuation.is_empty() {
                    return Err(MalformedCommit("parent header"));
                }
                parents.push(object_id(value, id_len)?);
            }
            "committer" => {
                if committer_time.is_some() || !continuation.is_empty() {
                    return Err(MalformedCommit("committer header"));
                }
                committer_time = Some(committer_seconds(value)?);
            }
            _ => {}
        }
        if name != "tree" && name != "parent" {
            seen_parent_end = true;
        }
        if name == sig_name {
            if signature.is_some() {
                return Err(MalformedCommit("more than one signature header"));
            }
            let mut text = value.to_owned();
            for c in &continuation {
                text.push('\n');
                let c = c
                    .strip_suffix('\n')
                    .ok_or(MalformedCommit("unterminated header"))?;
                text.push_str(&c[1..]);
            }
            text.push('\n');
            signature = Some(text);
        }
    }
    payload.extend_from_slice(&raw[header_end..]);

    let signature = signature.map(|text| {
        if text.starts_with("-----BEGIN PGP SIGNATURE-----") {
            SignatureBlock::OpenPgp(text)
        } else if text.starts_with("-----BEGIN SSH SIGNATURE-----") {
            SignatureBlock::Ssh(text)
        } else {
            SignatureBlock::Other
        }
    });

    Ok(ParsedCommit {
        tree: tree.ok_or(MalformedCommit("no tree"))?,
        parents,
        committer_time: committer_time.ok_or(MalformedCommit("no committer"))?,
        signature,
        payload,
    })
}

/// Index of the blank line that ends the headers (the position of its `\n`).
fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(2).position(|w| w == b"\n\n").map(|i| i + 1)
}

fn object_id(value: &str, len: usize) -> Result<CommitId, MalformedCommit> {
    let id = CommitId::parse(value).map_err(|_| MalformedCommit("object id"))?;
    if id.as_str().len() == len {
        Ok(id)
    } else {
        Err(MalformedCommit("object id length"))
    }
}

/// `Name <email> 1700000000 +0000` → 1700000000. Only the part after the last `>` is read.
fn committer_seconds(value: &str) -> Result<i64, MalformedCommit> {
    let (_, rest) = value.rsplit_once('>').ok_or(MalformedCommit("committer"))?;
    let mut parts = rest.split_ascii_whitespace();
    let secs = parts.next().ok_or(MalformedCommit("committer time"))?;
    let tz = parts.next().ok_or(MalformedCommit("committer zone"))?;
    if parts.next().is_some()
        || tz.len() != 5
        || !matches!(tz.as_bytes()[0], b'+' | b'-')
        || !tz[1..].bytes().all(|b| b.is_ascii_digit())
        || secs.is_empty()
        || !secs.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(MalformedCommit("committer time"));
    }
    secs.parse().map_err(|_| MalformedCommit("committer time"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    const P1: &str = "1111111111111111111111111111111111111111";

    #[test]
    fn strips_the_signature_header_and_reads_parents_and_time() {
        let raw = format!(
            "tree {TREE}\nparent {P1}\nauthor A <a@x> 5 +0000\ncommitter C <c@x> 1700000000 +0100\ngpgsig -----BEGIN SSH SIGNATURE-----\n abc\n \n -----END SSH SIGNATURE-----\n\nmsg\n"
        );
        let c = parse_commit(raw.as_bytes(), false).unwrap();
        assert_eq!(c.tree.as_str(), TREE);
        assert_eq!(c.parents, vec![CommitId::parse(P1).unwrap()]);
        assert_eq!(c.committer_time, 1_700_000_000);
        assert_eq!(
            c.signature,
            Some(SignatureBlock::Ssh(
                "-----BEGIN SSH SIGNATURE-----\nabc\n\n-----END SSH SIGNATURE-----\n".into()
            ))
        );
        let want = format!(
            "tree {TREE}\nparent {P1}\nauthor A <a@x> 5 +0000\ncommitter C <c@x> 1700000000 +0100\n\nmsg\n"
        );
        assert_eq!(String::from_utf8(c.payload).unwrap(), want);
    }

    #[test]
    fn refuses_malformed_commits() {
        let ok = format!("tree {TREE}\ncommitter C <c@x> 1 +0000\n\nm");
        assert!(parse_commit(ok.as_bytes(), false).is_ok());
        for bad in [
            format!("tree {TREE}\ncommitter C <c@x> 1 +0000\nm"),
            "committer C <c@x> 1 +0000\n\nm".to_owned(),
            format!("tree {TREE}\n\nm"),
            format!("tree {TREE}\ntree {TREE}\ncommitter C <c@x> 1 +0000\n\nm"),
            format!("tree {TREE}\ncommitter C <c@x> 1 +0000\ncommitter C <c@x> 1 +0000\n\nm"),
            format!("tree {TREE}\ncommitter C <c@x> x +0000\n\nm"),
            format!(
                "tree {}\ncommitter C <c@x> 1 +0000\n\nm",
                TREE.to_uppercase()
            ),
            format!("tree {TREE}\ncommitter C <c@x> 1 +0000\ngpgsig a\ngpgsig b\n\nm"),
            format!("tree {TREE}\ncommitter C <c@x> 1 +0000\nparent {P1}\n\nm"),
        ] {
            assert!(parse_commit(bad.as_bytes(), false).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn sha256_repositories_use_gpgsig_sha256() {
        let tree = "a".repeat(64);
        let raw = format!(
            "tree {tree}\ncommitter C <c@x> 1 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n x\ngpgsig-sha256 -----BEGIN SSH SIGNATURE-----\n y\n\nm"
        );
        let c = parse_commit(raw.as_bytes(), true).unwrap();
        assert!(matches!(c.signature, Some(SignatureBlock::Ssh(_))));
        // Both signature headers are removed from the payload.
        assert_eq!(
            String::from_utf8(c.payload).unwrap(),
            format!("tree {tree}\ncommitter C <c@x> 1 +0000\n\nm")
        );
    }
}
