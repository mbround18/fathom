//! Join tokens and join requests (spec 055 T138 and T145, contracts/join.md).
//!
//! A join token is a JWT signed by Keel's join signing key ([`keel_exec::join_token`]). It
//! carries what a host needs to find and pin this Keel, and it says which workspace a host
//! knocks on. It grants nothing: a host that presents one becomes a **join request** that an
//! operator accepts or rejects.
//!
//! - The signing key is made at first start ([`bootstrap`]), sealed with the master key under
//!   `join_signing_key:<kid>`. Rotating it retires every token.
//! - A workspace has one active token or none (the default). Keel keeps its `jti` and SHA-256,
//!   never the token. Rotating or disabling it rejects the requests made with it
//!   (`token_rotated`).
//! - Requests are unique per (workspace, key fingerprint). Pending ones expire after 7 days,
//!   lazily. An operator's rejection is answered `JoinRejected` for 30 days.
//! - Bounds: `--join-pending-max` pending requests per workspace (more are `queue_full`, not
//!   stored), `--join-rate` attempts per source per minute (more are closed before any database
//!   write).
//! - Accepting creates the host exactly as an enrollment token does, without a certificate. The
//!   runner still authenticates as an unknown key; its next `JoinRequest` gets `Enrolled`.
//!
//! The token is never logged, audited or returned after the reply that shows it once.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use keel_core::WorkspaceId;
use keel_core::authz::Capability;
use keel_exec::ca;
use keel_exec::codec;
use keel_exec::frames::Frame;
use keel_exec::join_token::{
    self, AUDIENCE, JoinClaims, JoinSigningKey, JoinTokenError, JoinVerifyingKey,
};
use keel_exec::server_config::Access;
use keel_protocol::{
    Cmd, Event, JoinRequestInfo, JoinRequestState, JoinTokenInfo, JoinTokenSecret,
    MAX_JOIN_TOKEN_DAYS,
};
use russh::keys::{PublicKey, ssh_key::public::KeyData};
use serde_json::json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::bindings::{db_failed, enum_from, internal, not_found, op_error, require, ts};
use crate::dispatch::Request;
use crate::governance::{check_admin, deny};
use crate::hosts::{self, pending, preview, recheck};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::runner_listener::RunnerPath;
use crate::serve::AppState;

/// A pending request expires after this long.
pub const PENDING_TTL: Duration = Duration::from_secs(7 * 86_400);
/// An operator's rejection holds this long.
pub const REJECTED_HOLD: Duration = Duration::from_secs(30 * 86_400);
/// The rate limit's window.
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// The most sources the rate limiter tracks; past it, new sources are closed.
const RATE_SOURCES: usize = 10_000;
/// How long a join connection has to send its frame.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(30);
/// The largest join or enroll request read. A real one is a token, names and a few dozen
/// profiles, well under this; the codec's 16 MiB cap is for `Pack` (spec 067 T066 F2).
pub(crate) const FIRST_FRAME_MAX: usize = 64 * 1024;
/// The most requests `ListJoinRequests` returns.
const LIST_LIMIT: i64 = 500;
/// The most `allowed_profiles` a host may claim.
const MAX_ALLOWED_PROFILES: usize = 64;
/// The longest source shown for a request.
const MAX_SOURCE: usize = 300;

// ---- errors ----------------------------------------------------------------------------------

/// Why the join machinery failed. Carries no key material and never the token.
#[derive(Debug)]
pub enum JoinError {
    NoMasterKey,
    Db(sqlx::Error),
    Seal(crate::seal::SealError),
    Key(String),
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMasterKey => f.write_str("the join signing key needs the master key"),
            Self::Db(e) => write!(f, "database: {e}"),
            Self::Seal(e) => write!(f, "join signing key: {e}"),
            Self::Key(e) => write!(f, "join: {e}"),
        }
    }
}

impl std::error::Error for JoinError {}

impl From<sqlx::Error> for JoinError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

impl From<crate::seal::SealError> for JoinError {
    fn from(e: crate::seal::SealError) -> Self {
        Self::Seal(e)
    }
}

impl From<hosts::SshError> for JoinError {
    fn from(e: hosts::SshError) -> Self {
        match e {
            hosts::SshError::Db(e) => Self::Db(e),
            other => Self::Key(other.to_string()),
        }
    }
}

// ---- rate limit ------------------------------------------------------------------------------

/// Join attempts per source in the last minute, in memory. Cheap to clone.
#[derive(Clone, Default)]
pub struct JoinLimiter {
    inner: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl JoinLimiter {
    /// Counts an attempt from `source`; false once it has made `per_minute` in the last minute.
    #[must_use]
    pub fn allow(&self, source: &str, per_minute: u32) -> bool {
        let now = Instant::now();
        let Ok(mut map) = self.inner.lock() else {
            return false;
        };
        if map.len() >= RATE_SOURCES {
            map.retain(|_, q| {
                q.back()
                    .is_some_and(|t| now.duration_since(*t) < RATE_WINDOW)
            });
            if map.len() >= RATE_SOURCES && !map.contains_key(source) {
                return false;
            }
        }
        let q = map.entry(source.to_owned()).or_default();
        while q
            .front()
            .is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW)
        {
            q.pop_front();
        }
        if q.len() >= usize::try_from(per_minute).unwrap_or(usize::MAX) {
            return false;
        }
        q.push_back(now);
        true
    }
}

/// The rate limit's key: the peer address for a direct connection, the relay alone for a
/// relayed one. The peer a relay claims is never trusted, so no key is derived from it (spec 067
/// T062): every host behind one relay shares that relay's budget.
#[must_use]
pub fn source_key(path: &RunnerPath) -> String {
    match path {
        RunnerPath::Direct { peer: Some(p) } => p.ip().to_string(),
        RunnerPath::Direct { peer: None } => "direct".to_owned(),
        RunnerPath::Relay { relay_id, .. } => format!("relay:{relay_id}"),
    }
}

/// What a request shows as its source.
fn source_shown(path: &RunnerPath) -> String {
    let s = match path {
        RunnerPath::Direct { peer: Some(p) } => p.ip().to_string(),
        other => other.to_string(),
    };
    s.chars().take(MAX_SOURCE).collect()
}

// ---- the signing key -------------------------------------------------------------------------

fn key_aad(kid: &str) -> Vec<u8> {
    format!("join_signing_key:{kid}").into_bytes()
}

async fn lock(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("select pg_advisory_xact_lock(hashtext('keel_join'))")
        .execute(conn)
        .await
        .map(|_| ())
}

