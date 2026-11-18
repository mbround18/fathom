//! Relays (spec 067): Keel's relay-link key (T011), registering a relay (T023), and the
//! [`RelaySupervisor`] that dials every relay and serves the runner streams it carries (T020,
//! T021).
//!
//! [`bootstrap`] runs at start, after [`crate::hosts::bootstrap`]: when no `active`
//! `relay_link_keys` row exists it generates an Ed25519 key, seals its private half under the
//! master key (`aad` = `relay_link_key:<id>`), stores it and audits `relay_link_key.create`;
//! then it loads the active key into [`LinkKey`]. Keel authenticates to every relay with this
//! key; an owner pastes its public line into the relay's `hub_keys`. Without it, no relay is
//! dialled.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use keel_core::WorkspaceId;
use keel_exec::codec::CodecError;
use keel_exec::relay::{
    self, CacheMissReason, Frame, LINK_USER, LinkClientPolicy, LinkViolation, RELAY_SUBSYSTEM,
    RunnerChannel, ViolationSink, link_client_config,
};
use keel_protocol::{self as wire, Event};
use russh::client::{AuthResult, Handle};
use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, PrivateKey, PrivateKeyWithHashAlg, PublicKey};
use russh::{ChannelMsg, ChannelStream};
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use tokio::io::WriteHalf;
use tokio::net::TcpStream;
use tokio::sync::{Notify, broadcast, mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::runner_listener::{RunnerHost, RunnerPath, StreamEnd, serve_relayed};
use crate::serve::AppState;

/// The audit action for a new relay-link key.
pub const ACTION_CREATE: &str = "relay_link_key.create";

/// Keel's loaded relay-link key. The private key is only reachable through [`Self::key`].
pub struct RelayLinkKey {
    pub id: Uuid,
    /// The OpenSSH public line (`ssh-ed25519 AAAA… keel`), for a relay's `hub_keys`.
    pub public_line: String,
    /// `SHA256:…`.
    pub fingerprint: String,
    key: Arc<PrivateKey>,
}

impl RelayLinkKey {
    /// The private key, to authenticate a relay link.
    #[must_use]
    pub fn key(&self) -> Arc<PrivateKey> {
        Arc::clone(&self.key)
    }
}

impl std::fmt::Debug for RelayLinkKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayLinkKey")
            .field("id", &self.id)
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

/// Shared handle to the loaded relay-link key; empty (and so dialling nothing) until
/// [`bootstrap`].
#[derive(Clone, Default)]
pub struct LinkKey {
    inner: Arc<RwLock<Option<Arc<RelayLinkKey>>>>,
}

impl LinkKey {
    #[must_use]
    pub fn get(&self) -> Option<Arc<RelayLinkKey>> {
        self.inner.read().ok().and_then(|g| g.clone())
    }

    fn set(&self, key: RelayLinkKey) {
        if let Ok(mut g) = self.inner.write() {
            *g = Some(Arc::new(key));
        }
    }
}

/// Why the relay-link key could not be made or loaded. Carries no key material.
#[derive(Debug)]
pub enum RelayKeyError {
    NoMasterKey,
    Db(sqlx::Error),
    Seal(crate::seal::SealError),
    Key(String),
}

impl std::fmt::Display for RelayKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMasterKey => f.write_str("the relay-link key needs the master key"),
            Self::Db(e) => write!(f, "database: {e}"),
            Self::Seal(e) => write!(f, "relay-link key: {e}"),
            Self::Key(e) => write!(f, "relay-link key: {e}"),
        }
    }
}

impl std::error::Error for RelayKeyError {}

impl From<sqlx::Error> for RelayKeyError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

impl From<crate::seal::SealError> for RelayKeyError {
    fn from(e: crate::seal::SealError) -> Self {
        Self::Seal(e)
    }
}

fn aad(id: Uuid) -> Vec<u8> {
    format!("relay_link_key:{id}").into_bytes()
}

fn key_error(e: impl std::fmt::Display) -> RelayKeyError {
    RelayKeyError::Key(e.to_string())
}

fn public_line(key: &PrivateKey) -> Result<String, RelayKeyError> {
    let mut public = key.public_key().clone();
    public.set_comment("keel");
    public.to_openssh().map_err(key_error)
}

/// Generates a key, seals and stores it (`active`, or `pending` for a rotation) and audits it.
/// The caller holds the lock and, for `active`, has made sure no active key exists. Returns the
/// new key's fingerprint.
async fn create(
    conn: &mut PgConnection,
    master: &crate::seal::MasterKey,
    actor: &Actor,
    active: bool,
) -> Result<String, RelayKeyError> {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).map_err(key_error)?;
    let id = Uuid::now_v7();
    let pem = key.to_openssh(LineEnding::LF).map_err(key_error)?;
    let sealed = master.seal(&aad(id), pem.as_bytes())?;
    drop(pem);
    let fingerprint = keel_exec::ca::fingerprint(key.public_key().key_data());
    sqlx::query(
        "insert into relay_link_keys (id, public_key, fingerprint, sealed_private_key, state, \
         activated_at) values ($1, $2, $3, $4, \
         case when $5 then 'active' else 'pending' end, case when $5 then now() end)",
    )
    .bind(id)
    .bind(public_line(&key)?)
    .bind(&fingerprint)
    .bind(sealed)
    .bind(active)
    .execute(&mut *conn)
    .await?;
    audit::record(
        conn,
        Entry {
            actor,
            action: ACTION_CREATE,
            workspace: None,
            target: json!({ "relay_link_key": id }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "fingerprint": fingerprint,
                "state": if active { "active" } else { "pending" },
            }),
        },
    )
    .await?;
    Ok(fingerprint)
}

/// Makes the relay-link key if none is active, and loads it.
///
/// # Errors
///
/// No master key, the database, or a sealed key that does not open.
pub async fn bootstrap(app: &AppState) -> Result<(), RelayKeyError> {
    let master = app
        .repos
        .master_key
        .clone()
        .ok_or(RelayKeyError::NoMasterKey)?;
    ensure(&app.db, &master).await?;
    app.relay_link.set(load(&app.db, &master).await?);
    Ok(())
}

/// Makes the relay-link key if none is active. Safe to run from several Keels at once.
///
/// # Errors
///
/// The database, or sealing failed.
pub async fn ensure(
    db: &sqlx::PgPool,
    master: &crate::seal::MasterKey,
) -> Result<(), RelayKeyError> {
    let mut tx = db.begin().await?;
    sqlx::query("select pg_advisory_xact_lock(hashtext('keel_relay_link_keys'))")
        .execute(&mut *tx)
        .await?;
    let (n,): (i64,) =
        sqlx::query_as("select count(*) from relay_link_keys where state = 'active'")
            .fetch_one(&mut *tx)
            .await?;
    if n == 0 {
        create(&mut tx, master, &Actor::System, true).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Opens the active relay-link key.
///
/// # Errors
///
/// As [`bootstrap`]; also when no key is active.
pub async fn load(
    db: &sqlx::PgPool,
    master: &crate::seal::MasterKey,
) -> Result<RelayLinkKey, RelayKeyError> {
    let row: Option<(Uuid, String, String, Vec<u8>)> = sqlx::query_as(
        "select id, public_key, fingerprint, sealed_private_key from relay_link_keys \
         where state = 'active'",
    )
    .fetch_optional(db)
    .await?;
    let Some((id, public_line, fingerprint, sealed)) = row else {
        return Err(RelayKeyError::Key("no active key".into()));
    };
    let pem = master.open(&aad(id), &sealed)?;
    let pem = std::str::from_utf8(secrecy::ExposeSecret::expose_secret(&pem))
        .map_err(|_| RelayKeyError::Key("sealed key is not text".into()))?;
    let key = PrivateKey::from_openssh(pem).map_err(key_error)?;
    if key.algorithm() != Algorithm::Ed25519
        || keel_exec::ca::fingerprint(key.public_key().key_data()) != fingerprint
    {
        return Err(RelayKeyError::Key(
            "sealed key does not match its row".into(),
        ));
    }
    Ok(RelayLinkKey {
        id,
        public_line,
        fingerprint,
        key: Arc::new(key),
    })
}

// ---- registration (T023) --------------------------------------------------------------------

/// The audit action for a registered relay.
pub const ACTION_REGISTER: &str = "relay.register";
/// The audit action for a relay link's state change (`connected` / `unreachable`).
pub const ACTION_STATE: &str = "relay.state";
/// The audit action for a relay taken out of service after a protocol violation (T059).
pub const ACTION_QUARANTINE: &str = "relay.quarantine";
/// The audit action for refused streams and limit hits, coalesced per window (T061).
pub const ACTION_REFUSED: &str = "relay.refused";
/// The audit action for revoking a relay (T027).
pub const ACTION_REVOKE: &str = "relay.revoke";
/// The audit action for releasing a quarantined relay (T027, T059).
pub const ACTION_RELEASE: &str = "relay.release";
/// The audit action for editing a relay (spec 067 US3).
pub const ACTION_UPDATE: &str = "relay.update";
/// The audit action for activating a pending relay-link key (spec 067 US3).
pub const ACTION_KEY_ACTIVATE: &str = "relay_link_key.activate";
/// The audit action for retiring a pending relay-link key (spec 067 US3).
pub const ACTION_KEY_RETIRE: &str = "relay_link_key.retire";

/// The longest `dial_address` accepted (the column's check).
const MAX_DIAL_ADDRESS: usize = 300;

/// What an owner enters to register a relay (US3 wraps it in preview/confirm).
#[derive(Debug, Clone, Copy)]
pub struct RelayRegistration<'a> {
    pub workspace: Uuid,
    pub zone: Uuid,
    pub name: &'a str,
    /// `host:port`, with a DNS name, an IPv4 literal or `[v6]`.
    pub dial_address: &'a str,
    /// The relay's `ssh-ed25519` public line, as `keel-relay key` printed it.
    pub host_key: &'a str,
    /// The Keel host the relay runs on, if any.
    pub host: Option<Uuid>,
}

/// A registered relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredRelay {
    pub id: Uuid,
    /// `SHA256:…` of the pinned host key.
    pub fingerprint: String,
}

/// Why a registration was refused.
#[derive(Debug)]
pub enum RegisterError {
    /// Not `^[a-z0-9][a-z0-9-]{0,62}$`, or `direct`.
    Name,
    DialAddress(&'static str),
    HostKey(&'static str),
    /// No such zone in the workspace.
    Zone,
    /// No such host in the workspace.
    Host,
    /// The name is taken in the workspace.
    NameTaken,
    /// Another relay already pins this host key.
    HostKeyTaken,
    /// Update only: no such relay in the workspace.
    Relay,
    /// Update only: the relay is revoked, and a revoked relay is never changed.
    Revoked,
    Db(sqlx::Error),
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name => f.write_str(
                "relay name: lower-case letters, digits and '-', at most 63, not 'direct'",
            ),
            Self::DialAddress(why) => write!(f, "dial address: {why}"),
            Self::HostKey(why) => write!(f, "host key: {why}"),
            Self::Zone => f.write_str("no such zone in this workspace"),
            Self::Host => f.write_str("no such host in this workspace"),
            Self::NameTaken => f.write_str("a relay of that name exists in this workspace"),
            Self::HostKeyTaken => f.write_str("another relay already has that host key"),
            Self::Relay => f.write_str("no such relay"),
            Self::Revoked => f.write_str("the relay is revoked"),
            Self::Db(e) => write!(f, "database: {e}"),
        }
    }
}

impl std::error::Error for RegisterError {}

impl From<sqlx::Error> for RegisterError {
    fn from(e: sqlx::Error) -> Self {
        let (unique, on_key) = e.as_database_error().map_or((false, false), |d| {
            (
                d.is_unique_violation(),
                d.constraint().is_some_and(|c| c.contains("fingerprint")),
            )
        });
        match (unique, on_key) {
            (true, true) => Self::HostKeyTaken,
            (true, false) => Self::NameTaken,
            _ => Self::Db(e),
        }
    }
}

