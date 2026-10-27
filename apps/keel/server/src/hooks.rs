//! `POST /hooks/{receiver_id}` (spec 055 T050, contracts/receivers.md).
//!
//! The public endpoint source-control hosts call. It is unauthenticated except for the signature,
//! limited per receiver and per source address, and its body is capped at 1 MiB. Checks run in
//! the contract's order, and every refusal is audited:
//!
//! 1. the receiver exists (`404 unknown_receiver`) and is active (`404 receiver_disabled`);
//! 2. the signature verifies against the current secret, or the previous one until it expires
//!    (`401 bad_signature`);
//! 3. the delivery id is new for this receiver (`200 {"decision":"duplicate"}`);
//! 4. the ref is a branch (`202 ref_not_branch`);
//! 5. a bound, active binding matches the URL and branch (`403 no_matching_binding` or
//!    `binding_disabled`);
//! 6. `202 {"decision":"accepted","delivery":<id>}`, then the fetch runs in the background.
//!
//! Only the clone URLs, `ref` and the delivery id are read from the payload; commit data in it
//! is ignored, since the fetch decides the head. Replies never echo the payload and never say
//! which bindings exist. Nothing here logs a secret, a signature or a body.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, KeyInit, Mac};
use keel_core::domain::WorkspaceId;
use keel_protocol::{Event, ReceiverProvider};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use sha2::Sha256;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::bindings::{enum_from, url_key};
use crate::serve::{AppState, Peer, error};
use crate::sync::{self, Trigger};

/// The largest delivery body.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// The longest delivery id kept.
const MAX_DELIVERY_ID: usize = 200;

/// Fixed-window request limits for the hook endpoint.
#[derive(Debug)]
pub struct Limiter {
    window: Duration,
    per_receiver: u32,
    per_address: u32,
    counts: Mutex<HashMap<Key, (Instant, u32)>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Receiver(Uuid),
    Address(IpAddr),
    /// Any path that is not a receiver id, counted together.
    Malformed,
}

impl Default for Limiter {
    /// 120 deliveries a minute per receiver, 60 per source address.
    fn default() -> Self {
        Self::new(Duration::from_secs(60), 120, 60)
    }
}

impl Limiter {
    #[must_use]
    pub fn new(window: Duration, per_receiver: u32, per_address: u32) -> Self {
        Self {
            window,
            per_receiver,
            per_address,
            counts: Mutex::new(HashMap::new()),
        }
    }

    /// Counts one request; false once either limit is spent for this window.
    pub fn allow(&self, receiver: Option<Uuid>, address: Option<IpAddr>, now: Instant) -> bool {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let window = self.window;
        counts.retain(|_, (start, _)| now.saturating_duration_since(*start) < window);
        let mut take = |key: Key, limit: u32| {
            let slot = counts.entry(key).or_insert((now, 0));
            slot.1 = slot.1.saturating_add(1);
            slot.1 <= limit
        };
        let by_receiver = take(
            receiver.map_or(Key::Malformed, Key::Receiver),
            self.per_receiver,
        );
        let by_address = address.is_none_or(|a| take(Key::Address(a), self.per_address));
        by_receiver && by_address
    }
}

/// A refusal: the status, the code, and whether it is recorded in `deliveries`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    UnknownReceiver,
    ReceiverDisabled,
    BadSignature,
    RefNotBranch,
    NoMatchingBinding,
    BindingDisabled,
    PayloadInvalid,
}