/// Makes, seals, stores and audits a new signing key. The caller holds the lock and has
/// retired any active one.
async fn create_key(
    conn: &mut PgConnection,
    master: &crate::seal::MasterKey,
    actor: &Actor,
    action: &str,
) -> Result<(JoinSigningKey, OffsetDateTime), JoinError> {
    let key = JoinSigningKey::generate();
    let kid = key.kid();
    let sealed = master.seal(&key_aad(&kid), &key.to_bytes())?;
    let (created,): (OffsetDateTime,) = sqlx::query_as(
        "insert into join_signing_key (kid, public_key, sealed_private_key) values ($1, $2, $3) \
         returning created_at",
    )
    .bind(&kid)
    .bind(key.public_bytes().to_vec())
    .bind(sealed)
    .fetch_one(&mut *conn)
    .await?;
    audit::record(
        conn,
        Entry {
            actor,
            action,
            workspace: None,
            target: json!({ "join_signing_key": kid }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({}),
        },
    )
    .await?;
    Ok((key, created))
}

/// The active signing key, made if there is none. The caller holds the lock.
async fn active_key(
    conn: &mut PgConnection,
    master: &crate::seal::MasterKey,
    actor: &Actor,
) -> Result<JoinSigningKey, JoinError> {
    let row: Option<(String, Vec<u8>)> = sqlx::query_as(
        "select kid, sealed_private_key from join_signing_key where retired_at is null",
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some((kid, sealed)) = row else {
        return Ok(create_key(conn, master, actor, "join_signing_key_created")
            .await?
            .0);
    };
    let opened = master.open(&key_aad(&kid), &sealed)?;
    let bytes: [u8; 32] = secrecy::ExposeSecret::expose_secret(&opened)
        .try_into()
        .map_err(|_| JoinError::Key("sealed join signing key has the wrong length".into()))?;
    let key = JoinSigningKey::from_bytes(&bytes);
    if key.kid() != kid {
        return Err(JoinError::Key(
            "sealed join signing key does not match its kid".into(),
        ));
    }
    Ok(key)
}

/// Makes the join signing key if there is none. Runs at start.
///
/// # Errors
///
/// No master key, the database, or a sealed key that does not open.
pub async fn bootstrap(app: &AppState) -> Result<(), JoinError> {
    let master = app.repos.master_key.clone().ok_or(JoinError::NoMasterKey)?;
    let mut tx = app.db.begin().await?;
    lock(&mut tx).await?;
    active_key(&mut tx, &master, &Actor::System).await?;
    tx.commit().await?;
    Ok(())
}

/// The keys tokens are checked with: the active one.
async fn verifying_keys(db: &sqlx::PgPool) -> Result<Vec<JoinVerifyingKey>, sqlx::Error> {
    let rows: Vec<(Vec<u8>,)> =
        sqlx::query_as("select public_key from join_signing_key where retired_at is null")
            .fetch_all(db)
            .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(b,)| {
            let b: [u8; 32] = b.as_slice().try_into().ok()?;
            JoinVerifyingKey::from_bytes(&b)
        })
        .collect())
}

/// The issuer every token names and is checked against: Keel's public origin.
fn issuer(app: &AppState) -> String {
    crate::settings::public_url(&app.cfg)
}

fn now_unix() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

// ---- requests --------------------------------------------------------------------------------

const REQUEST_COLUMNS: &str = "j.id, j.workspace_id, j.ssh_key_fingerprint, j.inventory_name, \
     j.allowed_profiles, t.profiles as token_profiles, j.require_signed, j.hostname, j.os, j.arch, \
     j.runner_version, j.source_addr, j.state, j.reason, j.comment, j.jti, j.first_seen_at, \
     j.last_seen_at, (select u.display_name from users u where u.id = j.decided_by) as decided_by, \
     j.decided_at, j.host_id";

#[derive(sqlx::FromRow)]
struct RequestRow {
    id: Uuid,
    workspace_id: Uuid,
    ssh_key_fingerprint: String,
    inventory_name: String,
    allowed_profiles: Vec<String>,
    token_profiles: Vec<String>,
    require_signed: bool,
    hostname: String,
    os: String,
    arch: String,
    runner_version: String,
    source_addr: String,
    state: String,
    reason: Option<String>,
    comment: Option<String>,
    jti: String,
    first_seen_at: OffsetDateTime,
    last_seen_at: OffsetDateTime,
    decided_by: Option<String>,
    decided_at: Option<OffsetDateTime>,
    host_id: Option<Uuid>,
}

impl RequestRow {
    fn info(self) -> JoinRequestInfo {
        let allowed: BTreeSet<&String> = self.allowed_profiles.iter().collect();
        let suggested = self
            .token_profiles
            .iter()
            .filter(|p| allowed.contains(p))
            .cloned()
            .collect();
        JoinRequestInfo {
            id: self.id,
            workspace: self.workspace_id,
            fingerprint: self.ssh_key_fingerprint,
            inventory_name: self.inventory_name,
            suggested_profiles: suggested,
            allowed_profiles: self.allowed_profiles,
            require_signed: self.require_signed,
            hostname: self.hostname,
            os: self.os,
            arch: self.arch,
            runner_version: self.runner_version,
            source: self.source_addr,
            // An unknown state reads as rejected: fail closed.
            state: enum_from(&self.state).unwrap_or(JoinRequestState::Rejected),
            reason: self.reason,
            comment: self.comment,
            jti: self.jti,
            first_seen_at: ts(self.first_seen_at),
            last_seen_at: ts(self.last_seen_at),
            decided_by: self.decided_by,
            decided_at: self.decided_at.map(ts),
            host: self.host_id,
        }
    }
}

async fn request_info(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<JoinRequestInfo>, sqlx::Error> {
    let row: Option<RequestRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {REQUEST_COLUMNS} from join_requests j join join_tokens t on t.jti = j.jti \
         where j.id = $1"
    )))
    .bind(id)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(RequestRow::info))
}

/// Pending requests older than [`PENDING_TTL`] become `expired`.
async fn expire_stale(conn: &mut PgConnection, workspace: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "update join_requests set state = 'expired', reason = 'expired' \
         where workspace_id = $1 and state = 'pending' and first_seen_at < $2",
    )
    .bind(workspace)
    .bind(OffsetDateTime::now_utc() - PENDING_TTL)
    .execute(conn)
    .await
    .map(|_| ())
}

/// Rejects the pending requests made with any of `jtis` (`token_rotated`).
async fn reject_for_tokens(
    conn: &mut PgConnection,
    jtis: &[String],
    by: Option<Uuid>,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "update join_requests set state = 'rejected', reason = 'token_rotated', \
         decided_by = $2, decided_at = now() where state = 'pending' and jti = any($1) \
         returning id",
    )
    .bind(jtis)
    .bind(by)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// Pushes `JoinRequestDecided` for each of `ids` to its workspace.
async fn push_decided(app: &AppState, ids: &[Uuid]) {
    let Ok(mut conn) = app.db.acquire().await else {
        return;
    };
    for id in ids {
        if let Ok(Some(info)) = request_info(&mut conn, *id).await {
            app.repos
                .push(info.workspace, Event::JoinRequestDecided { request: info });
        }
    }
}

// ---- the runner side (keel-join-v1) ----------------------------------------------------------

/// What a host sent in its `JoinRequest`.
pub struct JoinAttempt<'a> {
    pub token: &'a str,
    pub inventory_name: &'a str,
    pub allowed_profiles: &'a [String],
    pub require_signed: bool,
    pub hostname: &'a str,
    pub os: &'a str,
    pub arch: &'a str,
    pub runner_version: &'a str,
}

/// How a join attempt was answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinOutcome {
    /// `JoinPending`: the request waits for an operator.
    Pending(Uuid),
    /// `Enrolled`: the request was accepted; the host's first certificate.
    Enrolled { host: Uuid, cert: String },
    /// `JoinRejected { reason }`.
    Rejected(String),
    /// Closed without a reply (rate limited, a malformed frame, a token that does not verify).
    Closed(&'static str),
}

/// A string the host claims: not empty (unless `may_be_empty`), at most `max` bytes, no control
/// characters.
fn claim_ok(v: &str, max: usize, may_be_empty: bool) -> bool {
    (may_be_empty || !v.is_empty()) && v.len() <= max && !v.chars().any(char::is_control)
}