/// A zone or relay name: `^[a-z0-9][a-z0-9-]{0,62}$`, and not the reserved `direct`.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && name != "direct"
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// Checks a dial address: `host:port` with a DNS name, an IPv4 literal or a bracketed IPv6
/// literal, and a port of 1–65535. No scheme, user, path or whitespace.
///
/// # Errors
///
/// What is wrong with it, for the owner.
pub fn check_dial_address(s: &str) -> Result<(), &'static str> {
    const PORT: &str = "the port is a number from 1 to 65535";
    if s.len() < 3 || s.len() > MAX_DIAL_ADDRESS {
        return Err("must be host:port");
    }
    if s.bytes()
        .any(|c| c.is_ascii_whitespace() || c.is_ascii_control() || b"/@?#".contains(&c))
    {
        return Err("only host:port, without a scheme, user or path");
    }
    let port = if let Some(rest) = s.strip_prefix('[') {
        let (v6, port) = rest
            .split_once("]:")
            .ok_or("an IPv6 address is [address]:port")?;
        v6.parse::<std::net::Ipv6Addr>()
            .map_err(|_| "not an IPv6 address")?;
        port
    } else {
        let (host, port) = s.rsplit_once(':').ok_or("must be host:port")?;
        if host.contains(':') {
            return Err("an IPv6 address is [address]:port");
        }
        let label_ok = |l: &str| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        };
        let host = host.strip_suffix('.').unwrap_or(host);
        if host.is_empty() || host.len() > 253 || !host.split('.').all(label_ok) {
            return Err("not a host name or IP address");
        }
        port
    };
    if port.is_empty() || port.len() > 5 || !port.bytes().all(|c| c.is_ascii_digit()) {
        return Err(PORT);
    }
    match port.parse::<u32>() {
        Ok(1..=65_535) => Ok(()),
        _ => Err(PORT),
    }
}

/// Parses a relay host key: exactly one `ssh-ed25519` OpenSSH public line. Returns the key and
/// its `SHA256:` fingerprint.
///
/// # Errors
///
/// What is wrong with it, for the owner.
pub fn parse_host_key(line: &str) -> Result<(KeyData, String), &'static str> {
    let line = line.trim();
    if line.is_empty() {
        return Err("paste the line `keel-relay key` printed");
    }
    if line.contains(['\n', '\r']) {
        return Err("exactly one key line");
    }
    if !line.starts_with("ssh-ed25519 ") {
        return Err("only ssh-ed25519 keys");
    }
    let key = PublicKey::from_openssh(line).map_err(|_| "not an OpenSSH public key line")?;
    if key.algorithm() != Algorithm::Ed25519 {
        return Err("only ssh-ed25519 keys");
    }
    let data = key.key_data().clone();
    let fingerprint = keel_exec::ca::fingerprint(&data);
    Ok((data, fingerprint))
}

/// Registers a relay in `conn`'s transaction and audits `relay.register`. The supervisor picks
/// it up on its next pass; [`LiveRelays::resync`] (after the commit) makes that immediate.
///
/// # Errors
///
/// A field fails validation, the zone or host is not in the workspace, a duplicate, or the
/// database.
pub async fn register_relay(
    conn: &mut PgConnection,
    actor: &Actor,
    r: &RelayRegistration<'_>,
) -> Result<RegisteredRelay, RegisterError> {
    let fingerprint = check_registration(conn, r).await?;
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into relays (id, workspace_id, zone_id, name, dial_address, host_key, \
         host_key_fingerprint, host_id) values ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(r.workspace)
    .bind(r.zone)
    .bind(r.name)
    .bind(r.dial_address)
    .bind(r.host_key.trim())
    .bind(&fingerprint)
    .bind(r.host)
    .execute(&mut *conn)
    .await?;
    audit::record(
        conn,
        Entry {
            actor,
            action: ACTION_REGISTER,
            workspace: Some(WorkspaceId(r.workspace)),
            target: json!({ "relay": id }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "name": r.name,
                "zone": r.zone,
                "dial_address": r.dial_address,
                "fingerprint": fingerprint,
                "host": r.host,
            }),
        },
    )
    .await?;
    Ok(RegisteredRelay { id, fingerprint })
}

/// Edits relay `relay` of `r.workspace` in `conn`'s transaction (spec 067 US3): the same checks
/// as [`register_relay`], then audits `relay.update` with what changed. The supervisor redials
/// it on its next pass when the address or host key changed; [`LiveRelays::resync`] (after the
/// commit) makes that immediate. A quarantine stays: editing a relay is not releasing it.
///
/// # Errors
///
/// As [`register_relay`]; [`RegisterError::Relay`] or [`RegisterError::Revoked`].
pub async fn update_relay(
    conn: &mut PgConnection,
    actor: &Actor,
    relay: Uuid,
    r: &RelayRegistration<'_>,
) -> Result<RegisteredRelay, RegisterError> {
    type Before = (
        Option<OffsetDateTime>,
        Uuid,
        String,
        String,
        String,
        Option<Uuid>,
    );
    let before: Option<Before> = sqlx::query_as(
        "select revoked_at, zone_id, name, dial_address, host_key_fingerprint, host_id \
         from relays where id = $1 and workspace_id = $2 for update",
    )
    .bind(relay)
    .bind(r.workspace)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((revoked, zone, name, dial, old_fp, host)) = before else {
        return Err(RegisterError::Relay);
    };
    if revoked.is_some() {
        return Err(RegisterError::Revoked);
    }
    let fingerprint = check_registration(conn, r).await?;
    sqlx::query(
        "update relays set zone_id = $2, name = $3, dial_address = $4, host_key = $5, \
         host_key_fingerprint = $6, host_id = $7, updated_at = now() where id = $1",
    )
    .bind(relay)
    .bind(r.zone)
    .bind(r.name)
    .bind(r.dial_address)
    .bind(r.host_key.trim())
    .bind(&fingerprint)
    .bind(r.host)
    .execute(&mut *conn)
    .await?;
    audit::record(
        conn,
        Entry {
            actor,
            action: ACTION_UPDATE,
            workspace: Some(WorkspaceId(r.workspace)),
            target: json!({ "relay": relay }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "before": {
                    "name": name, "zone": zone, "dial_address": dial,
                    "fingerprint": old_fp, "host": host,
                },
                "after": {
                    "name": r.name, "zone": r.zone, "dial_address": r.dial_address,
                    "fingerprint": fingerprint, "host": r.host,
                },
            }),
        },
    )
    .await?;
    Ok(RegisteredRelay {
        id: relay,
        fingerprint,
    })
}

/// The checks [`register_relay`] and [`update_relay`] share (and their previews run). Returns
/// the host key's fingerprint.
pub(crate) async fn check_registration(
    conn: &mut PgConnection,
    r: &RelayRegistration<'_>,
) -> Result<String, RegisterError> {
    if !valid_name(r.name) {
        return Err(RegisterError::Name);
    }
    check_dial_address(r.dial_address).map_err(RegisterError::DialAddress)?;
    let (_, fingerprint) = parse_host_key(r.host_key).map_err(RegisterError::HostKey)?;
    let zone: Option<(Uuid,)> =
        sqlx::query_as("select id from zones where id = $1 and workspace_id = $2")
            .bind(r.zone)
            .bind(r.workspace)
            .fetch_optional(&mut *conn)
            .await?;
    if zone.is_none() {
        return Err(RegisterError::Zone);
    }
    if let Some(host) = r.host {
        let found: Option<(Uuid,)> =
            sqlx::query_as("select id from hosts where id = $1 and workspace_id = $2")
                .bind(host)
                .bind(r.workspace)
                .fetch_optional(&mut *conn)
                .await?;
        if found.is_none() {
            return Err(RegisterError::Host);
        }
    }
    Ok(fingerprint)
}

// ---- revoke and release (T027) ---------------------------------------------------------------

/// Why a revoke or release changed nothing.
#[derive(Debug)]
pub enum RelayActionError {
    /// No such relay in the workspace.
    NotFound,
    /// Revoked already: revocation is final, and a revoked relay cannot be released.
    Revoked,
    /// Release only: the relay is not quarantined.
    NotQuarantined,
    Db(sqlx::Error),
}

impl std::fmt::Display for RelayActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("no such relay"),
            Self::Revoked => f.write_str("the relay is revoked"),
            Self::NotQuarantined => f.write_str("the relay is not quarantined"),
            Self::Db(e) => write!(f, "database: {e}"),
        }
    }
}

impl std::error::Error for RelayActionError {}

impl From<sqlx::Error> for RelayActionError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

/// A relay row's revocation and quarantine, locked for an update.
async fn lock_relay(
    conn: &mut PgConnection,
    workspace: Uuid,
    relay: Uuid,
) -> Result<(bool, Option<String>), RelayActionError> {
    let row: Option<(bool, Option<String>)> = sqlx::query_as(
        "select revoked_at is not null, quarantine_reason from relays \
         where id = $1 and workspace_id = $2 for update",
    )
    .bind(relay)
    .bind(workspace)
    .fetch_optional(&mut *conn)
    .await?;
    row.ok_or(RelayActionError::NotFound)
}

/// Revokes a relay (T027): sets `revoked_at` and audits `relay.revoke` in one transaction,
/// then stops its link at once. The link sends `Close { "revoked" }` and drops every runner
/// stream it carries, so runners on it lose their sessions and their in-flight runs follow
/// the lost-runner path (`lost`). The relay is never dialled again.
///
/// The caller has authorised `actor` (an owner, after preview and step-up: spec 067 US3).
///
/// # Errors
///
/// [`RelayActionError::NotFound`], [`RelayActionError::Revoked`], or the database.
pub async fn revoke_relay(
    app: &AppState,
    actor: &Actor,
    workspace: Uuid,
    relay: Uuid,
) -> Result<(), RelayActionError> {
    let mut tx = app.db.begin().await?;
    let (revoked, quarantine) = lock_relay(&mut tx, workspace, relay).await?;
    if revoked {
        return Err(RelayActionError::Revoked);
    }
    sqlx::query("update relays set revoked_at = now(), updated_at = now() where id = $1")
        .bind(relay)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry {
            actor,
            action: ACTION_REVOKE,
            workspace: Some(WorkspaceId(workspace)),
            target: json!({ "relay": relay }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({ "quarantine_reason": quarantine }),
        },
    )
    .await?;
    tx.commit().await?;
    app.relay_live.stop(relay, "revoked");
    app.relay_live.resync();
    Ok(())
}

/// Releases a quarantined relay (T027, T059): clears `quarantined_at` and the reason and audits
/// `relay.release` (with the reason it was quarantined for) in one transaction; the supervisor
/// then dials it again.
///
/// `PreviewRelayRelease` ([`crate::zones`], spec 067 US3) calls this after its preview and
/// step-up.
///
/// # Errors
///
/// [`RelayActionError::NotFound`], [`RelayActionError::Revoked`],
/// [`RelayActionError::NotQuarantined`], or the database.
pub async fn release_relay(
    app: &AppState,
    actor: &Actor,
    workspace: Uuid,
    relay: Uuid,
) -> Result<(), RelayActionError> {
    let mut tx = app.db.begin().await?;
    let (revoked, quarantine) = lock_relay(&mut tx, workspace, relay).await?;
    if revoked {
        return Err(RelayActionError::Revoked);
    }
    let Some(reason) = quarantine else {
        return Err(RelayActionError::NotQuarantined);
    };
    sqlx::query(
        "update relays set quarantined_at = null, quarantine_reason = null, \
         updated_at = now() where id = $1",
    )
    .bind(relay)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        Entry {
            actor,
            action: ACTION_RELEASE,
            workspace: Some(WorkspaceId(workspace)),
            target: json!({ "relay": relay }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({ "quarantine_reason": reason }),
        },
    )
    .await?;
    tx.commit().await?;
    app.relay_live.resync();
    Ok(())
}

