//! The embedded SSH server runners connect to (spec 055 T082, contracts/runner.md).
//!
//! Built on `keel_exec::server_config`: Ed25519 only, public-key auth only, no shell, exec,
//! pty, env or forwarding, and one subsystem per connection decided at authentication:
//! - a valid runner certificate of an enrolled, non-revoked host: `keel-runner-v1`, handed to
//!   the run hub ([`crate::queue::attach`]);
//! - the bare enrolled key within `--runner-max-offline` of its last certificate's expiry:
//!   `keel-runner-v1`, `RenewCert` only;
//! - an unknown key: `keel-enroll-v1`, one `Enroll` frame.
//!
//! The server presents an ephemeral host key with a host certificate from the host CA, for
//! the names in `--runner-host-names`; the certificate is re-issued daily and whenever the host
//! CA changes. With no CAs loaded every connection is dropped.
//!
//! Enrollment checks the token (single use, 15 minutes, stored hashed), that the token names
//! the active host CA, that `inventory_name` matches, and that the token's profiles are within
//! the host's `allowed_profiles`. Any failure closes the connection, burns the token and is
//! audited. The token itself is never logged or audited.
//!
//! A connection arrives over TCP ([`spawn`]) or, from spec 067, as a relay's `forwarded-tcpip`
//! channel. Both go through [`serve_connection`] with the [`RunnerPath`] they came by; the SSH
//! session, its authentication and its subsystems are the same either way.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use keel_core::WorkspaceId;
use keel_exec::ca::{self, HostCertRequest, RunnerCertVerifier, RunnerIdentity};
use keel_exec::codec;
use keel_exec::frames::Frame;
use keel_exec::krl::Krl;
use keel_exec::server_config::{
    Access, BareKey, KeelSshHandler, RunnerAuthority, SubsystemService, server_config,
};
use russh::keys::ssh_key::public::KeyData;
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::Msg;
use serde_json::json;
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::hosts::{self, HOST_CERT_TTL, token_fingerprint, token_hash};
use crate::relays::RelayAttempt;
use crate::serve::AppState;

/// How long a fresh connection has to send its first frame.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(30);
/// A renew-only connection closes after this long idle.
const RENEW_IDLE: Duration = Duration::from_secs(60);
/// The longest an enroll or join session lives, whatever it is doing: one request and one
/// answer, then the connection closes (spec 067 T066 F3). Over a relay the auth deadline
/// applies as well, since neither counts as an authenticated runner.
const NON_RUNNER_LIFETIME: Duration = Duration::from_secs(60);
/// How long a finished session has to flush its last frame before the connection is dropped.
const CLOSE_GRACE: Duration = Duration::from_secs(5);
/// The host certificate is re-issued after this long.
const HOST_CERT_REFRESH: Duration = Duration::from_secs(24 * 3600);
/// The most `allowed_profiles` a host may claim.
const MAX_ALLOWED_PROFILES: usize = 64;

/// [`RunnerAuthority`] over the database and the loaded CAs.
pub struct Authority {
    app: AppState,
}

impl RunnerAuthority for Authority {
    fn verifier(&self) -> Arc<RunnerCertVerifier> {
        // No CAs: an empty verifier trusts nothing.
        self.app.ssh.get().map_or_else(
            || Arc::new(RunnerCertVerifier::new(Vec::new(), Krl::default())),
            |c| Arc::clone(&c.verifier),
        )
    }

    async fn is_enrolled(&self, identity: &RunnerIdentity, key: &KeyData) -> bool {
        let Ok(host) = Uuid::parse_str(&identity.runner_id) else {
            return false;
        };
        let r: Result<(bool,), sqlx::Error> = sqlx::query_as(
            "select exists (select 1 from hosts h join ssh_certs c on c.host_id = h.id \
             where h.id = $1 and h.ssh_key_fingerprint = $2 and h.state <> 'revoked' \
             and h.inventory_name = $3 and c.serial = $4 and c.revoked_at is null)",
        )
        .bind(host)
        .bind(ca::fingerprint(key))
        .bind(&identity.inventory_name)
        .bind(hosts::serial_to_db(identity.serial))
        .fetch_one(&self.app.db)
        .await;
        r.is_ok_and(|(ok,)| ok)
    }

