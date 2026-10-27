//! Bare mirrors of bound repositories (spec 055 T045, research R5).
//!
//! Each binding has one bare mirror at `<data_dir>/mirrors/<binding id>.git`. A fetch updates
//! `refs/keel/upstream/<branch>` only; the binding's accepted head is the server's to track.
//!
//! - **No hooks.** gix never runs hooks on fetch, the mirror is bare (nothing is checked out), and
//!   `core.hooksPath` is pointed at `/dev/null` for anything else that might look.
//! - **Isolated configuration.** The user's and system's git configuration are never read, so no
//!   credential helper, `insteadOf` rewrite or `sshCommand` from the host leaks in.
//! - **Transports.** HTTPS and SSH only (`file://` for tests and `--insecure-dev`, when the caller
//!   allows it). Credentials in the URL are refused.
//! - **HTTPS** authenticates with a token through the credential callback, never in the URL.
//! - **SSH** uses a deploy key and the binding's pinned `known_host`, with
//!   `StrictHostKeyChecking=yes` and `BatchMode=yes`: an unknown or changed host key fails the
//!   fetch. The key and the known-hosts line are written, mode `0600`, to a fresh directory under
//!   the caller's scratch directory and removed when the fetch ends.
//!
//! Errors never carry credentials.

use std::fmt;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use gix::bstr::ByteSlice;
use keel_core::ports::{CommitId, GitError, GitSource};
use uuid::Uuid;

/// How to authenticate a fetch. Borrowed: the caller owns (and zeroes) the secret.
#[derive(Clone, Copy)]
pub enum Auth<'a> {
    /// A public repository.
    None,
    /// HTTPS with a token. `username` defaults to `x-access-token`.
    HttpsToken {
        username: Option<&'a str>,
        token: &'a str,
    },
    /// SSH with a deploy key (OpenSSH private key) and the pinned host key line
    /// (`host key-type base64`, as `ssh-keyscan` prints it).
    SshKey {
        private_key: &'a [u8],
        known_host: &'a str,
    },
}

impl fmt::Debug for Auth<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Auth::None => f.write_str("None"),
            Auth::HttpsToken { .. } => f.write_str("HttpsToken([redacted])"),
            Auth::SshKey { known_host, .. } => {
                write!(f, "SshKey([redacted], known_host: {known_host:?})")
            }
        }
    }
}

/// One fetch.
#[derive(Debug, Clone, Copy)]
pub struct FetchRequest<'a> {
    pub url: &'a str,
    pub branch: &'a str,
    pub auth: Auth<'a>,
    /// Where SSH key material is written for the length of the fetch.
    pub scratch: &'a Path,
    /// Allow `file://` URLs and local paths (tests, `--insecure-dev`).
    pub allow_local: bool,
}

/// What a tree entry is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    /// A symbolic link, with its target as stored.
    Symlink(String),
    /// A submodule commit.
    Submodule,
}

/// A commit, as signed mode's preview reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub id: CommitId,
    /// The first line of the message.
    pub summary: String,
    /// Carries a signature header (`gpgsig`). Whether a trusted signer made it is keel-trust's.
    pub signed: bool,
}

/// Why a mirror operation failed. Never carries a credential.
#[derive(Debug, thiserror::Error)]
pub enum MirrorError {
    #[error("repository URL refused: {0}")]
    UrlRefused(String),
    #[error("invalid branch name: {0}")]
    BranchInvalid(String),
    #[error("branch not found on the remote: {0}")]
    BranchNotFound(String),
    #[error("SSH needs the host key pinned (known_host)")]
    KnownHostRequired,
    #[error("SSH key material could not be prepared: {0}")]
    Scratch(String),
    #[error("fetch failed: {0}")]
    Fetch(String),
    #[error("mirror: {0}")]
    Repo(String),
    #[error("object not found: {0}")]
    NotFound(String),
}

impl From<MirrorError> for GitError {
    fn from(e: MirrorError) -> Self {
        match e {
            MirrorError::NotFound(what) | MirrorError::BranchNotFound(what) => {
                GitError::RefNotFound(what)
            }
            other => GitError::Unavailable(other.to_string()),
        }
    }
}