// ---- relay-link key rotation (spec 067 US3, T030) -------------------------------------------

/// Why a relay-link key rotation step changed nothing.
#[derive(Debug)]
pub enum LinkKeyError {
    /// Rotate: a pending key exists already; activate or retire it first.
    PendingExists,
    /// Activate or retire: no pending key has that fingerprint.
    NotPending,
    Key(RelayKeyError),
}

impl std::fmt::Display for LinkKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PendingExists => f.write_str("a new relay-link key is already pending"),
            Self::NotPending => f.write_str("no pending relay-link key has that fingerprint"),
            Self::Key(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for LinkKeyError {}

impl From<RelayKeyError> for LinkKeyError {
    fn from(e: RelayKeyError) -> Self {
        Self::Key(e)
    }
}

impl From<sqlx::Error> for LinkKeyError {
    fn from(e: sqlx::Error) -> Self {
        Self::Key(RelayKeyError::Db(e))
    }
}

async fn lock_link_keys(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("select pg_advisory_xact_lock(hashtext('keel_relay_link_keys'))")
        .execute(conn)
        .await?;
    Ok(())
}

fn master(app: &AppState) -> Result<std::sync::Arc<crate::seal::MasterKey>, LinkKeyError> {
    app.repos
        .master_key
        .clone()
        .ok_or(LinkKeyError::Key(RelayKeyError::NoMasterKey))
}

/// Starts a rotation: a new `pending` relay-link key, audited `relay_link_key.create`. Keel
/// keeps dialling with the active key until the pending one is activated, so an owner can add
/// the new public line to every relay's `hub_keys` first. Returns its fingerprint.
///
/// # Errors
///
/// [`LinkKeyError::PendingExists`], no master key, or the database.
pub async fn rotate_link_key(app: &AppState, actor: &Actor) -> Result<String, LinkKeyError> {
    let master = master(app)?;
    let mut tx = app.db.begin().await?;
    lock_link_keys(&mut tx).await?;
    let (n,): (i64,) =
        sqlx::query_as("select count(*) from relay_link_keys where state = 'pending'")
            .fetch_one(&mut *tx)
            .await?;
    if n > 0 {
        return Err(LinkKeyError::PendingExists);
    }
    let fingerprint = create(&mut tx, &master, actor, false).await?;
    tx.commit().await?;
    Ok(fingerprint)
}

/// Activates the pending key `fingerprint`: the active one is retired, the pending one becomes
/// active (audited `relay_link_key.activate`), and this Keel dials with it from the next link
/// on. Links already up stay up.
///
/// # Errors
///
/// [`LinkKeyError::NotPending`], no master key, a sealed key that does not open, or the
/// database.
pub async fn activate_link_key(
    app: &AppState,
    actor: &Actor,
    fingerprint: &str,
) -> Result<(), LinkKeyError> {
    let master = master(app)?;
    let mut tx = app.db.begin().await?;
    lock_link_keys(&mut tx).await?;
    let pending: Option<(Uuid,)> = sqlx::query_as(
        "select id from relay_link_keys where fingerprint = $1 and state = 'pending'",
    )
    .bind(fingerprint)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id,)) = pending else {
        return Err(LinkKeyError::NotPending);
    };
    let retired: Vec<(String,)> = sqlx::query_as(
        "update relay_link_keys set state = 'retired', retired_at = now() \
         where state = 'active' returning fingerprint",
    )
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("update relay_link_keys set state = 'active', activated_at = now() where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry {
            actor,
            action: ACTION_KEY_ACTIVATE,
            workspace: None,
            target: json!({ "relay_link_key": id }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "fingerprint": fingerprint,
                "retired": retired.into_iter().map(|r| r.0).collect::<Vec<_>>(),
            }),
        },
    )
    .await?;
    tx.commit().await?;
    app.relay_link.set(load(&app.db, &master).await?);
    Ok(())
}

/// Abandons a rotation: the pending key `fingerprint` is retired (audited
/// `relay_link_key.retire`). The active key cannot be retired; activating another retires it.
///
/// # Errors
///
/// [`LinkKeyError::NotPending`] or the database.
pub async fn retire_link_key(
    app: &AppState,
    actor: &Actor,
    fingerprint: &str,
) -> Result<(), LinkKeyError> {
    let mut tx = app.db.begin().await?;
    lock_link_keys(&mut tx).await?;
    let row: Option<(Uuid,)> = sqlx::query_as(
        "update relay_link_keys set state = 'retired', retired_at = now() \
         where fingerprint = $1 and state = 'pending' returning id",
    )
    .bind(fingerprint)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id,)) = row else {
        return Err(LinkKeyError::NotPending);
    };
    audit::record(
        &mut tx,
        Entry {
            actor,
            action: ACTION_KEY_RETIRE,
            workspace: None,
            target: json!({ "relay_link_key": id }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({ "fingerprint": fingerprint }),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The relay-link keys, public halves only: active first, then pending, then the newest
/// retired ones.
///
/// # Errors
///
/// The database.
pub async fn link_keys(db: &PgPool) -> Result<Vec<wire::RelayLinkKey>, sqlx::Error> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "select fingerprint, public_key, state from relay_link_keys order by \
         case state when 'active' then 0 when 'pending' then 1 else 2 end, created_at desc \
         limit 20",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(fingerprint, public_line, state)| wire::RelayLinkKey {
            fingerprint,
            public_line,
            state: match state.as_str() {
                "active" => wire::RelayLinkKeyState::Active,
                "pending" => wire::RelayLinkKeyState::Pending,
                _ => wire::RelayLinkKeyState::Retired,
            },
        })
        .collect())
}

// ---- wire views and derived reach (spec 067 US3, T029, T031) --------------------------------

/// How often an RTT, version or stream-count change alone is broadcast per relay (a state
/// change always is).
pub const DETAIL_PUSH_INTERVAL: Duration = Duration::from_secs(30);

#[derive(sqlx::FromRow)]
struct InfoRow {
    id: Uuid,
    workspace_id: Uuid,
    zone_id: Uuid,
    name: String,
    dial_address: String,
    host_key_fingerprint: String,
    host_id: Option<Uuid>,
    revoked_at: Option<OffsetDateTime>,
    quarantined_at: Option<OffsetDateTime>,
    quarantine_reason: Option<String>,
}

const fn wire_state(s: RelayState) -> wire::RelayState {
    match s {
        RelayState::Connecting => wire::RelayState::Connecting,
        RelayState::Connected => wire::RelayState::Connected,
        RelayState::Unreachable => wire::RelayState::Unreachable,
        RelayState::Quarantined => wire::RelayState::Quarantined,
    }
}

/// A relay row and its live state as the wire shows them. Revoked wins; with no live state (no
/// supervisor yet, or relays off) a quarantined row reads `quarantined` and any other
/// `connecting`.
fn wire_info(row: InfoRow, live: Option<RelayLive>) -> Option<wire::RelayInfo> {
    let name = wire::ZoneName::new(row.name).ok()?;
    let ts = crate::bindings::ts;
    let live = live.filter(|_| row.revoked_at.is_none());
    let (state, since) = match (&row.revoked_at, &live, &row.quarantine_reason) {
        (Some(at), _, _) => (wire::RelayState::Revoked, Some(ts(*at))),
        (None, Some(l), _) => (wire_state(l.state), Some(ts(l.since))),
        (None, None, Some(_)) => (wire::RelayState::Quarantined, row.quarantined_at.map(ts)),
        (None, None, None) => (wire::RelayState::Connecting, None),
    };
    let last_error = live
        .as_ref()
        .and_then(|l| l.last_error.clone())
        .or_else(|| {
            (state == wire::RelayState::Quarantined)
                .then(|| row.quarantine_reason.clone())
                .flatten()
        });
    Some(wire::RelayInfo {
        id: row.id,
        workspace: row.workspace_id,
        zone: row.zone_id,
        name,
        dial_address: row.dial_address,
        host_key_fingerprint: row.host_key_fingerprint,
        host: row.host_id,
        state,
        since,
        rtt_ms: live
            .as_ref()
            .and_then(|l| l.rtt_ms)
            .map(|r| u32::try_from(r).unwrap_or(u32::MAX)),
        version: live.as_ref().and_then(|l| l.version.clone()),
        streams: live.as_ref().map(|l| l.streams),
        last_error,
        revoked_at: row.revoked_at.map(ts),
    })
}

/// Every relay of `workspace` as `ListRelays` shows it: the rows, with live state. Revoked
/// ones are listed last.
///
/// # Errors
///
/// The database.
pub async fn relay_infos(
    app: &AppState,
    workspace: Uuid,
) -> Result<Vec<wire::RelayInfo>, sqlx::Error> {
    let rows: Vec<InfoRow> = sqlx::query_as(
        "select id, workspace_id, zone_id, name, dial_address, host_key_fingerprint, host_id, \
         revoked_at, quarantined_at, quarantine_reason from relays where workspace_id = $1 \
         order by revoked_at is not null, name",
    )
    .bind(workspace)
    .fetch_all(&app.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let live = app.relay_live.get(r.id);
            wire_info(r, live)
        })
        .collect())
}

/// The relay a host's `last_path` came through, if it came through one.
#[must_use]
pub fn last_relay(last_path: Option<&serde_json::Value>) -> Option<Uuid> {
    let p = last_path?;
    if p.get("kind")?.as_str()? != "relay" {
        return None;
    }
    p.get("relay")?.as_str()?.parse().ok()
}

/// A host's reach (data-model.md, "Derived reach"), from its session (`connected`), its zone
/// (`None`: direct), the relay of its last session, and its workspace's relays:
///
/// 1. connected: how;
/// 2. a direct zone, or a zone with a connected relay: `disconnected` (a path exists);
/// 3. its last session came over a relay now revoked: `relay_revoked`;
/// 4. no non-revoked relay in its zone: `no_path`;
/// 5. a relay of its zone is still connecting: `disconnected`;
/// 6. otherwise every relay of its zone is unreachable or quarantined: `relay_unreachable`.
#[must_use]
pub fn derive_reach(
    connected: Option<&RunnerPath>,
    zone: Option<Uuid>,
    last_relay: Option<Uuid>,
    relays: &[wire::RelayInfo],
) -> wire::HostReach {
    use wire::{HostReach, RelayState as S};
    if let Some(path) = connected {
        let path = match path.relay() {
            Some(relay) => wire::RunnerPath::Relay { relay },
            None => wire::RunnerPath::Direct,
        };
        return HostReach::Connected { path };
    }
    let Some(zone) = zone else {
        return HostReach::Disconnected;
    };
    let serving: Vec<&wire::RelayInfo> = relays
        .iter()
        .filter(|r| r.zone == zone && r.state != S::Revoked)
        .collect();
    if serving.iter().any(|r| r.state == S::Connected) {
        return HostReach::Disconnected;
    }
    if let Some(last) = last_relay
        && relays.iter().any(|r| r.id == last && r.state == S::Revoked)
    {
        return HostReach::RelayRevoked { relay: last };
    }
    if serving.is_empty() {
        return HostReach::NoPath;
    }
    if serving.iter().any(|r| r.state == S::Connecting) {
        return HostReach::Disconnected;
    }
    HostReach::RelayUnreachable {
        relays: serving.iter().map(|r| r.id).collect(),
    }
}

