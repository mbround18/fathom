//! The adapters against a local axum fake that records every request and answers from a script.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::IntoResponse;
use keel_forge::{
    ChangeRequest, CommitStatusState, FileEdit, Forge, ForgeClient, ForgeConfig, ForgeError,
    ForgeStatus, Provider,
};
use serde_json::{Value, json};

const TOKEN: &str = "fake-token-abc123";

#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    path: String,
    auth: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct Fake {
    log: Arc<Mutex<Vec<Recorded>>>,
    script: Arc<HashMap<(String, String), (u16, Value)>>,
}

async fn handle(
    State(fake): State<Fake>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let path = uri.path().to_owned();
    fake.log.lock().unwrap().push(Recorded {
        method: method.to_string(),
        path: path.clone(),
        auth: headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });
    let (status, value) = fake
        .script
        .get(&(method.to_string(), path))
        .cloned()
        .unwrap_or((404, json!({"message": "not scripted"})));
    (StatusCode::from_u16(status).unwrap(), axum::Json(value))
}

/// Starts a fake answering `script` (`(method, path) -> (status, body)`).
async fn fake(script: &[(&str, &str, u16, Value)]) -> (String, Fake) {
    let fake = Fake {
        log: Arc::default(),
        script: Arc::new(
            script
                .iter()
                .map(|(m, p, s, v)| (((*m).to_owned(), (*p).to_owned()), (*s, v.clone())))
                .collect(),
        ),
    };
    let app = Router::new().fallback(handle).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), fake)
}

fn client(provider: Provider, base: &str) -> ForgeClient {
    ForgeClient::new(ForgeConfig {
        provider,
        repo_url: "ssh://git@forge.example.test/ops/ansible.git".into(),
        api_base: Some(base.to_owned()),
        token: TOKEN.into(),
    })
    .unwrap()
}

fn change() -> ChangeRequest {
    ChangeRequest {
        base_branch: "main".into(),
        new_branch: "keel/add-host-newbox".into(),
        title: "Add host newbox".into(),
        body: "Proposed by Keel.".into(),
        files: vec![FileEdit {
            path: "inventory/hosts.yml".into(),
            expected_blob_sha: Some("1111111111111111111111111111111111111111".into()),
            new_content: b"all:\n".to_vec(),
        }],
    }
}

fn log(f: &Fake) -> Vec<Recorded> {
    f.log.lock().unwrap().clone()
}

#[tokio::test]
async fn forgejo_change_files_then_pull() {
    let (base, f) = fake(&[
        (
            "POST",
            "/repos/ops/ansible/contents",
            201,
            json!({"commit": {}}),
        ),
        (
            "POST",
            "/repos/ops/ansible/pulls",
            201,
            json!({"number": 7, "html_url": "https://forge.example.test/ops/ansible/pulls/7"}),
        ),
    ])
    .await;
    let opened = client(Provider::Forgejo, &base)
        .open_change(&change())
        .await
        .unwrap();
    assert_eq!(opened.number, 7);
    assert_eq!(opened.branch, "keel/add-host-newbox");
    let reqs = log(&f);
    assert_eq!(reqs.len(), 2);
    let cf = &reqs[0];
    assert_eq!(cf.auth.as_deref(), Some(format!("token {TOKEN}").as_str()));
    assert_eq!(cf.body["branch"], "main");
    assert_eq!(cf.body["new_branch"], "keel/add-host-newbox");
    let file = &cf.body["files"][0];
    assert_eq!(file["operation"], "update");
    assert_eq!(file["sha"], "1111111111111111111111111111111111111111");
    assert_eq!(file["content"], "YWxsOgo=");
    assert_eq!(reqs[1].body["head"], "keel/add-host-newbox");
    assert_eq!(reqs[1].body["base"], "main");
}

#[tokio::test]
async fn forgejo_sha_mismatch_is_a_conflict_and_no_pull_is_opened() {
    let (base, f) = fake(&[(
        "POST",
        "/repos/ops/ansible/contents",
        409,
        json!({"message": "sha does not match"}),
    )])
    .await;
    let err = client(Provider::Gitea, &base)
        .open_change(&change())
        .await
        .unwrap_err();
    assert_eq!(err, ForgeError::Conflict("sha does not match".into()));
    assert_eq!(err.code(), "forge_refused");
    assert_eq!(log(&f).len(), 1);
}

#[tokio::test]
async fn refusals_map_and_never_carry_the_token() {
    let (base, _f) = fake(&[(
        "POST",
        "/repos/ops/ansible/contents",
        403,
        json!({"message": "branch is protected"}),
    )])
    .await;
    let err = client(Provider::Forgejo, &base)
        .open_change(&change())
        .await
        .unwrap_err();
    assert_eq!(
        err,
        ForgeError::Refused {
            status: 403,
            message: "branch is protected".into()
        }
    );
    assert!(!format!("{err} {err:?}").contains(TOKEN));

    let (base, _f) = fake(&[("GET", "/repos/ops/ansible", 401, json!({}))]).await;
    assert_eq!(
        client(Provider::Forgejo, &base).probe().await,
        Err(ForgeError::Unauthorized)
    );
}