/// Configuration every open of a mirror carries.
const BASE_CONFIG: &[&str] = &[
    "core.hooksPath=/dev/null",
    "core.askPass=",
    "credential.helper=",
    "protocol.allow=never",
    "protocol.https.allow=always",
    "protocol.ssh.allow=always",
    "gc.auto=0",
];

/// The ssh command for a deploy key and a pinned host key file. Paths are single-quoted, so a
/// path that contains a quote is refused (`None`).
#[must_use]
pub fn ssh_command(key: &Path, known_hosts: &Path) -> Option<String> {
    let key = key.to_str()?;
    let hosts = known_hosts.to_str()?;
    if key.contains('\'') || hosts.contains('\'') {
        return None;
    }
    Some(format!(
        "ssh -F /dev/null -i '{key}' -o IdentitiesOnly=yes -o IdentityAgent=none \
         -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile='{hosts}' \
         -o GlobalKnownHostsFile=/dev/null -o UpdateHostKeys=no"
    ))
}

/// A binding's bare mirror.
#[derive(Debug, Clone)]
pub struct Mirror {
    path: PathBuf,
}

impl Mirror {
    /// Opens `<data_dir>/mirrors/<binding>.git`, creating an empty bare repository first.
    ///
    /// # Errors
    ///
    /// The directory cannot be created or is not a repository.
    pub fn open(data_dir: &Path, binding: Uuid) -> Result<Self, MirrorError> {
        let path = data_dir.join("mirrors").join(format!("{binding}.git"));
        if !path.join("HEAD").is_file() {
            std::fs::create_dir_all(&path).map_err(|e| MirrorError::Repo(e.kind().to_string()))?;
            gix::init_bare(&path).map_err(|e| MirrorError::Repo(e.to_string()))?;
            // Templates may add sample hooks; nothing runs them, but keep the mirror clean.
            let _ = std::fs::remove_dir_all(path.join("hooks"));
        }
        let m = Self { path };
        m.repo(&[])?;
        Ok(m)
    }

