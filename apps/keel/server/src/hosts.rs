//! Hosts, enrollment tokens and the two SSH CAs (spec 055 US7, T081 and T083).
//!
//! [`bootstrap`] runs at start: it makes any missing CA (host and runner, Ed25519), seals its
//! private key under the master key (`aad` = `ssh_ca:<id>`) and loads both into [`Ssh`], with
//! the KRL built from revoked certificates and revoked hosts' keys. Without them, enrollment,
//! the runner listener and certificate renewal all fail closed.
//!
//! Commands:
//! - `ListHosts`, `GetHost`: `View` on the workspace.
//! - `PreviewHostEnroll`: `Operate`. Confirming it mints a one-time token
//!   `keel-enroll-v1.<secret>.<host CA fingerprint>`, valid 15 minutes. Keel keeps only its
//!   SHA-256; the token is in the reply and nowhere else (not in the audit log, not in a log
//!   line).
//! - `PreviewHostRevoke`: `Administer` with a fresh passkey. Revokes the host and every
//!   certificate it holds, pushes the new KRL to every runner, closes the host's session (its
//!   in-flight runs become `lost`).
//! - `GetSshCa`: public keys and fingerprints only, to anyone signed in.
//! - `PreviewSshCaRotate`: instance admin with a fresh passkey. The old CA retires once the
//!   certificates it signed expire.

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use keel_core::WorkspaceId;
use keel_core::authz::{self, Capability};
use keel_exec::ca::{self, CaKind, RunnerCertRequest, RunnerCertVerifier, SshCa, TrustedCa};
use keel_exec::frames::Frame;
use keel_exec::krl::Krl;
use keel_protocol::{
    Cmd, EnrollToken, Event, HostCertInfo, HostDetail, HostInfo, HostStatus, SshCaInfo, SshCaKind,
    SshCaState,
};
use rand::TryRng;
use rand::rngs::SysRng;
use russh::keys::{Certificate, PublicKey, ssh_key::public::KeyData};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::bindings::{db_failed, enum_from, enum_str, internal, not_found, op_error, require, ts};
use crate::dispatch::{PendingOp, Request};
use crate::governance::{check_admin, deny};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::serve::AppState;

/// The token's prefix (and version).
pub const TOKEN_PREFIX: &str = "keel-enroll-v1";
/// How long an enrollment token is good for.
pub const TOKEN_TTL: Duration = Duration::from_secs(15 * 60);
/// The listener's host certificate lifetime; a retiring host CA stays trusted this long.
pub const HOST_CERT_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// The most hosts `ListHosts` returns.
const LIST_LIMIT: i64 = 1000;
/// The most certificates `GetHost` shows.
const CERT_LIMIT: i64 = 50;

// ---- state -----------------------------------------------------------------------------------

/// The loaded CAs and the KRL.
pub struct Cas {
    pub host: SshCa,
    pub host_id: Uuid,
    pub runner: SshCa,
    pub runner_id: Uuid,
    /// Active and retiring runner CAs, and the KRL: what the listener checks certificates with.
    pub verifier: Arc<RunnerCertVerifier>,
    pub krl: Krl,
}

impl Cas {
    /// The `@cert-authority * …` line for runners' `known_hosts` (`keel_runner_host_ca`).
    #[must_use]
    pub fn cert_authority(&self) -> String {
        self.host.cert_authority_line("*")
    }
}

/// Shared handle to the loaded CAs; empty (and so failing closed) until [`bootstrap`].
#[derive(Clone, Default)]
pub struct Ssh {
    inner: Arc<RwLock<Option<Arc<Cas>>>>,
}

impl Ssh {
    #[must_use]
    pub fn get(&self) -> Option<Arc<Cas>> {
        self.inner.read().ok().and_then(|g| g.clone())
    }

    fn set(&self, cas: Cas) {
        if let Ok(mut g) = self.inner.write() {
            *g = Some(Arc::new(cas));
        }
    }
}

/// Why the CAs could not be made or loaded. Carries no key material.
#[derive(Debug)]
pub enum SshError {
    NoMasterKey,
    Db(sqlx::Error),
    Seal(crate::seal::SealError),
    Ca(String),
}

impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMasterKey => f.write_str("the SSH CAs need the master key"),
            Self::Db(e) => write!(f, "database: {e}"),
            Self::Seal(e) => write!(f, "SSH CA key: {e}"),
            Self::Ca(e) => write!(f, "SSH CA: {e}"),
        }
    }
}

impl std::error::Error for SshError {}

impl From<sqlx::Error> for SshError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

impl From<crate::seal::SealError> for SshError {
    fn from(e: crate::seal::SealError) -> Self {
        Self::Seal(e)
    }
}

impl From<ca::CaError> for SshError {
    fn from(e: ca::CaError) -> Self {
        Self::Ca(e.to_string())
    }
}

const fn kind_of(k: SshCaKind) -> CaKind {
    match k {
        SshCaKind::Host => CaKind::Host,
        SshCaKind::Runner => CaKind::Runner,
    }
}

fn aad(id: Uuid) -> Vec<u8> {
    format!("ssh_ca:{id}").into_bytes()
}

fn unix(t: OffsetDateTime) -> u64 {
    u64::try_from(t.unix_timestamp()).unwrap_or(0)
}

fn at(secs: u64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(i64::try_from(secs).unwrap_or(i64::MAX))
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// Serials are u64 on the wire and in certificates, `bigint` in the database.
pub(crate) const fn serial_to_db(s: u64) -> i64 {
    i64::from_ne_bytes(s.to_ne_bytes())
}

pub(crate) const fn serial_from_db(s: i64) -> u64 {
    u64::from_ne_bytes(s.to_ne_bytes())
}

fn public_key(line: &str) -> Result<KeyData, SshError> {
    PublicKey::from_openssh(line)
        .map(|k| k.key_data().clone())
        .map_err(|e| SshError::Ca(e.to_string()))
}

/// Generates a CA of `kind`, seals and stores it as active, and audits it. The caller holds
/// the CA lock and has made sure no active CA of `kind` exists.
async fn create_ca(
    conn: &mut PgConnection,
    key: &crate::seal::MasterKey,
    kind: SshCaKind,
    actor: &Actor,
    action: &str,
) -> Result<(), SshError> {
    let ca = SshCa::generate(kind_of(kind))?;
    let id = Uuid::now_v7();
    let pem = ca.to_openssh()?;
    let sealed = key.seal(&aad(id), pem.expose().as_bytes())?;
    drop(pem);
    sqlx::query(
        "insert into ssh_cas (id, kind, public_key, fingerprint, sealed_private_key) \
         values ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(enum_str(&kind))
    .bind(ca.public_line())
    .bind(ca.fingerprint())
    .bind(sealed)
    .execute(&mut *conn)
    .await?;
    audit::record(
        conn,
        Entry {
            actor,
            action,
            workspace: None,
            target: json!({ "ssh_ca": id, "kind": enum_str(&kind) }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({ "fingerprint": ca.fingerprint() }),
        },
    )
    .await?;
    Ok(())
}

async fn lock_cas(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("select pg_advisory_xact_lock(hashtext('keel_ssh_cas'))")
        .execute(conn)
        .await
        .map(|_| ())
}

/// Makes any missing CA, retires CAs whose certificates have all expired, and loads them.
///
/// # Errors
///
/// No master key, the database, or a sealed key that does not open.
pub async fn bootstrap(app: &AppState) -> Result<(), SshError> {
    let key = app.repos.master_key.clone().ok_or(SshError::NoMasterKey)?;
    ensure(&app.db, &key).await?;
    reload(app).await
}

/// Makes any missing CA and retires CAs whose certificates have all expired.
///
/// # Errors
///
/// The database, or sealing failed.
pub async fn ensure(db: &sqlx::PgPool, key: &crate::seal::MasterKey) -> Result<(), SshError> {
    let mut tx = db.begin().await?;
    lock_cas(&mut tx).await?;
    for kind in [SshCaKind::Host, SshCaKind::Runner] {
        let (n,): (i64,) =
            sqlx::query_as("select count(*) from ssh_cas where kind = $1 and state = 'active'")
                .bind(enum_str(&kind))
                .fetch_one(&mut *tx)
                .await?;
        if n == 0 {
            create_ca(&mut tx, key, kind, &Actor::System, "ssh_ca_created").await?;
        }
    }
    sqlx::query(
        "update ssh_cas set state = 'retired' where state = 'retiring' and not_after < now()",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Reloads the CAs and rebuilds the KRL from the database.
///
/// # Errors
///
/// As [`bootstrap`].
pub async fn reload(app: &AppState) -> Result<(), SshError> {
    let key = app.repos.master_key.clone().ok_or(SshError::NoMasterKey)?;
    let cas = load(&app.db, &key).await?;
    app.ssh.set(cas);
    Ok(())
}

/// Opens the active CAs and builds the runner verifier and the KRL.
///
/// # Errors
///
/// As [`bootstrap`].
pub async fn load(db: &sqlx::PgPool, key: &crate::seal::MasterKey) -> Result<Cas, SshError> {
    type CaRow = (
        Uuid,
        String,
        String,
        Vec<u8>,
        String,
        Option<OffsetDateTime>,
    );
    let rows: Vec<CaRow> = sqlx::query_as(
        "select id, kind, public_key, sealed_private_key, state, not_after from ssh_cas \
         where state <> 'retired' order by created_at",
    )
    .fetch_all(db)
    .await?;
    let mut host = None;
    let mut runner = None;
    let mut trusted = Vec::new();
    for (id, kind, public, sealed, state, not_after) in rows {
        let kind: SshCaKind = enum_from(&kind).ok_or_else(|| SshError::Ca(kind.clone()))?;
        if state == "active" {
            let pem = key.open(&aad(id), &sealed)?;
            let pem = std::str::from_utf8(secrecy::ExposeSecret::expose_secret(&pem))
                .map_err(|_| SshError::Ca("sealed CA key is not text".into()))?;
            let ca = SshCa::from_openssh(kind_of(kind), pem)?;
            if kind == SshCaKind::Runner {
                trusted.push(TrustedCa::active(ca.key_data().clone()));
                runner = Some((id, ca));
            } else {
                host = Some((id, ca));
            }
        } else if kind == SshCaKind::Runner {
            let until = not_after.map_or(0, unix);
            trusted.push(TrustedCa::retiring(public_key(&public)?, until));
        }
    }
    let (Some((host_id, host)), Some((runner_id, runner))) = (host, runner) else {
        return Err(SshError::Ca("an active CA is missing".into()));
    };
    let now = unix(OffsetDateTime::now_utc());
    let mut krl = Krl::new(now, now);
    let revoked: Vec<(i64, String)> = sqlx::query_as(
        "select c.serial, ca.public_key from ssh_certs c join ssh_cas ca on ca.id = c.ca_id \
         where c.revoked_at is not null and c.valid_before > now()",
    )
    .fetch_all(db)
    .await?;
    for (serial, ca_line) in revoked {
        krl.revoke_serial(&public_key(&ca_line)?, serial_from_db(serial));
    }
    let keys: Vec<(String,)> = sqlx::query_as(
        "select ssh_public_key from hosts where state = 'revoked' and ssh_public_key is not null",
    )
    .fetch_all(db)
    .await?;
    for (line,) in keys {
        if let Ok(k) = public_key(&line) {
            krl.revoke_key(&k);
        }
    }
    Ok(Cas {
        verifier: Arc::new(RunnerCertVerifier::new(trusted, krl.clone())),
        krl,
        host,
        host_id,
        runner,
        runner_id,
    })
}

/// Reloads the KRL and sends it to every connected runner.
pub(crate) async fn publish_krl(app: &AppState) {
    match reload(app).await {
        Ok(()) => {
            if let Some(cas) = app.ssh.get() {
                app.runs.broadcast(&Frame::Krl {
                    bytes: cas.krl.to_bytes(),
                });
            }
        }
        Err(e) => tracing::error!(error = %e, "reloading the SSH CAs failed"),
    }
}

// ---- certificates ----------------------------------------------------------------------------

/// Issues and records a runner certificate for `host`'s enrolled `key`, with principals
/// `host:<name>` and `profile:<p>` for each profile both assigned and allowed.
pub(crate) async fn issue_cert(
    app: &AppState,
    conn: &mut PgConnection,
    host: Uuid,
    inventory_name: &str,
    key: &KeyData,
) -> Result<Certificate, SshError> {
    let cas = app.ssh.get().ok_or(SshError::Ca("not loaded".into()))?;
    let (allowed,): (Vec<String>,) =
        sqlx::query_as("select allowed_profiles from hosts where id = $1")
            .bind(host)
            .fetch_one(&mut *conn)
            .await?;
    let assigned: Vec<(String,)> = sqlx::query_as(
        "select p.name from host_profiles hp join profiles p on p.id = hp.profile_id \
         where hp.host_id = $1 order by p.name",
    )
    .bind(host)
    .fetch_all(&mut *conn)
    .await?;
    let assigned: Vec<String> = assigned.into_iter().map(|r| r.0).collect();
    let runner_id = host.to_string();
    // A random 64-bit serial; re-issue on the (practically impossible) collision.
    for _ in 0..4 {
        let cert = cas.runner.issue_runner_cert(&RunnerCertRequest {
            runner_id: &runner_id,
            inventory_name,
            assigned_profiles: &assigned,
            allowed_profiles: &allowed,
            public_key: key,
            ttl: app.cfg.runner_cert_ttl,
            now: SystemTime::now(),
        })?;
        let n = sqlx::query(
            "insert into ssh_certs (id, serial, ca_id, host_id, principals, valid_after, \
             valid_before) values ($1, $2, $3, $4, $5, $6, $7) on conflict (serial) do nothing",
        )
        .bind(Uuid::now_v7())
        .bind(serial_to_db(cert.serial()))
        .bind(cas.runner_id)
        .bind(host)
        .bind(cert.valid_principals().to_vec())
        .bind(at(cert.valid_after()))
        .bind(at(cert.valid_before()))
        .execute(&mut *conn)
        .await?
        .rows_affected();
        if n == 1 {
            return Ok(cert);
        }
    }
    Err(SshError::Ca("no free serial".into()))
}

/// `RenewCert`: a fresh certificate for `host`'s enrolled key, as its OpenSSH line. Refused
/// for a revoked host.
pub(crate) async fn renew(app: &AppState, host: Uuid) -> Result<String, SshError> {
    let mut tx = app.db.begin().await?;
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "select inventory_name, ssh_public_key from hosts where id = $1 and state <> 'revoked'",
    )
    .bind(host)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((name, Some(line))) = row else {
        return Err(SshError::Ca("unknown or revoked host".into()));
    };
    let key = public_key(&line)?;
    let cert = issue_cert(app, &mut tx, host, &name, &key).await?;
    audit::record(
        &mut tx,
        Entry {
            actor: &Actor::Host(host),
            action: "host_cert_renewed",
            workspace: None,
            target: json!({ "host": host }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({ "serial": cert.serial().to_string() }),
        },
    )
    .await?;
    tx.commit().await?;
    cert.to_openssh().map_err(|e| SshError::Ca(e.to_string()))
}

// ---- enrollment tokens -----------------------------------------------------------------------

/// SHA-256 of a whole token: what `host_enrollments.token_hash` holds.
#[must_use]
pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// A fresh token for the host CA with fingerprint `fp`.
fn new_token(fp: &str) -> Option<String> {
    let mut b = [0u8; 32];
    SysRng.try_fill_bytes(&mut b).ok()?;
    Some(format!(
        "{TOKEN_PREFIX}.{}.{fp}",
        crate::credentials::hex(&b)
    ))
}

/// The host CA fingerprint a token names, if it is well formed.
#[must_use]
pub fn token_fingerprint(token: &str) -> Option<&str> {
    let mut parts = token.splitn(3, '.');
    let (prefix, secret, fp) = (parts.next()?, parts.next()?, parts.next()?);
    (prefix == TOKEN_PREFIX
        && secret.len() == 64
        && secret.bytes().all(|b| b.is_ascii_hexdigit())
        && fp.starts_with("SHA256:"))
    .then_some(fp)
}

// ---- commands --------------------------------------------------------------------------------

pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::ListHosts {
            workspace,
            profile,
            drift,
        } => {
            if let Err(d) = require(app, req, "list_hosts", Capability::View, workspace).await {
                return d;
            }
            // Drift is not tracked yet: no host is known to have drifted.
            if drift == Some(true) {
                return vec![Event::Hosts { items: Vec::new() }];
            }
            match list(app, workspace, profile.as_ref().map(|p| p.as_str())).await {
                Ok(items) => vec![Event::Hosts { items }],
                Err(e) => db_failed(&e),
            }
        }
        Cmd::GetHost { host } => {
            let ws = match host_workspace(app, host).await {
                Ok(Some(ws)) => ws,
                Ok(None) => return not_found("host"),
                Err(e) => return db_failed(&e),
            };
            if let Err(d) = require(app, req, "get_host", Capability::View, ws).await {
                return d;
            }
            host_event(app, host).await
        }
        Cmd::PreviewHostEnroll {
            workspace,
            inventory_name,
            profiles,
        } => {
            let profiles: Vec<String> = profiles.0.iter().map(|p| p.as_str().to_owned()).collect();
            preview_enroll(
                app,
                req,
                workspace,
                inventory_name.as_str().to_owned(),
                profiles,
            )
            .await
        }
        Cmd::PreviewHostRevoke { host } => preview_revoke(app, req, host).await,
        Cmd::GetSshCa => ssh_ca_event(app).await,
        Cmd::PreviewSshCaRotate { kind } => preview_rotate(app, req, kind).await,
        _ => internal(),
    }
}

fn pending<F, Fut>(f: F) -> PendingOp
where
    F: FnOnce(AppState, Caller) -> Fut + Send + 'static,
    Fut: Future<Output = Vec<Event>> + Send + 'static,
{
    Box::new(move |app, caller| Box::pin(f(app, caller)))
}

fn preview(app: &AppState, req: &Request<'_>, draft: Draft, op: PendingOp) -> Vec<Event> {
    let key = req.caller.preview_key(req.session);
    if key.is_empty() {
        return internal();
    }
    vec![Event::Preview(app.previews.create(&key, draft, op))]
}

/// Re-checks `cap` when a confirmed operation runs (grants may have changed since the preview).
fn recheck(caller: &Caller, action: &str, cap: Capability, ws: Uuid) -> Result<(), Vec<Event>> {
    let refused = match &caller.token {
        Some(t) if t.workspace.0 != ws => Some("token_scope"),
        _ => authz::require(&crate::bindings::subject(caller), cap, WorkspaceId(ws), &[])
            .err()
            .map(|r| r.code()),
    };
    match refused {
        None => Ok(()),
        Some(reason) => Err(vec![Event::Denied {
            action: action.to_owned(),
            reason: reason.to_owned(),
        }]),
    }
}

async fn host_workspace(app: &AppState, host: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    let row: Option<(Uuid,)> = sqlx::query_as("select workspace_id from hosts where id = $1")
        .bind(host)
        .fetch_optional(&app.db)
        .await?;
    Ok(row.map(|r| r.0))
}

const HOST_COLUMNS: &str = "h.id, h.workspace_id, h.inventory_name, h.state, h.allowed_profiles, \
     h.require_signed, h.runner_version, h.ansible_version, h.enrolled_at, h.last_seen_at, \
     h.revoked_at, array(select p.name from host_profiles hp join profiles p \
       on p.id = hp.profile_id where hp.host_id = h.id order by p.name) as profiles, \
     h.python_version, h.override_quorum, h.ledger_head";

#[derive(sqlx::FromRow)]
struct HostRow {
    id: Uuid,
    workspace_id: Uuid,
    inventory_name: String,
    state: String,
    allowed_profiles: Vec<String>,
    require_signed: bool,
    runner_version: Option<String>,
    ansible_version: Option<String>,
    enrolled_at: OffsetDateTime,
    last_seen_at: Option<OffsetDateTime>,
    revoked_at: Option<OffsetDateTime>,
    profiles: Vec<String>,
    python_version: Option<String>,
    override_quorum: Option<i32>,
    ledger_head: Option<String>,
}

impl HostRow {
    fn info(&self) -> HostInfo {
        HostInfo {
            id: self.id,
            workspace: self.workspace_id,
            inventory_name: self.inventory_name.clone(),
            // An unknown state reads as revoked: fail closed.
            state: enum_from(&self.state).unwrap_or(HostStatus::Revoked),
            profiles: self.profiles.clone(),
            allowed_profiles: self.allowed_profiles.clone(),
            require_signed: self.require_signed,
            runner_version: self.runner_version.clone(),
            ansible_version: self.ansible_version.clone(),
            enrolled_at: ts(self.enrolled_at),
            last_seen_at: self.last_seen_at.map(ts),
            revoked_at: self.revoked_at.map(ts),
        }
    }
}

async fn list(
    app: &AppState,
    workspace: Uuid,
    profile: Option<&str>,
) -> Result<Vec<HostInfo>, sqlx::Error> {
    let rows: Vec<HostRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {HOST_COLUMNS} from hosts h where h.workspace_id = $1 \
         and ($2::text is null or exists (select 1 from host_profiles hp \
           join profiles p on p.id = hp.profile_id where hp.host_id = h.id and p.name = $2)) \
         order by h.inventory_name limit $3"
    )))
    .bind(workspace)
    .bind(profile)
    .bind(LIST_LIMIT)
    .fetch_all(&app.db)
    .await?;
    Ok(rows.iter().map(HostRow::info).collect())
}