/// Every host of `workspace` that is not revoked, with its reach given `relays` (the
/// workspace's, from [`relay_infos`]).
///
/// # Errors
///
/// The database.
pub async fn workspace_reach(
    app: &AppState,
    workspace: Uuid,
    relays: &[wire::RelayInfo],
) -> Result<Vec<(Uuid, wire::HostReach)>, sqlx::Error> {
    let rows: Vec<(Uuid, Option<Uuid>, Option<serde_json::Value>)> = sqlx::query_as(
        "select id, zone_id, last_path from hosts where workspace_id = $1 \
         and state <> 'revoked'",
    )
    .bind(workspace)
    .fetch_all(&app.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(host, zone, last)| {
            let path = app.runs.path(host);
            let reach = derive_reach(path.as_ref(), zone, last_relay(last.as_ref()), relays);
            (host, reach)
        })
        .collect())
}

/// Pushes `HostReachChanged` for `host` to its workspace: its runner just connected or
/// disconnected (T031). A failure is logged; the next `ListHosts` has the truth.
pub async fn push_host_reach(app: &AppState, host: Uuid) {
    let result = async {
        let row: Option<(Uuid, Option<Uuid>, Option<serde_json::Value>)> =
            sqlx::query_as("select workspace_id, zone_id, last_path from hosts where id = $1")
                .bind(host)
                .fetch_optional(&app.db)
                .await?;
        let Some((ws, zone, last)) = row else {
            return Ok(());
        };
        let relays = relay_infos(app, ws).await?;
        let path = app.runs.path(host);
        let reach = derive_reach(path.as_ref(), zone, last_relay(last.as_ref()), &relays);
        app.repos.push(ws, Event::HostReachChanged { host, reach });
        Ok::<_, sqlx::Error>(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!(host = %host, error = %e, "pushing a host's reach failed");
    }
}

/// Pushes `RelayChanged` for every [`RelayChange`] the supervisor broadcasts, and
/// `HostReachChanged` for each host of that workspace whose reach differs from the last one
/// pushed (T031). Runs for the supervisor's life.
async fn publish(app: AppState, mut changes: broadcast::Receiver<RelayChange>) {
    let mut pushed: HashMap<Uuid, wire::HostReach> = HashMap::new();
    loop {
        let change = match changes.recv().await {
            Ok(c) => c,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(missed = n, "relay changes lagged");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let ws = change.workspace;
        let result = async {
            let relays = relay_infos(&app, ws).await?;
            if let Some(info) = relays.iter().find(|r| r.id == change.relay) {
                app.repos.push(ws, Event::RelayChanged(info.clone()));
            }
            for (host, reach) in workspace_reach(&app, ws, &relays).await? {
                if pushed.get(&host) != Some(&reach) {
                    pushed.insert(host, reach.clone());
                    app.repos.push(ws, Event::HostReachChanged { host, reach });
                }
            }
            Ok::<_, sqlx::Error>(())
        }
        .await;
        if let Err(e) = result {
            tracing::warn!(relay = %change.relay, error = %e, "pushing a relay change failed");
        }
    }
}

// ---- run preview warning (spec 067 US3, T032) -----------------------------------------------

/// The longest list of dependent hosts a warning names.
const WARN_HOSTS: usize = 10;

/// The warning for a run that targets `relay_host`, which runs `relay`, the only relay of a
/// zone whose hosts `dependents` the run also targets.
#[must_use]
pub fn relay_dependency_warning(relay: &str, relay_host: &str, dependents: &[String]) -> String {
    let mut names = dependents
        .iter()
        .take(WARN_HOSTS)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if dependents.len() > WARN_HOSTS {
        names.push_str(&format!(" and {} more", dependents.len() - WARN_HOSTS));
    }
    format!(
        "This run includes {relay_host}, which runs relay {relay}, the only relay for \
         {names}. If the run restarts or breaks the relay, those hosts lose their path to Keel \
         and their part of the run is lost. Consider running {relay_host} separately."
    )
}

/// The [`relay_dependency_warning`]s for a run of `workspace` reaching `targets`: one per
/// non-revoked relay whose host is a target and which is the only non-revoked relay of a zone
/// that holds other targets.
///
/// # Errors
///
/// The database.
pub async fn run_warnings(
    db: &PgPool,
    workspace: Uuid,
    targets: &[Uuid],
) -> Result<Vec<String>, sqlx::Error> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(String, String, Vec<String>)> = sqlx::query_as(
        "select r.name, rh.inventory_name, \
           array_agg(h.inventory_name order by h.inventory_name) \
         from relays r \
         join hosts rh on rh.id = r.host_id \
         join hosts h on h.zone_id = r.zone_id and h.id = any($2) and h.id <> r.host_id \
         where r.workspace_id = $1 and r.revoked_at is null and r.host_id = any($2) \
           and not exists (select 1 from relays o where o.zone_id = r.zone_id \
             and o.revoked_at is null and o.id <> r.id) \
         group by r.id, r.name, rh.inventory_name order by r.name",
    )
    .bind(workspace)
    .bind(targets)
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .map(|(relay, host, deps)| relay_dependency_warning(relay, host, deps))
        .collect())
}

// ---- live state (T020) ----------------------------------------------------------------------

/// A relay link's state as the supervisor sees it (data-model.md "State transitions").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayState {
    /// Dialling, or linked but no `Pong` yet.
    Connecting,
    /// Linked, and the last `Ping` was answered in time.
    Connected,
    /// The dial failed, the link dropped, or a `Pong` is late. Keel keeps redialling.
    Unreachable,
    /// Out of service after a protocol violation until an owner releases it (T059).
    Quarantined,
}

impl RelayState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Unreachable => "unreachable",
            Self::Quarantined => "quarantined",
        }
    }
}

/// One relay's live state: in memory only, meaningless after a restart (data-model.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayLive {
    pub state: RelayState,
    /// The last measured `Ping` → `Pong` round trip.
    pub rtt_ms: Option<u64>,
    /// The relay's version, from `Hello` / `Pong` (as it claims; capped).
    pub version: Option<String>,
    /// Runner streams Keel is carrying for this relay now (Keel's own count).
    pub streams: u32,
    /// The stream count the relay last claimed in `Pong` (advisory).
    pub reported_streams: Option<u32>,
    /// Why the relay is not connected (or the last thing that went wrong).
    pub last_error: Option<String>,
    /// When `state` last changed.
    pub since: OffsetDateTime,
}

impl RelayLive {
    fn new(state: RelayState) -> Self {
        Self {
            state,
            rtt_ms: None,
            version: None,
            streams: 0,
            reported_streams: None,
            last_error: None,
            since: OffsetDateTime::now_utc(),
        }
    }
}

/// A relay's state changed (`live`), or it is no longer supervised (`None`: revoked).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayChange {
    pub relay: Uuid,
    pub workspace: Uuid,
    pub live: Option<RelayLive>,
}

struct LiveInner {
    map: RwLock<HashMap<Uuid, (Uuid, RelayLive)>>,
    changes: broadcast::Sender<RelayChange>,
    resync: Notify,
    /// The supervisor keeping this state, so code holding only [`AppState`] can stop a link,
    /// note a refusal or rate-limit an attempt. Weak: the supervisor owns the app state.
    supervisor: RwLock<Weak<Supervisor>>,
    /// When each relay's change was last broadcast, to throttle detail-only changes.
    sent: Mutex<HashMap<Uuid, Instant>>,
}

/// Every supervised relay's [`RelayLive`], shared through [`AppState::relay_live`]. Readers
/// (`ListRelays`, a host's reach) take snapshots; the WebSocket subscribes to
/// [`RelayChange`]s, sent on every state change, and on an RTT, version or stream-count change
/// at most every [`DETAIL_PUSH_INTERVAL`].
#[derive(Clone)]
pub struct LiveRelays {
    inner: Arc<LiveInner>,
}

impl Default for LiveRelays {
    fn default() -> Self {
        let (changes, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(LiveInner {
                map: RwLock::new(HashMap::new()),
                changes,
                resync: Notify::new(),
                supervisor: RwLock::new(Weak::new()),
                sent: Mutex::new(HashMap::new()),
            }),
        }
    }
}

impl LiveRelays {
    /// One relay's live state, if it is supervised.
    #[must_use]
    pub fn get(&self, relay: Uuid) -> Option<RelayLive> {
        self.inner
            .map
            .read()
            .ok()
            .and_then(|m| m.get(&relay).map(|(_, l)| l.clone()))
    }

    /// Every supervised relay of `workspace` (all of them for `None`).
    #[must_use]
    pub fn snapshot(&self, workspace: Option<Uuid>) -> Vec<(Uuid, RelayLive)> {
        self.inner
            .map
            .read()
            .map(|m| {
                m.iter()
                    .filter(|(_, (ws, _))| workspace.is_none_or(|w| w == *ws))
                    .map(|(id, (_, l))| (*id, l.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// State changes from now on.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<RelayChange> {
        self.inner.changes.subscribe()
    }

    /// Asks the supervisor to re-read the relay rows now (after a register, revoke, release or
    /// edit is committed) instead of at its next health tick.
    pub fn resync(&self) {
        self.inner.resync.notify_one();
    }

    async fn resynced(&self) {
        self.inner.resync.notified().await;
    }

    fn supervisor(&self) -> Option<Arc<Supervisor>> {
        self.inner.supervisor.read().ok().and_then(|w| w.upgrade())
    }

    /// Stops one relay's link now, if it is supervised: sends `Close { reason }` and drops the
    /// link and every runner stream it carries ([`RelaySupervisor::stop`]).
    pub fn stop(&self, relay: Uuid, reason: &str) {
        if let Some(sup) = self.supervisor() {
            sup.stop(relay, reason);
        }
    }

    /// Notes a refusal on `path`'s relay: coalesced into one `relay.refused` row per relay and
    /// reason per window, and logged only once per window (T061). A direct path is ignored.
    pub fn refused(&self, path: &RunnerPath, reason: &'static str, detail: &str) {
        let Some(relay) = path.relay() else {
            return;
        };
        let workspace = self
            .inner
            .map
            .read()
            .ok()
            .and_then(|m| m.get(&relay).map(|(ws, _)| *ws));
        if let (Some(sup), Some(workspace)) = (self.supervisor(), workspace) {
            sup.shared.refusals.note(relay, workspace, reason, detail);
        }
    }

    /// Whether one more `attempt` may come through `path` (T062): a token bucket per relay. A
    /// refused attempt is noted as `relay.refused` with [`RelayAttempt::reason`]. A direct path
    /// is always allowed: the limit exists because one relay concentrates many peers behind one
    /// link. Joins are not counted here: [`crate::join::JoinLimiter`] already keys a relayed
    /// join on the relay alone (`relay:<id>`), so it owns them.
    #[must_use]
    pub fn attempt(&self, path: &RunnerPath, attempt: RelayAttempt) -> bool {
        let Some(relay) = path.relay() else {
            return true;
        };
        let Some(sup) = self.supervisor() else {
            return true;
        };
        let opts = &sup.shared.opts;
        let allowed = sup.shared.attempts.lock().is_ok_and(|mut buckets| {
            buckets
                .entry(relay)
                .or_insert_with(|| Bucket::per_minute(opts.enroll_per_minute, opts.enroll_burst))
                .take_at(Instant::now())
        });
        if !allowed {
            self.refused(path, attempt.reason(), attempt.as_str());
        }
        allowed
    }

    /// Applies `f` to a relay's state (creating it as `connecting`); a state change resets
    /// `since` and is broadcast. Any other change (RTT, version, streams, error) is broadcast
    /// only if nothing was for this relay in the last [`DETAIL_PUSH_INTERVAL`].
    fn update(&self, relay: Uuid, workspace: Uuid, f: impl FnOnce(&mut RelayLive)) {
        let changed = {
            let Ok(mut m) = self.inner.map.write() else {
                return;
            };
            let entry = m
                .entry(relay)
                .or_insert_with(|| (workspace, RelayLive::new(RelayState::Connecting)));
            let before = entry.1.clone();
            f(&mut entry.1);
            let now = Instant::now();
            let Ok(mut sent) = self.inner.sent.lock() else {
                return;
            };
            let due = if entry.1.state != before.state {
                entry.1.since = OffsetDateTime::now_utc();
                true
            } else {
                entry.1 != before
                    && sent
                        .get(&relay)
                        .is_none_or(|t| now.duration_since(*t) >= DETAIL_PUSH_INTERVAL)
            };
            due.then(|| {
                sent.insert(relay, now);
                entry.1.clone()
            })
        };
        if let Some(after) = changed {
            let _ = self.inner.changes.send(RelayChange {
                relay,
                workspace,
                live: Some(after),
            });
        }
    }

    fn remove(&self, relay: Uuid) {
        let removed = self
            .inner
            .map
            .write()
            .ok()
            .and_then(|mut m| m.remove(&relay));
        if let Ok(mut sent) = self.inner.sent.lock() {
            sent.remove(&relay);
        }
        if let Some((workspace, _)) = removed {
            let _ = self.inner.changes.send(RelayChange {
                relay,
                workspace,
                live: None,
            });
        }
    }

    fn ids(&self) -> Vec<Uuid> {
        self.inner
            .map
            .read()
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default()
    }
}

/// An attempt rate-limited per relay ([`LiveRelays::attempt`], T062).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayAttempt {
    /// A runner enrolling (`keel-enroll-v1`).
    Enroll,
}

impl RelayAttempt {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Enroll => "enroll",
        }
    }

    /// The `relay.refused` reason when the rate is exceeded.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Enroll => "enroll_rate",
        }
    }
}

