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
use keel_core::enroll_template;
use keel_exec::ca::{self, CaKind, RunnerCertRequest, RunnerCertVerifier, SshCa, TrustedCa};
use keel_exec::frames::Frame;
use keel_exec::krl::Krl;
use keel_protocol::{
    Cmd, EnrollToken, Event, HostCertInfo, HostDetail, HostInfo, HostProfilesChange,
    HostProfilesPreview, HostStatus, MAX_HOST_PROFILES, ProfileSuggestion, SshCaInfo, SshCaKind,
    SshCaState, SuggestionAction,
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

pub const fn serial_from_db(s: i64) -> u64 {
    u64::from_ne_bytes(s.to_ne_bytes())
}

pub(crate) fn public_key(line: &str) -> Result<KeyData, SshError> {
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

/// What [`settle_certs`] did for one runner session.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CertSettlement {
    /// Older certificates with other principals, now revoked and in the KRL.
    pub revoked: Vec<u64>,
    /// A fresh certificate pushed because the session's one predates a profile change.
    pub pushed: Option<u64>,
}

/// After a runner authenticates with the certificate `serial` (spec 068 T038, owner decision
/// 2026-11-17 "revoke via KRL"):
///
/// - Every older, unrevoked certificate of `host` whose principals differ from this one is
///   revoked and the KRL is rebroadcast: the runner has shown it holds the newer certificate,
///   so the one a profile change superseded can go. Renewals keep their principals and revoke
///   nothing.
/// - When a newer certificate with other principals exists (the runner was offline when its
///   profiles changed and never got the push), a fresh certificate is issued and pushed to the
///   session. The old one stays valid until the runner comes back with the new one, so an
///   offline runner is never locked out.
///
/// # Errors
///
/// A database or CA failure.
pub async fn settle_certs(
    app: &AppState,
    host: Uuid,
    serial: u64,
) -> Result<CertSettlement, SshError> {
    let mut out = CertSettlement::default();
    let mut tx = app.db.begin().await?;
    let revoked: Vec<(i64,)> = sqlx::query_as(
        "update ssh_certs o set revoked_at = now() from ssh_certs s \
         where s.host_id = $1 and s.serial = $2 and o.host_id = $1 and o.id < s.id \
         and o.revoked_at is null and o.principals <> s.principals \
         returning o.serial",
    )
    .bind(host)
    .bind(serial_to_db(serial))
    .fetch_all(&mut *tx)
    .await?;
    out.revoked = revoked.iter().map(|r| serial_from_db(r.0)).collect();
    if !out.revoked.is_empty() {
        let ws: Option<Uuid> = sqlx::query_scalar("select workspace_id from hosts where id = $1")
            .bind(host)
            .fetch_optional(&mut *tx)
            .await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &Actor::Host(host),
                action: "host_cert_superseded",
                workspace: ws.map(WorkspaceId),
                target: json!({ "host": host }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({
                    "serials": out.revoked.iter().map(u64::to_string).collect::<Vec<_>>(),
                    "by": serial.to_string(),
                }),
            },
        )
        .await?;
    }
    let (behind,): (bool,) = sqlx::query_as(
        "select exists (select 1 from ssh_certs n join ssh_certs s on s.host_id = n.host_id \
         where s.host_id = $1 and s.serial = $2 and n.id > s.id \
         and n.principals <> s.principals)",
    )
    .bind(host)
    .bind(serial_to_db(serial))
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    if !out.revoked.is_empty() {
        publish_krl(app).await;
    }
    if behind {
        let line = renew(app, host).await?;
        out.pushed = Certificate::from_openssh(&line).ok().map(|c| c.serial());
        app.runs.send(host, Frame::Cert { cert: line });
    }
    Ok(out)
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
            changed_outside,
        } => {
            if let Err(d) = require(app, req, "list_hosts", Capability::View, workspace).await {
                return d;
            }
            // Drift is not tracked yet: no host is known to have drifted.
            if drift == Some(true) {
                return vec![Event::Hosts { items: Vec::new() }];
            }
            match list(
                app,
                workspace,
                profile.as_ref().map(|p| p.as_str()),
                changed_outside,
            )
            .await
            {
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
            auto_converge,
        } => {
            let profiles: Vec<String> = profiles.0.iter().map(|p| p.as_str().to_owned()).collect();
            preview_enroll(
                app,
                req,
                workspace,
                inventory_name.as_str().to_owned(),
                profiles,
                auto_converge.unwrap_or(true),
            )
            .await
        }
        Cmd::PreviewHostRevoke { host } => preview_revoke(app, req, host).await,
        Cmd::PreviewHostProfiles(change) => preview_profiles(app, req, change).await,
        Cmd::ListProfileSuggestions { workspace, host_id } => {
            let action = "list_profile_suggestions";
            if let Err(d) = require(app, req, action, Capability::View, workspace).await {
                return d;
            }
            match suggestions(app, workspace, host_id).await {
                Ok(items) => vec![Event::ProfileSuggestions { items }],
                Err(e) => db_failed(&e),
            }
        }
        Cmd::DismissProfileSuggestion {
            host_id,
            profile,
            action,
        } => dismiss_suggestion(app, req, host_id, profile, action).await,
        Cmd::GetSshCa => ssh_ca_event(app).await,
        Cmd::PreviewSshCaRotate { kind } => preview_rotate(app, req, kind).await,
        _ => internal(),
    }
}