fn profile_ok(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 63
        && p.bytes().enumerate().all(|(i, b)| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || (i > 0 && (b == b'_' || b == b'-'))
        })
}

/// The frame is within its bounds (contracts/join.md, "Server rules").
fn well_formed(a: &JoinAttempt<'_>) -> bool {
    !a.token.is_empty()
        && a.token.len() <= join_token::MAX_TOKEN_LEN
        && claim_ok(a.inventory_name, 253, false)
        && claim_ok(a.hostname, 253, true)
        && claim_ok(a.os, 128, true)
        && matches!(a.arch, "x86_64" | "aarch64")
        && claim_ok(a.runner_version, 64, false)
        && a.allowed_profiles.len() <= MAX_ALLOWED_PROFILES
        && a.allowed_profiles.iter().all(|p| profile_ok(p))
}

const fn token_refusal(e: &JoinTokenError) -> &'static str {
    match e {
        JoinTokenError::Malformed | JoinTokenError::Header => "malformed_token",
        JoinTokenError::UnknownKey => "unknown_key",
        JoinTokenError::Signature => "bad_signature",
        JoinTokenError::Issuer => "wrong_issuer",
        JoinTokenError::Audience => "wrong_audience",
        JoinTokenError::Expired => "expired_token",
        JoinTokenError::Lifetime => "bad_lifetime",
    }
}

/// Audits a refused attempt by fingerprint, source and `jti` (when readable), never the token.
async fn audit_refusal(
    conn: &mut PgConnection,
    workspace: Option<Uuid>,
    fingerprint: &str,
    jti: Option<&str>,
    source: &str,
    reason: &str,
) -> Result<(), sqlx::Error> {
    audit::record(
        conn,
        Entry {
            actor: &Actor::System,
            action: "join_refused",
            workspace: workspace.map(WorkspaceId),
            target: json!({ "fingerprint": fingerprint }),
            decision: Decision::Denied,
            reason: Some(reason),
            context: json!({ "source": source, "jti": jti }),
        },
    )
    .await
    .map(|_| ())
}

/// A `jti` read from a token without checking it, if it looks like one: for the audit only.
fn readable_jti(token: &str) -> Option<String> {
    join_token::decode_unverified(token)
        .ok()
        .map(|(_, c)| c.jti)
        .filter(|j| j.len() == 32 && j.bytes().all(|b| b.is_ascii_hexdigit()))
}

type Existing = (
    Uuid,
    String,
    Option<String>,
    Option<OffsetDateTime>,
    String,
    Option<Uuid>,
);

/// Answers one `JoinRequest` from the host with `key`, which came by `path`.
///
/// # Errors
///
/// The database, the CA or sealing failed.
pub async fn join_key(
    app: &AppState,
    key: &KeyData,
    req: &JoinAttempt<'_>,
    path: &RunnerPath,
) -> Result<JoinOutcome, JoinError> {
    // Before anything touches the database.
    if !app.join.allow(&source_key(path), app.cfg.join_rate) {
        return Ok(JoinOutcome::Closed("rate_limited"));
    }
    join_admitted(app, key, req, path).await
}

/// [`join_key`] once the rate limit admitted the attempt.
async fn join_admitted(
    app: &AppState,
    key: &KeyData,
    req: &JoinAttempt<'_>,
    path: &RunnerPath,
) -> Result<JoinOutcome, JoinError> {
    let fp = ca::fingerprint(key);
    let source = source_shown(path);
    let refuse = |workspace: Option<Uuid>, jti: Option<String>, why: &'static str| {
        let (fp, source) = (fp.clone(), source.clone());
        async move {
            let mut tx = app.db.begin().await?;
            audit_refusal(&mut tx, workspace, &fp, jti.as_deref(), &source, why).await?;
            tx.commit().await?;
            Ok::<_, JoinError>(JoinOutcome::Closed(why))
        }
    };
    if !well_formed(req) {
        return refuse(None, None, "malformed_frame").await;
    }
    let keys = verifying_keys(&app.db).await?;
    let claims = match join_token::verify(req.token, &keys, &issuer(app), now_unix()) {
        Ok(c) => c,
        Err(e) => return refuse(None, readable_jti(req.token), token_refusal(&e)).await,
    };
    let Ok(ws) = Uuid::parse_str(&claims.sub) else {
        return refuse(None, Some(claims.jti), "malformed_token").await;
    };
    // A relay carries joins into its own workspace only (T066 F4); the caller records the
    // refusal against the relay, since the relay, not the token, is the suspect.
    if let Some(relay) = path.relay()
        && crate::relays::relay_workspace(&app.db, relay).await != Some(ws)
    {
        return Ok(JoinOutcome::Closed("workspace_mismatch"));
    }
    let mut tx = app.db.begin().await?;
    expire_stale(&mut tx, ws).await?;
    let existing: Option<Existing> = sqlx::query_as(
        "select id, state, reason, decided_at, jti, host_id from join_requests \
         where workspace_id = $1 and ssh_key_fingerprint = $2 for update",
    )
    .bind(ws)
    .bind(&fp)
    .fetch_optional(&mut *tx)
    .await?;
    let reject = |tx: sqlx::Transaction<'static, sqlx::Postgres>, why: String| {
        let (fp, source, jti) = (fp.clone(), source.clone(), claims.jti.clone());
        async move {
            let mut tx = tx;
            audit_refusal(&mut tx, Some(ws), &fp, Some(&jti), &source, &why).await?;
            tx.commit().await?;
            Ok::<_, JoinError>(JoinOutcome::Rejected(why))
        }
    };
    if let Some((id, state, reason, decided_at, jti, host)) = &existing {
        match state.as_str() {
            // Accepted: the host exists; its certificate is issued now, whatever the token's
            // `jti` (the operator's decision is what lets it in).
            "accepted" => return enrol_accepted(app, tx, key, ws, *id, *host, &source).await,
            // An operator's rejection holds for 30 days.
            "rejected"
                if reason.as_deref() == Some("rejected")
                    && decided_at.is_none_or(|d| OffsetDateTime::now_utc() - d < REJECTED_HOLD) =>
            {
                return reject(tx, "rejected".to_owned()).await;
            }
            // Expired or rotated away: asking again with the same token gets the same answer;
            // a new token starts over.
            "rejected" | "expired" if *jti == claims.jti => {
                return reject(tx, reason.clone().unwrap_or_else(|| state.clone())).await;
            }
            _ => {}
        }
    }
    let (known,): (bool,) =
        sqlx::query_as("select exists (select 1 from hosts where ssh_key_fingerprint = $1)")
            .bind(&fp)
            .fetch_one(&mut *tx)
            .await?;
    if known {
        return reject(tx, "known_key".to_owned()).await;
    }
    let (active,): (bool,) = sqlx::query_as(
        "select exists (select 1 from join_tokens where jti = $1 and workspace_id = $2 \
         and retired_at is null and token_hash = $3 and expires_at > now())",
    )
    .bind(&claims.jti)
    .bind(ws)
    .bind(join_token::token_hash(req.token).to_vec())
    .fetch_one(&mut *tx)
    .await?;
    if !active {
        return reject(tx, "token_retired".to_owned()).await;
    }
    let line = PublicKey::from(key.clone())
        .to_openssh()
        .map_err(|e| JoinError::Key(e.to_string()))?;
    let mut allowed = req.allowed_profiles.to_vec();
    allowed.sort();
    allowed.dedup();
    let id = match &existing {
        Some((id, state, ..)) if state == "pending" => {
            // A retry: new facts, same request, same decision.
            sqlx::query(
                "update join_requests set inventory_name = $2, allowed_profiles = $3, \
                 require_signed = $4, hostname = $5, os = $6, arch = $7, runner_version = $8, \
                 source_addr = $9, last_seen_at = now() where id = $1",
            )
            .bind(id)
            .bind(req.inventory_name)
            .bind(&allowed)
            .bind(req.require_signed)
            .bind(req.hostname)
            .bind(req.os)
            .bind(req.arch)
            .bind(req.runner_version)
            .bind(&source)
            .execute(&mut *tx)
            .await?;
            *id
        }
        _ => {
            let (n,): (i64,) = sqlx::query_as(
                "select count(*) from join_requests where workspace_id = $1 and state = 'pending'",
            )
            .bind(ws)
            .fetch_one(&mut *tx)
            .await?;
            if n >= i64::from(app.cfg.join_pending_max) {
                return reject(tx, "queue_full".to_owned()).await;
            }
            let id = existing.as_ref().map_or_else(Uuid::now_v7, |e| e.0);
            sqlx::query(
                "insert into join_requests (id, workspace_id, jti, ssh_key_fingerprint, \
                 ssh_public_key, inventory_name, allowed_profiles, require_signed, hostname, os, \
                 arch, runner_version, source_addr) \
                 values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
                 on conflict (workspace_id, ssh_key_fingerprint) do update set \
                 jti = excluded.jti, ssh_public_key = excluded.ssh_public_key, \
                 inventory_name = excluded.inventory_name, \
                 allowed_profiles = excluded.allowed_profiles, \
                 require_signed = excluded.require_signed, hostname = excluded.hostname, \
                 os = excluded.os, arch = excluded.arch, \
                 runner_version = excluded.runner_version, source_addr = excluded.source_addr, \
                 state = 'pending', reason = null, comment = null, first_seen_at = now(), \
                 last_seen_at = now(), decided_by = null, decided_at = null, host_id = null",
            )
            .bind(id)
            .bind(ws)
            .bind(&claims.jti)
            .bind(&fp)
            .bind(&line)
            .bind(req.inventory_name)
            .bind(&allowed)
            .bind(req.require_signed)
            .bind(req.hostname)
            .bind(req.os)
            .bind(req.arch)
            .bind(req.runner_version)
            .bind(&source)
            .execute(&mut *tx)
            .await?;
            audit::record(
                &mut tx,
                Entry {
                    actor: &Actor::System,
                    action: "join_requested",
                    workspace: Some(WorkspaceId(ws)),
                    target: json!({ "join_request": id, "fingerprint": fp }),
                    decision: Decision::Allowed,
                    reason: None,
                    context: json!({ "jti": claims.jti, "source": source }),
                },
            )
            .await?;
            id
        }
    };
    let info = request_info(&mut tx, id).await?;
    tx.commit().await?;
    if let Some(request) = info {
        app.repos.push(ws, Event::JoinRequested { request });
    }
    Ok(JoinOutcome::Pending(id))
}