impl Refusal {
    const fn code(self) -> &'static str {
        match self {
            Refusal::UnknownReceiver => "unknown_receiver",
            Refusal::ReceiverDisabled => "receiver_disabled",
            Refusal::BadSignature => "bad_signature",
            Refusal::RefNotBranch => "ref_not_branch",
            Refusal::NoMatchingBinding => "no_matching_binding",
            Refusal::BindingDisabled => "binding_disabled",
            Refusal::PayloadInvalid => "payload_invalid",
        }
    }

    const fn status(self) -> StatusCode {
        match self {
            Refusal::UnknownReceiver | Refusal::ReceiverDisabled => StatusCode::NOT_FOUND,
            Refusal::BadSignature => StatusCode::UNAUTHORIZED,
            Refusal::RefNotBranch => StatusCode::ACCEPTED,
            Refusal::NoMatchingBinding | Refusal::BindingDisabled => StatusCode::FORBIDDEN,
            Refusal::PayloadInvalid => StatusCode::BAD_REQUEST,
        }
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// The provider's delivery id header.
fn delivery_id(provider: ReceiverProvider, headers: &HeaderMap) -> Option<String> {
    let v = match provider {
        ReceiverProvider::Github => header(headers, "x-github-delivery"),
        ReceiverProvider::Gitlab => header(headers, "x-gitlab-event-uuid"),
        ReceiverProvider::Forgejo => {
            header(headers, "x-forgejo-delivery").or_else(|| header(headers, "x-gitea-delivery"))
        }
        ReceiverProvider::Generic => header(headers, "x-keel-delivery"),
    }?;
    let v = v.trim();
    (!v.is_empty()).then(|| v.to_owned())
}

/// What the delivery's event header says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Push,
    Ping,
    Other,
}

fn event_kind(provider: ReceiverProvider, headers: &HeaderMap) -> Kind {
    let is = |v: Option<&str>, want: &str| v.is_some_and(|v| v.trim().eq_ignore_ascii_case(want));
    match provider {
        ReceiverProvider::Github => match header(headers, "x-github-event") {
            Some("push") => Kind::Push,
            Some("ping") => Kind::Ping,
            _ => Kind::Other,
        },
        ReceiverProvider::Gitlab => {
            if is(header(headers, "x-gitlab-event"), "Push Hook") {
                Kind::Push
            } else {
                Kind::Other
            }
        }
        ReceiverProvider::Forgejo => {
            let ev =
                header(headers, "x-forgejo-event").or_else(|| header(headers, "x-gitea-event"));
            if is(ev, "push") {
                Kind::Push
            } else {
                Kind::Other
            }
        }
        ReceiverProvider::Generic => Kind::Push,
    }
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) || s.len() > 256 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn hmac_ok(secret: &[u8], body: &[u8], hex_sig: Option<&str>) -> bool {
    let Some(sig) = hex_sig.and_then(unhex) else {
        return false;
    };
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);
    // Constant-time.
    mac.verify_slice(&sig).is_ok()
}

/// Whether the request carries a valid signature for `secret`.
fn signature_ok(
    provider: ReceiverProvider,
    headers: &HeaderMap,
    body: &[u8],
    secret: &[u8],
) -> bool {
    let prefixed = |name: &str| header(headers, name).and_then(|v| v.strip_prefix("sha256="));
    match provider {
        ReceiverProvider::Github => hmac_ok(secret, body, prefixed("x-hub-signature-256")),
        ReceiverProvider::Generic => hmac_ok(secret, body, prefixed("x-keel-signature")),
        ReceiverProvider::Forgejo => {
            let sig = header(headers, "x-forgejo-signature")
                .or_else(|| header(headers, "x-gitea-signature"));
            hmac_ok(secret, body, sig)
        }
        ReceiverProvider::Gitlab => match std::str::from_utf8(secret) {
            Ok(s) => web_auth::csrf::csrf_token_valid(s, header(headers, "x-gitlab-token")),
            Err(_) => false,
        },
    }
}