    async fn bare_key(&self, key: &KeyData, now: SystemTime) -> BareKey {
        let r: Result<Option<(Uuid, String, Option<OffsetDateTime>)>, sqlx::Error> =
            sqlx::query_as(
                "select h.id, h.state, max(c.valid_before) from hosts h \
                 left join ssh_certs c on c.host_id = h.id and c.revoked_at is null \
                 where h.ssh_key_fingerprint = $1 group by h.id, h.state",
            )
            .bind(ca::fingerprint(key))
            .fetch_optional(&self.app.db)
            .await;
        match r {
            Ok(None) => BareKey::Unknown,
            // Accepted from a join request and not yet certified: back to the join subsystem
            // for its first certificate (contracts/join.md).
            Ok(Some((id, state, None))) if state != "revoked" => {
                match crate::join::awaiting_cert(&self.app.db, id).await {
                    Ok(true) => BareKey::Unknown,
                    _ => BareKey::Refused,
                }
            }
            Ok(Some((id, state, Some(until)))) if state != "revoked" => {
                let now = OffsetDateTime::from(now);
                if now <= until + self.app.cfg.runner_max_offline {
                    BareKey::Renewable {
                        host_id: id.to_string(),
                    }
                } else {
                    BareKey::Refused
                }
            }
            // Revoked, never certified, or the database failed: refuse.
            _ => BareKey::Refused,
        }
    }
}

/// How a runner connection reached Keel (spec 067). Kept with the run hub's session and
/// written to `hosts.last_path` on `Hello`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerPath {
    /// Straight to the runner listener. `peer` is the TCP peer address, when known.
    Direct { peer: Option<SocketAddr> },
    /// Through a relay's link. `advisory_peer` is the runner address the relay claimed in the
    /// `forwarded-tcpip` open: shown and logged, never trusted.
    Relay {
        relay_id: Uuid,
        advisory_peer: Option<String>,
    },
}

/// The longest relay-claimed peer kept (in `hosts.last_path` and logs).
const MAX_ADVISORY_PEER: usize = 128;

impl RunnerPath {
    /// The relay a connection came through, if any.
    #[must_use]
    pub fn relay(&self) -> Option<Uuid> {
        match self {
            RunnerPath::Direct { .. } => None,
            RunnerPath::Relay { relay_id, .. } => Some(*relay_id),
        }
    }

    /// The `hosts.last_path` value (data-model.md): `{"kind":"direct","peer":..}` or
    /// `{"kind":"relay","relay":..,"peer":..}`; `peer` is null when unknown.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            RunnerPath::Direct { peer } => {
                json!({ "kind": "direct", "peer": peer.map(|p| p.to_string()) })
            }
            RunnerPath::Relay {
                relay_id,
                advisory_peer,
            } => json!({
                "kind": "relay",
                "relay": relay_id,
                // Claimed by the relay; capped so a hostile one cannot bloat the row.
                "peer": advisory_peer
                    .as_deref()
                    .map(|p| p.chars().take(MAX_ADVISORY_PEER).collect::<String>()),
            }),
        }
    }
}

impl std::fmt::Display for RunnerPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerPath::Direct { peer: Some(p) } => write!(f, "direct {p}"),
            RunnerPath::Direct { peer: None } => f.write_str("direct"),
            RunnerPath::Relay {
                relay_id,
                advisory_peer,
            } => {
                write!(f, "relay {relay_id}")?;
                if let Some(p) = advisory_peer {
                    let p: String = p.chars().take(MAX_ADVISORY_PEER).collect();
                    write!(f, " (claims {})", p.escape_debug())?;
                }
                Ok(())
            }
        }
    }
}

/// [`SubsystemService`]: enrollment, renewal, and runner sessions, for one connection.
pub struct Service {
    app: AppState,
    path: RunnerPath,
    /// Signalled once an authenticated runner (or a renew-only host) asked for its subsystem,
    /// which lifts [`serve_relayed`]'s deadline. Enroll and join never signal it: they stay
    /// under the deadline until they end (T066 F3).
    started: Option<Arc<Notify>>,
    /// Signalled when the subsystem ended, which ends the connection: one subsystem per
    /// connection, never an idle session left open behind it.
    done: Arc<Notify>,
}

impl Service {
    /// Logs a refusal: once per window and as a coalesced `relay.refused` row on a relay path
    /// (a relay can repeat one as fast as it likes, T061), as a log line on a direct one.
    fn refused(&self, reason: &'static str, detail: &str) {
        if self.path.relay().is_some() {
            self.app.relay_live.refused(&self.path, reason, detail);
        } else {
            tracing::info!(path = %self.path, reason, detail, "runner connection refused");
        }
    }

    /// Lifts [`serve_relayed`]'s deadline: an authenticated runner or renewing host.
    fn start(&self) {
        if let Some(started) = &self.started {
            started.notify_one();
        }
    }