async fn detail(app: &AppState, host: Uuid) -> Result<Option<HostDetail>, sqlx::Error> {
    let row: Option<HostRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {HOST_COLUMNS} from hosts h where h.id = $1"
    )))
    .bind(host)
    .fetch_optional(&app.db)
    .await?;
    let Some(row) = row else { return Ok(None) };
    type CertRow = (
        i64,
        Vec<String>,
        OffsetDateTime,
        OffsetDateTime,
        Option<OffsetDateTime>,
    );
    let certs: Vec<CertRow> = sqlx::query_as(
        "select serial, principals, valid_after, valid_before, revoked_at from ssh_certs \
         where host_id = $1 order by created_at desc limit $2",
    )
    .bind(host)
    .bind(CERT_LIMIT)
    .fetch_all(&app.db)
    .await?;
    Ok(Some(HostDetail {
        host: row.info(),
        python_version: row.python_version.clone(),
        override_quorum: row.override_quorum.and_then(|q| u32::try_from(q).ok()),
        ledger_head: row.ledger_head.clone(),
        certs: certs
            .into_iter()
            .map(
                |(serial, principals, after, before, revoked)| HostCertInfo {
                    serial: serial_from_db(serial).to_string(),
                    principals,
                    valid_after: ts(after),
                    valid_before: ts(before),
                    revoked_at: revoked.map(ts),
                },
            )
            .collect(),
    }))
}

