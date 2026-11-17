//! GitHub: read the base branch's head, create the new branch at it, `PUT` each file with its
//! `sha` precondition, then open a pull request.

use serde::Deserialize;
use serde_json::json;

use crate::forgejo::{Permissions, Pull, link, status_state};
use crate::{
    ChangeRequest, CommitStatusState, Forge, ForgeAccess, ForgeError, ForgeFuture, ForgeStatus,
    Http, OpenedChange, RepoRef, b64, check_sha, encode_path,
};

#[derive(Debug, Clone)]
pub struct Github {
    http: Http,
    repo: RepoRef,
}

#[derive(Deserialize)]
struct Repo {
    default_branch: String,
    #[serde(default)]
    permissions: Option<Permissions>,
}

/// `GET /repos/{o}/{r}/commits/{sha}/status`: the latest status per context.
#[derive(Deserialize)]
struct Combined {
    #[serde(default)]
    statuses: Vec<Status>,
}

#[derive(Deserialize)]
struct Status {
    context: String,
    state: String,
    #[serde(default)]
    target_url: Option<String>,
}

/// `GET /repos/{o}/{r}/commits/{sha}/check-runs`.
#[derive(Deserialize)]
struct CheckRuns {
    #[serde(default)]
    check_runs: Vec<CheckRun>,
}

#[derive(Deserialize)]
struct CheckRun {
    name: String,
    status: String,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
}

/// A check run: pending until `completed`; then `success`, `neutral` and `skipped` pass, the
/// failing conclusions fail, and anything else (`cancelled`, unknown) is an error.
fn check_run_state(status: &str, conclusion: Option<&str>) -> CommitStatusState {
    if status != "completed" {
        return CommitStatusState::Pending;
    }
    match conclusion {
        Some("success" | "neutral" | "skipped") => CommitStatusState::Success,
        Some("failure" | "timed_out" | "action_required" | "startup_failure" | "stale") => {
            CommitStatusState::Failure
        }
        _ => CommitStatusState::Error,
    }
}

/// The most check runs read per commit (one page).
const CHECK_RUNS_PAGE: u32 = 100;

#[derive(Deserialize)]
struct Ref {
    object: RefObject,
}

#[derive(Deserialize)]
struct RefObject {
    sha: String,
}

impl Github {
    #[must_use]
    pub fn new(http: Http, repo: RepoRef) -> Self {
        Self { http, repo }
    }

    fn repo_path(&self) -> String {
        format!(
            "/repos/{}/{}",
            encode_path(&self.repo.owner),
            encode_path(&self.repo.repo)
        )
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header("Authorization", format!("Bearer {}", self.http.token()))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }
}

impl Forge for Github {
    fn probe(&self) -> ForgeFuture<'_, ForgeAccess> {
        Box::pin(self.probe_inner())
    }

    fn open_change<'a>(&'a self, change: &'a ChangeRequest) -> ForgeFuture<'a, OpenedChange> {
        Box::pin(self.open_change_inner(change))
    }

    fn commit_statuses<'a>(&'a self, sha: &'a str) -> ForgeFuture<'a, Vec<ForgeStatus>> {
        Box::pin(self.commit_statuses_inner(sha))
    }
}

impl Github {
    /// Commit statuses, then check runs (named by their check name). A context reported both
    /// ways appears twice; the caller treats any non-success as not passing.
    async fn commit_statuses_inner(&self, sha: &str) -> Result<Vec<ForgeStatus>, ForgeError> {
        check_sha(sha)?;
        let url = self
            .http
            .url(&format!("{}/commits/{sha}/status", self.repo_path()));
        let combined: Combined = self
            .http
            .send(self.auth(self.http.client().get(url)))
            .await?;
        let url = self.http.url(&format!(
            "{}/commits/{sha}/check-runs?per_page={CHECK_RUNS_PAGE}",
            self.repo_path()
        ));
        let runs: CheckRuns = self
            .http
            .send(self.auth(self.http.client().get(url)))
            .await?;
        let mut out: Vec<ForgeStatus> = combined
            .statuses
            .into_iter()
            .map(|s| ForgeStatus {
                state: status_state(&s.state),
                context: s.context,
                target_url: link(s.target_url),
            })
            .collect();
        out.extend(runs.check_runs.into_iter().map(|r| ForgeStatus {
            state: check_run_state(&r.status, r.conclusion.as_deref()),
            context: r.name,
            target_url: link(r.html_url),
        }));
        Ok(out)
    }

    async fn probe_inner(&self) -> Result<ForgeAccess, ForgeError> {
        let url = self.http.url(&self.repo_path());
        let repo: Repo = self
            .http
            .send(self.auth(self.http.client().get(url)))
            .await?;
        Ok(ForgeAccess {
            can_push: repo.permissions.is_some_and(|p| p.push),
            default_branch: repo.default_branch,
        })
    }

    async fn open_change_inner(&self, change: &ChangeRequest) -> Result<OpenedChange, ForgeError> {
        let rp = self.repo_path();
        let c = self.http.client();
        let url = self.http.url(&format!(
            "{rp}/git/ref/heads/{}",
            encode_path(&change.base_branch)
        ));
        let base: Ref = self.http.send(self.auth(c.get(url))).await?;
        let url = self.http.url(&format!("{rp}/git/refs"));
        let new_ref = json!({
            "ref": format!("refs/heads/{}", change.new_branch),
            "sha": base.object.sha,
        });
        let _: serde_json::Value = self
            .http
            .send(self.auth(c.post(url)).json(&new_ref))
            .await?;
        for f in &change.files {
            let mut body = json!({
                "message": change.title,
                "content": b64(&f.new_content),
                "branch": change.new_branch,
            });
            if let Some(sha) = &f.expected_blob_sha {
                body["sha"] = json!(sha);
            }
            let url = self
                .http
                .url(&format!("{rp}/contents/{}", encode_path(&f.path)));
            let _: serde_json::Value = self.http.send(self.auth(c.put(url)).json(&body)).await?;
        }
        let pull = json!({
            "base": change.base_branch,
            "head": change.new_branch,
            "title": change.title,
            "body": change.body,
        });
        let url = self.http.url(&format!("{rp}/pulls"));
        let pr: Pull = self.http.send(self.auth(c.post(url)).json(&pull)).await?;
        Ok(OpenedChange {
            url: pr.html_url,
            branch: change.new_branch.clone(),
            number: pr.number,
        })
    }
}
