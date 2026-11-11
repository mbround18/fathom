//! GitHub: read the base branch's head, create the new branch at it, `PUT` each file with its
//! `sha` precondition, then open a pull request.

use serde::Deserialize;
use serde_json::json;

use crate::forgejo::{Permissions, Pull};
use crate::{
    ChangeRequest, Forge, ForgeError, Http, OpenedChange, Probe, RepoRef, b64, encode_path,
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