/// An accepted request's next attempt: the host's first (or a fresh) certificate.
async fn enrol_accepted(
    app: &AppState,
    mut tx: sqlx::Transaction<'static, sqlx::Postgres>,
    key: &KeyData,
    ws: Uuid,
    request: Uuid,
    host: Option<Uuid>,
    source: &str,
) -> Result<JoinOutcome, JoinError> {
    let fp = ca::fingerprint(key);
    let row: Option<(String, String, Option<String>)> = match host {
        Some(h) => {
            sqlx::query_as(
                "select inventory_name, state, ssh_key_fingerprint from hosts where id = $1",
            )
            .bind(h)
            .fetch_optional(&mut *tx)
            .await?
        }
        None => None,
    };
    let (Some(host), Some((name, state, Some(host_fp)))) = (host, row) else {
        audit_refusal(&mut tx, Some(ws), &fp, None, source, "known_key").await?;
        tx.commit().await?;
        return Ok(JoinOutcome::Rejected("known_key".to_owned()));
    };
    if state == "revoked" || host_fp != fp {
        audit_refusal(&mut tx, Some(ws), &fp, None, source, "known_key").await?;
        tx.commit().await?;
        return Ok(JoinOutcome::Rejected("known_key".to_owned()));
    }
    let cert = hosts::issue_cert(app, &mut tx, host, &name, key).await?;
    audit::record(
        &mut tx,
        Entry {
            actor: &Actor::Host(host),
            action: "host_enrolled",
            workspace: Some(WorkspaceId(ws)),
            target: json!({ "host": host, "inventory_name": name }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({ "join_request": request, "serial": cert.serial().to_string() }),
        },
    )
    .await?;
    tx.commit().await?;
    let cert = cert
        .to_openssh()
        .map_err(|e| JoinError::Key(e.to_string()))?;
    Ok(JoinOutcome::Enrolled { host, cert })
}

/// An unknown key's `keel-join-v1` stream: one `JoinRequest`, answered `JoinPending`,
/// `Enrolled` or `JoinRejected`, or closed.
pub async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    app: &AppState,
    key: &KeyData,
    path: &RunnerPath,
    mut stream: S,
) {
    // The rate limit comes before the frame is read, as enroll's does (T066 F2): a source
    // over its budget cannot make Keel read (or buffer) anything.
    if !app.join.allow(&source_key(path), app.cfg.join_rate) {
        rate_limited(app, path);
        return;
    }
    let read = codec::read_frame_capped(&mut stream, FIRST_FRAME_MAX);
    let frame = match tokio::time::timeout(FIRST_FRAME_TIMEOUT, read).await {
        Ok(Ok(Some(f))) => f,
        _ => return,
    };
    if !Access::JoinOnly.permits(&frame) {
        return;
    }
    let Frame::JoinRequest {
        token,
        inventory_name,
        allowed_profiles,
        require_signed,
        hostname,
        os,
        arch,
        runner_version,
    } = frame
    else {
        return;
    };
    let attempt = JoinAttempt {
        token: token.expose(),
        inventory_name: &inventory_name,
        allowed_profiles: &allowed_profiles,
        require_signed,
        hostname: &hostname,
        os: &os,
        arch: &arch,
        runner_version: &runner_version,
    };
    let reply = match join_admitted(app, key, &attempt, path).await {
        Ok(JoinOutcome::Pending(id)) => Frame::JoinPending {
            request_id: id.to_string(),
        },
        Ok(JoinOutcome::Enrolled { host, cert }) => Frame::Enrolled {
            host_id: host.to_string(),
            cert,
        },
        Ok(JoinOutcome::Rejected(reason)) => {
            tracing::info!(%reason, %path, "join request rejected");
            Frame::JoinRejected { reason }
        }
        Ok(JoinOutcome::Closed("workspace_mismatch")) => {
            app.relay_live.refused(path, "workspace_mismatch", "join");
            return;
        }
        Ok(JoinOutcome::Closed(reason)) => {
            tracing::info!(reason, %path, "join attempt refused");
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "join attempt failed");
            return;
        }
    };
    let _ = codec::write_frame(&mut stream, &reply).await;
}