async fn host_event(app: &AppState, host: Uuid) -> Vec<Event> {
    match detail(app, host).await {
        Ok(Some(d)) => vec![Event::Host(d)],
        Ok(None) => not_found("host"),
        Err(e) => db_failed(&e),
    }
}

async fn preview_enroll(
    app: &AppState,
    req: &Request<'_>,
    workspace: Uuid,
    inventory_name: String,
    profiles: Vec<String>,
) -> Vec<Event> {
    const ACTION: &str = "preview_host_enroll";
    if let Err(d) = require(app, req, ACTION, Capability::Operate, workspace).await {
        return d;
    }
    if app.repos.master_key.is_none() || app.ssh.get().is_none() {
        return op_error(
            "ssh_ca_unavailable",
            "The SSH certificate authorities are not loaded.",
        );
    }
    if let Err(e) = check_enroll(app, workspace, &inventory_name, &profiles).await {
        return e;
    }
    let summary = format!(
        "Enrol host {inventory_name} with profiles {}. The token is shown once and expires in \
         15 minutes.",
        if profiles.is_empty() {
            "(none)".to_owned()
        } else {
            profiles.join(", ")
        }
    );
    let affected = vec![inventory_name.clone()];
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Operate, workspace) {
            return d;
        }
        if let Err(e) = check_enroll(&app, workspace, &inventory_name, &profiles).await {
            return e;
        }
        create_enrollment(&app, &caller, workspace, inventory_name, profiles).await
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

