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
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use keel_core::WorkspaceId;
use keel_exec::codec::CodecError;
use keel_exec::relay::{
    self, CacheMissReason, Frame, LINK_USER, LinkClientPolicy, LinkViolation, RELAY_SUBSYSTEM,
    RunnerChannel, ViolationSink, link_client_config,
};
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
use crate::runner_listener::{RunnerHost, RunnerPath, serve_connection};
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

/// Generates a key, seals and stores it as active, and audits it. The caller holds the lock and
/// has made sure no active key exists.
async fn create(
    conn: &mut PgConnection,
    master: &crate::seal::MasterKey,
    actor: &Actor,
) -> Result<(), RelayKeyError> {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).map_err(key_error)?;
    let id = Uuid::now_v7();
    let pem = key.to_openssh(LineEnding::LF).map_err(key_error)?;
    let sealed = master.seal(&aad(id), pem.as_bytes())?;
    drop(pem);
    let fingerprint = keel_exec::ca::fingerprint(key.public_key().key_data());
    sqlx::query(
        "insert into relay_link_keys (id, public_key, fingerprint, sealed_private_key, state, \
         activated_at) values ($1, $2, $3, $4, 'active', now())",
    )
    .bind(id)
    .bind(public_line(&key)?)
    .bind(&fingerprint)
    .bind(sealed)
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
            context: json!({ "fingerprint": fingerprint }),
        },
    )
    .await?;
    Ok(())
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
        create(&mut tx, master, &Actor::System).await?;
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
}

/// Every supervised relay's [`RelayLive`], shared through [`AppState::relay_live`]. Readers
/// (`ListRelays`, a host's reach) take snapshots; the WebSocket subscribes to
/// [`RelayChange`]s, sent on every state change (not on each RTT or stream-count update).
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

    /// Applies `f` to a relay's state (creating it as `connecting`); a state change resets
    /// `since` and is broadcast.
    fn update(&self, relay: Uuid, workspace: Uuid, f: impl FnOnce(&mut RelayLive)) {
        let changed = {
            let Ok(mut m) = self.inner.map.write() else {
                return;
            };
            let entry = m
                .entry(relay)
                .or_insert_with(|| (workspace, RelayLive::new(RelayState::Connecting)));
            let before = entry.1.state;
            f(&mut entry.1);
            (entry.1.state != before).then(|| {
                entry.1.since = OffsetDateTime::now_utc();
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
}

impl SupervisorOptions {
    #[must_use]
    pub fn from_config(cfg: &crate::config::Config) -> Self {
        Self {
            health_interval: cfg.relay_health_interval,
            ..Self::default()
        }
    }
}

impl Default for SupervisorOptions {
    fn default() -> Self {
        Self {
            health_interval: Duration::from_secs(30),
            hello_timeout: relay::HELLO_TIMEOUT,
            max_streams: 256,
            stream_rate: 20,
            stream_burst: 50,
            max_unanswered_pings: 4,
            refusal_window: Duration::from_secs(60),
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
}

struct Link {
    row: RelayRow,
    stop: watch::Sender<Option<String>>,
    task: JoinHandle<()>,
}

struct Supervisor {
    shared: Arc<Shared>,
    links: Mutex<HashMap<Uuid, Link>>,
}

impl Drop for Supervisor {
    fn drop(&mut self) {
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
        Self {
            inner: Arc::new(Supervisor {
                shared: Arc::new(Shared {
                    app,
                    host,
                    opts,
                    refusals,
                }),
                links: Mutex::new(HashMap::new()),
            }),
        }
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
        if let Ok(mut links) = self.inner.links.lock()
            && let Some(link) = links.remove(&relay)
        {
            let _ = link.stop.send(Some(reason.to_owned()));
        }
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
            russh::client::connect_stream(Arc::new(link_client_config()), tcp, policy),
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
                    Ok(Frame::CacheGet { req, .. }) => {
                        // Keel does not serve runner releases yet (spec 067 US5).
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
                advisory_peer: Some(peer),
            };
            serve_connection(shared.app.clone(), config, ch.channel.into_stream(), path).await;
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
}
