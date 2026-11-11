//! The adapters against a local axum fake that records every request and answers from a script.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::IntoResponse;
use keel_forge::{ChangeRequest, FileEdit, Forge, ForgeClient, ForgeConfig, ForgeError, Provider};
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
