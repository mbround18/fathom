//! Keel's forge adapters (spec 066 R8): open a pull request that changes files, and probe whether
//! a token can, on Forgejo, Gitea or GitHub, over their HTTP APIs. Neither path needs a git push.
//!
//! Every write goes to a new branch, and every changed file carries the blob `sha` Keel computed
//! its edit against, so a file changed on the forge since then is refused
//! ([`ForgeError::Conflict`]) rather than overwritten.
//!
//! The [`Forge`] trait is defined here for now; spec 066 puts the port in `keel-core`'s
//! `ports.rs`, and integration moves it there (or re-exports it) once both land.

use std::future::Future;

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

pub mod forgejo;
pub mod github;

pub use forgejo::Forgejo;
pub use github::Github;

/// Which forge API a connection speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Forgejo,
    Gitea,
    Github,
}

/// A repository's owner and name on its forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub owner: String,
    pub repo: String,
}

/// One file in a change. `expected_blob_sha` is the blob the edit was computed against; `None`
/// creates the file (and fails if it exists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    pub path: String,
    pub expected_blob_sha: Option<String>,
    pub new_content: Vec<u8>,
}

/// A pull request to open: `files` committed to `new_branch` (created from `base_branch`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequest {
    pub base_branch: String,
    pub new_branch: String,
    pub title: String,
    pub body: String,
    pub files: Vec<FileEdit>,
}

/// The opened pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedChange {
    pub url: String,
    pub branch: String,
    pub number: u64,
}

/// What the connection's token may do on the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub can_push: bool,
    pub default_branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForgeError {
    #[error("cannot tell the repository from {0:?}")]
    Url(String),
    #[error("the forge rejected the token")]
    Unauthorized,
    #[error("the forge has no such repository, branch or file")]
    NotFound,
    /// A file changed on the forge since the edit was computed (the `sha` precondition).
    #[error("a file changed on the forge since the edit was made: {0}")]
    Conflict(String),
    #[error("the forge refused ({status}): {message}")]
    Refused { status: u16, message: String },
    #[error("could not reach the forge: {0}")]
    Transport(String),
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

/// The forge port.
pub trait Forge {
    fn probe(&self) -> impl Future<Output = Result<Probe, ForgeError>> + Send;
    fn open_change(
        &self,
        change: &ChangeRequest,
    ) -> impl Future<Output = Result<OpenedChange, ForgeError>> + Send;
}

/// A connection to one repository, as configured on a workspace.
pub struct ForgeConfig {
    pub provider: Provider,
    /// The binding's clone URL; owner and repository are read from it.
    pub repo_url: String,
    /// Overrides [`default_api_base`].
    pub api_base: Option<String>,
    pub token: SecretString,
}

/// A forge client for whichever provider the config names.
#[derive(Debug, Clone)]
pub enum ForgeClient {
    Forgejo(Forgejo),
    Github(Github),
}

impl ForgeClient {
    /// # Errors
    ///
    /// The repository URL names no owner and repository, or the HTTP client cannot be built.
    pub fn new(config: ForgeConfig) -> Result<Self, ForgeError> {
        let repo = parse_repo_url(&config.repo_url)?;
        let base = match config.api_base {
            Some(b) => b,
            None => default_api_base(config.provider, &config.repo_url)?,
        };
        let http = Http::new(base, config.token)?;
        Ok(match config.provider {
            Provider::Forgejo | Provider::Gitea => Self::Forgejo(Forgejo::new(http, repo)),
            Provider::Github => Self::Github(Github::new(http, repo)),
        })
    }
}

impl Forge for ForgeClient {
    async fn probe(&self) -> Result<Probe, ForgeError> {
        match self {
            Self::Forgejo(f) => f.probe().await,
            Self::Github(g) => g.probe().await,
        }
    }

    async fn open_change(&self, change: &ChangeRequest) -> Result<OpenedChange, ForgeError> {
        match self {
            Self::Forgejo(f) => f.open_change(change).await,
            Self::Github(g) => g.open_change(change).await,
        }
    }
}

/// Splits a clone URL into host and path: `https://host[:port]/o/r(.git)`,
/// `ssh://git@host[:port]/o/r.git` or `git@host:o/r.git`.
fn split_url(url: &str) -> Option<(&str, &str)> {
    if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        Some((host, path))
    } else {
        let (user_host, path) = url.split_once(':')?;
        let host = user_host.rsplit_once('@').map_or(user_host, |(_, h)| h);
        Some((host, path))
    }
}

/// The owner and repository a clone URL names: its last two path segments, `.git` dropped.
///
/// # Errors
///
/// The URL has no host or fewer than two path segments.
pub fn parse_repo_url(url: &str) -> Result<RepoRef, ForgeError> {
    let bad = || ForgeError::Url(url.to_owned());
    let (host, path) = split_url(url.trim()).ok_or_else(bad)?;
    if host.is_empty() {
        return Err(bad());
    }
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut segs = path.rsplit('/').filter(|s| !s.is_empty());
    let (Some(repo), Some(owner)) = (segs.next(), segs.next()) else {
        return Err(bad());
    };
    Ok(RepoRef {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
    })
}