/// The profiles exist in the workspace and no host has that name yet.
async fn check_enroll(
    app: &AppState,
    workspace: Uuid,
    inventory_name: &str,
    profiles: &[String],
) -> Result<(), Vec<Event>> {
    let found: Vec<(String,)> =
        sqlx::query_as("select name from profiles where workspace_id = $1 and name = any($2)")
            .bind(workspace)
            .bind(profiles)
            .fetch_all(&app.db)
            .await
            .map_err(|e| db_failed(&e))?;
    let found: BTreeSet<String> = found.into_iter().map(|r| r.0).collect();
    if let Some(missing) = profiles.iter().find(|p| !found.contains(*p)) {
        return Err(op_error(
            "unknown_profile",
            &format!("The workspace has no profile {missing}."),
        ));
    }
    let (exists,): (bool,) = sqlx::query_as(
        "select exists (select 1 from hosts where workspace_id = $1 and inventory_name = $2)",
    )
    .bind(workspace)
    .bind(inventory_name)
    .fetch_one(&app.db)
    .await
    .map_err(|e| db_failed(&e))?;
    if exists {
        return Err(op_error(
            "host_exists",
            &format!("A host named {inventory_name} is already enrolled in this workspace."),
        ));
    }
    Ok(())
}

async fn create_enrollment(
    app: &AppState,
    caller: &Caller,
    workspace: Uuid,
    inventory_name: String,
    profiles: Vec<String>,
) -> Vec<Event> {
    let Some(cas) = app.ssh.get() else {
        return op_error(
            "ssh_ca_unavailable",
            "The SSH certificate authorities are not loaded.",
        );
    };
    let Some(token) = new_token(&cas.host.fingerprint()) else {
        return internal();
    };
    let id = Uuid::now_v7();
    let expires = OffsetDateTime::now_utc() + TOKEN_TTL;
    let result: Result<(), sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        sqlx::query(
            "insert into host_enrollments (id, workspace_id, inventory_name, profiles, \
             token_hash, expires_at, created_by) values ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(workspace)
        .bind(&inventory_name)
        .bind(&profiles)
        .bind(token_hash(&token))
        .bind(expires)
        .bind(caller.user.id)
        .execute(&mut *tx)
        .await?;
        // Never the token, nor its hash.
        audit::record(
            &mut tx,
            Entry {
                actor: &caller.actor(),
                action: "host_enrollment_created",
                workspace: Some(WorkspaceId(workspace)),
                target: json!({ "enrollment": id, "inventory_name": inventory_name }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({ "profiles": profiles, "expires_at": ts(expires) }),
            },
        )
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        return db_failed(&e);
    }
    vec![Event::HostEnrollment {
        id,
        workspace,
        inventory_name,
        profiles,
        expires_at: ts(expires),
        token: EnrollToken(token),
    }]
}

async fn preview_revoke(app: &AppState, req: &Request<'_>, host: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_host_revoke";
    let row: Option<(Uuid, String, String)> =
        match sqlx::query_as("select workspace_id, inventory_name, state from hosts where id = $1")
            .bind(host)
            .fetch_optional(&app.db)
            .await
        {
            Ok(r) => r,
            Err(e) => return db_failed(&e),
        };
    let Some((ws, name, state)) = row else {
        return not_found("host");
    };
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    if state == "revoked" {
        return op_error("already_revoked", "The host is already revoked.");
    }
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        match revoke(&app, &caller, host, ws).await {
            Ok(true) => {}
            Ok(false) => return op_error("already_revoked", "The host is already revoked."),
            Err(e) => return db_failed(&e),
        }
        publish_krl(&app).await;
        app.runs.close(host);
        app.repos.push(
            ws,
            Event::HostState {
                host,
                state: HostStatus::Revoked,
            },
        );
        host_event(&app, host).await
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary: format!(
                "Revoke host {name}: its certificates are revoked, its runner is disconnected \
                 and any run in progress on it is marked lost. It must be enrolled again."
            ),
            affected: vec![name],
            step_up: true,
        },
        op,
    )
}

