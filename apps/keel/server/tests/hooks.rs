//! `POST /hooks/{receiver_id}` (spec 055 T039, contracts/receivers.md): the order of checks and
//! each refusal, per provider; the previous secret until it expires; the 1 MiB cap; payloads are
//! never echoed. Local bare repositories only. Needs the test cluster; skips without it.

mod common;
mod repos;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use keel_protocol::{Cmd, DeliveryDecision, Event, ObjectName, ReceiverProvider};
use repos::{Harness, Source, bind, manifest};
use serde_json::{Value, json};
use sha2::Sha256;
use uuid::Uuid;

const MARKER: &str = "ECHO-MARKER-7f3a";

struct Receiver {
    id: Uuid,
    secret: String,
}

async fn receiver(h: &Harness, name: &str, provider: ReceiverProvider) -> Receiver {
    let (p, out) = h
        .confirm(Cmd::PreviewReceiverCreate {
            workspace: h.workspace,
            name: ObjectName::new(name).expect("name"),
            provider,
        })
        .await;
    assert!(p.step_up, "creating a receiver needs step-up");
    let (url, secret) = match out.as_slice() {
        [
            _,
            Event::ReceiverSecret { url, secret },
            Event::Receivers { .. },
        ] => (url.clone(), secret.clone()),
        other => panic!("receiver create: {other:?}"),
    };
    let id = url
        .strip_prefix("https://keel.example.com/hooks/")
        .and_then(|s| Uuid::parse_str(s).ok())
        .expect("hook url");
    // The secret is never listed.
    let listed = serde_json::to_string(
        &h.cmd(Cmd::ListReceivers {
            workspace: h.workspace,
        })
        .await,
    )
    .expect("json");
    assert!(!listed.contains(&secret));
    Receiver { id, secret }
}

async fn link(h: &Harness, r: &Receiver, binding: Uuid) {
    let (p, _) = h
        .confirm(Cmd::PreviewReceiverBind {
            receiver: r.id,
            binding,
        })
        .await;
    assert!(p.step_up, "binding a receiver needs step-up");
}

fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).expect("key");
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn push_body(url: &str, r#ref: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "ref": r#ref,
        "repository": { "clone_url": url, "description": MARKER },
        "head_commit": { "id": "0000000000000000000000000000000000000000", "message": MARKER },
    }))
    .expect("json")
}

/// A GitHub-style delivery.
async fn github(
    h: &Harness,
    receiver: Uuid,
    secret: &str,
    delivery: &str,
    event: &str,
    body: Vec<u8>,
) -> (StatusCode, Value, String) {
    let sig = format!("sha256={}", sign(secret, &body));
    let req = Request::post(format!("/hooks/{receiver}"))
        .header("content-type", "application/json")
        .header("x-github-event", event)
        .header("x-github-delivery", delivery)
        .header("x-hub-signature-256", sig)
        .body(Body::from(body))
        .expect("req");
    let (status, body) = h.call(req).await;
    let text = String::from_utf8_lossy(&body).into_owned();
    assert!(
        !text.contains(MARKER),
        "the payload is never echoed: {text}"
    );
    (
        status,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
        text,
    )
}