    /// Serves the subsystem; `true` once it is over, `false` while it lives on elsewhere (an
    /// attached runner session, which the run hub owns).
    async fn serve_access(
        &self,
        access: Access,
        key: KeyData,
        stream: russh::ChannelStream<Msg>,
    ) -> bool {
        match access {
            Access::JoinOnly => {
                let join = crate::join::serve(&self.app, &key, &self.path, stream);
                let _ = tokio::time::timeout(NON_RUNNER_LIFETIME, join).await;
            }
            Access::EnrollOnly => {
                let _ = tokio::time::timeout(NON_RUNNER_LIFETIME, self.enroll(&key, stream)).await;
            }
            Access::RenewOnly { host_id } => {
                let Ok(host) = Uuid::parse_str(&host_id) else {
                    return true;
                };
                if !self.host_within_relay(host).await {
                    self.refused("workspace_mismatch", &host.to_string());
                    return true;
                }
                self.start();
                renew_only(&self.app, host, stream).await;
            }
            Access::Runner(identity) => {
                self.start();
                let Ok(host) = Uuid::parse_str(&identity.runner_id) else {
                    return true;
                };
                if !self.host_within_relay(host).await {
                    self.refused("workspace_mismatch", &host.to_string());
                    return true;
                }
                let path = self.path.clone();
                let Err(e) =
                    crate::queue::attach_on(self.app.clone(), host, stream, path.clone()).await
                else {
                    return false;
                };
                if path.relay().is_some() {
                    self.refused("runner_session_refused", &format!("{host}: {e}"));
                } else {
                    tracing::warn!(host = %host, %path, error = %e, "runner session refused");
                }
            }
        }
        true
    }

    async fn enroll(&self, key: &KeyData, stream: russh::ChannelStream<Msg>) {
        // One relay can front any number of peers: its enrollments are rate-limited (T062). A
        // refused one closes the stream before a frame is read.
        if !self
            .app
            .relay_live
            .attempt(&self.path, RelayAttempt::Enroll)
        {
            return;
        }
        let Some(within) = self.relay_workspace().await else {
            self.refused("workspace_mismatch", "enroll");
            return;
        };
        match enroll_within(&self.app, key, stream, within).await {
            Ok(()) => {}
            Err(EnrollEnd::Refused(EnrollRefusal::WorkspaceMismatch)) => {
                self.refused("workspace_mismatch", "enroll");
            }
            Err(reason) => self.refused("enroll_refused", reason.as_str()),
        }
    }

    /// The workspace this connection is bound to (T066 F4): `Some(None)` on a direct path,
    /// `Some(Some(ws))` for a relay's, `None` when the relay's cannot be read (fail closed).
    async fn relay_workspace(&self) -> Option<Option<Uuid>> {
        match self.path.relay() {
            None => Some(None),
            Some(relay) => crate::relays::relay_workspace(&self.app.db, relay)
                .await
                .map(Some),
        }
    }

    /// Whether `host` may be served on this path: always on a direct one, and over a relay only
    /// when the host is in the relay's workspace. A relay never carries another workspace's
    /// runner, whatever key it holds (T066 F4).
    async fn host_within_relay(&self, host: Uuid) -> bool {
        let Some(within) = self.relay_workspace().await else {
            return false;
        };
        let Some(ws) = within else { return true };
        sqlx::query_as::<_, (Uuid,)>("select workspace_id from hosts where id = $1")
            .bind(host)
            .fetch_optional(&self.app.db)
            .await
            .ok()
            .flatten()
            .is_some_and(|(host_ws,)| host_ws == ws)
    }
}

impl SubsystemService for Service {
    async fn serve(&self, _access: Access, _stream: russh::ChannelStream<Msg>) {
        // Without the authenticated key nothing can be checked: drop the stream.
        self.done.notify_one();
    }

    async fn serve_with_key(
        &self,
        access: Access,
        key: KeyData,
        stream: russh::ChannelStream<Msg>,
    ) {
        if self.serve_access(access, key, stream).await {
            self.done.notify_one();
        }
    }
}

/// A renew-only connection: answers each `RenewCert` with `Cert`; anything else closes it.
async fn renew_only<S: AsyncRead + AsyncWrite + Unpin>(app: &AppState, host: Uuid, mut stream: S) {
    let access = Access::RenewOnly {
        host_id: host.to_string(),
    };
    loop {
        let frame = match tokio::time::timeout(RENEW_IDLE, codec::read_frame(&mut stream)).await {
            Ok(Ok(Some(f))) => f,
            _ => return,
        };
        if !access.permits(&frame) {
            return;
        }
        match hosts::renew(app, host).await {
            Ok(cert) => {
                if codec::write_frame(&mut stream, &Frame::Cert { cert })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(e) => {
                tracing::warn!(host = %host, error = %e, "renewing a runner certificate failed");
                return;
            }
        }
    }
}

// ---- enrollment ------------------------------------------------------------------------------

/// Why an enrollment was refused: the audit `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollRefusal {
    MalformedToken,
    UnknownToken,
    TokenReused,
    Expired,
    CaMismatch,
    InventoryNameMismatch,
    BadAllowedProfiles,
    ProfileNotAllowed,
    KeyEnrolled,
    HostExists,
    ProfileMissing,
    CaUnavailable,
    /// The token is for another workspace than the relay that carried it (T066 F4). Neither
    /// burned nor audited as an enrollment: the relay is the suspect, not the token.
    WorkspaceMismatch,
}