pub(crate) fn pending<F, Fut>(f: F) -> PendingOp
where
    F: FnOnce(AppState, Caller) -> Fut + Send + 'static,
    Fut: Future<Output = Vec<Event>> + Send + 'static,
{
    Box::new(move |app, caller| Box::pin(f(app, caller)))
}

pub(crate) fn preview(
    app: &AppState,
    req: &Request<'_>,
    draft: Draft,
    op: PendingOp,
) -> Vec<Event> {
    let key = req.caller.preview_key(req.session);
    if key.is_empty() {
        return internal();
    }
    vec![Event::Preview(app.previews.create(&key, draft, op))]
}

/// Re-checks `cap` when a confirmed operation runs (grants may have changed since the preview).
pub(crate) fn recheck(
    caller: &Caller,
    action: &str,
    cap: Capability,
    ws: Uuid,
) -> Result<(), Vec<Event>> {
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
     h.python_version, h.override_quorum, h.ledger_head, h.changed_outside, h.age_recipient, \
     h.zone_id, h.last_path, \
     (select count(*) from profile_suggestions s where s.host_id = h.id \
       and s.dismissed_at is null) as suggestions";

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
    changed_outside: bool,
    age_recipient: Option<String>,
    zone_id: Option<Uuid>,
    last_path: Option<serde_json::Value>,
    suggestions: i64,
}

impl HostRow {
    /// The wire view. `relays` are the workspace's ([`crate::relays::relay_infos`]), for the
    /// host's derived reach (spec 067 T031).
    fn info(
        &self,
        app: &AppState,
        listing: &crate::repo_catalog::Listing,
        relays: &[keel_protocol::RelayInfo],
    ) -> HostInfo {
        let path = app.runs.path(self.id);
        let reach = crate::relays::derive_reach(
            path.as_ref(),
            self.zone_id,
            crate::relays::last_relay(self.last_path.as_ref()),
            relays,
        );
        HostInfo {
            zone: self.zone_id,
            reach,
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
            changed_outside: self.changed_outside,
            age_recipient: self.age_recipient.clone(),
            // Spec 066 FR-023: from the workspace's primary catalog.
            inventory_status: listing.status(&self.inventory_name),
            // Spec 068: the open converge is not filled in yet.
            converge: None,
            suggestions: u16::try_from(self.suggestions).unwrap_or(u16::MAX),
        }
    }
}