// ---- the supervisor (T020, T021) ------------------------------------------------------------

/// Tunables of [`RelaySupervisor`]. [`Self::from_config`] gives the production values.
#[derive(Debug, Clone)]
pub struct SupervisorOptions {
    /// `Ping` period, redial backoff cap, dial/login timeout, and how often relay rows are
    /// re-read.
    pub health_interval: Duration,
    /// The relay's `Hello` must arrive within this after the control channel opens.
    pub hello_timeout: Duration,
    /// Concurrent carried runner streams per relay (Keel's own cap, whatever the relay says).
    pub max_streams: usize,
    /// New streams per second per relay (token bucket rate) …
    pub stream_rate: u32,
    /// … and its burst.
    pub stream_burst: u32,
    /// Unanswered `Ping`s after which the link is dropped.
    pub max_unanswered_pings: usize,
    /// `relay.refused` rows are coalesced per relay and reason over this window.
    pub refusal_window: Duration,
    /// From a carried stream's open to the runner's SSH authentication and subsystem.
    pub auth_deadline: Duration,
    /// The SSH channel window granted on a relay link.
    pub window_size: u32,
    /// The largest SSH packet accepted on a relay link.
    pub maximum_packet_size: u32,
    /// A relay link with no traffic for this long is dropped.
    pub inactivity_timeout: Duration,
    /// Enrollment (and join) attempts per minute per relay …
    pub enroll_per_minute: u32,
    /// … and their burst.
    pub enroll_burst: u32,
}

impl SupervisorOptions {
    #[must_use]
    pub fn from_config(cfg: &crate::config::Config) -> Self {
        let l = &cfg.relay_limits;
        Self {
            health_interval: cfg.relay_health_interval,
            max_streams: usize::try_from(l.max_streams).unwrap_or(usize::MAX),
            stream_rate: l.stream_rate,
            stream_burst: l.stream_burst,
            max_unanswered_pings: usize::try_from(l.max_unanswered_pings).unwrap_or(usize::MAX),
            auth_deadline: l.auth_deadline,
            window_size: l.window_size,
            maximum_packet_size: l.maximum_packet_size,
            inactivity_timeout: l.inactivity_timeout,
            enroll_per_minute: l.enroll_per_minute,
            enroll_burst: l.enroll_burst,
            ..Self::default()
        }
    }

    /// The SSH client config for one dial: [`link_client_config`] with these limits.
    #[must_use]
    pub fn client_config(&self) -> russh::client::Config {
        russh::client::Config {
            window_size: self.window_size,
            maximum_packet_size: self.maximum_packet_size,
            inactivity_timeout: Some(self.inactivity_timeout),
            ..link_client_config()
        }
    }
}

impl Default for SupervisorOptions {
    fn default() -> Self {
        let limits = crate::config::RelayLimits::default();
        Self {
            health_interval: Duration::from_secs(30),
            hello_timeout: relay::HELLO_TIMEOUT,
            max_streams: usize::try_from(limits.max_streams).unwrap_or(usize::MAX),
            stream_rate: limits.stream_rate,
            stream_burst: limits.stream_burst,
            max_unanswered_pings: usize::try_from(limits.max_unanswered_pings)
                .unwrap_or(usize::MAX),
            refusal_window: Duration::from_secs(60),
            auth_deadline: limits.auth_deadline,
            window_size: limits.window_size,
            maximum_packet_size: limits.maximum_packet_size,
            inactivity_timeout: limits.inactivity_timeout,
            enroll_per_minute: limits.enroll_per_minute,
            enroll_burst: limits.enroll_burst,
        }
    }
}

/// The first redial delay; it doubles up to the health interval.
const FIRST_BACKOFF: Duration = Duration::from_millis(500);
/// The longest relay-claimed string kept (version, advisory peer, detail).
const MAX_CLAIMED: usize = 128;

fn capped(s: &str) -> String {
    s.chars().take(MAX_CLAIMED).collect()
}

/// A `relays` row the supervisor dials.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct RelayRow {
    id: Uuid,
    workspace_id: Uuid,
    name: String,
    dial_address: String,
    host_key: String,
    quarantine_reason: Option<String>,
}

/// Coalesces `relay.refused` per `(relay, reason)` over a window into one audit row with a
/// `count` (T061), and logs only the first of each window.
struct Refusals {
    window: Duration,
    pending: Mutex<HashMap<(Uuid, &'static str), PendingRefusal>>,
}

struct PendingRefusal {
    workspace: Uuid,
    count: u64,
    detail: String,
    since: Instant,
}

impl Refusals {
    fn note(&self, relay: Uuid, workspace: Uuid, reason: &'static str, detail: &str) {
        let Ok(mut pending) = self.pending.lock() else {
            return;
        };
        let entry = pending.entry((relay, reason)).or_insert_with(|| {
            tracing::warn!(%relay, reason, detail = %detail.escape_debug(), "relay refused");
            PendingRefusal {
                workspace,
                count: 0,
                detail: capped(detail),
                since: Instant::now(),
            }
        });
        entry.count = entry.count.saturating_add(1);
    }

    /// Writes the windows that have closed (all of them with `force`).
    async fn flush(&self, db: &PgPool, force: bool) {
        let due: Vec<((Uuid, &'static str), PendingRefusal)> = {
            let Ok(mut pending) = self.pending.lock() else {
                return;
            };
            let keys: Vec<_> = pending
                .iter()
                .filter(|(_, p)| force || p.since.elapsed() >= self.window)
                .map(|(k, _)| *k)
                .collect();
            keys.into_iter()
                .filter_map(|k| pending.remove(&k).map(|p| (k, p)))
                .collect()
        };
        for ((relay, reason), p) in due {
            let written = async {
                let mut tx = db.begin().await?;
                audit::record(
                    &mut tx,
                    Entry {
                        actor: &Actor::System,
                        action: ACTION_REFUSED,
                        workspace: Some(WorkspaceId(p.workspace)),
                        target: json!({ "relay": relay }),
                        decision: Decision::Denied,
                        reason: Some(reason),
                        context: json!({ "count": p.count, "detail": p.detail }),
                    },
                )
                .await?;
                tx.commit().await
            }
            .await;
            if let Err(e) = written {
                tracing::error!(%relay, reason, error = %e, "auditing a relay refusal failed");
            }
        }
    }
}

/// A token bucket: `rate` per second, up to `burst`.
struct Bucket {
    tokens: f64,
    rate: f64,
    burst: f64,
    last: Instant,
}

impl Bucket {
    fn new(rate: u32, burst: u32) -> Self {
        Self {
            tokens: f64::from(burst),
            rate: f64::from(rate),
            burst: f64::from(burst),
            last: Instant::now(),
        }
    }

    fn per_minute(rate: u32, burst: u32) -> Self {
        Self {
            rate: f64::from(rate) / 60.0,
            ..Self::new(rate, burst)
        }
    }