/// Waits for the delivery's background sync to settle.
async fn settled(h: &Harness, delivery: &str) -> (String, Option<String>) {
    for _ in 0..200 {
        let row: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("select sync_state, commit from deliveries where id = $1::uuid")
                .bind(delivery)
                .fetch_optional(&h.db.owner)
                .await
                .expect("delivery");
        if let Some((Some(state), commit)) = row
            && state != "syncing"
        {
            return (state, commit);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("delivery {delivery} did not settle");
}

/// Asserts a 202 accepted reply and waits for its sync, so no fetch outlives the test.
async fn accepted(h: &Harness, status: StatusCode, body: &[u8]) {
    let v: Value = serde_json::from_slice(body).expect("json");
    assert_eq!(
        (status, v["decision"].as_str()),
        (StatusCode::ACCEPTED, Some("accepted")),
        "{v}"
    );
    let (state, _) = settled(h, v["delivery"].as_str().expect("id")).await;
    assert_ne!(state, "sync_failed");
}

fn code(v: &Value) -> &str {
    v.get("error").and_then(Value::as_str).unwrap_or("")
}

#[tokio::test]
async fn checks_run_in_order_and_refusals_are_recorded() {
    let Some(h) = Harness::new("hook_checks").await else {
        return;
    };
    let src = Source::new(&manifest(&[]));
    let url = src.url();
    let binding = bind(&h, &src).await;
    let r = receiver(&h, "gh", ReceiverProvider::Github).await;
    link(&h, &r, binding).await;
    let main = || push_body(&url, "refs/heads/main");

    // 1. Unknown receiver, and a path that is not an id.
    let (s, v, _) = github(&h, Uuid::now_v7(), &r.secret, "d0", "push", main()).await;
    assert_eq!((s, code(&v)), (StatusCode::NOT_FOUND, "unknown_receiver"));
    let req = Request::post("/hooks/not-an-id")
        .body(Body::from("{}"))
        .expect("req");
    assert_eq!(h.call(req).await.0, StatusCode::NOT_FOUND);

    // 2. A bad signature does not claim the delivery id.
    let (s, v, _) = github(&h, r.id, "wrong-secret", "d1", "push", main()).await;
    assert_eq!((s, code(&v)), (StatusCode::UNAUTHORIZED, "bad_signature"));

    // 6. Accepted, then fetched in the background.
    let head = src.commit("roles/web.yaml", "x: 1\n", "change");
    src.push();
    let (s, v, _) = github(&h, r.id, &r.secret, "d1", "push", main()).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["decision"], "accepted");
    let delivery = v["delivery"].as_str().expect("delivery id").to_owned();
    let (state, commit) = settled(&h, &delivery).await;
    assert_eq!(state, "synced");
    assert_eq!(commit.as_deref(), Some(head.as_str()));
    let (last,): (Option<String>,) =
        sqlx::query_as("select last_commit from repo_bindings where id = $1")
            .bind(binding)
            .fetch_one(&h.db.owner)
            .await
            .expect("binding");
    assert_eq!(last.as_deref(), Some(head.as_str()));

    // 3. The same delivery id again.
    let (s, v, _) = github(&h, r.id, &r.secret, "d1", "push", main()).await;
    assert_eq!((s, v), (StatusCode::OK, json!({ "decision": "duplicate" })));

    // A push without changes syncs as a duplicate head.
    let (_, v, _) = github(&h, r.id, &r.secret, "d2", "push", main()).await;
    let (state, _) = settled(&h, v["delivery"].as_str().expect("id")).await;
    assert_eq!(state, "duplicate");

    // 4. Not a branch.
    let (s, v, _) = github(
        &h,
        r.id,
        &r.secret,
        "d3",
        "push",
        push_body(&url, "refs/tags/v1"),
    )
    .await;
    assert_eq!((s, code(&v)), (StatusCode::ACCEPTED, "ref_not_branch"));

    // 5. No bound binding for that URL or branch, then a disabled one.
    let other = push_body("https://git.example.com/ops/other.git", "refs/heads/main");
    let (s, v, _) = github(&h, r.id, &r.secret, "d4", "push", other).await;
    assert_eq!(
        (s, code(&v)),
        (StatusCode::FORBIDDEN, "no_matching_binding")
    );
    let (s, v, _) = github(
        &h,
        r.id,
        &r.secret,
        "d5",
        "push",
        push_body(&url, "refs/heads/dev"),
    )
    .await;
    assert_eq!(
        (s, code(&v)),
        (StatusCode::FORBIDDEN, "no_matching_binding")
    );
    h.confirm(Cmd::PreviewBindingDisable { binding }).await;
    let (s, v, _) = github(&h, r.id, &r.secret, "d6", "push", main()).await;
    assert_eq!((s, code(&v)), (StatusCode::FORBIDDEN, "binding_disabled"));

    // GitHub's ping answers 200 and is recorded.
    let (s, v, _) = github(
        &h,
        r.id,
        &r.secret,
        "d7",
        "ping",
        b"{\"zen\":\"x\"}".to_vec(),
    )
    .await;
    assert_eq!((s, v["decision"].as_str()), (StatusCode::OK, Some("ping")));

    // 1. A disabled receiver.
    h.confirm(Cmd::PreviewReceiverDisable { receiver: r.id })
        .await;
    let (s, v, _) = github(&h, r.id, &r.secret, "d8", "push", main()).await;
    assert_eq!((s, code(&v)), (StatusCode::NOT_FOUND, "receiver_disabled"));

    // Every check is recorded, and refusals are audited.
    let out = h
        .cmd(Cmd::ListDeliveries {
            workspace: h.workspace,
            receiver: Some(r.id),
            binding: None,
            decision: Some(DeliveryDecision::Refused),
            from: None,
            to: None,
        })
        .await;
    let Some(Event::Deliveries { items }) = out.first() else {
        panic!("{out:?}");
    };
    let mut refusals: Vec<String> = items
        .iter()
        .map(|d| {
            serde_json::to_value(d.refusal)
                .expect("json")
                .as_str()
                .unwrap_or("")
                .to_owned()
        })
        .collect();
    refusals.sort();
    assert_eq!(
        refusals,
        [
            "bad_signature",
            "binding_disabled",
            "no_matching_binding",
            "no_matching_binding",
            "receiver_disabled",
            "ref_not_branch",
        ]
    );
    let audited = h
        .audit_actions()
        .await
        .iter()
        .filter(|(a, d)| a == "delivery_refused" && d == "denied")
        .count();
    assert_eq!(
        audited, 8,
        "six recorded refusals and two unknown receivers"
    );
    assert!(!h.audit_text().await.contains(&r.secret));
    h.finish().await;
}

#[tokio::test]
async fn the_previous_secret_verifies_until_it_expires() {
    let Some(h) = Harness::new("hook_rotation").await else {
        return;
    };
    let src = Source::new(&manifest(&[]));
    let url = src.url();
    let binding = bind(&h, &src).await;
    let r = receiver(&h, "gh", ReceiverProvider::Github).await;
    link(&h, &r, binding).await;

    let (p, out) = h
        .confirm(Cmd::PreviewReceiverRotate {
            receiver: r.id,
            previous_valid_for: Some(3600),
        })
        .await;
    assert!(p.step_up);
    let new = match out.as_slice() {
        [_, Event::ReceiverSecret { secret, .. }, ..] => secret.clone(),
        other => panic!("{other:?}"),
    };
    assert_ne!(new, r.secret);

    let body = || push_body(&url, "refs/heads/main");
    let (s, _, t) = github(&h, r.id, &r.secret, "old-1", "push", body()).await;
    accepted(&h, s, t.as_bytes()).await;
    let (s, _, t) = github(&h, r.id, &new, "new-1", "push", body()).await;
    accepted(&h, s, t.as_bytes()).await;

    sqlx::query("update receivers set previous_expires_at = now() - interval '1 second'")
        .execute(&h.db.owner)
        .await
        .expect("expire");
    let (s, v, _) = github(&h, r.id, &r.secret, "old-2", "push", body()).await;
    assert_eq!((s, code(&v)), (StatusCode::UNAUTHORIZED, "bad_signature"));
    let (s, _, t) = github(&h, r.id, &new, "new-2", "push", body()).await;
    accepted(&h, s, t.as_bytes()).await;
    h.finish().await;
}

#[tokio::test]
async fn gitlab_forgejo_and_generic_signatures() {
    let Some(h) = Harness::new("hook_providers").await else {
        return;
    };
    let src = Source::new(&manifest(&[]));
    let url = src.url();
    let binding = bind(&h, &src).await;

    let gl = receiver(&h, "gl", ReceiverProvider::Gitlab).await;
    link(&h, &gl, binding).await;
    let body = serde_json::to_vec(&json!({
        "ref": "refs/heads/main",
        "project": { "git_http_url": url },
    }))
    .expect("json");
    let gitlab = |token: &str, id: &str| {
        Request::post(format!("/hooks/{}", gl.id))
            .header("x-gitlab-event", "Push Hook")
            .header("x-gitlab-event-uuid", id)
            .header("x-gitlab-token", token)
            .body(Body::from(body.clone()))
            .expect("req")
    };
    assert_eq!(
        h.call(gitlab("nope", "g1")).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (s, b) = h.call(gitlab(&gl.secret, "g1")).await;
    accepted(&h, s, &b).await;

    let fj = receiver(&h, "fj", ReceiverProvider::Forgejo).await;
    link(&h, &fj, binding).await;
    let body = push_body(&url, "refs/heads/main");
    let req = Request::post(format!("/hooks/{}", fj.id))
        .header("x-gitea-event", "push")
        .header("x-gitea-delivery", "f1")
        .header("x-gitea-signature", sign(&fj.secret, &body))
        .body(Body::from(body))
        .expect("req");
    let (s, b) = h.call(req).await;
    accepted(&h, s, &b).await;

    let gen_r = receiver(&h, "generic", ReceiverProvider::Generic).await;
    link(&h, &gen_r, binding).await;
    let body =
        serde_json::to_vec(&json!({ "repository": url, "ref": "refs/heads/main" })).expect("json");
    let generic = |delivery: Option<&str>| {
        let mut b = Request::post(format!("/hooks/{}", gen_r.id)).header(
            "x-keel-signature",
            format!("sha256={}", sign(&gen_r.secret, &body)),
        );
        if let Some(d) = delivery {
            b = b.header("x-keel-delivery", d);
        }
        b.body(Body::from(body.clone())).expect("req")
    };
    // The generic delivery id is required.
    let (s, v) = h.call(generic(None)).await;
    let v: Value = serde_json::from_slice(&v).expect("json");
    assert_eq!((s, code(&v)), (StatusCode::BAD_REQUEST, "payload_invalid"));
    let (s, b) = h.call(generic(Some("k1"))).await;
    accepted(&h, s, &b).await;
    h.finish().await;
}

#[tokio::test]
async fn bodies_over_one_mebibyte_are_refused() {
    let Some(h) = Harness::new("hook_body_cap").await else {
        return;
    };
    let r = receiver(&h, "gh", ReceiverProvider::Github).await;
    let big = vec![b' '; keel_server::hooks::MAX_BODY_BYTES + 1];
    let (s, _, _) = github(&h, r.id, &r.secret, "big", "push", big).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    // Bigger than the 64 KiB command cap, under the hook cap: read and checked.
    let mut body = push_body("https://git.example.com/x.git", "refs/heads/main");
    body.extend(vec![b' '; 200 * 1024]);
    let (s, v, _) = github(&h, r.id, &r.secret, "mid", "push", body).await;
    assert_eq!(
        (s, code(&v)),
        (StatusCode::FORBIDDEN, "no_matching_binding")
    );
    h.finish().await;
}