async fn list(
    app: &AppState,
    workspace: Uuid,
    profile: Option<&str>,
    changed_outside: Option<bool>,
) -> Result<Vec<HostInfo>, sqlx::Error> {
    let rows: Vec<HostRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {HOST_COLUMNS} from hosts h where h.workspace_id = $1 \
         and ($2::text is null or exists (select 1 from host_profiles hp \
           join profiles p on p.id = hp.profile_id where hp.host_id = h.id and p.name = $2)) \
         and ($4::boolean is null or h.changed_outside = $4) \
         order by h.inventory_name limit $3"
    )))
    .bind(workspace)
    .bind(profile)
    .bind(LIST_LIMIT)
    .bind(changed_outside)
    .fetch_all(&app.db)
    .await?;
    let listing = crate::repo_catalog::listing(&app.db, workspace).await?;
    let relays = crate::relays::relay_infos(app, workspace).await?;
    Ok(rows
        .iter()
        .map(|r| r.info(app, &listing, &relays))
        .collect())
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
    let outside_runs = crate::outside::newest(&app.db, host).await?;
    let listing = crate::repo_catalog::listing(&app.db, row.workspace_id).await?;
    let relays = crate::relays::relay_infos(app, row.workspace_id).await?;
    Ok(Some(HostDetail {
        outside_runs,
        host: row.info(app, &listing, &relays),
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

pub(crate) async fn host_event(app: &AppState, host: Uuid) -> Vec<Event> {
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
    auto_converge: bool,
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
         15 minutes.{}",
        if profiles.is_empty() {
            "(none)".to_owned()
        } else {
            profiles.join(", ")
        },
        // Spec 068 US2: say so when the host will be converged once it enrolls.
        if auto_converge && app.cfg.converge && !profiles.is_empty() {
            " Keel converges it to these profiles once it enrolls."
        } else {
            ""
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
        create_enrollment(
            &app,
            &caller,
            workspace,
            inventory_name,
            profiles,
            auto_converge,
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
            step_up: false,
        },
        op,
    )
}

/// The profiles exist in the workspace and no host has that name yet.
pub(crate) async fn check_enroll(
    app: &AppState,
    workspace: Uuid,
    inventory_name: &str,
    profiles: &[String],
) -> Result<(), Vec<Event>> {
    // The System workspace runs only the profiles its configuration allows (spec 060 FR-037).
    let mut conn = app.db.acquire().await.map_err(|e| db_failed(&e))?;
    if let Some(p) = crate::system::disallowed_profile(&mut conn, workspace, profiles)
        .await
        .map_err(|e| db_failed(&e))?
    {
        return Err(op_error(
            "profile_not_allowed",
            &format!("The System workspace does not allow the profile {p}."),
        ));
    }
    drop(conn);
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
    auto_converge: bool,
) -> Vec<Event> {
    let new = match NewEnrollment::mint(app) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let result: Result<String, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let slug = insert_enrollment(
            &mut tx,
            caller,
            workspace,
            &inventory_name,
            &profiles,
            &new,
            None,
            auto_converge,
        )
        .await?;
        tx.commit().await?;
        Ok(slug)
    }
    .await;
    match result {
        Ok(slug) => {
            vec![enrollment_event(app, new, workspace, &slug, inventory_name, profiles).await]
        }
        Err(e) => db_failed(&e),
    }
}

/// A token about to be stored: its row id, the token itself (shown once) and its expiry.
pub(crate) struct NewEnrollment {
    pub id: Uuid,
    token: String,
    pub expires: OffsetDateTime,
}

impl NewEnrollment {
    /// A fresh token for the active host CA.
    pub(crate) fn mint(app: &AppState) -> Result<Self, Vec<Event>> {
        let Some(cas) = app.ssh.get() else {
            return Err(op_error(
                "ssh_ca_unavailable",
                "The SSH certificate authorities are not loaded.",
            ));
        };
        let Some(token) = new_token(&cas.host.fingerprint()) else {
            return Err(internal());
        };
        Ok(Self {
            id: Uuid::now_v7(),
            token,
            expires: OffsetDateTime::now_utc() + TOKEN_TTL,
        })
    }
}

/// Stores `new` (its hash only) and audits `host_enrollment_created` in the caller's
/// transaction; a rolled token's audit context names the one it replaced. Returns the
/// workspace slug.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_enrollment(
    tx: &mut PgConnection,
    caller: &Caller,
    workspace: Uuid,
    inventory_name: &str,
    profiles: &[String],
    new: &NewEnrollment,
    rolled_from: Option<Uuid>,
    auto_converge: bool,
) -> Result<String, sqlx::Error> {
    let slug: String = sqlx::query_scalar("select slug from workspaces where id = $1")
        .bind(workspace)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query(
        "insert into host_enrollments (id, workspace_id, inventory_name, profiles, \
         token_hash, expires_at, created_by, rolled_from, auto_converge) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, \
         coalesce((select o.auto_converge from host_enrollments o where o.id = $8), $9))",
    )
    .bind(new.id)
    .bind(workspace)
    .bind(inventory_name)
    .bind(profiles)
    .bind(token_hash(&new.token))
    .bind(new.expires)
    .bind(caller.user.id)
    .bind(rolled_from)
    .bind(auto_converge)
    .execute(&mut *tx)
    .await?;
    // A rolled token keeps the one it replaces' choice (the insert above copies it).
    let mut context = json!({ "profiles": profiles, "expires_at": ts(new.expires) });
    if rolled_from.is_none() {
        context["auto_converge"] = json!(auto_converge);
    }
    if let Some(old) = rolled_from {
        context["rolled_from"] = json!(old);
    }
    // Never the token, nor its hash.
    audit::record(
        tx,
        Entry {
            actor: &caller.actor(),
            action: "host_enrollment_created",
            workspace: Some(WorkspaceId(workspace)),
            target: json!({ "enrollment": new.id, "inventory_name": inventory_name }),
            decision: Decision::Allowed,
            reason: None,
            context,
        },
    )
    .await?;
    Ok(slug)
}

/// The `HostEnrollment` reply for a stored token. The snippet is the instance template rendered
/// with the token (spec 060, FR-004); when the settings can't be read, the default template is
/// used and `instructions_unavailable` says so. Nothing here logs the token.
pub(crate) async fn enrollment_event(
    app: &AppState,
    new: NewEnrollment,
    workspace: Uuid,
    slug: &str,
    inventory_name: String,
    profiles: Vec<String>,
) -> Event {
    let (template, instructions, unavailable) = match crate::settings::load(&app.db).await {
        Ok(s) => (s.template().to_owned(), s.instructions, false),
        Err(e) => {
            tracing::warn!(error = %e, "enrolment settings unreadable, using the defaults");
            (
                enroll_template::DEFAULT_TEMPLATE.to_owned(),
                String::new(),
                true,
            )
        }
    };
    let values = enroll_template::Values {
        token: new.token.clone(),
        name: inventory_name.clone(),
        profiles: profiles.join(","),
        workspace: slug.to_owned(),
        runner: crate::settings::runner_address(&app.cfg).unwrap_or_default(),
        url: crate::settings::public_url(&app.cfg),
        expires: ts(new.expires),
    };
    Event::HostEnrollment {
        id: new.id,
        workspace,
        inventory_name,
        profiles,
        expires_at: ts(new.expires),
        snippet: enroll_template::render(&template, &values),
        token: EnrollToken(new.token),
        instructions,
        instructions_unavailable: unavailable,
    }
}

// ---- profiles (spec 068 US3) -----------------------------------------------------------------

/// A host's assigned profile names, sorted.
async fn assigned_profiles(
    conn: &mut PgConnection,
    host: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "select p.name from host_profiles hp join profiles p on p.id = hp.profile_id \
         where hp.host_id = $1 order by p.name",
    )
    .bind(host)
    .fetch_all(conn)
    .await
}