/// A join over its source's rate limit.
fn rate_limited(app: &AppState, path: &RunnerPath) {
    if path.relay().is_some() {
        // A relay's joins share one budget keyed on the relay (`source_key`, spec 067 T062);
        // hitting it is a coalesced `relay.refused`, logged once per window (T061).
        app.relay_live.refused(path, "join_rate", "join");
    } else {
        tracing::info!(reason = "rate_limited", %path, "join attempt refused");
    }
}

/// Whether `host`, which has no certificate yet, was let in by an accepted join request: its
/// bare key then reaches the join subsystem again to collect its certificate.
pub async fn awaiting_cert(db: &sqlx::PgPool, host: Uuid) -> Result<bool, sqlx::Error> {
    let (ok,): (bool,) = sqlx::query_as(
        "select exists (select 1 from join_requests j join hosts h on h.id = j.host_id \
         where j.host_id = $1 and j.state = 'accepted' and h.state <> 'revoked' \
         and not exists (select 1 from ssh_certs c where c.host_id = h.id))",
    )
    .bind(host)
    .fetch_one(db)
    .await?;
    Ok(ok)
}

// ---- commands --------------------------------------------------------------------------------

pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::ListJoinRequests { workspace, state } => list(app, req, workspace, state).await,
        Cmd::PreviewJoinAccept {
            request,
            inventory_name,
            profiles,
            auto_converge,
        } => {
            let profiles: Vec<String> = profiles.0.iter().map(|p| p.as_str().to_owned()).collect();
            preview_accept(
                app,
                req,
                request,
                inventory_name.as_str().to_owned(),
                profiles,
                auto_converge.unwrap_or(true),
            )
            .await
        }
        Cmd::PreviewJoinReject { request, comment } => {
            preview_reject(app, req, request, comment.map(|c| c.as_str().to_owned())).await
        }
        Cmd::GetJoinToken { workspace } => {
            if let Err(d) =
                require(app, req, "get_join_token", Capability::Operate, workspace).await
            {
                return d;
            }
            match token_info(&app.db, workspace).await {
                Ok(info) => vec![Event::JoinToken(info)],
                Err(e) => db_failed(&e),
            }
        }
        Cmd::PreviewJoinTokenIssue {
            workspace,
            expires_in_days,
            inventory_name,
            profiles,
        } => {
            let profiles = profiles
                .map(|p| p.0.iter().map(|p| p.as_str().to_owned()).collect())
                .unwrap_or_default();
            let name = inventory_name.map(|n| n.as_str().to_owned());
            preview_issue(app, req, workspace, expires_in_days, name, profiles).await
        }
        Cmd::PreviewJoinTokenRotate {
            workspace,
            expires_in_days,
        } => preview_rotate(app, req, workspace, expires_in_days).await,
        Cmd::PreviewJoinTokenDisable { workspace } => preview_disable(app, req, workspace).await,
        Cmd::PreviewJoinSigningKeyRotate => preview_key_rotate(app, req).await,
        _ => internal(),
    }
}

async fn list(
    app: &AppState,
    req: &Request<'_>,
    workspace: Uuid,
    state: Option<JoinRequestState>,
) -> Vec<Event> {
    if let Err(d) = require(
        app,
        req,
        "list_join_requests",
        Capability::Operate,
        workspace,
    )
    .await
    {
        return d;
    }
    let result: Result<Vec<JoinRequestInfo>, sqlx::Error> = async {
        let mut conn = app.db.acquire().await?;
        expire_stale(&mut conn, workspace).await?;
        let rows: Vec<RequestRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select {REQUEST_COLUMNS} from join_requests j join join_tokens t on t.jti = j.jti \
             where j.workspace_id = $1 and ($2::text is null or j.state = $2) \
             order by j.last_seen_at desc, j.id limit $3"
        )))
        .bind(workspace)
        .bind(state.map(|s| crate::bindings::enum_str(&s)))
        .bind(LIST_LIMIT)
        .fetch_all(&mut *conn)
        .await?;
        Ok(rows.into_iter().map(RequestRow::info).collect())
    }
    .await;
    match result {
        Ok(items) => vec![Event::JoinRequests { items }],
        Err(e) => db_failed(&e),
    }
}

/// A request's workspace, state, allowed profiles and fingerprint.
type RequestFacts = (Uuid, String, Vec<String>, String);

async fn request_facts(app: &AppState, id: Uuid) -> Result<Option<RequestFacts>, sqlx::Error> {
    let ws: Option<(Uuid,)> =
        sqlx::query_as("select workspace_id from join_requests where id = $1")
            .bind(id)
            .fetch_optional(&app.db)
            .await?;
    let Some((ws,)) = ws else { return Ok(None) };
    let mut conn = app.db.acquire().await?;
    expire_stale(&mut conn, ws).await?;
    sqlx::query_as(
        "select workspace_id, state, allowed_profiles, ssh_key_fingerprint from join_requests \
         where id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
}

fn not_pending() -> Vec<Event> {
    op_error(
        "join_request_not_pending",
        "The join request is no longer pending.",
    )
}

/// The request is pending, `profiles` are within its `allowed_profiles`, the name is free, the
/// profiles exist and its key belongs to no host.
async fn check_accept(
    app: &AppState,
    request: Uuid,
    name: &str,
    profiles: &[String],
) -> Result<(Uuid, String), Vec<Event>> {
    let Some((ws, state, allowed, fp)) = request_facts(app, request)
        .await
        .map_err(|e| db_failed(&e))?
    else {
        return Err(not_found("join request"));
    };
    if state != "pending" {
        return Err(not_pending());
    }
    let allowed: BTreeSet<&String> = allowed.iter().collect();
    if let Some(p) = profiles.iter().find(|p| !allowed.contains(p)) {
        return Err(op_error(
            "profile_not_allowed",
            &format!("The host does not allow the profile {p}."),
        ));
    }
    hosts::check_enroll(app, ws, name, profiles).await?;
    let (known,): (bool,) =
        sqlx::query_as("select exists (select 1 from hosts where ssh_key_fingerprint = $1)")
            .bind(&fp)
            .fetch_one(&app.db)
            .await
            .map_err(|e| db_failed(&e))?;
    if known {
        return Err(op_error(
            "known_key",
            "This host key already belongs to a host.",
        ));
    }
    Ok((ws, fp))
}

async fn preview_accept(
    app: &AppState,
    req: &Request<'_>,
    request: Uuid,
    name: String,
    profiles: Vec<String>,
    auto_converge: bool,
) -> Vec<Event> {
    const ACTION: &str = "preview_join_accept";
    let ws = match request_facts(app, request).await {
        Ok(Some((ws, ..))) => ws,
        Ok(None) => return not_found("join request"),
        Err(e) => return db_failed(&e),
    };
    if let Err(d) = require(app, req, ACTION, Capability::Operate, ws).await {
        return d;
    }
    if app.ssh.get().is_none() {
        return op_error(
            "ssh_ca_unavailable",
            "The SSH certificate authorities are not loaded.",
        );
    }
    let fp = match check_accept(app, request, &name, &profiles).await {
        Ok((_, fp)) => fp,
        Err(e) => return e,
    };
    let mut summary = format!(
        "Accept the join request from the host with key {fp} as {name}, with profiles {}. Its \
         runner gets its certificate the next time it asks.",
        if profiles.is_empty() {
            "(none)".to_owned()
        } else {
            profiles.join(", ")
        }
    );
    if auto_converge && app.cfg.converge {
        summary.push_str(" Keel then converges it to its profiles.");
    }
    let affected = vec![name.clone()];
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Operate, ws) {
            return d;
        }
        if let Err(e) = check_accept(&app, request, &name, &profiles).await {
            return e;
        }
        decide(
            &app,
            &caller,
            ws,
            request,
            Decided::Accept {
                name,
                profiles,
                auto_converge,
            },
        )
        .await
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: true,
        },
        op,
    )
}