async fn revoke(
    app: &AppState,
    caller: &Caller,
    host: Uuid,
    ws: Uuid,
) -> Result<bool, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let n = sqlx::query(
        "update hosts set state = 'revoked', revoked_at = now() \
         where id = $1 and state <> 'revoked'",
    )
    .bind(host)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n != 1 {
        return Ok(false);
    }
    let serials: Vec<(i64,)> = sqlx::query_as(
        "update ssh_certs set revoked_at = now() where host_id = $1 and revoked_at is null \
         returning serial",
    )
    .bind(host)
    .fetch_all(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        Entry {
            actor: &caller.actor(),
            action: "host_revoked",
            workspace: Some(WorkspaceId(ws)),
            target: json!({ "host": host }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "serials": serials
                    .iter()
                    .map(|s| serial_from_db(s.0).to_string())
                    .collect::<Vec<_>>(),
            }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

async fn ssh_ca_event(app: &AppState) -> Vec<Event> {
    let Some(cas) = app.ssh.get() else {
        return op_error(
            "ssh_ca_unavailable",
            "The SSH certificate authorities are not loaded.",
        );
    };
    type Row = (
        Uuid,
        String,
        String,
        String,
        String,
        Option<OffsetDateTime>,
        OffsetDateTime,
    );
    let rows: Vec<Row> = match sqlx::query_as(
        "select id, kind, state, fingerprint, public_key, not_after, created_at from ssh_cas \
         where state <> 'retired' order by kind, created_at",
    )
    .fetch_all(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let cas_info = rows
        .into_iter()
        .filter_map(
            |(id, kind, state, fingerprint, public_key, not_after, created)| {
                Some(SshCaInfo {
                    id,
                    kind: enum_from(&kind)?,
                    state: enum_from::<SshCaState>(&state)?,
                    fingerprint,
                    public_key,
                    not_after: not_after.map(ts),
                    created_at: ts(created),
                })
            },
        )
        .collect();
    vec![Event::SshCa {
        cas: cas_info,
        cert_authority: cas.cert_authority(),
    }]
}

async fn preview_rotate(app: &AppState, req: &Request<'_>, kind: SshCaKind) -> Vec<Event> {
    const ACTION: &str = "preview_ssh_ca_rotate";
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
    if app.repos.master_key.is_none() || app.ssh.get().is_none() {
        return op_error(
            "ssh_ca_unavailable",
            "The SSH certificate authorities are not loaded.",
        );
    }
    let summary = match kind {
        SshCaKind::Runner => "Rotate the runner CA. Runners get certificates from the new CA \
             when they renew; the old CA stays trusted until the certificates it signed expire."
            .to_owned(),
        SshCaKind::Host => "Rotate the host CA. Every runner must be given the new \
             keel_runner_host_ca before it can connect again."
            .to_owned(),
    };
    let op = pending(move |app, caller| async move {
        if let Err(reason) = check_admin(&caller) {
            return vec![Event::Denied {
                action: ACTION.to_owned(),
                reason: reason.to_owned(),
            }];
        }
        if let Err(e) = rotate(&app, &caller, kind).await {
            tracing::error!(error = %e, "rotating an SSH CA failed");
            return internal();
        }
        publish_krl(&app).await;
        ssh_ca_event(&app).await
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected: vec![format!("{} CA", enum_str(&kind))],
            step_up: true,
        },
        op,
    )
}

async fn rotate(app: &AppState, caller: &Caller, kind: SshCaKind) -> Result<(), SshError> {
    let key = app.repos.master_key.clone().ok_or(SshError::NoMasterKey)?;
    let mut tx = app.db.begin().await?;
    lock_cas(&mut tx).await?;
    // The old CA stays trusted until the last certificate it signed expires: runner
    // certificates are recorded; the listener's host certificate lives HOST_CERT_TTL.
    let fallback = OffsetDateTime::now_utc()
        + match kind {
            SshCaKind::Host => HOST_CERT_TTL,
            SshCaKind::Runner => app.cfg.runner_cert_ttl,
        };
    sqlx::query(
        "update ssh_cas a set state = 'retiring', not_after = greatest($2, \
           (select max(valid_before) from ssh_certs c where c.ca_id = a.id)) \
         where kind = $1 and state = 'active'",
    )
    .bind(enum_str(&kind))
    .bind(fallback)
    .execute(&mut *tx)
    .await?;
    create_ca(&mut tx, &key, kind, &caller.actor(), "ssh_ca_rotated").await?;
    tx.commit().await?;
    Ok(())
}