    fn take_at(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = elapsed.mul_add(self.rate, self.tokens).min(self.burst);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// What every link task shares.
struct Shared {
    app: AppState,
    host: RunnerHost,
    opts: SupervisorOptions,
    refusals: Refusals,
    /// Enrollment and join attempts per relay ([`LiveRelays::attempt`]).
    attempts: Mutex<HashMap<Uuid, Bucket>>,
}

struct Link {
    row: RelayRow,
    stop: watch::Sender<Option<String>>,
    task: JoinHandle<()>,
}

struct Supervisor {
    shared: Arc<Shared>,
    links: Mutex<HashMap<Uuid, Link>>,
    /// Pushes relay changes and host reach to the WebSocket ([`publish`], T031).
    publisher: JoinHandle<()>,
}

impl Supervisor {
    fn stop(&self, relay: Uuid, reason: &str) {
        if let Ok(mut links) = self.links.lock()
            && let Some(link) = links.remove(&relay)
        {
            let _ = link.stop.send(Some(reason.to_owned()));
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.publisher.abort();
        if let Ok(links) = self.links.lock() {
            for link in links.values() {
                link.task.abort();
            }
        }
    }
}

/// Dials every relay that is neither revoked nor quarantined and keeps one SSH link to each
/// (T020): pinned host key, Keel's relay-link key, the `keel-relay-v1` control channel, a `Ping`
/// every health interval, redial with backoff. Each `forwarded-tcpip` channel the relay opens is
/// served as a runner connection with [`RunnerPath::Relay`] (T021). Live state goes to
/// [`AppState::relay_live`].
///
/// Keel never trusts the relay: a protocol violation quarantines it (T059); streams before the
/// control channel is up, over `max_streams` or over the rate are refused and audited
/// (coalesced) without quarantine (T060, T061); a stream is served only if the relay row is
/// still usable when it starts.
#[derive(Clone)]
pub struct RelaySupervisor {
    inner: Arc<Supervisor>,
}

impl RelaySupervisor {
    /// A supervisor with no links yet; [`Self::sync`] starts them.
    #[must_use]
    pub fn new(app: AppState, host: RunnerHost, opts: SupervisorOptions) -> Self {
        let refusals = Refusals {
            window: opts.refusal_window,
            pending: Mutex::new(HashMap::new()),
        };
        let live = app.relay_live.clone();
        let publisher = tokio::spawn(publish(app.clone(), live.subscribe()));
        let inner = Arc::new(Supervisor {
            shared: Arc::new(Shared {
                app,
                host,
                opts,
                refusals,
                attempts: Mutex::new(HashMap::new()),
            }),
            links: Mutex::new(HashMap::new()),
            publisher,
        });
        if let Ok(mut sup) = live.inner.supervisor.write() {
            *sup = Arc::downgrade(&inner);
        }
        Self { inner }
    }

    /// Starts a supervisor: re-reads the relay rows now, then every health interval and on
    /// [`LiveRelays::resync`], and flushes coalesced refusals. Aborting the task stops the
    /// re-reads; [`Self::shutdown`] closes the links.
    #[must_use]
    pub fn spawn(
        app: AppState,
        host: RunnerHost,
        opts: SupervisorOptions,
    ) -> (Self, JoinHandle<()>) {
        let sup = Self::new(app, host, opts);
        let looped = sup.clone();
        let task = tokio::spawn(async move {
            loop {
                if let Err(e) = looped.sync().await {
                    tracing::error!(error = %e, "reading the relays failed");
                }
                looped.flush_refusals(false).await;
                let live = looped.live().clone();
                tokio::select! {
                    () = tokio::time::sleep(looped.inner.shared.opts.health_interval) => {}
                    () = live.resynced() => {}
                }
            }
        });
        (sup, task)
    }

    /// The live state this supervisor keeps (the same as [`AppState::relay_live`]).
    #[must_use]
    pub fn live(&self) -> &LiveRelays {
        &self.inner.shared.app.relay_live
    }

    /// Re-reads the relay rows: starts a link for each new one, restarts one whose address or
    /// key changed (or whose link task ended), and stops (closing its streams) one that is
    /// revoked or quarantined.
    ///
    /// # Errors
    ///
    /// The database.
    pub async fn sync(&self) -> Result<(), sqlx::Error> {
        let rows: Vec<RelayRow> = sqlx::query_as(
            "select id, workspace_id, name, dial_address, host_key, quarantine_reason \
             from relays where revoked_at is null order by id",
        )
        .fetch_all(&self.inner.shared.app.db)
        .await?;
        let live = self.live();
        let Ok(mut links) = self.inner.links.lock() else {
            return Ok(());
        };
        let mut seen = HashSet::new();
        for row in rows {
            seen.insert(row.id);
            if let Some(reason) = &row.quarantine_reason {
                if let Some(link) = links.remove(&row.id) {
                    let _ = link.stop.send(Some("quarantined".into()));
                }
                let reason = reason.clone();
                live.update(row.id, row.workspace_id, |l| {
                    if l.state != RelayState::Quarantined {
                        l.last_error = Some(reason);
                    }
                    l.state = RelayState::Quarantined;
                    l.rtt_ms = None;
                    l.streams = 0;
                });
                continue;
            }
            let current = links.get(&row.id).is_some_and(|l| {
                l.row.dial_address == row.dial_address
                    && l.row.host_key == row.host_key
                    && !l.task.is_finished()
            });
            if current {
                continue;
            }
            if let Some(old) = links.remove(&row.id) {
                let _ = old.stop.send(Some("relay changed".into()));
            }
            let (stop, stopped) = watch::channel(None);
            let run = LinkRun {
                shared: Arc::clone(&self.inner.shared),
                row: row.clone(),
                last_audited: None,
            };
            let task = tokio::spawn(run.run(stopped));
            links.insert(row.id, Link { row, stop, task });
        }
        let gone: Vec<Uuid> = links
            .keys()
            .filter(|id| !seen.contains(id))
            .copied()
            .collect();
        for id in gone {
            if let Some(link) = links.remove(&id) {
                let _ = link.stop.send(Some("revoked".into()));
            }
        }
        for id in live.ids() {
            if !seen.contains(&id) {
                live.remove(id);
            }
        }
        Ok(())
    }

    /// Stops one relay's link: sends `Close { reason }`, drops the link and every runner
    /// stream it carries. The next [`Self::sync`] redials it unless its row says otherwise.
    pub fn stop(&self, relay: Uuid, reason: &str) {
        self.inner.stop(relay, reason);
    }

    /// Stops every link (Keel is shutting down): each sends `Close { "shutdown" }`.
    pub fn shutdown(&self) {
        if let Ok(mut links) = self.inner.links.lock() {
            for (_, link) in links.drain() {
                let _ = link.stop.send(Some("shutdown".into()));
            }
        }
    }

    /// Writes coalesced `relay.refused` rows whose window closed (every one with `force`).
    pub async fn flush_refusals(&self, force: bool) {
        let shared = &self.inner.shared;
        shared.refusals.flush(&shared.app.db, force).await;
    }
}

/// How one link ended.
#[derive(Debug)]
enum LinkEnd {
    /// Asked to stop; `Close` was sent.
    Stopped,
    /// Something an honest relay never does: quarantine.
    Violation(LinkViolation),
    /// The dial failed or the link dropped; `up` if it had been connected.
    Lost { error: String, up: bool },
}

fn lost(error: impl std::fmt::Display) -> LinkEnd {
    LinkEnd::Lost {
        error: error.to_string(),
        up: false,
    }
}

fn violation_detail(v: &LinkViolation) -> String {
    match v {
        LinkViolation::HostKey { fingerprint } => format!("presented {fingerprint}"),
        LinkViolation::Channel { kind, detail } | LinkViolation::Request { kind, detail } => {
            format!("{kind} {detail}")
        }
        LinkViolation::SecondSession => "second session channel".into(),
        LinkViolation::Frame { detail } => detail.clone(),
    }
}

/// The frame tag, for an out-of-order report (never the frame's contents).
const fn frame_tag(f: &Frame) -> &'static str {
    match f {
        Frame::Hello { .. } => "Hello",
        Frame::Ping { .. } => "Ping",
        Frame::Pong { .. } => "Pong",
        Frame::CacheGet { .. } => "CacheGet",
        Frame::CacheChunk { .. } => "CacheChunk",
        Frame::CacheMiss { .. } => "CacheMiss",
        Frame::Close { .. } => "Close",
    }
}

type FrameRead = Result<Option<Frame>, CodecError>;

/// What a control-channel read gave, mapped to a link end when it is not a frame.
fn control_end(read: Option<FrameRead>) -> Result<Frame, LinkEnd> {
    match read {
        Some(Ok(Some(frame))) => Ok(frame),
        Some(Err(e)) => Err(LinkViolation::frame(&e).map_or_else(|| lost(&e), LinkEnd::Violation)),
        Some(Ok(None)) | None => Err(lost("the control channel closed")),
    }
}

/// Whether a relay may still carry a stream: not revoked, not quarantined (T059: a stream that
/// races a revoke or quarantine is refused).
async fn relay_usable(db: &PgPool, relay: Uuid) -> bool {
    sqlx::query_as::<_, (bool,)>(
        "select revoked_at is null and quarantined_at is null from relays where id = $1",
    )
    .bind(relay)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .is_some_and(|(ok,)| ok)
}

/// The advisory peer as [`RunnerPath`] keeps it: `addr:port`, `[v6]:port`. Display only.
fn advisory_peer(addr: &str, port: u32) -> String {
    let addr = capped(addr);
    if addr.contains(':') {
        format!("[{addr}]:{port}")
    } else {
        format!("{addr}:{port}")
    }
}

type ControlWriter = WriteHalf<ChannelStream<russh::client::Msg>>;

/// Opens the control channel: one session channel carrying [`RELAY_SUBSYSTEM`].
async fn open_control(
    handle: &Handle<LinkClientPolicy>,
) -> Result<ChannelStream<russh::client::Msg>, String> {
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| format!("control channel: {e}"))?;
    channel
        .request_subsystem(true, RELAY_SUBSYSTEM)
        .await
        .map_err(|e| format!("control channel: {e}"))?;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(channel.into_stream()),
            Some(ChannelMsg::Failure) => {
                let _ = channel.close().await;
                return Err("the relay refused the control subsystem".into());
            }
            Some(_) => {}
            None => return Err("the control channel closed".into()),
        }
    }
}

/// Refuses a runner channel: closes it (dropping a channel does not).
async fn close_channel(ch: RunnerChannel) {
    let _ = ch.channel.close().await;
}

/// What a link's receivers are.
struct LinkIo {
    violations: mpsc::UnboundedReceiver<LinkViolation>,
    runners: mpsc::UnboundedReceiver<RunnerChannel>,
}

/// One relay's link task.
struct LinkRun {
    shared: Arc<Shared>,
    row: RelayRow,
    /// The last state written as `relay.state`, so a relay down for days is one row, not one
    /// per redial.
    last_audited: Option<RelayState>,
}

impl LinkRun {
    fn live(&self) -> &LiveRelays {
        &self.shared.app.relay_live
    }

    fn update(&self, f: impl FnOnce(&mut RelayLive)) {
        self.live().update(self.row.id, self.row.workspace_id, f);
    }

    fn refuse(&self, reason: &'static str, detail: &str) {
        self.shared
            .refusals
            .note(self.row.id, self.row.workspace_id, reason, detail);
    }

    /// Sets the state (and error), auditing `connected` / `unreachable` when it changed.
    async fn set_state(&mut self, state: RelayState, error: Option<String>) {
        self.update(|l| {
            l.state = state;
            if state == RelayState::Connected {
                l.last_error = None;
            } else if let Some(e) = &error {
                l.last_error = Some(e.clone());
            }
        });
        let audited = matches!(state, RelayState::Connected | RelayState::Unreachable);
        if !audited || self.last_audited == Some(state) {
            return;
        }
        self.last_audited = Some(state);
        tracing::info!(relay = %self.row.id, name = %self.row.name, state = state.as_str(),
            error = error.as_deref().unwrap_or(""), "relay link");
        let written = async {
            let mut tx = self.shared.app.db.begin().await?;
            audit::record(
                &mut tx,
                Entry {
                    actor: &Actor::System,
                    action: ACTION_STATE,
                    workspace: Some(WorkspaceId(self.row.workspace_id)),
                    target: json!({ "relay": self.row.id }),
                    decision: Decision::Allowed,
                    reason: None,
                    context: json!({ "state": state.as_str(), "error": error }),
                },
            )
            .await?;
            tx.commit().await
        }
        .await;
        if let Err(e) = written {
            tracing::error!(relay = %self.row.id, error = %e, "auditing a relay state failed");
        }
    }

    /// Takes the relay out of service: sets `quarantined_at` and the reason and audits
    /// `relay.quarantine` in one transaction, unless it is already quarantined or revoked.
    async fn quarantine(&self, v: &LinkViolation) {
        let reason = v.reason();
        let detail = capped(&violation_detail(v));
        let shared = &self.shared;
        let written = async {
            let mut tx = shared.app.db.begin().await?;
            let done = sqlx::query(
                "update relays set quarantined_at = now(), quarantine_reason = $2, \
                 updated_at = now() where id = $1 and quarantined_at is null \
                 and revoked_at is null",
            )
            .bind(self.row.id)
            .bind(reason)
            .execute(&mut *tx)
            .await?;
            if done.rows_affected() == 1 {
                audit::record(
                    &mut tx,
                    Entry {
                        actor: &Actor::System,
                        action: ACTION_QUARANTINE,
                        workspace: Some(WorkspaceId(self.row.workspace_id)),
                        target: json!({ "relay": self.row.id }),
                        decision: Decision::Denied,
                        reason: Some(reason),
                        context: json!({ "detail": detail }),
                    },
                )
                .await?;
            }
            tx.commit().await
        }
        .await;
        if let Err(e) = written {
            tracing::error!(relay = %self.row.id, error = %e, "quarantining a relay failed");
        }
        tracing::warn!(relay = %self.row.id, name = %self.row.name, reason,
            detail = %detail.escape_debug(), "relay quarantined");
        self.update(|l| {
            l.state = RelayState::Quarantined;
            l.last_error = Some(format!("{reason}: {detail}"));
            l.rtt_ms = None;
            l.streams = 0;
        });
        shared.refusals.flush(&shared.app.db, true).await;
    }

    async fn run(mut self, mut stop: watch::Receiver<Option<String>>) {
        let cap = self.shared.opts.health_interval;
        let mut backoff = FIRST_BACKOFF.min(cap);
        loop {
            if stop.borrow().is_some() {
                return;
            }
            // A relay that stays down stays `unreachable` across redials.
            let down = self
                .live()
                .get(self.row.id)
                .is_some_and(|l| l.state == RelayState::Unreachable);
            if !down {
                self.set_state(RelayState::Connecting, None).await;
            }
            match self.dial(&mut stop).await {
                LinkEnd::Stopped => return,
                LinkEnd::Violation(v) => {
                    self.quarantine(&v).await;
                    return;
                }
                LinkEnd::Lost { error, up } => {
                    self.update(|l| {
                        l.streams = 0;
                        l.rtt_ms = None;
                    });
                    self.set_state(RelayState::Unreachable, Some(error)).await;
                    if up {
                        backoff = FIRST_BACKOFF.min(cap);
                    }
                }
            }
            tokio::select! {
                () = tokio::time::sleep(backoff) => {}
                changed = stop.changed() => {
                    if changed.is_err() || stop.borrow().is_some() {
                        return;
                    }
                }
            }
            backoff = (backoff * 2).min(cap);
        }
    }