async fn preview_reject(
    app: &AppState,
    req: &Request<'_>,
    request: Uuid,
    comment: Option<String>,
) -> Vec<Event> {
    const ACTION: &str = "preview_join_reject";
    let (ws, state, fp) = match request_facts(app, request).await {
        Ok(Some((ws, state, _, fp))) => (ws, state, fp),
        Ok(None) => return not_found("join request"),
        Err(e) => return db_failed(&e),
    };
    if let Err(d) = require(app, req, ACTION, Capability::Operate, ws).await {
        return d;
    }
    if state != "pending" {
        return not_pending();
    }
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Operate, ws) {
            return d;
        }
        decide(&app, &caller, ws, request, Decided::Reject { comment }).await
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary: format!(
                "Reject the join request from the host with key {fp}. The host is told so and \
                 stops asking; the same key is refused for 30 days."
            ),
            affected: vec![fp],
            step_up: false,
        },
        op,
    )
}

enum Decided {
    Accept {
        name: String,
        profiles: Vec<String>,
        auto_converge: bool,
    },
    Reject {
        comment: Option<String>,
    },
}

async fn decide(
    app: &AppState,
    caller: &Caller,
    ws: Uuid,
    request: Uuid,
    decided: Decided,
) -> Vec<Event> {
    // The host an accepted request created, to converge once its row is committed.
    let mut converge = None;
    let result: Result<Option<JoinRequestInfo>, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        type Row = (String, String, String, Vec<String>, bool);
        let row: Option<Row> = sqlx::query_as(
            "select state, ssh_key_fingerprint, ssh_public_key, allowed_profiles, require_signed \
             from join_requests where id = $1 for update",
        )
        .bind(request)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((state, fp, line, allowed, require_signed)) = row else {
            return Ok(None);
        };
        if state != "pending" {
            return Ok(None);
        }
        match decided {
            Decided::Accept {
                name,
                profiles,
                auto_converge,
            } => {
                let host = Uuid::now_v7();
                sqlx::query(
                    "insert into hosts (id, workspace_id, inventory_name, ssh_key_fingerprint, \
                     ssh_public_key, allowed_profiles, require_signed, state) \
                     values ($1, $2, $3, $4, $5, $6, $7, 'offline')",
                )
                .bind(host)
                .bind(ws)
                .bind(&name)
                .bind(&fp)
                .bind(&line)
                .bind(&allowed)
                .bind(require_signed)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "insert into host_profiles (host_id, profile_id, assigned_by) \
                     select $1, id, $3 from profiles where workspace_id = $2 and name = any($4)",
                )
                .bind(host)
                .bind(ws)
                .bind(caller.user.id)
                .bind(&profiles)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "update join_requests set state = 'accepted', decided_by = $2, \
                     decided_at = now(), host_id = $3 where id = $1",
                )
                .bind(request)
                .bind(caller.user.id)
                .bind(host)
                .execute(&mut *tx)
                .await?;
                audit::record(
                    &mut tx,
                    Entry {
                        actor: &caller.actor(),
                        action: "join_accepted",
                        workspace: Some(WorkspaceId(ws)),
                        target: json!({ "join_request": request, "host": host }),
                        decision: Decision::Allowed,
                        reason: None,
                        context: json!({
                            "inventory_name": name,
                            "profiles": profiles,
                            "fingerprint": fp,
                            "auto_converge": auto_converge,
                        }),
                    },
                )
                .await?;
                if auto_converge {
                    converge = Some(host);
                }
            }
            Decided::Reject { comment } => {
                sqlx::query(
                    "update join_requests set state = 'rejected', reason = 'rejected', \
                     comment = $2, decided_by = $3, decided_at = now() where id = $1",
                )
                .bind(request)
                .bind(&comment)
                .bind(caller.user.id)
                .execute(&mut *tx)
                .await?;
                audit::record(
                    &mut tx,
                    Entry {
                        actor: &caller.actor(),
                        action: "join_rejected",
                        workspace: Some(WorkspaceId(ws)),
                        target: json!({ "join_request": request }),
                        decision: Decision::Allowed,
                        reason: None,
                        context: json!({ "fingerprint": fp }),
                    },
                )
                .await?;
            }
        }
        let info = request_info(&mut tx, request).await?;
        tx.commit().await?;
        Ok(info)
    }
    .await;
    match result {
        Ok(Some(request)) => {
            // Spec 068 US2 scenario 3 (T034): converge the accepted host to the profiles chosen
            // here, after the acceptance committed. A failure is logged and leaves it accepted.
            if let Some(host) = converge
                && let Err(err) = crate::converge::on_enrolled(app, host).await
            {
                tracing::error!(%host, error = %err, "converge on join acceptance failed");
            }
            app.repos.push(
                ws,
                Event::JoinRequestDecided {
                    request: request.clone(),
                },
            );
            vec![Event::JoinRequestDecided { request }]
        }
        Ok(None) => not_pending(),
        Err(e) => db_failed(&e),
    }
}

// ---- tokens ----------------------------------------------------------------------------------

async fn token_info(db: &sqlx::PgPool, ws: Uuid) -> Result<JoinTokenInfo, sqlx::Error> {
    type Row = (
        String,
        OffsetDateTime,
        OffsetDateTime,
        Option<String>,
        Option<String>,
        Vec<String>,
    );
    let row: Option<Row> = sqlx::query_as(
        "select t.jti, t.expires_at, t.created_at, \
         (select u.display_name from users u where u.id = t.created_by), t.inventory_name, \
         t.profiles from join_tokens t where t.workspace_id = $1 and t.retired_at is null",
    )
    .bind(ws)
    .fetch_optional(db)
    .await?;
    Ok(match row {
        Some((jti, expires, created, by, name, profiles)) => JoinTokenInfo {
            workspace: ws,
            active: true,
            jti: Some(jti),
            expires_at: Some(ts(expires)),
            created_at: Some(ts(created)),
            created_by: by,
            inventory_name: name,
            profiles,
        },
        None => JoinTokenInfo {
            workspace: ws,
            active: false,
            jti: None,
            expires_at: None,
            created_at: None,
            created_by: None,
            inventory_name: None,
            profiles: Vec::new(),
        },
    })
}

/// The pending requests a rotate or disable would reject: their fingerprints.
async fn pending_for_active(db: &sqlx::PgPool, ws: Uuid) -> Result<Vec<String>, sqlx::Error> {
    let mut conn = db.acquire().await?;
    expire_stale(&mut conn, ws).await?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "select j.inventory_name, j.ssh_key_fingerprint from join_requests j \
         join join_tokens t on t.jti = j.jti \
         where j.workspace_id = $1 and j.state = 'pending' and t.retired_at is null \
         order by j.first_seen_at",
    )
    .bind(ws)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, fp)| format!("{name} ({fp})"))
        .collect())
}