/// The repository URLs and ref a push payload names. Nothing else is read.
fn read_payload(body: &[u8]) -> Option<(Vec<String>, String)> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let r#ref = v.get("ref")?.as_str()?.to_owned();
    let mut urls = Vec::new();
    if let Some(s) = v.get("repository").and_then(Value::as_str) {
        urls.push(s.to_owned());
    }
    for obj in ["repository", "project"] {
        for field in ["clone_url", "ssh_url", "git_http_url", "git_ssh_url"] {
            if let Some(s) = v
                .get(obj)
                .and_then(|o| o.get(field))
                .and_then(Value::as_str)
            {
                urls.push(s.to_owned());
            }
        }
    }
    urls.dedup();
    (!urls.is_empty()).then_some((urls, r#ref))
}

/// Reads the delivery id and cuts it to the stored length (on a character boundary).
fn clip(id: &str) -> String {
    id.chars().take(MAX_DELIVERY_ID).collect()
}

struct Receiver {
    id: Uuid,
    workspace: Uuid,
    provider: ReceiverProvider,
    active: bool,
    current: Vec<u8>,
    previous: Option<Vec<u8>>,
    previous_expires_at: Option<OffsetDateTime>,
}

async fn load(app: &AppState, id: Uuid) -> Result<Option<Receiver>, sqlx::Error> {
    type Row = (
        Uuid,
        String,
        String,
        Vec<u8>,
        Option<Vec<u8>>,
        Option<OffsetDateTime>,
    );
    let row: Option<Row> = sqlx::query_as(
        "select workspace_id, provider, status, secret_current, secret_previous, \
         previous_expires_at from receivers where id = $1",
    )
    .bind(id)
    .fetch_optional(&app.db)
    .await?;
    Ok(row.and_then(
        |(workspace, provider, status, current, previous, expires)| {
            Some(Receiver {
                id,
                workspace,
                provider: enum_from(&provider)?,
                active: status == "active",
                current,
                previous,
                previous_expires_at: expires,
            })
        },
    ))
}

impl Receiver {
    /// Tries the current secret, then the previous one while it has not expired. Fails closed
    /// when a secret cannot be opened.
    fn verify(&self, app: &AppState, headers: &HeaderMap, body: &[u8]) -> Result<bool, ()> {
        let key = app.repos.master_key.as_ref().ok_or(())?;
        let aad = crate::receivers::aad(self.id);
        let current = key.open(&aad, &self.current).map_err(|_| ())?;
        if signature_ok(self.provider, headers, body, current.expose_secret()) {
            return Ok(true);
        }
        let now = app.clock.now();
        if let (Some(prev), Some(until)) = (&self.previous, self.previous_expires_at)
            && until > now
        {
            let prev = key.open(&aad, prev).map_err(|_| ())?;
            return Ok(signature_ok(
                self.provider,
                headers,
                body,
                prev.expose_secret(),
            ));
        }
        Ok(false)
    }
}

/// A delivery row to write.
struct Record<'a> {
    receiver: &'a Receiver,
    delivery_id: String,
    binding: Option<Uuid>,
    refusal: Option<Refusal>,
    sync_state: Option<&'static str>,
}

/// Writes the delivery (and, for a refusal, its audit entry). `Ok(None)`: the delivery id was
/// already claimed, so this is a duplicate.
async fn record(
    app: &AppState,
    r: Record<'_>,
    peer: Option<IpAddr>,
) -> Result<Option<Uuid>, sqlx::Error> {
    let id = Uuid::now_v7();
    let mut tx = app.db.begin().await?;
    let n = sqlx::query(
        "insert into deliveries (id, receiver_id, provider_delivery_id, received_at, binding_id, \
         decision, refusal, sync_state) values ($1, $2, $3, $4, $5, $6, $7, $8) \
         on conflict do nothing",
    )
    .bind(id)
    .bind(r.receiver.id)
    .bind(&r.delivery_id)
    .bind(app.clock.now())
    .bind(r.binding)
    .bind(if r.refusal.is_some() {
        "refused"
    } else {
        "accepted"
    })
    .bind(r.refusal.map(Refusal::code))
    .bind(r.sync_state)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        tx.rollback().await?;
        return Ok(None);
    }
    if let Some(refusal) = r.refusal {
        audit_refusal(&mut tx, Some(r.receiver), refusal, &r.delivery_id, peer).await?;
    }
    tx.commit().await?;
    Ok(Some(id))
}

async fn audit_refusal(
    tx: &mut sqlx::PgConnection,
    receiver: Option<&Receiver>,
    refusal: Refusal,
    delivery_id: &str,
    peer: Option<IpAddr>,
) -> Result<(), sqlx::Error> {
    audit::record(
        tx,
        Entry {
            actor: &receiver.map_or(Actor::System, |r| Actor::Receiver(r.id)),
            action: "delivery_refused",
            workspace: receiver.map(|r| WorkspaceId(r.workspace)),
            target: json!({ "receiver": receiver.map(|r| r.id) }),
            decision: Decision::Denied,
            reason: Some(refusal.code()),
            context: json!({
                "delivery_id": delivery_id,
                "peer": peer.map(|p| p.to_string()),
            }),
        },
    )
    .await
    .map(|_| ())
}