    /// One dial: TCP, SSH with the pinned key, login, the control channel, then
    /// [`Self::serve`]. The link is disconnected when it returns.
    async fn dial(&mut self, stop: &mut watch::Receiver<Option<String>>) -> LinkEnd {
        let wait = self.shared.opts.health_interval;
        let Some(key) = self.shared.app.relay_link.get() else {
            return lost("no relay-link key is loaded");
        };
        let (pinned, fingerprint) = match parse_host_key(&self.row.host_key) {
            Ok(k) => k,
            Err(why) => return lost(format!("stored host key: {why}")),
        };
        let address = self.row.dial_address.clone();
        let tcp = match tokio::time::timeout(wait, TcpStream::connect(&address)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return lost(format!("dial {address}: {e}")),
            Err(_) => return lost(format!("dial {address}: timed out")),
        };
        let (violation_tx, mut violations) = mpsc::unbounded_channel();
        let sink: ViolationSink = Arc::new(move |v| {
            let _ = violation_tx.send(v);
        });
        let (runners_tx, runners) = mpsc::unbounded_channel();
        let policy = LinkClientPolicy::new(pinned, sink, runners_tx);
        let connected = tokio::time::timeout(
            wait,
            russh::client::connect_stream(Arc::new(self.shared.opts.client_config()), tcp, policy),
        )
        .await;
        let mut handle = match connected {
            Ok(Ok(h)) => h,
            // A refused host key is reported through the sink before the handshake fails.
            Ok(Err(e)) => {
                return violations
                    .try_recv()
                    .map_or_else(|_| lost(format!("handshake: {e}")), LinkEnd::Violation);
            }
            Err(_) => return lost("handshake timed out"),
        };
        let auth = tokio::time::timeout(
            wait,
            handle.authenticate_publickey(LINK_USER, PrivateKeyWithHashAlg::new(key.key(), None)),
        )
        .await;
        let end = match auth {
            Ok(Ok(AuthResult::Success)) => {
                match tokio::time::timeout(wait, open_control(&handle)).await {
                    Ok(Ok(control)) => {
                        let io = LinkIo {
                            violations,
                            runners,
                        };
                        self.serve(control, io, &fingerprint, stop).await
                    }
                    Ok(Err(e)) => lost(e),
                    Err(_) => lost("opening the control channel timed out"),
                }
            }
            Ok(Ok(_)) => lost("the relay refused Keel's relay-link key (is it in hub_keys?)"),
            Ok(Err(e)) => lost(format!("login: {e}")),
            Err(_) => lost("login timed out"),
        };
        let why = match &end {
            LinkEnd::Stopped => "stopped",
            LinkEnd::Violation(_) => "protocol violation",
            LinkEnd::Lost { .. } => "link lost",
        };
        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, why, "")
            .await;
        end
    }

    /// Runs an open control channel: waits for `Hello`, then pings, answers `CacheGet` and
    /// carries runner streams until the link ends.
    async fn serve(
        &mut self,
        control: ChannelStream<russh::client::Msg>,
        mut io: LinkIo,
        fingerprint: &str,
        stop: &mut watch::Receiver<Option<String>>,
    ) -> LinkEnd {
        let opts = self.shared.opts.clone();
        let (mut rd, mut wr) = tokio::io::split(control);
        // `read_frame` is not cancel-safe: one task reads, the loop selects on its channel.
        let (frames_tx, mut frames) = mpsc::channel::<FrameRead>(8);
        let mut reader = JoinSet::new();
        reader.spawn(async move {
            loop {
                let read = relay::read_frame(&mut rd).await;
                let last = !matches!(read, Ok(Some(_)));
                if frames_tx.send(read).await.is_err() || last {
                    return;
                }
            }
        });

        // Hello first: until it arrives, no stream is carried.
        let deadline = tokio::time::sleep(opts.hello_timeout);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                () = &mut deadline => {
                    return lost(format!("no Hello within {:?}", opts.hello_timeout));
                }
                read = frames.recv() => match control_end(read) {
                    Ok(Frame::Hello { version, relay_key_fingerprint, .. }) => {
                        if relay_key_fingerprint != fingerprint {
                            return LinkEnd::Violation(LinkViolation::Frame {
                                detail: format!(
                                    "Hello names host key {}",
                                    capped(&relay_key_fingerprint)
                                ),
                            });
                        }
                        let version = capped(&version);
                        self.update(|l| l.version = Some(version));
                        break;
                    }
                    Ok(other) => {
                        return LinkEnd::Violation(LinkViolation::out_of_order(format!(
                            "{} before Hello",
                            frame_tag(&other)
                        )));
                    }
                    Err(end) => return end,
                },
                ch = io.runners.recv() => {
                    let Some(ch) = ch else { return lost("the link closed") };
                    let peer = advisory_peer(&ch.originator_address, ch.originator_port);
                    self.refuse("stream_before_control", &peer);
                    close_channel(ch).await;
                }
                v = io.violations.recv() => {
                    return v.map_or_else(|| lost("the link closed"), LinkEnd::Violation);
                }
                changed = stop.changed() => {
                    if let Some(end) = self.stopping(changed.is_err(), stop, &mut wr).await {
                        return end;
                    }
                }
            }
        }

        let mut ticker = tokio::time::interval(opts.health_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut seq = 0_u64;
        let mut outstanding: VecDeque<(u64, Instant)> = VecDeque::new();
        let mut streams: JoinSet<()> = JoinSet::new();
        let mut bucket = Bucket::new(opts.stream_rate, opts.stream_burst);
        let mut up = false;
        let with_up = |end: LinkEnd, up: bool| match end {
            LinkEnd::Lost { error, .. } => LinkEnd::Lost { error, up },
            other => other,
        };
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if outstanding.len() >= opts.max_unanswered_pings {
                        return LinkEnd::Lost {
                            error: format!("{} Pings unanswered", outstanding.len()),
                            up,
                        };
                    }
                    if !outstanding.is_empty() {
                        self.set_state(
                            RelayState::Unreachable,
                            Some("no Pong within the health interval".into()),
                        )
                        .await;
                    }
                    seq += 1;
                    if let Err(e) = write(&mut wr, &Frame::Ping { seq }, opts.health_interval).await {
                        return LinkEnd::Lost { error: e, up };
                    }
                    outstanding.push_back((seq, Instant::now()));
                }
                read = frames.recv() => match control_end(read) {
                    Ok(Frame::Pong { seq: answered, version, streams: claimed, .. }) => {
                        let Some(at) = outstanding.iter().position(|(s, _)| *s == answered) else {
                            return LinkEnd::Violation(LinkViolation::out_of_order(format!(
                                "Pong {answered} answers no Ping"
                            )));
                        };
                        let sent = outstanding[at].1;
                        outstanding.drain(..=at);
                        let rtt = u64::try_from(sent.elapsed().as_millis()).unwrap_or(u64::MAX);
                        let version = capped(&version);
                        self.update(|l| {
                            l.rtt_ms = Some(rtt);
                            l.version = Some(version);
                            l.reported_streams = Some(claimed);
                        });
                        if outstanding.is_empty() {
                            self.set_state(RelayState::Connected, None).await;
                            up = true;
                        }
                    }
                    Ok(Frame::CacheGet { req, release, path }) => {
                        // Keel does not serve runner releases yet (spec 067 US5). Not a
                        // violation, but audited (coalesced), as the hostile matrix asks.
                        self.refuse("cache_get", &format!("{release}/{path}"));
                        let miss = Frame::CacheMiss { req, reason: CacheMissReason::Unavailable };
                        if let Err(e) = write(&mut wr, &miss, opts.health_interval).await {
                            return LinkEnd::Lost { error: e, up };
                        }
                    }
                    Ok(other) => {
                        return LinkEnd::Violation(LinkViolation::out_of_order(frame_tag(&other)));
                    }
                    Err(end) => return with_up(end, up),
                },
                ch = io.runners.recv() => {
                    let Some(ch) = ch else {
                        return LinkEnd::Lost { error: "the link closed".into(), up };
                    };
                    self.carry(ch, &mut streams, &mut bucket).await;
                    let n = u32::try_from(streams.len()).unwrap_or(u32::MAX);
                    self.update(|l| l.streams = n);
                }
                Some(_) = streams.join_next(), if !streams.is_empty() => {
                    let n = u32::try_from(streams.len()).unwrap_or(u32::MAX);
                    self.update(|l| l.streams = n);
                }
                v = io.violations.recv() => {
                    return v.map_or_else(
                        || LinkEnd::Lost { error: "the link closed".into(), up },
                        LinkEnd::Violation,
                    );
                }
                changed = stop.changed() => {
                    if let Some(end) = self.stopping(changed.is_err(), stop, &mut wr).await {
                        return end;
                    }
                }
            }
        }
    }

    /// A stop was signalled: sends `Close` and ends, or `None` if it was not a stop.
    async fn stopping(
        &self,
        closed: bool,
        stop: &watch::Receiver<Option<String>>,
        wr: &mut ControlWriter,
    ) -> Option<LinkEnd> {
        let reason = if closed {
            Some("shutdown".to_owned())
        } else {
            stop.borrow().clone()
        }?;
        let close = Frame::Close {
            reason: capped(&reason),
        };
        let _ = write(wr, &close, self.shared.opts.health_interval).await;
        Some(LinkEnd::Stopped)
    }

    /// Carries one runner channel if the limits allow and the relay is still usable.
    async fn carry(&self, ch: RunnerChannel, streams: &mut JoinSet<()>, bucket: &mut Bucket) {
        let peer = advisory_peer(&ch.originator_address, ch.originator_port);
        if streams.len() >= self.shared.opts.max_streams {
            self.refuse("max_streams", &peer);
            close_channel(ch).await;
            return;
        }
        if !bucket.take_at(Instant::now()) {
            self.refuse("stream_rate", &peer);
            close_channel(ch).await;
            return;
        }
        let shared = Arc::clone(&self.shared);
        let (relay, workspace) = (self.row.id, self.row.workspace_id);
        streams.spawn(async move {
            let config = if relay_usable(&shared.app.db, relay).await {
                shared.host.config(&shared.app)
            } else {
                shared
                    .refusals
                    .note(relay, workspace, "relay_unusable", &peer);
                None
            };
            let Some(config) = config else {
                close_channel(ch).await;
                return;
            };
            let path = RunnerPath::Relay {
                relay_id: relay,
                advisory_peer: Some(peer.clone()),
            };
            let stream = ch.channel.into_stream();
            let deadline = shared.opts.auth_deadline;
            // A runner that fails to authenticate is the runner's problem, not the relay's:
            // refused and audited, never a quarantine (T026).
            match serve_relayed(shared.app.clone(), config, stream, path, deadline).await {
                StreamEnd::Served => {}
                StreamEnd::Unauthenticated => {
                    shared
                        .refusals
                        .note(relay, workspace, "runner_unauthenticated", &peer);
                }
                StreamEnd::AuthDeadline => {
                    shared
                        .refusals
                        .note(relay, workspace, "auth_deadline", &peer);
                }
            }
        });
    }
}