/// Profiles after a change, then the ones it adds and removes.
type ProfileChange = (Vec<String>, Vec<String>, Vec<String>);

/// The effective change against `assigned`: (profiles after, added, removed). Adding what is
/// assigned or removing what is not is dropped; nothing left is `no_change`.
fn profile_change(
    assigned: &[String],
    change: &HostProfilesChange,
) -> Result<ProfileChange, Vec<Event>> {
    let add: BTreeSet<&String> = change.add.iter().collect();
    let remove: BTreeSet<&String> = change.remove.iter().collect();
    if add.intersection(&remove).next().is_some() {
        return Err(op_error(
            "invalid_change",
            "A profile cannot be both added and removed.",
        ));
    }
    let added: Vec<String> = add
        .into_iter()
        .filter(|p| !assigned.contains(p))
        .cloned()
        .collect();
    let removed: Vec<String> = remove
        .into_iter()
        .filter(|p| assigned.contains(p))
        .cloned()
        .collect();
    if added.is_empty() && removed.is_empty() {
        return Err(op_error(
            "no_change",
            "The host already has those profiles.",
        ));
    }
    let after: BTreeSet<String> = assigned
        .iter()
        .filter(|p| !removed.contains(p))
        .chain(&added)
        .cloned()
        .collect();
    if after.len() > MAX_HOST_PROFILES {
        return Err(op_error(
            "too_many_profiles",
            &format!("A host can have at most {MAX_HOST_PROFILES} profiles."),
        ));
    }
    Ok((after.into_iter().collect(), added, removed))
}