fn reply(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

fn refused(refusal: Refusal) -> Response {
    error(refusal.status(), refusal.code())
}

fn unavailable() -> Response {
    error(StatusCode::SERVICE_UNAVAILABLE, "unavailable")
}

/// Records a refusal, pushes it, and answers it.
async fn refuse(
    app: &AppState,
    receiver: &Receiver,
    delivery_id: String,
    binding: Option<Uuid>,
    refusal: Refusal,
    peer: Option<IpAddr>,
) -> Response {
    let rec = Record {
        receiver,
        delivery_id,
        binding,
        refusal: Some(refusal),
        sync_state: None,
    };
    match record(app, rec, peer).await {
        Ok(Some(id)) => {
            push(app, receiver.workspace, id).await;
            refused(refusal)
        }
        Ok(None) => reply(StatusCode::OK, json!({ "decision": "duplicate" })),
        Err(e) => {
            tracing::error!(error = %e, "delivery record failed");
            unavailable()
        }
    }
}

async fn push(app: &AppState, workspace: Uuid, delivery: Uuid) {
    if let Some(info) = crate::receivers::delivery(app, delivery).await {
        app.repos
            .push(workspace, Event::DeliveryReceived { delivery: info });
    }
}

/// The handler.
pub async fn receive(
    State(app): State<AppState>,
    Path(raw): Path<String>,
    Peer(peer): Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let peer = peer.map(|p| p.ip());
    let id = Uuid::parse_str(&raw).ok();
    if !app.repos.hook_limits.allow(id, peer, Instant::now()) {
        return error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    }

    // 1. The receiver exists and is active.
    let receiver = match id {
        Some(id) => match load(&app, id).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "receiver lookup failed");
                return unavailable();
            }
        },
        None => None,
    };
    let Some(receiver) = receiver else {
        let audited = async {
            let mut tx = app.db.begin().await?;
            audit_refusal(&mut tx, None, Refusal::UnknownReceiver, "", peer).await?;
            tx.commit().await
        }
        .await;
        if let Err(e) = audited {
            tracing::error!(error = %e, "audit write failed");
        }
        return refused(Refusal::UnknownReceiver);
    };
    let provider = receiver.provider;
    let delivery = delivery_id(provider, &headers);
    let stored_id = clip(delivery.as_deref().unwrap_or(""));
    if !receiver.active {
        return refuse(
            &app,
            &receiver,
            stored_id,
            None,
            Refusal::ReceiverDisabled,
            peer,
        )
        .await;
    }

    // 2. The signature.
    match receiver.verify(&app, &headers, &body) {
        Ok(true) => {}
        Ok(false) => {
            return refuse(
                &app,
                &receiver,
                stored_id,
                None,
                Refusal::BadSignature,
                peer,
            )
            .await;
        }
        Err(()) => {
            tracing::error!(receiver = %receiver.id, "receiver secret cannot be opened");
            return unavailable();
        }
    }
    let Some(delivery) = delivery.filter(|d| d.chars().count() <= MAX_DELIVERY_ID) else {
        return refuse(
            &app,
            &receiver,
            stored_id,
            None,
            Refusal::PayloadInvalid,
            peer,
        )
        .await;
    };

    // 3. A new delivery id. (The insert below settles a race between two copies.)
    let seen: Result<Option<(Uuid,)>, _> = sqlx::query_as(
        "select id from deliveries where receiver_id = $1 and provider_delivery_id = $2 \
         and refusal is distinct from 'bad_signature' \
         and refusal is distinct from 'receiver_disabled'",
    )
    .bind(receiver.id)
    .bind(&delivery)
    .fetch_optional(&app.db)
    .await;
    match seen {
        Ok(Some(_)) => return reply(StatusCode::OK, json!({ "decision": "duplicate" })),
        Ok(None) => {}
        Err(e) => {
            tracing::error!(error = %e, "delivery lookup failed");
            return unavailable();
        }
    }

    match event_kind(provider, &headers) {
        Kind::Push => {}
        Kind::Ping => {
            let rec = Record {
                receiver: &receiver,
                delivery_id: delivery,
                binding: None,
                refusal: None,
                sync_state: None,
            };
            return match record(&app, rec, peer).await {
                Ok(Some(id)) => {
                    push(&app, receiver.workspace, id).await;
                    reply(StatusCode::OK, json!({ "decision": "ping" }))
                }
                Ok(None) => reply(StatusCode::OK, json!({ "decision": "duplicate" })),
                Err(e) => {
                    tracing::error!(error = %e, "delivery record failed");
                    unavailable()
                }
            };
        }
        // Another event (issues, tags ...): recorded, nothing fetched.
        Kind::Other => {
            return refuse(
                &app,
                &receiver,
                delivery,
                None,
                Refusal::PayloadInvalid,
                peer,
            )
            .await;
        }
    }
    let Some((urls, r#ref)) = read_payload(&body) else {
        return refuse(
            &app,
            &receiver,
            delivery,
            None,
            Refusal::PayloadInvalid,
            peer,
        )
        .await;
    };

    // 4. A branch.
    let Some(branch) = r#ref.strip_prefix("refs/heads/").filter(|b| !b.is_empty()) else {
        return refuse(&app, &receiver, delivery, None, Refusal::RefNotBranch, peer).await;
    };

    // 5. A bound binding for that URL and branch.
    let keys: Vec<String> = urls.iter().filter_map(|u| url_key(&app, u)).collect();
    let matches: Result<Vec<(Uuid, String)>, _> = sqlx::query_as(
        "select b.id, b.status from receiver_bindings rb \
         join repo_bindings b on b.id = rb.binding_id \
         where rb.receiver_id = $1 and b.branch = $2 and b.url_key = any($3) order by b.id",
    )
    .bind(receiver.id)
    .bind(branch)
    .bind(&keys)
    .fetch_all(&app.db)
    .await;
    let matches = match matches {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "binding lookup failed");
            return unavailable();
        }
    };
    let active: Vec<Uuid> = matches
        .iter()
        .filter(|(_, s)| s == "active")
        .map(|(b, _)| *b)
        .collect();
    let Some(&first) = active.first() else {
        let (refusal, binding) = match matches.first() {
            Some((b, _)) => (Refusal::BindingDisabled, Some(*b)),
            None => (Refusal::NoMatchingBinding, None),
        };
        return refuse(&app, &receiver, delivery, binding, refusal, peer).await;
    };

    // 6. Accepted; the fetch runs after the reply.
    let rec = Record {
        receiver: &receiver,
        delivery_id: delivery,
        binding: Some(first),
        refusal: None,
        sync_state: Some("syncing"),
    };
    let id = match record(&app, rec, peer).await {
        Ok(Some(id)) => id,
        Ok(None) => return reply(StatusCode::OK, json!({ "decision": "duplicate" })),
        Err(e) => {
            tracing::error!(error = %e, "delivery record failed");
            return unavailable();
        }
    };
    push(&app, receiver.workspace, id).await;
    let (rid, workspace) = (receiver.id, receiver.workspace);
    let task_app = app.clone();
    tokio::spawn(async move {
        follow(&task_app, rid, workspace, id, active).await;
    });
    reply(
        StatusCode::ACCEPTED,
        json!({ "decision": "accepted", "delivery": id }),
    )
}