#[tokio::test]
async fn probe_reads_permissions_push() {
    let (base, f) = fake(&[(
        "GET",
        "/repos/ops/ansible",
        200,
        json!({"default_branch": "main", "permissions": {"admin": false, "push": true, "pull": true}}),
    )])
    .await;
    let p = client(Provider::Forgejo, &base).probe().await.unwrap();
    assert!(p.can_push);
    assert_eq!(p.default_branch, "main");
    assert_eq!(log(&f)[0].method, "GET");

    let (base, _f) = fake(&[(
        "GET",
        "/repos/ops/ansible",
        200,
        json!({"default_branch": "trunk", "permissions": {"push": false, "pull": true}}),
    )])
    .await;
    let p = client(Provider::Github, &base).probe().await.unwrap();
    assert!(!p.can_push);
    assert_eq!(p.default_branch, "trunk");
}

#[tokio::test]
async fn github_ref_branch_put_then_pull() {
    let (base, f) = fake(&[
        (
            "GET",
            "/repos/ops/ansible/git/ref/heads/main",
            200,
            json!({"ref": "refs/heads/main", "object": {"sha": "abcabcabcabcabcabcabcabcabcabcabcabcabca"}}),
        ),
        ("POST", "/repos/ops/ansible/git/refs", 201, json!({})),
        (
            "PUT",
            "/repos/ops/ansible/contents/inventory/hosts.yml",
            200,
            json!({"content": {}}),
        ),
        (
            "POST",
            "/repos/ops/ansible/pulls",
            201,
            json!({"number": 12, "html_url": "https://github.example.test/ops/ansible/pull/12"}),
        ),
    ])
    .await;
    let opened = client(Provider::Github, &base)
        .open_change(&change())
        .await
        .unwrap();
    assert_eq!(opened.number, 12);
    let reqs = log(&f);
    let order: Vec<_> = reqs
        .iter()
        .map(|r| (r.method.as_str(), r.path.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            ("GET", "/repos/ops/ansible/git/ref/heads/main"),
            ("POST", "/repos/ops/ansible/git/refs"),
            ("PUT", "/repos/ops/ansible/contents/inventory/hosts.yml"),
            ("POST", "/repos/ops/ansible/pulls"),
        ]
    );
    assert!(
        reqs.iter()
            .all(|r| r.auth.as_deref() == Some(&*format!("Bearer {TOKEN}")))
    );
    assert_eq!(reqs[1].body["ref"], "refs/heads/keel/add-host-newbox");
    assert_eq!(
        reqs[1].body["sha"],
        "abcabcabcabcabcabcabcabcabcabcabcabcabca"
    );
    assert_eq!(reqs[2].body["branch"], "keel/add-host-newbox");
    assert_eq!(
        reqs[2].body["sha"],
        "1111111111111111111111111111111111111111"
    );
}

#[tokio::test]
async fn github_sha_mismatch_stops_before_the_pull() {
    let (base, f) = fake(&[
        (
            "GET",
            "/repos/ops/ansible/git/ref/heads/main",
            200,
            json!({"object": {"sha": "abcabcabcabcabcabcabcabcabcabcabcabcabca"}}),
        ),
        ("POST", "/repos/ops/ansible/git/refs", 201, json!({})),
        (
            "PUT",
            "/repos/ops/ansible/contents/inventory/hosts.yml",
            409,
            json!({"message": "inventory/hosts.yml does not match 1111"}),
        ),
    ])
    .await;
    let err = client(Provider::Github, &base)
        .open_change(&change())
        .await
        .unwrap_err();
    assert!(matches!(err, ForgeError::Conflict(_)), "{err:?}");
    assert!(log(&f).iter().all(|r| !r.path.ends_with("/pulls")));
}

#[tokio::test]
async fn new_file_omits_sha() {
    let (base, f) = fake(&[
        ("POST", "/repos/ops/ansible/contents", 201, json!({})),
        (
            "POST",
            "/repos/ops/ansible/pulls",
            201,
            json!({"number": 1, "html_url": "u"}),
        ),
    ])
    .await;
    let mut c = change();
    c.files[0].expected_blob_sha = None;
    client(Provider::Forgejo, &base)
        .open_change(&c)
        .await
        .unwrap();
    let file = &log(&f)[0].body["files"][0];
    assert_eq!(file["operation"], "create");
    assert!(file.get("sha").is_none());
}

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn status(context: &str, state: CommitStatusState, url: Option<&str>) -> ForgeStatus {
    ForgeStatus {
        context: context.into(),
        state,
        target_url: url.map(str::to_owned),
    }
}