/// `https://<host>/api/v1` for Forgejo and Gitea (the port kept for an https URL, dropped for
/// ssh, whose port is not the web one); `https://api.github.com` for GitHub.
///
/// # Errors
///
/// The URL has no host.
pub fn default_api_base(provider: Provider, repo_url: &str) -> Result<String, ForgeError> {
    if provider == Provider::Github {
        return Ok("https://api.github.com".to_owned());
    }
    let url = repo_url.trim();
    let (host, _) = split_url(url).ok_or_else(|| ForgeError::Url(url.to_owned()))?;
    let host = if url.starts_with("https://") || url.starts_with("http://") {
        host
    } else {
        host.split(':').next().unwrap_or(host)
    };
    if host.is_empty() {
        return Err(ForgeError::Url(url.to_owned()));
    }
    Ok(format!("https://{host}/api/v1"))
}

/// Percent-encodes a repository path for a URL, keeping `/`.
pub(crate) fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub(crate) fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The HTTP side shared by both adapters: base URL, token, client.
#[derive(Clone)]
pub struct Http {
    base: String,
    token: SecretString,
    client: reqwest::Client,
}

impl std::fmt::Debug for Http {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Http")
            .field("base", &self.base)
            .field("token", &"[redacted]")
            .finish_non_exhaustive()
    }
}

/// A forge's error body: `{"message": ...}` on all three.
#[derive(Deserialize)]
struct ErrorBody {
    message: Option<String>,
}

const MESSAGE_LIMIT: usize = 500;

impl Http {
    fn new(base: String, token: SecretString) -> Result<Self, ForgeError> {
        let client = reqwest::Client::builder()
            .user_agent("keel")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ForgeError::Transport(e.to_string()))?;
        Ok(Self {
            base: base.trim_end_matches('/').to_owned(),
            token,
            client,
        })
    }

    pub(crate) fn token(&self) -> &str {
        self.token.expose_secret()
    }

    /// Sends a request and decodes a 2xx JSON body; maps everything else to a [`ForgeError`].
    pub(crate) async fn send<T: serde::de::DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<T, ForgeError> {
        let resp = req
            .send()
            .await
            .map_err(|e| ForgeError::Transport(e.without_url().to_string()))?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return resp
                .json::<T>()
                .await
                .map_err(|e| ForgeError::Response(e.without_url().to_string()));
        }
        let text = resp.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ErrorBody>(&text)
            .ok()
            .and_then(|b| b.message)
            .unwrap_or(text);
        let message: String = message.chars().take(MESSAGE_LIMIT).collect();
        Err(match status {
            401 => ForgeError::Unauthorized,
            404 => ForgeError::NotFound,
            409 => ForgeError::Conflict(message),
            _ => ForgeError::Refused { status, message },
        })
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub(crate) fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_urls() {
        let want = RepoRef {
            owner: "ops".into(),
            repo: "ansible".into(),
        };
        for url in [
            "https://git.example.test/ops/ansible.git",
            "https://git.example.test/ops/ansible",
            "https://git.example.test:3000/ops/ansible/",
            "ssh://git@git.example.test:2222/ops/ansible.git",
            "git@git.example.test:ops/ansible.git",
        ] {
            assert_eq!(parse_repo_url(url).unwrap(), want, "{url}");
        }
        assert!(parse_repo_url("https://git.example.test/ansible").is_err());
        assert!(parse_repo_url("nonsense").is_err());
    }

    #[test]
    fn api_bases() {
        let f = |p, u| default_api_base(p, u).unwrap();
        assert_eq!(
            f(Provider::Forgejo, "https://git.example.test/o/r.git"),
            "https://git.example.test/api/v1"
        );
        assert_eq!(
            f(Provider::Gitea, "https://git.example.test:3000/o/r"),
            "https://git.example.test:3000/api/v1"
        );
        assert_eq!(
            f(Provider::Forgejo, "ssh://git@git.example.test:2222/o/r.git"),
            "https://git.example.test/api/v1"
        );
        assert_eq!(
            f(Provider::Forgejo, "git@git.example.test:o/r.git"),
            "https://git.example.test/api/v1"
        );
        assert_eq!(
            f(Provider::Github, "git@github.com:o/r.git"),
            "https://api.github.com"
        );
    }

    #[test]
    fn paths_are_encoded_but_keep_slashes() {
        assert_eq!(encode_path("inventory/hosts.yml"), "inventory/hosts.yml");
        assert_eq!(encode_path("a b/c#d"), "a%20b/c%23d");
    }

    #[test]
    fn token_is_not_in_debug() {
        let http = Http::new("https://x.test".into(), SecretString::from("s3cret-token")).unwrap();
        assert!(!format!("{http:?}").contains("s3cret"));
    }
}