impl EnrollRefusal {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MalformedToken => "malformed_token",
            Self::UnknownToken => "unknown_token",
            Self::TokenReused => "token_reused",
            Self::Expired => "expired",
            Self::CaMismatch => "ca_mismatch",
            Self::InventoryNameMismatch => "inventory_name_mismatch",
            Self::BadAllowedProfiles => "bad_allowed_profiles",
            Self::ProfileNotAllowed => "profile_not_allowed",
            Self::KeyEnrolled => "key_enrolled",
            Self::HostExists => "host_exists",
            Self::ProfileMissing => "profile_missing",
            Self::CaUnavailable => "ca_unavailable",
            Self::WorkspaceMismatch => "workspace_mismatch",
        }
    }
}

/// What a runner asked to enrol with.
pub struct EnrollRequest<'a> {
    pub token: &'a str,
    pub inventory_name: &'a str,
    pub allowed_profiles: &'a [String],
    pub require_signed: bool,
}

/// Why an enrollment stream ended without a host: no `Enroll` frame, or it was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollEnd {
    /// No valid `Enroll` frame arrived in time.
    NoFrame,
    /// Refused (audited as `host_enroll_refused`).
    Refused(EnrollRefusal),
    /// Keel failed (logged).
    Failed,
}

impl EnrollEnd {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoFrame => "no_enroll_frame",
            Self::Refused(r) => r.as_str(),
            Self::Failed => "failed",
        }
    }
}

/// An unknown key's `keel-enroll-v1` stream: one `Enroll`, answered `Enrolled` or closed.
///
/// # Errors
///
/// Why no host was enrolled; the caller logs it.
pub async fn enroll<S: AsyncRead + AsyncWrite + Unpin>(
    app: &AppState,
    key: &KeyData,
    stream: S,
) -> Result<(), EnrollEnd> {
    enroll_within(app, key, stream, None).await
}

/// [`enroll`], refusing a token for any workspace but `within` when it is set (a relay's).
async fn enroll_within<S: AsyncRead + AsyncWrite + Unpin>(
    app: &AppState,
    key: &KeyData,
    mut stream: S,
    within: Option<Uuid>,
) -> Result<(), EnrollEnd> {
    let read = codec::read_frame_capped(&mut stream, crate::join::FIRST_FRAME_MAX);
    let frame = match tokio::time::timeout(FIRST_FRAME_TIMEOUT, read).await {
        Ok(Ok(Some(f))) => f,
        _ => return Err(EnrollEnd::NoFrame),
    };
    if !Access::EnrollOnly.permits(&frame) {
        return Err(EnrollEnd::NoFrame);
    }
    let Frame::Enroll {
        token,
        inventory_name,
        allowed_profiles,
        require_signed,
    } = frame
    else {
        return Err(EnrollEnd::NoFrame);
    };
    let req = EnrollRequest {
        token: token.expose(),
        inventory_name: &inventory_name,
        allowed_profiles: &allowed_profiles,
        require_signed,
    };
    match enroll_key_within(app, key, &req, within).await {
        Ok((host_id, cert)) => {
            let _ = codec::write_frame(
                &mut stream,
                &Frame::Enrolled {
                    host_id: host_id.to_string(),
                    cert,
                },
            )
            .await;
            Ok(())
        }
        Err(Ok(reason)) => Err(EnrollEnd::Refused(reason)),
        Err(Err(e)) => {
            tracing::error!(error = %e, "enrollment failed");
            Err(EnrollEnd::Failed)
        }
    }
}

type EnrollResult = Result<(Uuid, String), Result<EnrollRefusal, hosts::SshError>>;

struct Enrollment {
    id: Uuid,
    workspace: Uuid,
    inventory_name: String,
    profiles: Vec<String>,
    expires_at: OffsetDateTime,
    used: bool,
    created_by: Option<Uuid>,
}

/// Checks `req` and enrols `key`: the host's id and its first certificate (OpenSSH line).
/// A refusal burns the token (when it names one) and is audited; the token is not.
///
/// # Errors
///
/// `Err(Ok(reason))` for a refusal, `Err(Err(_))` when the database or the CA failed.
pub async fn enroll_key(app: &AppState, key: &KeyData, req: &EnrollRequest<'_>) -> EnrollResult {
    enroll_key_within(app, key, req, None).await
}