/// `PreviewHostProfiles`: what the change adds and removes, the added profiles outside the
/// host's own ceiling (its runner will refuse their jobs), its certificate principals after and
/// the converge steps it would then have. Confirming needs step-up.
async fn preview_profiles(
    app: &AppState,
    req: &Request<'_>,
    change: HostProfilesChange,
) -> Vec<Event> {
    const ACTION: &str = "preview_host_profiles";
    let host = change.host_id;
    let row: Option<(Uuid, String, String, Vec<String>)> = match sqlx::query_as(
        "select workspace_id, inventory_name, state, allowed_profiles from hosts where id = $1",
    )
    .bind(host)
    .fetch_optional(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let Some((ws, name, state, allowed)) = row else {
        return not_found("host");
    };
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    if state == "revoked" {
        return op_error("host_revoked", "The host is revoked.");
    }
    let assigned = match app.db.acquire().await {
        Ok(mut c) => match assigned_profiles(&mut c, host).await {
            Ok(a) => a,
            Err(e) => return db_failed(&e),
        },
        Err(e) => return db_failed(&e),
    };
    let (after, add, remove) = match profile_change(&assigned, &change) {
        Ok(c) => c,
        Err(d) => return d,
    };
    let known: Vec<String> = match sqlx::query_scalar(
        "select name from profiles where workspace_id = $1 and name = any($2)",
    )
    .bind(ws)
    .bind(&add)
    .fetch_all(&app.db)
    .await
    {
        Ok(k) => k,
        Err(e) => return db_failed(&e),
    };
    let unknown: Vec<&str> = add
        .iter()
        .filter(|p| !known.contains(p))
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        return op_error(
            "unknown_profile",
            &format!(
                "No repository defines profile {} in this workspace.",
                unknown.join(", ")
            ),
        );
    }
    let outside_ceiling: Vec<String> = add
        .iter()
        .filter(|p| !allowed.contains(p))
        .cloned()
        .collect();
    let principals_after: Vec<String> = std::iter::once(format!("host:{name}"))
        .chain(
            after
                .iter()
                .filter(|p| allowed.contains(p))
                .map(|p| format!("profile:{p}")),
        )
        .collect();
    let plan_after = match crate::converge::steps_with(app, ws, host, &name, &allowed, &after).await
    {
        Ok(s) => s,
        Err(e) => return db_failed(&e),
    };
    let mut summary = format!("Set host {name}'s profiles to ");
    summary.push_str(&if after.is_empty() {
        "(none)".to_owned()
    } else {
        after.join(", ")
    });
    summary.push_str(". Its certificate is re-issued with the new profiles.");
    if !outside_ceiling.is_empty() {
        summary.push_str(&format!(
            " Outside the host's own ceiling, so its runner will refuse their jobs: {}.",
            outside_ceiling.join(", ")
        ));
    }
    let preview = HostProfilesPreview {
        change: HostProfilesChange {
            host_id: host,
            add: add.clone(),
            remove: remove.clone(),
        },
        outside_ceiling,
        principals_after,
        plan_after,
    };
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        match set_profiles(&app, &caller, host, ws, &add, &remove).await {
            Ok(Ok(profiles)) => app.repos.push(
                ws,
                Event::HostProfilesChanged {
                    host_id: host,
                    profiles,
                },
            ),
            Ok(Err(d)) => return d,
            Err(e) => return db_failed(&e),
        }
        host_event(&app, host).await
    });
    let key = req.caller.preview_key(req.session);
    if key.is_empty() {
        return internal();
    }
    let p = app.previews.create(
        &key,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected: vec![name],
            step_up: true,
        },
        op,
    );
    vec![
        Event::HostProfilesPreview {
            preview_token: p.preview_id,
            preview,
        },
        Event::Preview(p),
    ]
}