#[tokio::test]
async fn forgejo_statuses_read_the_combined_status() {
    let path = format!("/repos/ops/ansible/commits/{SHA}/status");
    let (base, f) = fake(&[(
        "GET",
        path.as_str(),
        200,
        json!({
            "state": "failure",
            "sha": SHA,
            "statuses": [
                {"context": "ci/lint", "status": "success", "target_url": "https://ci.example.test/1"},
                {"context": "ci/test", "status": "failure", "target_url": ""},
                {"context": "ci/slow", "status": "pending"},
                {"context": "ci/warn", "status": "warning"},
                {"context": "ci/old", "state": "error"}
            ]
        }),
    )])
    .await;
    let got = client(Provider::Forgejo, &base)
        .commit_statuses(SHA)
        .await
        .unwrap();
    assert_eq!(
        got,
        vec![
            status(
                "ci/lint",
                CommitStatusState::Success,
                Some("https://ci.example.test/1")
            ),
            status("ci/test", CommitStatusState::Failure, None),
            status("ci/slow", CommitStatusState::Pending, None),
            status("ci/warn", CommitStatusState::Error, None),
            status("ci/old", CommitStatusState::Error, None),
        ]
    );
    let reqs = log(&f);
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(
        reqs[0].auth.as_deref(),
        Some(format!("token {TOKEN}").as_str())
    );
}

#[tokio::test]
async fn forgejo_statuses_with_none_reported_are_empty() {
    let path = format!("/repos/ops/ansible/commits/{SHA}/status");
    let (base, _f) = fake(&[(
        "GET",
        path.as_str(),
        200,
        json!({"state": "", "statuses": null}),
    )])
    .await;
    let got = client(Provider::Gitea, &base)
        .commit_statuses(SHA)
        .await
        .unwrap();
    assert!(got.is_empty());
}

#[tokio::test]
async fn github_statuses_add_check_runs() {
    let status_path = format!("/repos/ops/ansible/commits/{SHA}/status");
    let runs_path = format!("/repos/ops/ansible/commits/{SHA}/check-runs");
    let (base, f) = fake(&[
        (
            "GET",
            status_path.as_str(),
            200,
            json!({"state": "success", "statuses": [
                {"context": "ci/legacy", "state": "success", "target_url": "https://ci.example.test/l"}
            ]}),
        ),
        (
            "GET",
            runs_path.as_str(),
            200,
            json!({"total_count": 5, "check_runs": [
                {"name": "build", "status": "completed", "conclusion": "success",
                 "html_url": "https://github.example.test/r/1"},
                {"name": "lint", "status": "completed", "conclusion": "skipped"},
                {"name": "test", "status": "in_progress", "conclusion": null},
                {"name": "deploy", "status": "completed", "conclusion": "timed_out"},
                {"name": "flaky", "status": "completed", "conclusion": "cancelled"}
            ]}),
        ),
    ])
    .await;
    let got = client(Provider::Github, &base)
        .commit_statuses(SHA)
        .await
        .unwrap();
    assert_eq!(
        got,
        vec![
            status(
                "ci/legacy",
                CommitStatusState::Success,
                Some("https://ci.example.test/l")
            ),
            status(
                "build",
                CommitStatusState::Success,
                Some("https://github.example.test/r/1")
            ),
            status("lint", CommitStatusState::Success, None),
            status("test", CommitStatusState::Pending, None),
            status("deploy", CommitStatusState::Failure, None),
            status("flaky", CommitStatusState::Error, None),
        ]
    );
    let reqs = log(&f);
    assert_eq!(reqs.len(), 2);
    assert_eq!(
        reqs[0].auth.as_deref(),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}

#[tokio::test]
async fn statuses_refuse_a_bad_sha_without_a_request_and_map_errors() {
    let (base, f) = fake(&[]).await;
    for p in [Provider::Forgejo, Provider::Github] {
        let err = client(p, &base)
            .commit_statuses("../../admin")
            .await
            .unwrap_err();
        assert!(matches!(err, ForgeError::Url(_)), "{err:?}");
    }
    assert!(log(&f).is_empty(), "a request was sent");
    // Unscripted: the forge says 404.
    let err = client(Provider::Forgejo, &base)
        .commit_statuses(SHA)
        .await
        .unwrap_err();
    assert_eq!(err, ForgeError::NotFound);
}

/// A forge that does not implement `commit_statuses` says `Unsupported`.
#[tokio::test]
async fn statuses_default_to_unsupported() {
    struct Bare;
    impl Forge for Bare {
        fn probe(&self) -> keel_forge::ForgeFuture<'_, keel_forge::ForgeAccess> {
            Box::pin(async { Err(ForgeError::NotFound) })
        }
        fn open_change<'a>(
            &'a self,
            _change: &'a ChangeRequest,
        ) -> keel_forge::ForgeFuture<'a, keel_forge::OpenedChange> {
            Box::pin(async { Err(ForgeError::NotFound) })
        }
    }
    assert_eq!(
        Bare.commit_statuses(SHA).await.unwrap_err(),
        ForgeError::Unsupported
    );
}