/// [`enroll_key`], refusing a token for any workspace but `within` when it is set: a relay
/// enrols hosts into its own workspace only (T066 F4). That refusal leaves the token as it was
/// and is recorded by the caller as a `relay.refused`, not a `host_enroll_refused`.
async fn enroll_key_within(
    app: &AppState,
    key: &KeyData,
    req: &EnrollRequest<'_>,
    within: Option<Uuid>,
) -> EnrollResult {
    let fail = |e: sqlx::Error| Err(Err(hosts::SshError::Db(e)));
    let Some(fp) = token_fingerprint(req.token) else {
        return match audit_refusal(app, None, None, EnrollRefusal::MalformedToken).await {
            Ok(()) => Err(Ok(EnrollRefusal::MalformedToken)),
            Err(e) => fail(e),
        };
    };
    let mut tx = match app.db.begin().await {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    type Row = (
        Uuid,
        Uuid,
        String,
        Vec<String>,
        OffsetDateTime,
        bool,
        Option<Uuid>,
    );
    let row: Option<Row> = match sqlx::query_as(
        "select id, workspace_id, inventory_name, profiles, expires_at, \
         used_at is not null or burned_reason is not null, created_by \
         from host_enrollments where token_hash = $1 for update",
    )
    .bind(token_hash(req.token))
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    let Some((id, workspace, inventory_name, profiles, expires_at, used, created_by)) = row else {
        drop(tx);
        return match audit_refusal(app, None, None, EnrollRefusal::UnknownToken).await {
            Ok(()) => Err(Ok(EnrollRefusal::UnknownToken)),
            Err(e) => fail(e),
        };
    };
    if within.is_some_and(|ws| ws != workspace) {
        drop(tx);
        return Err(Ok(EnrollRefusal::WorkspaceMismatch));
    }
    let e = Enrollment {
        id,
        workspace,
        inventory_name,
        profiles,
        expires_at,
        used,
        created_by,
    };
    match check(app, &mut tx, key, fp, &e, req).await {
        Ok(Ok(())) => {}
        Ok(Err(reason)) => {
            // Burn the token (a used one stays as it was) and audit, in one transaction.
            let r = async {
                if reason != EnrollRefusal::TokenReused {
                    sqlx::query(
                        "update host_enrollments set used_at = coalesce(used_at, now()), \
                         burned_reason = $2 where id = $1",
                    )
                    .bind(e.id)
                    .bind(reason.as_str())
                    .execute(&mut *tx)
                    .await?;
                }
                record_refusal(&mut tx, Some(e.workspace), Some(e.id), reason).await?;
                tx.commit().await
            }
            .await;
            return match r {
                Ok(()) => Err(Ok(reason)),
                Err(err) => fail(err),
            };
        }
        Err(err) => return fail(err),
    }
    match enrol(app, &mut tx, key, &e, req).await {
        Ok((host, cert)) => match tx.commit().await {
            Ok(()) => {
                // Spec 068 US2: converge the new host, after the enrollment committed. A failure
                // here is logged and leaves the host enrolled.
                if let Err(err) = crate::converge::on_enrolled(app, host).await {
                    tracing::error!(%host, error = %err, "converge on enrollment failed");
                }
                Ok((host, cert))
            }
            Err(err) => fail(err),
        },
        Err(err) => Err(Err(err)),
    }
}

/// Every refusal, in order. `Ok(Err(reason))` refuses.
async fn check(
    app: &AppState,
    conn: &mut sqlx::PgConnection,
    key: &KeyData,
    fp: &str,
    e: &Enrollment,
    req: &EnrollRequest<'_>,
) -> Result<Result<(), EnrollRefusal>, sqlx::Error> {
    use EnrollRefusal as R;
    if e.used {
        return Ok(Err(R::TokenReused));
    }
    if OffsetDateTime::now_utc() >= e.expires_at {
        return Ok(Err(R::Expired));
    }
    let Some(cas) = app.ssh.get() else {
        return Ok(Err(R::CaUnavailable));
    };
    if cas.host.fingerprint() != fp {
        return Ok(Err(R::CaMismatch));
    }
    if req.inventory_name != e.inventory_name {
        return Ok(Err(R::InventoryNameMismatch));
    }
    let valid_profile = |p: &String| {
        !p.is_empty()
            && p.len() <= 63
            && p.bytes().enumerate().all(|(i, b)| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || (i > 0 && (b == b'_' || b == b'-'))
            })
    };
    if req.allowed_profiles.len() > MAX_ALLOWED_PROFILES
        || !req.allowed_profiles.iter().all(valid_profile)
    {
        return Ok(Err(R::BadAllowedProfiles));
    }
    let allowed: BTreeSet<&String> = req.allowed_profiles.iter().collect();
    if !e.profiles.iter().all(|p| allowed.contains(p)) {
        return Ok(Err(R::ProfileNotAllowed));
    }
    let (key_known,): (bool,) =
        sqlx::query_as("select exists (select 1 from hosts where ssh_key_fingerprint = $1)")
            .bind(ca::fingerprint(key))
            .fetch_one(&mut *conn)
            .await?;
    if key_known {
        return Ok(Err(R::KeyEnrolled));
    }
    let (name_taken,): (bool,) = sqlx::query_as(
        "select exists (select 1 from hosts where workspace_id = $1 and inventory_name = $2)",
    )
    .bind(e.workspace)
    .bind(&e.inventory_name)
    .fetch_one(&mut *conn)
    .await?;
    if name_taken {
        return Ok(Err(R::HostExists));
    }
    let (found,): (i64,) =
        sqlx::query_as("select count(*) from profiles where workspace_id = $1 and name = any($2)")
            .bind(e.workspace)
            .bind(&e.profiles)
            .fetch_one(&mut *conn)
            .await?;
    if usize::try_from(found).ok() != Some(e.profiles.len()) {
        return Ok(Err(R::ProfileMissing));
    }
    Ok(Ok(()))
}

