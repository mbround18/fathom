//! Passkey ceremonies over the socket (spec 054), answered by `webauthn-authenticator-rs`'s
//! software passkey, as a browser would.

use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::SinkExt as _;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse, Url};

use super::ws::{Socket, ask, connect, csrf_of, hello};
use super::{App, ORIGIN, session_of};

pub type Authenticator = WebauthnAuthenticator<SoftPasskey>;

/// A subdomain of the relying party: the RP id still matches, the origin does not.
pub const ELSEWHERE: &str = "https://evil.depth.boop.ninja";

/// Request ids for the helpers' own commands, clear of the ones tests pick.
static NEXT: AtomicU64 = AtomicU64::new(50_000);

fn id() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub fn authenticator() -> Authenticator {
    // `true` lets the soft token claim user verification, which passkeys require.
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}

/// WebAuthn's JSON as a browser would send it: no nulls, no extension results.
fn browser_json(mut v: Value) -> Value {
    let obj = v.as_object_mut().expect("credential object");
    obj.remove("extensions");
    let response = obj["response"].as_object_mut().expect("response");
    response.retain(|_, f| !f.is_null());
    v
}

/// The browser's answer to registration `options`, made at `origin`.
pub fn attest(auth: &mut Authenticator, origin: &str, options: &Value) -> Value {
    let ccr: CreationChallengeResponse =
        serde_json::from_value(json!({ "publicKey": options })).expect("creation options");
    let reg = auth
        .do_registration(Url::parse(origin).unwrap(), ccr)
        .expect("softpasskey registers");
    browser_json(serde_json::to_value(reg).unwrap())
}

/// The browser's answer to assertion `options`, made at `origin`.
pub fn assert_with(auth: &mut Authenticator, origin: &str, options: &Value) -> Value {
    let rcr: RequestChallengeResponse =
        serde_json::from_value(json!({ "publicKey": options })).expect("request options");
    let cred = auth
        .do_authentication(Url::parse(origin).unwrap(), rcr)
        .expect("softpasskey asserts");
    browser_json(serde_json::to_value(cred).unwrap())
}

/// Signs `subject` in with a session that has not checked a passkey and opens a socket.
pub async fn open_unverified(
    a: &App,
    subject: &str,
) -> (Socket, tokio::task::JoinHandle<()>, String) {
    let session = a.sign_in_unverified(subject).await;
    let csrf = csrf_of(a, &session).await;
    let (mut ws, server) = connect(a, Some(&session)).await;
    ws.send(Message::text(hello(&csrf))).await.unwrap();
    (ws, server, session)
}

/// A `PasskeyRequest` (`{ceremony, options}`) out of `PasskeyChallenge` or `StepUpRequired`.
pub fn request_of(r: &Value) -> Value {
    match r["event"]["type"].as_str() {
        Some("PasskeyChallenge") => r["event"]["data"].clone(),
        Some("StepUpRequired") => r["event"]["data"]["request"].clone(),
        _ => panic!("no passkey request: {r}"),
    }
}

/// `PasskeyBegin{purpose}`; the reply.
pub async fn begin(ws: &mut Socket, purpose: Value) -> Value {
    ask(
        ws,
        id(),
        json!({ "type": "PasskeyBegin", "data": { "purpose": purpose } }),
    )
    .await
}

pub fn register(name: &str) -> Value {
    json!({ "kind": "register", "name": name })
}

pub fn verify() -> Value {
    json!({ "kind": "verify" })
}

/// `PasskeyFinish` for `ceremony` with `credential`; the reply.
pub async fn finish(ws: &mut Socket, ceremony: &Value, credential: Value) -> Value {
    ask(
        ws,
        id(),
        json!({ "type": "PasskeyFinish", "data": { "ceremony": ceremony, "credential": credential } }),
    )
    .await
}

/// Registers a passkey named `name` with `auth`; the `PasskeyRegistered` reply.
pub async fn enrol(ws: &mut Socket, auth: &mut Authenticator, name: &str) -> Value {
    let req = request_of(&begin(ws, register(name)).await);
    let r = finish(ws, &req["ceremony"], attest(auth, ORIGIN, &req["options"])).await;
    assert_eq!(r["event"]["type"], "PasskeyRegistered", "{r}");
    r
}

/// The answer to `request` (`{ceremony, options}`) made at `origin`, as `Confirm`/`OpenShell`
/// carry it.
pub fn answer(auth: &mut Authenticator, origin: &str, request: &Value) -> Value {
    json!({
        "ceremony": request["ceremony"],
        "credential": assert_with(auth, origin, &request["options"]),
    })
}

/// A whole `verify` ceremony; the reply (`Verification` on success).
pub async fn verify_now(ws: &mut Socket, auth: &mut Authenticator) -> Value {
    let req = request_of(&begin(ws, verify()).await);
    finish(
        ws,
        &req["ceremony"],
        assert_with(auth, ORIGIN, &req["options"]),
    )
    .await
}

/// This session's `Verification` data.
pub async fn verification(ws: &mut Socket) -> Value {
    let r = ask(ws, id(), json!({ "type": "GetVerification" })).await;
    assert_eq!(r["event"]["type"], "Verification", "{r}");
    r["event"]["data"].clone()
}

/// Makes `cookie`'s session's last check older than the step-up window (but still verified).
pub fn stale(a: &App, cookie: &str) {
    let long_ago =
        a.state.passkeys.now() - a.state.passkeys.window() - chrono::Duration::seconds(1);
    a.state
        .trust
        .sessions
        .set_verified(&session_of(cookie), long_ago);
}

/// Audit entries with `action` (its snake_case name).
pub fn audited(a: &App, action: &str) -> Vec<depths_protocol::AuditEntry> {
    a.audit_entries()
        .into_iter()
        .filter(|e| serde_json::to_value(e.action).unwrap() == action)
        .collect()
}
