//! Relays (spec 067). For now, Keel's relay-link key (T011).
//!
//! [`bootstrap`] runs at start, after [`crate::hosts::bootstrap`]: when no `active`
//! `relay_link_keys` row exists it generates an Ed25519 key, seals its private half under the
//! master key (`aad` = `relay_link_key:<id>`), stores it and audits `relay_link_key.create`;
//! then it loads the active key into [`LinkKey`]. Keel authenticates to every relay with this
//! key; an owner pastes its public line into the relay's `hub_keys`. Without it, no relay is
//! dialled.

use std::sync::{Arc, RwLock};

use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, PrivateKey};
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
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