/// Applies a confirmed profile change in one transaction: `host_profiles` (with `assigned_by`),
/// the suggestions it settles, a re-issued certificate when the host has a key and the CAs are
/// loaded, and `host.profiles_set`. The new certificate goes to the host's live session after
/// the commit; a host not connected gets one at its next renewal. Returns the profiles after.
async fn set_profiles(
    app: &AppState,
    caller: &Caller,
    host: Uuid,
    ws: Uuid,
    add: &[String],
    remove: &[String],
) -> Result<Result<Vec<String>, Vec<Event>>, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "select inventory_name, state, ssh_public_key from hosts where id = $1 for update",
    )
    .bind(host)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((name, state, key)) = row else {
        return Ok(Err(not_found("host")));
    };
    if state == "revoked" {
        return Ok(Err(op_error("host_revoked", "The host is revoked.")));
    }
    let before = assigned_profiles(&mut tx, host).await?;
    sqlx::query(
        "delete from host_profiles hp using profiles p \
         where hp.host_id = $1 and p.id = hp.profile_id and p.name = any($2)",
    )
    .bind(host)
    .bind(remove)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "insert into host_profiles (host_id, profile_id, assigned_by) \
         select $1, p.id, $3 from profiles p where p.workspace_id = $2 and p.name = any($4) \
         on conflict do nothing",
    )
    .bind(host)
    .bind(ws)
    .bind(caller.user.id)
    .bind(add)
    .execute(&mut *tx)
    .await?;
    let after = assigned_profiles(&mut tx, host).await?;
    if after.len() > MAX_HOST_PROFILES {
        return Ok(Err(op_error(
            "too_many_profiles",
            &format!("A host can have at most {MAX_HOST_PROFILES} profiles."),
        )));
    }
    // An add now assigned, or a remove now gone, is settled.
    sqlx::query(
        "delete from profile_suggestions where host_id = $1 and \
         ((action = 'add' and profile = any($2)) or (action = 'remove' and not profile = any($2)))",
    )
    .bind(host)
    .bind(&after)
    .execute(&mut *tx)
    .await?;
    let cert = match key {
        Some(line) if app.ssh.get().is_some() => {
            let issued = match public_key(&line) {
                Ok(k) => issue_cert(app, &mut tx, host, &name, &k).await,
                Err(e) => Err(e),
            };
            match issued {
                Ok(c) => Some(c),
                Err(SshError::Db(e)) => return Err(e),
                Err(e) => {
                    tracing::warn!(%host, error = %e, "re-issuing a certificate for new profiles failed");
                    None
                }
            }
        }
        _ => None,
    };
    let added: Vec<&String> = after.iter().filter(|p| !before.contains(p)).collect();
    let removed: Vec<&String> = before.iter().filter(|p| !after.contains(p)).collect();
    audit::record(
        &mut tx,
        Entry {
            actor: &caller.actor(),
            action: "host.profiles_set",
            workspace: Some(WorkspaceId(ws)),
            target: json!({ "host": host }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "added": added,
                "removed": removed,
                "profiles": after,
                "serial": cert.as_ref().map(|c| c.serial().to_string()),
            }),
        },
    )
    .await?;
    tx.commit().await?;
    if let Some(cert) = cert {
        match cert.to_openssh() {
            Ok(cert) => {
                app.runs.send(host, Frame::Cert { cert });
            }
            Err(e) => tracing::warn!(%host, error = %e, "encoding a re-issued certificate failed"),
        }
    }
    Ok(Ok(after))
}