    /// Removes a binding's mirror (binding removal).
    ///
    /// # Errors
    ///
    /// The directory exists and cannot be removed.
    pub fn remove(data_dir: &Path, binding: Uuid) -> Result<(), MirrorError> {
        let path = data_dir.join("mirrors").join(format!("{binding}.git"));
        match std::fs::remove_dir_all(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(MirrorError::Repo(e.kind().to_string())),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn repo(&self, extra: &[String]) -> Result<gix::Repository, MirrorError> {
        let overrides: Vec<String> = BASE_CONFIG
            .iter()
            .map(|s| (*s).to_owned())
            .chain(extra.iter().cloned())
            .collect();
        gix::open_opts(
            &self.path,
            gix::open::Options::isolated().config_overrides(overrides),
        )
        .map_err(|e| MirrorError::Repo(e.to_string()))
    }

    /// The ref a fetch of `branch` updates.
    #[must_use]
    pub fn upstream_ref(branch: &str) -> String {
        format!("refs/keel/upstream/{branch}")
    }

    /// Fetches `branch` from `url` and returns the fetched head.
    ///
    /// # Errors
    ///
    /// The URL or branch is refused, SSH has no pinned host key, the remote cannot be reached or
    /// refuses us, or the branch does not exist there.
    pub fn fetch(&self, req: &FetchRequest<'_>) -> Result<CommitId, MirrorError> {
        check_branch(req.branch)?;
        let scheme = check_url(req.url, req.allow_local)?;
        let mut extra = Vec::new();
        if req.allow_local {
            extra.push("protocol.file.allow=always".to_owned());
        }
        let _scratch = match (scheme, req.auth) {
            (
                Scheme::Ssh,
                Auth::SshKey {
                    private_key,
                    known_host,
                },
            ) => {
                let dir = SshScratch::new(req.scratch, private_key, known_host)?;
                let cmd = ssh_command(&dir.key, &dir.known_hosts)
                    .ok_or_else(|| MirrorError::Scratch("scratch path is not usable".to_owned()))?;
                extra.push(format!("core.sshCommand={cmd}"));
                extra.push("ssh.variant=ssh".to_owned());
                Some(dir)
            }
            (Scheme::Ssh, _) => return Err(MirrorError::KnownHostRequired),
            (_, Auth::SshKey { .. }) => {
                return Err(MirrorError::UrlRefused(
                    "a deploy key needs an ssh:// URL".to_owned(),
                ));
            }
            _ => None,
        };
        let repo = self.repo(&extra)?;
        let upstream = Self::upstream_ref(req.branch);
        let spec = format!("+refs/heads/{}:{upstream}", req.branch);
        let remote = repo
            .remote_at_without_url_rewrite(req.url)
            .map_err(|e| MirrorError::UrlRefused(e.to_string()))?
            .with_fetch_tags(gix::remote::fetch::Tags::None)
            .with_refspecs(Some(spec.as_str()), gix::remote::Direction::Fetch)
            .map_err(|e| MirrorError::Fetch(e.to_string()))?;
        let conn = remote
            .connect(gix::remote::Direction::Fetch)
            .map_err(|e| MirrorError::Fetch(e.to_string()))?;
        let token = match req.auth {
            Auth::HttpsToken { username, token } => Some((
                username.unwrap_or("x-access-token").to_owned(),
                token.to_owned(),
            )),
            _ => None,
        };
        let conn = conn.with_credentials(move |action| match (action, &token) {
            (gix::credentials::helper::Action::Get(ctx), Some((user, pass))) => {
                Ok(Some(gix::credentials::protocol::Outcome {
                    identity: gix::sec::identity::Account {
                        username: user.clone(),
                        password: pass.clone(),
                        oauth_refresh_token: None,
                    },
                    next: ctx.into(),
                }))
            }
            // No token: never prompt, never ask a helper.
            _ => Ok(None),
        });
        let outcome = conn
            .prepare_fetch(gix::progress::Discard, Default::default())
            .map_err(|e| fetch_error(&e.to_string(), req.branch))?
            .receive(gix::progress::Discard, &AtomicBool::new(false))
            .map_err(|e| fetch_error(&e.to_string(), req.branch))?;
        let wanted = format!("refs/heads/{}", req.branch);
        let found = outcome.ref_map.remote_refs.iter().any(|r| {
            let (name, _, _) = r.unpack();
            name.to_str().is_ok_and(|n| n == wanted)
        });
        if !found {
            return Err(MirrorError::BranchNotFound(req.branch.to_owned()));
        }
        drop(repo);
        self.resolve_ref(&upstream)
    }

    pub(crate) fn resolve_ref(&self, name: &str) -> Result<CommitId, MirrorError> {
        let repo = self.repo(&[])?;
        let mut r = repo
            .try_find_reference(name)
            .map_err(|e| MirrorError::Repo(e.to_string()))?
            .ok_or_else(|| MirrorError::NotFound(name.to_owned()))?;
        let id = r
            .peel_to_id()
            .map_err(|e| MirrorError::Repo(e.to_string()))?;
        Ok(CommitId(id.to_string()))
    }

    /// What `path` is at `commit`, without following symbolic links. `None` if absent.
    ///
    /// # Errors
    ///
    /// The commit is not in the mirror.
    pub fn entry(&self, commit: &CommitId, path: &str) -> Result<Option<EntryKind>, MirrorError> {
        let repo = self.repo(&[])?;
        let tree = commit_tree(&repo, commit)?;
        let path = path.trim_end_matches('/');
        if path.is_empty() {
            return Ok(Some(EntryKind::Dir));
        }
        let Some(entry) = tree
            .lookup_entry_by_path(path)
            .map_err(|e| MirrorError::Repo(e.to_string()))?
        else {
            return Ok(None);
        };
        let mode = entry.mode();
        Ok(Some(if mode.is_tree() {
            EntryKind::Dir
        } else if mode.is_link() {
            let blob = entry
                .object()
                .map_err(|e| MirrorError::Repo(e.to_string()))?;
            EntryKind::Symlink(String::from_utf8_lossy(&blob.data).into_owned())
        } else if mode.is_commit() {
            EntryKind::Submodule
        } else {
            EntryKind::File
        }))
    }

    /// The most recent `limit` commits reachable from `head`, newest first.
    ///
    /// # Errors
    ///
    /// The commit is not in the mirror.
    pub fn recent_commits(
        &self,
        head: &CommitId,
        limit: usize,
    ) -> Result<Vec<CommitInfo>, MirrorError> {
        let repo = self.repo(&[])?;
        let id = object_id(head)?;
        let walk = repo
            .rev_walk([id])
            .all()
            .map_err(|e| MirrorError::Repo(e.to_string()))?;
        let mut out = Vec::new();
        for info in walk.take(limit) {
            let info = info.map_err(|e| MirrorError::Repo(e.to_string()))?;
            let commit = repo
                .find_commit(info.id)
                .map_err(|e| MirrorError::Repo(e.to_string()))?;
            let decoded = commit
                .decode()
                .map_err(|e| MirrorError::Repo(e.to_string()))?;
            let signed = decoded
                .extra_headers()
                .find(gix::objs::commit::SIGNATURE_FIELD_NAME)
                .is_some();
            let summary = decoded.message_summary().to_str_lossy().into_owned();
            out.push(CommitInfo {
                id: CommitId(info.id.to_string()),
                summary,
                signed,
            });
        }
        Ok(out)
    }
}

impl GitSource for Mirror {
    /// The branch's last fetched head.
    fn resolve(&self, branch: &str) -> Result<CommitId, GitError> {
        check_branch(branch)?;
        Ok(self.resolve_ref(&Self::upstream_ref(branch))?)
    }

    fn read_file(&self, commit: &CommitId, path: &str) -> Result<Vec<u8>, GitError> {
        let repo = self.repo(&[])?;
        let tree = commit_tree(&repo, commit)?;
        let not_found = || GitError::PathNotFound {
            commit: commit.0.clone(),
            path: path.to_owned(),
        };
        let entry = tree
            .lookup_entry_by_path(path)
            .map_err(|e| GitError::Unavailable(e.to_string()))?
            .ok_or_else(not_found)?;
        if !entry.mode().is_blob() {
            return Err(not_found());
        }
        let blob = entry
            .object()
            .map_err(|e| GitError::Unavailable(e.to_string()))?;
        Ok(blob.data.clone())
    }
}

/// gix reports an unmatched refspec as a message, not a typed error.
fn fetch_error(msg: &str, branch: &str) -> MirrorError {
    if msg.contains("None of the refspec") {
        MirrorError::BranchNotFound(branch.to_owned())
    } else {
        MirrorError::Fetch(msg.to_owned())
    }
}

pub(crate) fn object_id(c: &CommitId) -> Result<gix::ObjectId, MirrorError> {
    gix::ObjectId::from_hex(c.0.as_bytes()).map_err(|_| MirrorError::NotFound(c.0.clone()))
}

fn commit_tree<'r>(
    repo: &'r gix::Repository,
    commit: &CommitId,
) -> Result<gix::Tree<'r>, MirrorError> {
    let id = object_id(commit)?;
    repo.find_commit(id)
        .map_err(|_| MirrorError::NotFound(commit.0.clone()))?
        .tree()
        .map_err(|e| MirrorError::Repo(e.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scheme {
    Https,
    Ssh,
    File,
}

/// Accepts `https://`, `ssh://` and scp-like `user@host:path` URLs without passwords, and
/// `file://` or absolute paths only when `allow_local`. The message never repeats the URL.
fn check_url(url: &str, allow_local: bool) -> Result<Scheme, MirrorError> {
    let refuse = |why: &str| Err(MirrorError::UrlRefused(why.to_owned()));
    if url.is_empty()
        || url.len() > 2048
        || url.chars().any(|c| c.is_control() || c.is_whitespace())
        || url.contains("::")
    {
        return refuse("malformed");
    }
    if url.starts_with('-') {
        return refuse("a URL may not start with '-'");
    }
    if let Some(rest) = url.strip_prefix("https://") {
        let authority = rest.split('/').next().unwrap_or_default();
        if authority.contains('@') {
            return refuse("credentials belong in a credential, not the URL");
        }
        if authority.is_empty() {
            return refuse("no host");
        }
        return Ok(Scheme::Https);
    }
    if let Some(rest) = url.strip_prefix("ssh://") {
        let authority = rest.split('/').next().unwrap_or_default();
        if authority.split('@').next().is_some_and(|u| u.contains(':')) && authority.contains('@') {
            return refuse("credentials belong in a credential, not the URL");
        }
        let host = authority.rsplit('@').next().unwrap_or_default();
        if host.is_empty() || host.starts_with('-') {
            return refuse("no host");
        }
        return Ok(Scheme::Ssh);
    }
    if url.starts_with("file://") || url.starts_with('/') {
        return if allow_local {
            Ok(Scheme::File)
        } else {
            refuse("local repositories are not allowed")
        };
    }
    if url.contains("://") {
        return refuse("only https:// and ssh:// are supported");
    }
    // scp-like: user@host:path
    if let Some((authority, path)) = url.split_once(':')
        && !authority.contains('/')
        && !path.is_empty()
    {
        let host = authority.rsplit('@').next().unwrap_or_default();
        if host.is_empty() || host.starts_with('-') {
            return refuse("no host");
        }
        return Ok(Scheme::Ssh);
    }
    refuse("unrecognized URL")
}

/// Git's branch-name rules, plus no leading `-`.
pub(crate) fn check_branch(branch: &str) -> Result<(), MirrorError> {
    let bad = || MirrorError::BranchInvalid(branch.chars().take(64).collect());
    if branch.is_empty() || branch.len() > 255 || branch.starts_with('-') {
        return Err(bad());
    }
    gix::validate::reference::name_partial(branch.as_bytes().as_bstr()).map_err(|_| bad())?;
    if branch == "HEAD" {
        return Err(bad());
    }
    Ok(())
}

/// A per-fetch directory holding the deploy key and the pinned host key, removed on drop.
struct SshScratch {
    dir: PathBuf,
    key: PathBuf,
    known_hosts: PathBuf,
}

impl SshScratch {
    fn new(scratch: &Path, private_key: &[u8], known_host: &str) -> Result<Self, MirrorError> {
        let line = known_host.trim();
        if line.is_empty() || line.contains('\n') {
            return Err(MirrorError::KnownHostRequired);
        }
        let err = |e: std::io::Error| MirrorError::Scratch(e.kind().to_string());
        let dir = scratch
            .join("ssh")
            .join(Uuid::now_v7().simple().to_string());
        std::fs::create_dir_all(&dir).map_err(err)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(err)?;
        let me = Self {
            key: dir.join("key"),
            known_hosts: dir.join("known_hosts"),
            dir,
        };
        write_private(&me.key, private_key).map_err(err)?;
        let mut hosts = line.as_bytes().to_vec();
        hosts.push(b'\n');
        write_private(&me.known_hosts, &hosts).map_err(err)?;
        Ok(me)
    }
}

impl Drop for SshScratch {
    fn drop(&mut self) {
        // Overwrite the key before unlinking it.
        if let Ok(meta) = std::fs::metadata(&self.key)
            && let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(&self.key)
        {
            let len = usize::try_from(meta.len()).unwrap_or(0);
            let _ = f.write_all(&vec![0u8; len]);
            let _ = f.sync_all();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    if !bytes.ends_with(b"\n") {
        f.write_all(b"\n")?;
    }
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            check_url("https://example.org/a.git", false).unwrap(),
            Scheme::Https
        );
        assert_eq!(
            check_url("ssh://git@example.org/a.git", false).unwrap(),
            Scheme::Ssh
        );
        assert_eq!(
            check_url("git@example.org:a/b.git", false).unwrap(),
            Scheme::Ssh
        );
        assert!(check_url("http://example.org/a.git", false).is_err());
        assert!(check_url("https://u:p@example.org/a.git", false).is_err());
        assert!(check_url("https://tok@example.org/a.git", false).is_err());
        assert!(check_url("file:///tmp/a.git", false).is_err());
        assert_eq!(check_url("file:///tmp/a.git", true).unwrap(), Scheme::File);
        assert!(check_url("ext::sh -c touch% /tmp/x", false).is_err());
        assert!(check_url("-oProxyCommand=x:y", false).is_err());
        assert!(check_url("ssh://-oProxyCommand=x/y", false).is_err());
    }

    #[test]
    fn branches() {
        assert!(check_branch("main").is_ok());
        assert!(check_branch("release/1.0").is_ok());
        assert!(check_branch("-x").is_err());
        assert!(check_branch("a..b").is_err());
        assert!(check_branch("a b").is_err());
        assert!(check_branch("HEAD").is_err());
        assert!(check_branch("").is_err());
    }
}