/// What issuing needs: the master key, the CAs and a runner address.
fn issue_ready(app: &AppState) -> Result<(), Vec<Event>> {
    if app.repos.master_key.is_none() || app.ssh.get().is_none() {
        return Err(op_error(
            "ssh_ca_unavailable",
            "The SSH certificate authorities are not loaded.",
        ));
    }
    if crate::settings::runner_address(&app.cfg).is_none() {
        return Err(op_error(
            "runner_address_unknown",
            "Keel does not know the address runners connect to: set KEEL_RUNNER_HOST_NAMES.",
        ));
    }
    Ok(())
}

fn days_ok(days: u16) -> Result<(), Vec<Event>> {
    if days == 0 || days > MAX_JOIN_TOKEN_DAYS {
        return Err(op_error(
            "bad_expiry",
            &format!("A join token lasts 1 to {MAX_JOIN_TOKEN_DAYS} days."),
        ));
    }
    Ok(())
}

async fn has_active(db: &sqlx::PgPool, ws: Uuid) -> Result<bool, sqlx::Error> {
    let (b,): (bool,) = sqlx::query_as(
        "select exists (select 1 from join_tokens where workspace_id = $1 and retired_at is null)",
    )
    .bind(ws)
    .fetch_one(db)
    .await?;
    Ok(b)
}

async fn preview_issue(
    app: &AppState,
    req: &Request<'_>,
    ws: Uuid,
    days: u16,
    name: Option<String>,
    profiles: Vec<String>,
) -> Vec<Event> {
    const ACTION: &str = "preview_join_token_issue";
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    if let Err(e) = issue_ready(app).and(days_ok(days)) {
        return e;
    }
    match has_active(&app.db, ws).await {
        Ok(false) => {}
        Ok(true) => {
            return op_error(
                "join_token_active",
                "The workspace already has a join token: rotate it instead.",
            );
        }
        Err(e) => return db_failed(&e),
    }
    let found: Result<Vec<(String,)>, _> =
        sqlx::query_as("select name from profiles where workspace_id = $1 and name = any($2)")
            .bind(ws)
            .bind(&profiles)
            .fetch_all(&app.db)
            .await;
    match found {
        Ok(found) => {
            let found: BTreeSet<String> = found.into_iter().map(|r| r.0).collect();
            if let Some(p) = profiles.iter().find(|p| !found.contains(*p)) {
                return op_error(
                    "unknown_profile",
                    &format!("The workspace has no profile {p}."),
                );
            }
        }
        Err(e) => return db_failed(&e),
    }
    let summary = format!(
        "Issue a join token for this workspace, valid {days} days. Hosts that present it can ask \
         to join; each request still needs an operator's approval. The token is shown once."
    );
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        mint(&app, &caller, ws, days, Mint::Issue { name, profiles }).await
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected: vec!["join token".to_owned()],
            step_up: true,
        },
        op,
    )
}

async fn preview_rotate(app: &AppState, req: &Request<'_>, ws: Uuid, days: u16) -> Vec<Event> {
    const ACTION: &str = "preview_join_token_rotate";
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    if let Err(e) = issue_ready(app).and(days_ok(days)) {
        return e;
    }
    match has_active(&app.db, ws).await {
        Ok(true) => {}
        Ok(false) => return op_error("no_join_token", "The workspace has no join token."),
        Err(e) => return db_failed(&e),
    }
    let affected = match pending_for_active(&app.db, ws).await {
        Ok(a) => a,
        Err(e) => return db_failed(&e),
    };
    let summary = format!(
        "Rotate the join token: the current one stops working now and a new one, valid {days} \
         days, is shown once. {} pending request(s) made with the current token are rejected.",
        affected.len()
    );
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        mint(&app, &caller, ws, days, Mint::Rotate).await
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: true,
        },
        op,
    )
}

enum Mint {
    Issue {
        name: Option<String>,
        profiles: Vec<String>,
    },
    Rotate,
}