/// Open suggestions in `workspace` (or for `host`), by host name then action and profile.
async fn suggestions(
    app: &AppState,
    workspace: Uuid,
    host: Option<Uuid>,
) -> Result<Vec<ProfileSuggestion>, sqlx::Error> {
    let rows: Vec<(Uuid, String, String, Vec<String>, OffsetDateTime)> = sqlx::query_as(
        "select s.host_id, s.profile, s.action, s.groups, s.first_seen_at \
         from profile_suggestions s join hosts h on h.id = s.host_id \
         where h.workspace_id = $1 and ($2::uuid is null or s.host_id = $2) \
           and s.dismissed_at is null and h.state <> 'revoked' \
         order by h.inventory_name, s.action, s.profile",
    )
    .bind(workspace)
    .bind(host)
    .fetch_all(&app.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(host_id, profile, action, groups, first)| {
            Some(ProfileSuggestion {
                host_id,
                profile,
                action: enum_from(&action)?,
                groups,
                first_seen_at: ts(first),
            })
        })
        .collect())
}

/// `DismissProfileSuggestion`: hidden until a sync suggests it from different groups.
async fn dismiss_suggestion(
    app: &AppState,
    req: &Request<'_>,
    host: Uuid,
    profile: String,
    action: SuggestionAction,
) -> Vec<Event> {
    const ACTION: &str = "dismiss_profile_suggestion";
    let ws = match host_workspace(app, host).await {
        Ok(Some(ws)) => ws,
        Ok(None) => return not_found("host"),
        Err(e) => return db_failed(&e),
    };
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    let action = enum_str(&action);
    let done: Result<bool, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let n = sqlx::query(
            "update profile_suggestions set dismissed_at = coalesce(dismissed_at, now()), \
             dismissed_by = coalesce(dismissed_by, $4) \
             where host_id = $1 and profile = $2 and action = $3",
        )
        .bind(host)
        .bind(&profile)
        .bind(&action)
        .bind(req.caller.user.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 0 {
            return Ok(false);
        }
        audit::record(
            &mut tx,
            Entry {
                actor: &req.caller.actor(),
                action: "profile_suggestion.dismissed",
                workspace: Some(WorkspaceId(ws)),
                target: json!({ "host": host, "profile": profile, "action": action }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({}),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }
    .await;
    match done {
        Ok(true) => {}
        Ok(false) => return not_found("suggestion"),
        Err(e) => return db_failed(&e),
    }
    match suggestions(app, ws, Some(host)).await {
        Ok(items) => vec![Event::ProfileSuggestions { items }],
        Err(e) => db_failed(&e),
    }
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