/// Writes one control frame, bounded by `wait`.
async fn write(wr: &mut ControlWriter, frame: &Frame, wait: Duration) -> Result<(), String> {
    match tokio::time::timeout(wait, relay::write_frame(wr, frame)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("control channel: {e}")),
        Err(_) => Err("control channel: write timed out".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for ok in ["edge", "a", "dc-1", "0rack", &"a".repeat(63)] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "",
            "direct",
            "-a",
            "Edge",
            "a_b",
            "a.b",
            "é",
            &"a".repeat(64),
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn dial_addresses() {
        for ok in [
            "relay.example.com:2222",
            "10.0.0.5:22",
            "[2001:db8::1]:2222",
            "relay:1",
            "relay.example.com.:65535",
        ] {
            assert_eq!(check_dial_address(ok), Ok(()), "{ok}");
        }
        for bad in [
            "relay",
            "relay:",
            "relay:0",
            "relay:65536",
            "relay:+22",
            "relay: 22",
            "ssh://relay:22",
            "user@relay:22",
            "relay:22/x",
            "2001:db8::1:22",
            "[2001:db8::1]",
            "[not-v6]:22",
            "-relay:22",
            "re lay:22",
            ":22",
            "a..b:22",
        ] {
            assert!(check_dial_address(bad).is_err(), "{bad}");
        }
        assert!(check_dial_address(&format!("{}:22", "a.".repeat(150))).is_err());
    }

    #[test]
    fn host_keys() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let line = key.public_key().to_openssh().unwrap();
        let (data, fp) = parse_host_key(&format!("  {line}\n")).unwrap();
        assert_eq!(&data, key.public_key().key_data());
        assert_eq!(fp, keel_exec::ca::fingerprint(&data));
        assert!(parse_host_key("").is_err());
        assert!(parse_host_key(&format!("{line}\n{line}")).is_err());
        assert!(parse_host_key("ssh-rsa AAAAB3NzaC1yc2E relay").is_err());
        assert!(parse_host_key("ssh-ed25519 not-base64 relay").is_err());
    }

    #[test]
    fn the_bucket_allows_a_burst_then_the_rate() {
        let mut b = Bucket::new(20, 3);
        let t0 = b.last;
        assert!(b.take_at(t0) && b.take_at(t0) && b.take_at(t0));
        assert!(!b.take_at(t0), "burst spent");
        assert!(!b.take_at(t0 + Duration::from_millis(10)), "half a token");
        assert!(
            b.take_at(t0 + Duration::from_millis(60)),
            "one token back after 50 ms"
        );
        assert!(b.take_at(t0 + Duration::from_secs(10)));
        assert!(b.take_at(t0 + Duration::from_secs(10)));
        assert!(b.take_at(t0 + Duration::from_secs(10)));
        assert!(
            !b.take_at(t0 + Duration::from_secs(10)),
            "capped at the burst"
        );
    }

    #[test]
    fn the_attempt_bucket_counts_per_minute() {
        let mut b = Bucket::per_minute(30, 2);
        let t0 = b.last;
        assert!(b.take_at(t0) && b.take_at(t0));
        assert!(!b.take_at(t0), "burst spent");
        assert!(!b.take_at(t0 + Duration::from_secs(1)), "half a token");
        assert!(
            b.take_at(t0 + Duration::from_secs(2)),
            "30 a minute: one per 2 s"
        );
    }

    /// T061: ten thousand refusals of one kind are one pending row with `count = 10000`.
    #[test]
    fn refusals_coalesce_per_relay_and_reason() {
        let r = Refusals {
            window: Duration::from_secs(60),
            pending: Mutex::new(HashMap::new()),
        };
        let (relay, other, ws) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        for i in 0..10_000 {
            r.note(relay, ws, "stream_rate", &format!("10.0.0.{}:1", i % 250));
        }
        r.note(relay, ws, "max_streams", "x");
        r.note(other, ws, "stream_rate", "y");
        let pending = r.pending.lock().unwrap();
        assert_eq!(pending.len(), 3);
        let p = &pending[&(relay, "stream_rate")];
        assert_eq!(p.count, 10_000);
        assert_eq!(p.detail, "10.0.0.0:1", "the first detail is kept");
        assert_eq!(pending[&(relay, "max_streams")].count, 1);
    }

    #[test]
    fn options_follow_the_relay_limits() {
        let opts = SupervisorOptions::default();
        let limits = crate::config::RelayLimits::default();
        assert_eq!(opts.auth_deadline, Duration::from_secs(10));
        assert_eq!(opts.max_streams, 256);
        assert_eq!(opts.window_size, limits.window_size);
        let cfg = opts.client_config();
        assert_eq!(cfg.window_size, 256 << 10);
        assert_eq!(cfg.maximum_packet_size, 32 << 10);
        assert_eq!(cfg.inactivity_timeout, Some(Duration::from_secs(120)));
        assert_eq!(cfg.keepalive_interval, Some(relay::LINK_KEEPALIVE));
        assert!(cfg.channel_buffer_size <= 32);
        let tight = SupervisorOptions {
            window_size: 64 << 10,
            maximum_packet_size: 8 << 10,
            inactivity_timeout: Duration::from_secs(45),
            ..SupervisorOptions::default()
        }
        .client_config();
        assert_eq!(
            (
                tight.window_size,
                tight.maximum_packet_size,
                tight.inactivity_timeout
            ),
            (64 << 10, 8 << 10, Some(Duration::from_secs(45)))
        );
    }

    #[test]
    fn advisory_peers_bracket_v6() {
        assert_eq!(advisory_peer("10.0.0.5", 40122), "10.0.0.5:40122");
        assert_eq!(advisory_peer("2001:db8::1", 22), "[2001:db8::1]:22");
    }

    #[test]
    fn live_state_broadcasts_only_state_changes() {
        let live = LiveRelays::default();
        let mut rx = live.subscribe();
        let (relay, ws) = (Uuid::now_v7(), Uuid::now_v7());
        live.update(relay, ws, |l| l.state = RelayState::Connected);
        assert_eq!(
            rx.try_recv().unwrap().live.unwrap().state,
            RelayState::Connected
        );
        live.update(relay, ws, |l| l.rtt_ms = Some(3));
        assert!(rx.try_recv().is_err(), "an RTT update is not a change");
        assert_eq!(live.get(relay).unwrap().rtt_ms, Some(3));
        assert_eq!(live.snapshot(Some(ws)).len(), 1);
        assert!(live.snapshot(Some(Uuid::now_v7())).is_empty());
        live.remove(relay);
        assert_eq!(rx.try_recv().unwrap().live, None);
        assert!(live.get(relay).is_none());
    }

    #[test]
    fn detail_changes_push_at_most_every_interval() {
        let live = LiveRelays::default();
        let mut rx = live.subscribe();
        let (relay, ws) = (Uuid::now_v7(), Uuid::now_v7());
        live.update(relay, ws, |l| l.state = RelayState::Connected);
        assert!(rx.try_recv().is_ok());
        live.update(relay, ws, |l| l.streams = 2);
        assert!(rx.try_recv().is_err(), "within the interval");
        // Backdate the last push past the interval.
        let old = Instant::now()
            .checked_sub(DETAIL_PUSH_INTERVAL + Duration::from_secs(1))
            .unwrap();
        live.inner.sent.lock().unwrap().insert(relay, old);
        live.update(relay, ws, |l| l.streams = 3);
        assert_eq!(rx.try_recv().unwrap().live.unwrap().streams, 3);
        // No change at all is never pushed, however long ago the last push was.
        live.inner.sent.lock().unwrap().insert(relay, old);
        live.update(relay, ws, |_| {});
        assert!(rx.try_recv().is_err());
        // A state change always is.
        live.update(relay, ws, |l| l.state = RelayState::Unreachable);
        assert!(rx.try_recv().is_ok());
    }

    fn info(zone: Uuid, state: wire::RelayState) -> wire::RelayInfo {
        wire::RelayInfo {
            id: Uuid::now_v7(),
            workspace: Uuid::nil(),
            zone,
            name: wire::ZoneName::new("r").unwrap(),
            dial_address: "relay.dmz:7832".into(),
            host_key_fingerprint: "SHA256:x".into(),
            host: None,
            state,
            since: None,
            rtt_ms: None,
            version: None,
            streams: None,
            last_error: None,
            revoked_at: None,
        }
    }

    #[test]
    fn reach_follows_the_precedence_table() {
        use wire::{HostReach, RelayState as S};
        let zone = Uuid::now_v7();
        let other = Uuid::now_v7();
        let up = info(zone, S::Connected);
        let down = info(zone, S::Unreachable);
        let quarantined = info(zone, S::Quarantined);
        let dialling = info(zone, S::Connecting);
        let revoked = info(zone, S::Revoked);
        let elsewhere = info(other, S::Connected);

        // 1. Connected: how.
        let via = RunnerPath::Relay {
            relay_id: up.id,
            advisory_peer: None,
        };
        assert_eq!(
            derive_reach(Some(&via), Some(zone), None, &[]),
            HostReach::Connected {
                path: wire::RunnerPath::Relay { relay: up.id }
            }
        );
        assert_eq!(
            derive_reach(Some(&RunnerPath::Direct { peer: None }), None, None, &[]),
            HostReach::Connected {
                path: wire::RunnerPath::Direct
            }
        );
        // 2. Direct, or a zone with a connected relay: a path exists.
        assert_eq!(
            derive_reach(None, None, Some(revoked.id), std::slice::from_ref(&revoked)),
            HostReach::Disconnected
        );
        assert_eq!(
            derive_reach(
                None,
                Some(zone),
                Some(revoked.id),
                &[up.clone(), revoked.clone()]
            ),
            HostReach::Disconnected
        );
        // 3. The last session's relay is revoked.
        assert_eq!(
            derive_reach(
                None,
                Some(zone),
                Some(revoked.id),
                &[down.clone(), revoked.clone()]
            ),
            HostReach::RelayRevoked { relay: revoked.id }
        );
        // 4. No non-revoked relay in the zone (another zone's relay does not count).
        assert_eq!(
            derive_reach(None, Some(zone), None, &[revoked.clone(), elsewhere]),
            HostReach::NoPath
        );
        assert_eq!(derive_reach(None, Some(zone), None, &[]), HostReach::NoPath);
        // 5. A relay is still connecting.
        assert_eq!(
            derive_reach(None, Some(zone), None, &[down.clone(), dialling]),
            HostReach::Disconnected
        );
        // 6. Every relay is unreachable or quarantined.
        assert_eq!(
            derive_reach(None, Some(zone), None, &[down.clone(), quarantined.clone()]),
            HostReach::RelayUnreachable {
                relays: vec![down.id, quarantined.id]
            }
        );
    }

    #[test]
    fn last_relay_reads_only_relay_paths() {
        let r = Uuid::now_v7();
        assert_eq!(
            last_relay(Some(
                &serde_json::json!({"kind": "relay", "relay": r, "peer": null})
            )),
            Some(r)
        );
        assert_eq!(
            last_relay(Some(&serde_json::json!({"kind": "direct", "relay": r}))),
            None
        );
        assert_eq!(
            last_relay(Some(&serde_json::json!({"kind": "relay", "relay": "nope"}))),
            None
        );
        assert_eq!(last_relay(None), None);
        // What the listener writes reads back.
        let path = RunnerPath::Relay {
            relay_id: r,
            advisory_peer: Some("10.0.0.5:4000".into()),
        };
        assert_eq!(last_relay(Some(&path.to_json())), Some(r));
    }

    #[test]
    fn the_dependency_warning_names_the_relay_and_caps_the_hosts() {
        let w = relay_dependency_warning("r1", "gw1", &["web1".into(), "web2".into()]);
        assert_eq!(
            w,
            "This run includes gw1, which runs relay r1, the only relay for web1, web2. If the \
             run restarts or breaks the relay, those hosts lose their path to Keel and their \
             part of the run is lost. Consider running gw1 separately."
        );
        let many: Vec<String> = (0..13).map(|i| format!("h{i}")).collect();
        let w = relay_dependency_warning("r1", "gw1", &many);
        assert!(w.contains("h9 and 3 more."), "{w}");
        assert!(!w.contains("h10"), "{w}");
    }
}