/// Records the host, its profiles and first certificate, and consumes the token.
async fn enrol(
    app: &AppState,
    conn: &mut sqlx::PgConnection,
    key: &KeyData,
    e: &Enrollment,
    req: &EnrollRequest<'_>,
) -> Result<(Uuid, String), hosts::SshError> {
    let host = Uuid::now_v7();
    let line = PublicKey::from(key.clone())
        .to_openssh()
        .map_err(|err| hosts::SshError::Ca(err.to_string()))?;
    let mut allowed = req.allowed_profiles.to_vec();
    allowed.sort();
    allowed.dedup();
    sqlx::query(
        "insert into hosts (id, workspace_id, inventory_name, ssh_key_fingerprint, \
         ssh_public_key, allowed_profiles, require_signed, state) \
         values ($1, $2, $3, $4, $5, $6, $7, 'offline')",
    )
    .bind(host)
    .bind(e.workspace)
    .bind(&e.inventory_name)
    .bind(ca::fingerprint(key))
    .bind(line)
    .bind(&allowed)
    .bind(req.require_signed)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "insert into host_profiles (host_id, profile_id, assigned_by) \
         select $1, id, $3 from profiles where workspace_id = $2 and name = any($4)",
    )
    .bind(host)
    .bind(e.workspace)
    .bind(e.created_by)
    .bind(&e.profiles)
    .execute(&mut *conn)
    .await?;
    sqlx::query("update host_enrollments set used_at = now(), host_id = $2 where id = $1")
        .bind(e.id)
        .bind(host)
        .execute(&mut *conn)
        .await?;
    let cert = hosts::issue_cert(app, conn, host, &e.inventory_name, key).await?;
    audit::record(
        conn,
        Entry {
            actor: &Actor::Host(host),
            action: "host_enrolled",
            workspace: Some(WorkspaceId(e.workspace)),
            target: json!({ "host": host, "inventory_name": e.inventory_name }),
            decision: Decision::Allowed,
            reason: None,
            context: json!({
                "enrollment": e.id,
                "profiles": e.profiles,
                "allowed_profiles": allowed,
                "require_signed": req.require_signed,
                "serial": cert.serial().to_string(),
            }),
        },
    )
    .await?;
    let cert = cert
        .to_openssh()
        .map_err(|err| hosts::SshError::Ca(err.to_string()))?;
    Ok((host, cert))
}

async fn record_refusal(
    conn: &mut sqlx::PgConnection,
    workspace: Option<Uuid>,
    enrollment: Option<Uuid>,
    reason: EnrollRefusal,
) -> Result<(), sqlx::Error> {
    audit::record(
        conn,
        Entry {
            actor: &Actor::System,
            action: "host_enroll_refused",
            workspace: workspace.map(WorkspaceId),
            target: json!({ "enrollment": enrollment }),
            decision: Decision::Denied,
            reason: Some(reason.as_str()),
            context: json!({}),
        },
    )
    .await
    .map(|_| ())
}

async fn audit_refusal(
    app: &AppState,
    workspace: Option<Uuid>,
    enrollment: Option<Uuid>,
    reason: EnrollRefusal,
) -> Result<(), sqlx::Error> {
    let mut tx = app.db.begin().await?;
    record_refusal(&mut tx, workspace, enrollment, reason).await?;
    tx.commit().await
}

// ---- the listener ----------------------------------------------------------------------------

/// The listener's host key and its current certificate.
struct HostIdentity {
    key: PrivateKey,
    cert: Option<(russh::keys::Certificate, Instant, String)>,
}