async fn mint(app: &AppState, caller: &Caller, ws: Uuid, days: u16, how: Mint) -> Vec<Event> {
    if let Err(e) = issue_ready(app) {
        return e;
    }
    let (Some(master), Some(cas), Some(server)) = (
        app.repos.master_key.clone(),
        app.ssh.get(),
        crate::settings::runner_address(&app.cfg),
    ) else {
        return internal();
    };
    let rotating = matches!(how, Mint::Rotate);
    type Minted = Result<(String, Vec<Uuid>), Vec<Event>>;
    let result: Result<Minted, JoinError> = async {
        let mut tx = app.db.begin().await?;
        lock(&mut tx).await?;
        let old: Option<(String, Option<String>, Vec<String>)> = sqlx::query_as(
            "select jti, inventory_name, profiles from join_tokens \
             where workspace_id = $1 and retired_at is null for update",
        )
        .bind(ws)
        .fetch_optional(&mut *tx)
        .await?;
        let (name, profiles, replaced, rejected) = match (how, old) {
            (Mint::Issue { .. }, Some(_)) => {
                return Ok(Err(op_error(
                    "join_token_active",
                    "The workspace already has a join token: rotate it instead.",
                )));
            }
            (Mint::Rotate, None) => {
                return Ok(Err(op_error(
                    "no_join_token",
                    "The workspace has no join token.",
                )));
            }
            (Mint::Issue { name, profiles }, None) => (name, profiles, None, Vec::new()),
            (Mint::Rotate, Some((jti, name, profiles))) => {
                sqlx::query("update join_tokens set retired_at = now() where jti = $1")
                    .bind(&jti)
                    .execute(&mut *tx)
                    .await?;
                let rejected =
                    reject_for_tokens(&mut tx, std::slice::from_ref(&jti), Some(caller.user.id))
                        .await?;
                (name, profiles, Some(jti), rejected)
            }
        };
        let key = active_key(&mut tx, &master, &caller.actor()).await?;
        let now = OffsetDateTime::now_utc();
        let expires = now + time::Duration::days(i64::from(days));
        let origin = issuer(app);
        let claims = JoinClaims {
            iss: origin.clone(),
            aud: AUDIENCE.to_owned(),
            sub: ws.to_string(),
            jti: join_token::new_jti(),
            iat: now.unix_timestamp(),
            exp: expires.unix_timestamp(),
            server,
            host_ca: cas.cert_authority(),
            rp_id: app.cfg.public_url.host_str().unwrap_or_default().to_owned(),
            origin,
            inventory_name: name.clone(),
            profiles: (!profiles.is_empty()).then(|| profiles.clone()),
        };
        let token = key.sign(&claims);
        sqlx::query(
            "insert into join_tokens (jti, workspace_id, kid, token_hash, inventory_name, \
             profiles, expires_at, created_by) values ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&claims.jti)
        .bind(ws)
        .bind(key.kid())
        .bind(join_token::token_hash(&token).to_vec())
        .bind(&name)
        .bind(&profiles)
        .bind(expires)
        .bind(caller.user.id)
        .execute(&mut *tx)
        .await?;
        // By `jti`, never the token nor its hash.
        audit::record(
            &mut tx,
            Entry {
                actor: &caller.actor(),
                action: if rotating {
                    "join_token_rotated"
                } else {
                    "join_token_issued"
                },
                workspace: Some(WorkspaceId(ws)),
                target: json!({ "jti": claims.jti }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({
                    "expires_at": ts(expires),
                    "replaced": replaced,
                    "rejected": rejected,
                }),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(Ok((token, rejected)))
    }
    .await;
    let (token, rejected) = match result {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return e,
        Err(JoinError::Db(e)) => return db_failed(&e),
        Err(e) => {
            tracing::error!(error = %e, "issuing a join token failed");
            return internal();
        }
    };
    push_decided(app, &rejected).await;
    match token_info(&app.db, ws).await {
        Ok(info) => vec![Event::JoinTokenSecret {
            info,
            token: JoinTokenSecret(token),
            rejected: u32::try_from(rejected.len()).unwrap_or(u32::MAX),
        }],
        Err(e) => db_failed(&e),
    }
}

async fn preview_disable(app: &AppState, req: &Request<'_>, ws: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_join_token_disable";
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    match has_active(&app.db, ws).await {
        Ok(true) => {}
        Ok(false) => return op_error("no_join_token", "The workspace has no join token."),
        Err(e) => return db_failed(&e),
    }
    let affected = match pending_for_active(&app.db, ws).await {
        Ok(a) => a,
        Err(e) => return db_failed(&e),
    };
    let summary = format!(
        "Disable the join token: hosts can no longer ask to join this workspace. {} pending \
         request(s) are rejected. Hosts already accepted are not affected.",
        affected.len()
    );
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        disable(&app, &caller, ws).await
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: true,
        },
        op,
    )
}

async fn disable(app: &AppState, caller: &Caller, ws: Uuid) -> Vec<Event> {
    let result: Result<Option<Vec<Uuid>>, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        lock(&mut tx).await?;
        let jtis: Vec<(String,)> = sqlx::query_as(
            "update join_tokens set retired_at = now() where workspace_id = $1 \
             and retired_at is null returning jti",
        )
        .bind(ws)
        .fetch_all(&mut *tx)
        .await?;
        let jtis: Vec<String> = jtis.into_iter().map(|r| r.0).collect();
        if jtis.is_empty() {
            return Ok(None);
        }
        let rejected = reject_for_tokens(&mut tx, &jtis, Some(caller.user.id)).await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &caller.actor(),
                action: "join_token_disabled",
                workspace: Some(WorkspaceId(ws)),
                target: json!({ "jti": jtis }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({ "rejected": rejected }),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(Some(rejected))
    }
    .await;
    match result {
        Ok(Some(rejected)) => {
            push_decided(app, &rejected).await;
            match token_info(&app.db, ws).await {
                Ok(info) => vec![Event::JoinToken(info)],
                Err(e) => db_failed(&e),
            }
        }
        Ok(None) => op_error("no_join_token", "The workspace has no join token."),
        Err(e) => db_failed(&e),
    }
}

async fn preview_key_rotate(app: &AppState, req: &Request<'_>) -> Vec<Event> {
    const ACTION: &str = "preview_join_signing_key_rotate";
    if let Err(reason) = check_admin(req.caller) {
        return deny(
            app,
            req.caller,
            ACTION,
            None,
            reason,
            crate::bindings::channel(req),
        )
        .await;
    }
    if app.repos.master_key.is_none() {
        return op_error(
            "master_key_unavailable",
            "The join signing key needs the master key.",
        );
    }
    let op = pending(move |app, caller| async move {
        if let Err(reason) = check_admin(&caller) {
            return vec![Event::Denied {
                action: ACTION.to_owned(),
                reason: reason.to_owned(),
            }];
        }
        match rotate_key(&app, &caller).await {
            Ok((event, rejected)) => {
                push_decided(&app, &rejected).await;
                vec![event]
            }
            Err(JoinError::Db(e)) => db_failed(&e),
            Err(e) => {
                tracing::error!(error = %e, "rotating the join signing key failed");
                internal()
            }
        }
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary: "Rotate Keel's join signing key. Every workspace's join token stops working \
                      at once and their pending requests are rejected; issue new tokens \
                      afterwards. Hosts already accepted are not affected."
                .to_owned(),
            affected: vec!["join signing key".to_owned()],
            step_up: true,
        },
        op,
    )
}

async fn rotate_key(app: &AppState, caller: &Caller) -> Result<(Event, Vec<Uuid>), JoinError> {
    let master = app.repos.master_key.clone().ok_or(JoinError::NoMasterKey)?;
    let mut tx = app.db.begin().await?;
    lock(&mut tx).await?;
    sqlx::query("update join_signing_key set retired_at = now() where retired_at is null")
        .execute(&mut *tx)
        .await?;
    let jtis: Vec<(String,)> = sqlx::query_as(
        "update join_tokens set retired_at = now() where retired_at is null returning jti",
    )
    .fetch_all(&mut *tx)
    .await?;
    let jtis: Vec<String> = jtis.into_iter().map(|r| r.0).collect();
    let rejected = reject_for_tokens(&mut tx, &jtis, Some(caller.user.id)).await?;
    let (key, created) = create_key(
        &mut tx,
        &master,
        &caller.actor(),
        "join_signing_key_rotated",
    )
    .await?;
    tx.commit().await?;
    Ok((
        Event::JoinSigningKey {
            kid: key.kid(),
            created_at: ts(created),
            retired_tokens: u32::try_from(jtis.len()).unwrap_or(u32::MAX),
        },
        rejected,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limiter_counts_per_source_per_minute() {
        let l = JoinLimiter::default();
        for _ in 0..3 {
            assert!(l.allow("a", 3));
        }
        assert!(!l.allow("a", 3));
        assert!(l.allow("b", 3));
    }

    #[test]
    fn rate_keys_and_sources() {
        let direct = RunnerPath::Direct {
            peer: Some("203.0.113.4:51234".parse().unwrap()),
        };
        assert_eq!(source_key(&direct), "203.0.113.4");
        assert_eq!(source_shown(&direct), "203.0.113.4");
        let relay = RunnerPath::Relay {
            relay_id: Uuid::nil(),
            advisory_peer: Some("198.51.100.7:4000".into()),
        };
        assert_eq!(source_key(&relay), format!("relay:{}", Uuid::nil()));
        // The claimed peer does not change the key.
        let other = RunnerPath::Relay {
            relay_id: Uuid::nil(),
            advisory_peer: Some("192.0.2.99:1".into()),
        };
        assert_eq!(source_key(&other), source_key(&relay));
        let long = RunnerPath::Relay {
            relay_id: Uuid::nil(),
            advisory_peer: Some("x".repeat(1000)),
        };
        assert!(source_shown(&long).chars().count() <= MAX_SOURCE);
    }

    #[test]
    fn frames_are_bounded() {
        let allowed = vec!["web".to_owned()];
        let ok = JoinAttempt {
            token: "a.b.c",
            inventory_name: "web-1",
            allowed_profiles: &allowed,
            require_signed: false,
            hostname: "web-1",
            os: "Debian GNU/Linux 13 (trixie)",
            arch: "x86_64",
            runner_version: "0.1.0",
        };
        assert!(well_formed(&ok));
        assert!(!well_formed(&JoinAttempt {
            arch: "riscv64",
            ..ok
        }));
        let long = "x".repeat(254);
        assert!(!well_formed(&JoinAttempt {
            hostname: &long,
            ..ok
        }));
        assert!(!well_formed(&JoinAttempt {
            inventory_name: "web\n1",
            ..ok
        }));
        let bad = vec!["Web".to_owned()];
        assert!(!well_formed(&JoinAttempt {
            allowed_profiles: &bad,
            ..ok
        }));
    }
}
