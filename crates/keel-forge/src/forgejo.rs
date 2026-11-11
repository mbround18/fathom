//! Forgejo and Gitea: one `ChangeFiles` call commits every file to a new branch, with each
//! file's `sha` as a precondition, then a pull request is opened from it.

use serde::Deserialize;
use serde_json::json;

use crate::{
    ChangeRequest, Forge, ForgeError, Http, OpenedChange, Probe, RepoRef, b64, encode_path,
};

#[derive(Debug, Clone)]
pub struct Forgejo {
    http: Http,
    repo: RepoRef,
}

#[derive(Deserialize)]
struct Repo {
    default_branch: String,
    #[serde(default)]
    permissions: Option<Permissions>,
}

#[derive(Deserialize)]
pub(crate) struct Permissions {
    #[serde(default)]
    pub(crate) push: bool,
}

#[derive(Deserialize)]
pub(crate) struct Pull {
    pub(crate) number: u64,
    pub(crate) html_url: String,
}

impl Forgejo {
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
        req.header("Authorization", format!("token {}", self.http.token()))
            .header("Accept", "application/json")
    }
}

impl Forge for Forgejo {
    async fn probe(&self) -> Result<Probe, ForgeError> {
        let url = self.http.url(&self.repo_path());
        let repo: Repo = self
            .http
            .send(self.auth(self.http.client().get(url)))
            .await?;
        Ok(Probe {
            can_push: repo.permissions.is_some_and(|p| p.push),
            default_branch: repo.default_branch,
        })
    }

    async fn open_change(&self, change: &ChangeRequest) -> Result<OpenedChange, ForgeError> {
        let files: Vec<_> = change
            .files
            .iter()
            .map(|f| {
                let mut v = json!({
                    "operation": if f.expected_blob_sha.is_some() { "update" } else { "create" },
                    "path": f.path,
                    "content": b64(&f.new_content),
                });
                if let Some(sha) = &f.expected_blob_sha {
                    v["sha"] = json!(sha);
                }
                v
            })
            .collect();
        let body = json!({
            "branch": change.base_branch,
            "new_branch": change.new_branch,
            "message": change.title,
            "files": files,
        });
        let url = self.http.url(&format!("{}/contents", self.repo_path()));
        let _: serde_json::Value = self
            .http
            .send(self.auth(self.http.client().post(url)).json(&body))
            .await?;
        let pull = json!({
            "base": change.base_branch,
            "head": change.new_branch,
            "title": change.title,
            "body": change.body,
        });
        let url = self.http.url(&format!("{}/pulls", self.repo_path()));
        let pr: Pull = self
            .http
            .send(self.auth(self.http.client().post(url)).json(&pull))
            .await?;
        Ok(OpenedChange {
            url: pr.html_url,
            branch: change.new_branch.clone(),
            number: pr.number,
        })
    }
}