impl HostIdentity {
    /// A server config with a current host certificate, or `None` without a host CA.
    fn config(&mut self, app: &AppState) -> Option<Arc<russh::server::Config>> {
        let cas = app.ssh.get()?;
        let ca_fp = cas.host.fingerprint();
        let fresh = self
            .cert
            .as_ref()
            .is_some_and(|(_, issued, fp)| *fp == ca_fp && issued.elapsed() < HOST_CERT_REFRESH);
        if !fresh {
            let cert = cas
                .host
                .issue_host_cert(&HostCertRequest {
                    host_names: &app.cfg.runner_host_names,
                    public_key: self.key.public_key().key_data(),
                    ttl: HOST_CERT_TTL,
                    now: SystemTime::now(),
                })
                .map_err(|e| tracing::error!(error = %e, "issuing the host certificate failed"))
                .ok()?;
            self.cert = Some((cert, Instant::now(), ca_fp));
        }
        let cert = self.cert.as_ref().map(|c| c.0.clone());
        Some(Arc::new(server_config(self.key.clone(), cert)))
    }
}

/// The runner-facing SSH host identity: one ephemeral key, shared by the TCP listener and every
/// relay link, with its host certificate re-issued as needed. Cheap to clone.
#[derive(Clone)]
pub struct RunnerHost {
    identity: Arc<Mutex<HostIdentity>>,
}

impl RunnerHost {
    /// A fresh ephemeral host key.
    ///
    /// # Errors
    ///
    /// The key could not be generated.
    pub fn new() -> Result<Self, String> {
        let key =
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).map_err(|e| e.to_string())?;
        Ok(Self {
            identity: Arc::new(Mutex::new(HostIdentity { key, cert: None })),
        })
    }

    /// The server config for the next connection, or `None` while no host CA is loaded (the
    /// connection must then be dropped).
    #[must_use]
    pub fn config(&self, app: &AppState) -> Option<Arc<russh::server::Config>> {
        self.identity.lock().ok().and_then(|mut i| i.config(app))
    }
}

/// Serves one runner connection over `stream`, which arrived by `path`, until it ends. The same
/// SSH server either way: authentication decides the subsystem, and a runner session reaches
/// the run hub with `path` attached.
pub async fn serve_connection<S>(
    app: AppState,
    config: Arc<russh::server::Config>,
    stream: S,
    path: RunnerPath,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let done = Arc::new(Notify::new());
    let authority = Arc::new(Authority { app: app.clone() });
    let service = Arc::new(Service {
        app,
        path,
        started: None,
        done: Arc::clone(&done),
    });
    let handler = KeelSshHandler::new(authority, service);
    if let Ok(mut session) = russh::server::run_stream(config, stream, handler).await {
        let handle = session.handle();
        tokio::select! {
            _ = &mut session => {}
            () = done.notified() => close(&mut session, &handle).await,
        }
    }
}

/// Closes a session whose subsystem ended, letting it flush its last frame for at most
/// [`CLOSE_GRACE`].
async fn close<F: Future + Unpin>(session: &mut F, handle: &russh::server::Handle) {
    let _ = handle
        .disconnect(
            russh::Disconnect::ByApplication,
            "session ended".into(),
            String::new(),
        )
        .await;
    let _ = tokio::time::timeout(CLOSE_GRACE, session).await;
}

/// How a relayed runner stream ended ([`serve_relayed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    /// The runner authenticated and was served, or an enroll or join ended in time.
    Served,
    /// The stream ended before the runner authenticated: a refused key or certificate, a
    /// failed handshake, or a hang-up.
    Unauthenticated,
    /// The runner had not authenticated by the deadline; the stream was closed.
    AuthDeadline,
}

/// [`serve_connection`] for a stream a relay carries (spec 067 T060): the runner must complete
/// the SSH handshake, authenticate and ask for its subsystem within `deadline` of the stream
/// opening, or the stream is closed. A stream that opens and idles costs Keel a bounded time.
///
/// Enroll and join never lift the deadline (T066 F3): a bare key's session, which any key can
/// open, must have asked and been answered within it. Every session ends with its subsystem.
pub async fn serve_relayed<S>(
    app: AppState,
    config: Arc<russh::server::Config>,
    stream: S,
    path: RunnerPath,
    deadline: Duration,
) -> StreamEnd
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let by = tokio::time::Instant::now() + deadline;
    let started = Arc::new(Notify::new());
    let done = Arc::new(Notify::new());
    let authority = Arc::new(Authority { app: app.clone() });
    let service = Arc::new(Service {
        app,
        path,
        started: Some(Arc::clone(&started)),
        done: Arc::clone(&done),
    });
    let handler = KeelSshHandler::new(authority, service);
    // `run_stream` waits for the peer's SSH id; dropping it on the deadline drops the stream.
    let mut session =
        match tokio::time::timeout_at(by, russh::server::run_stream(config, stream, handler)).await
        {
            Ok(Ok(s)) => s,
            Ok(Err(_)) => return StreamEnd::Unauthenticated,
            Err(_) => return StreamEnd::AuthDeadline,
        };
    let handle = session.handle();
    tokio::select! {
        biased;
        () = started.notified() => {
            tokio::select! {
                _ = &mut session => {}
                () = done.notified() => close(&mut session, &handle).await,
            }
            StreamEnd::Served
        }
        () = done.notified() => {
            close(&mut session, &handle).await;
            StreamEnd::Served
        }
        _ = &mut session => StreamEnd::Unauthenticated,
        () = tokio::time::sleep_until(by) => {
            let _ = handle
                .disconnect(
                    russh::Disconnect::ByApplication,
                    "authentication deadline".into(),
                    String::new(),
                )
                .await;
            StreamEnd::AuthDeadline
        }
    }
}

