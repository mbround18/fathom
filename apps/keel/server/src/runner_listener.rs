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

use std::collections::BTreeSet;
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
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::hosts::{self, HOST_CERT_TTL, token_fingerprint, token_hash};
use crate::serve::AppState;

/// How long a fresh connection has to send its first frame.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(30);
/// A renew-only connection closes after this long idle.
const RENEW_IDLE: Duration = Duration::from_secs(60);
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

/// [`SubsystemService`]: enrollment, renewal, and runner sessions.
pub struct Service {
    app: AppState,
}

impl SubsystemService for Service {
    async fn serve(&self, _access: Access, _stream: russh::ChannelStream<Msg>) {
        // Without the authenticated key nothing can be checked: drop the stream.
    }

    async fn serve_with_key(
        &self,
        access: Access,
        key: KeyData,
        stream: russh::ChannelStream<Msg>,
    ) {
        match access {
            Access::EnrollOnly => enroll(&self.app, &key, stream).await,
            Access::RenewOnly { host_id } => {
                if let Ok(host) = Uuid::parse_str(&host_id) {
                    renew_only(&self.app, host, stream).await;
                }
            }
            Access::Runner(identity) => {
                let Ok(host) = Uuid::parse_str(&identity.runner_id) else {
                    return;
                };
                if let Err(e) = crate::queue::attach(self.app.clone(), host, stream).await {
                    tracing::warn!(host = %host, error = %e, "runner session refused");
                }
            }
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

/// An unknown key's `keel-enroll-v1` stream: one `Enroll`, answered `Enrolled` or closed.
pub async fn enroll<S: AsyncRead + AsyncWrite + Unpin>(
    app: &AppState,
    key: &KeyData,
    mut stream: S,
) {
    let frame =
        match tokio::time::timeout(FIRST_FRAME_TIMEOUT, codec::read_frame(&mut stream)).await {
            Ok(Ok(Some(f))) => f,
            _ => return,
        };
    if !Access::EnrollOnly.permits(&frame) {
        return;
    }
    let Frame::Enroll {
        token,
        inventory_name,
        allowed_profiles,
        require_signed,
    } = frame
    else {
        return;
    };
    let req = EnrollRequest {
        token: token.expose(),
        inventory_name: &inventory_name,
        allowed_profiles: &allowed_profiles,
        require_signed,
    };
    match enroll_key(app, key, &req).await {
        Ok((host_id, cert)) => {
            let _ = codec::write_frame(
                &mut stream,
                &Frame::Enrolled {
                    host_id: host_id.to_string(),
                    cert,
                },
            )
            .await;
        }
        Err(Ok(reason)) => {
            tracing::info!(reason = reason.as_str(), "enrollment refused");
        }
        Err(Err(e)) => tracing::error!(error = %e, "enrollment failed"),
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
            Ok(()) => Ok((host, cert)),
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

/// Serves runners on `listener` until the task is dropped.
///
/// # Errors
///
/// The host key could not be generated.
pub fn spawn(app: AppState, listener: TcpListener) -> Result<tokio::task::JoinHandle<()>, String> {
    let key =
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).map_err(|e| e.to_string())?;
    let identity = Mutex::new(HostIdentity { key, cert: None });
    let authority = Arc::new(Authority { app: app.clone() });
    let service = Arc::new(Service { app: app.clone() });
    Ok(tokio::spawn(async move {
        loop {
            let socket = match listener.accept().await {
                Ok((s, _)) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "runner listener accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let config = identity.lock().ok().and_then(|mut i| i.config(&app));
            let Some(config) = config else {
                // No CAs: refuse by closing.
                continue;
            };
            let handler = KeelSshHandler::new(Arc::clone(&authority), Arc::clone(&service));
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    }))
}