/// Syncs every matching binding; the delivery records the first one's outcome.
async fn follow(
    app: &AppState,
    receiver: Uuid,
    workspace: Uuid,
    delivery: Uuid,
    bindings: Vec<Uuid>,
) {
    let mut outcome: Option<(&'static str, Option<String>)> = None;
    for binding in bindings {
        let trigger = Trigger::Delivery { receiver, delivery };
        let result = match sync::sync_binding(app, binding, trigger).await {
            Ok(s) => (s.state, Some(s.commit)),
            Err(_) => ("sync_failed", None),
        };
        outcome.get_or_insert(result);
    }
    let Some((state, commit)) = outcome else {
        return;
    };
    let updated = sqlx::query("update deliveries set sync_state = $2, commit = $3 where id = $1")
        .bind(delivery)
        .bind(state)
        .bind(commit)
        .execute(&app.db)
        .await;
    if let Err(e) = updated {
        tracing::error!(error = %e, "delivery update failed");
        return;
    }
    push(app, workspace, delivery).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret).expect("key");
        mac.update(body);
        crate::credentials::hex(&mac.finalize().into_bytes())
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).expect("header"));
        }
        h
    }

    #[test]
    fn signatures_per_provider() {
        let (secret, body) = (
            b"s3cret".as_slice(),
            br#"{"ref":"refs/heads/main"}"#.as_slice(),
        );
        let sig = sign(secret, body);
        let gh = headers(&[("x-hub-signature-256", &format!("sha256={sig}"))]);
        assert!(signature_ok(ReceiverProvider::Github, &gh, body, secret));
        assert!(!signature_ok(
            ReceiverProvider::Github,
            &gh,
            b"other",
            secret
        ));
        assert!(!signature_ok(ReceiverProvider::Github, &gh, body, b"wrong"));
        let fj = headers(&[("x-gitea-signature", &sig)]);
        assert!(signature_ok(ReceiverProvider::Forgejo, &fj, body, secret));
        let gen_ok = headers(&[("x-keel-signature", &format!("sha256={sig}"))]);
        assert!(signature_ok(
            ReceiverProvider::Generic,
            &gen_ok,
            body,
            secret
        ));
        let gen_bare = headers(&[("x-keel-signature", &sig)]);
        assert!(!signature_ok(
            ReceiverProvider::Generic,
            &gen_bare,
            body,
            secret
        ));
        let gl = headers(&[("x-gitlab-token", "s3cret")]);
        assert!(signature_ok(ReceiverProvider::Gitlab, &gl, body, secret));
        let gl_bad = headers(&[("x-gitlab-token", "s3cre")]);
        assert!(!signature_ok(
            ReceiverProvider::Gitlab,
            &gl_bad,
            body,
            secret
        ));
        assert!(!signature_ok(
            ReceiverProvider::Github,
            &HeaderMap::new(),
            body,
            secret
        ));
    }

    #[test]
    fn payload_reads_only_urls_and_ref() {
        let body = br#"{"ref":"refs/heads/main","repository":{"clone_url":"https://h/a.git",
            "ssh_url":"git@h:a.git"},"head_commit":{"id":"ignored"}}"#;
        let (urls, r) = read_payload(body).expect("payload");
        assert_eq!(r, "refs/heads/main");
        assert_eq!(urls, vec!["https://h/a.git", "git@h:a.git"]);
        let generic = br#"{"repository":"https://h/a.git","ref":"refs/tags/v1"}"#;
        assert_eq!(
            read_payload(generic).expect("generic").0,
            vec!["https://h/a.git"]
        );
        assert!(read_payload(b"not json").is_none());
        assert!(read_payload(br#"{"ref":"refs/heads/main"}"#).is_none());
    }

    #[test]
    fn limiter_counts_per_window() {
        let l = Limiter::new(Duration::from_secs(60), 2, 3);
        let t = Instant::now();
        let r = Some(Uuid::nil());
        let a: Option<IpAddr> = Some([10, 0, 0, 1].into());
        assert!(l.allow(r, a, t));
        assert!(l.allow(r, a, t));
        assert!(!l.allow(r, a, t), "third for one receiver");
        // Refused requests still count: the address has spent its three.
        assert!(!l.allow(Some(Uuid::max()), a, t), "fourth from one address");
        let other: Option<IpAddr> = Some([10, 0, 0, 2].into());
        assert!(
            l.allow(Some(Uuid::max()), other, t),
            "another address, another receiver"
        );
        assert!(l.allow(r, a, t + Duration::from_secs(61)), "a new window");
    }
}