/// Serves runners on `listener` until the task is dropped, with a host identity of its own.
///
/// # Errors
///
/// The host key could not be generated.
pub fn spawn(app: AppState, listener: TcpListener) -> Result<tokio::task::JoinHandle<()>, String> {
    Ok(spawn_with(app, RunnerHost::new()?, listener))
}

/// Serves runners on `listener` with `host` (shared with the relay links) until the task is
/// dropped.
#[must_use]
pub fn spawn_with(
    app: AppState,
    host: RunnerHost,
    listener: TcpListener,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let (socket, peer) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "runner listener accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let Some(config) = host.config(&app) else {
                // No CAs: refuse by closing.
                continue;
            };
            let path = RunnerPath::Direct { peer: Some(peer) };
            tokio::spawn(serve_connection(app.clone(), config, socket, path));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_serialise_as_the_data_model_says() {
        let direct = RunnerPath::Direct {
            peer: Some("203.0.113.4:51234".parse().unwrap()),
        };
        assert_eq!(
            direct.to_json(),
            json!({"kind": "direct", "peer": "203.0.113.4:51234"})
        );
        assert_eq!(direct.relay(), None);
        assert_eq!(
            RunnerPath::Direct { peer: None }.to_json(),
            json!({"kind": "direct", "peer": null})
        );
        let relay = Uuid::nil();
        let claimed = RunnerPath::Relay {
            relay_id: relay,
            advisory_peer: Some("x".repeat(500)),
        };
        assert_eq!(claimed.relay(), Some(relay));
        let v = claimed.to_json();
        assert_eq!(v["kind"], "relay");
        assert_eq!(v["relay"], relay.to_string());
        assert_eq!(v["peer"].as_str().unwrap().len(), MAX_ADVISORY_PEER);
        // A claimed peer cannot forge log lines.
        let shown = RunnerPath::Relay {
            relay_id: relay,
            advisory_peer: Some("1.2.3.4:5\nfake".into()),
        }
        .to_string();
        assert!(!shown.contains('\n'), "{shown}");
    }

    /// The item (`fn`, `enum`, `struct`) a line belongs to: the last one declared above it.
    fn item_name(line: &str) -> Option<String> {
        let mut t = line.trim_start();
        for prefix in ["pub(crate) ", "pub ", "const ", "async "] {
            t = t.strip_prefix(prefix).unwrap_or(t);
        }
        let rest = ["fn ", "enum ", "struct "]
            .iter()
            .find_map(|kw| t.strip_prefix(kw))?;
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        (!name.is_empty()).then_some(name)
    }

    /// Spec 067 T062: the relay-claimed `advisory_peer` is display and audit only. A grep over
    /// the crate's source: outside tests it appears only where it is made (the relay link),
    /// kept (`RunnerPath`) or shown (`to_json`, `Display`). Anything deciding access must use
    /// the authenticated identity instead; a new use has to be added here on purpose.
    #[test]
    fn advisory_peer_is_read_only_for_display_and_audit() {
        let allowed: &[(&str, &str)] = &[
            ("relays.rs", "advisory_peer"),
            ("relays.rs", "admit"),
            ("relays.rs", "carry"),
            ("runner_listener.rs", "RunnerPath"),
            ("runner_listener.rs", "to_json"),
            ("runner_listener.rs", "fmt"),
        ];
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut dirs = vec![src];
        let mut uses = Vec::new();
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let file = path.file_name().unwrap().to_string_lossy().into_owned();
                let text = std::fs::read_to_string(&path).unwrap();
                let mut item = String::new();
                for line in text.lines() {
                    if line.trim_start().starts_with("mod tests") {
                        break;
                    }
                    if let Some(name) = item_name(line) {
                        item = name;
                    }
                    let code = line.split("//").next().unwrap_or("");
                    if code.contains("advisory_peer") {
                        uses.push((file.clone(), item.clone()));
                    }
                }
            }
        }
        assert!(
            uses.iter().any(|(f, i)| f == "relays.rs" && i == "carry"),
            "the scan found nothing: {uses:?}"
        );
        for (file, item) in &uses {
            assert!(
                allowed.iter().any(|(f, i)| f == file && i == item),
                "advisory_peer used in {file} `{item}`: it is relay-claimed, display only"
            );
        }
    }
}
