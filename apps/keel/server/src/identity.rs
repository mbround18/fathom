//! Users and sessions in PostgreSQL (spec 055 T025/T026).
//!
//! - Signing in upserts the user by `(issuer, subject)`. Without a role mapping the first user on
//!   an empty instance becomes `owner`; everyone after that is a `user`. A disabled user cannot
//!   sign in.
//! - With an OIDC role mapping (spec 060, US5; research R7) the instance role is recomputed from
//!   the provider's groups at every OIDC sign-in, the highest match winning. A change bumps the
//!   epoch and is audited with reason `identity_provider`; the last active owner is never
//!   demoted (reason `idp_names_no_owner`).
//! - A session row carries the user's `authz_epoch` at sign-in. It stays valid only while that
//!   still equals `users.authz_epoch`, the user is not disabled and it has not expired, so a
//!   narrowing change (which bumps the epoch) ends every session at its next command.
//! - Session ids here are web-auth's handles (the SHA-256 of the cookie), never the cookie.
//!   Neither is ever logged or audited.

use std::fmt;

use keel_core::authz::Grant;
use keel_core::{InstanceRole, TeamRole, WorkspaceId};
use keel_protocol::{Me, MeGrant, Verification};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::config::RoleMapping;

/// Key of the advisory lock that serialises sign-ins, so two first sign-ins cannot both become
/// owner ("keel-usr").
pub const SIGN_IN_LOCK: i64 = 0x6b65_656c_2d75_7372;

/// Who signed in, as the identity provider vouched for them.
#[derive(Debug, Clone)]
pub struct SignInIdentity<'a> {
    pub provider: &'a str,
    pub issuer: &'a str,
    pub subject: &'a str,
    pub display_name: &'a str,
    pub email: Option<&'a str>,
    /// The provider's groups or roles (OIDC: the mapped claim), read only by a role mapping.
    pub groups: &'a [String],
}

/// A user row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    pub issuer: String,
    pub subject: String,
    pub display_name: String,
    pub email: Option<String>,
    pub instance_role: InstanceRole,
    pub authz_epoch: i64,
}

impl User {
    #[must_use]
    pub fn actor(&self) -> Actor {
        Actor::User {
            id: self.id,
            issuer: self.issuer.clone(),
            subject: self.subject.clone(),
            display_name: self.display_name.clone(),
        }
    }
}

/// Why a sign-in was refused.
#[derive(Debug)]
pub enum SignInError {
    /// The account is disabled. The refusal is audited.
    Disabled,
    /// A role mapping requires a match and the identity has none (or came through another
    /// provider). Nothing is written; the caller audits `role_not_mapped`.
    NotMapped,
    Db(sqlx::Error),
}

impl fmt::Display for SignInError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignInError::Disabled => f.write_str("the account is disabled"),
            SignInError::NotMapped => f.write_str("no mapped instance role"),
            SignInError::Db(e) => write!(f, "database: {e}"),
        }
    }
}

impl From<sqlx::Error> for SignInError {
    fn from(e: sqlx::Error) -> Self {
        SignInError::Db(e)
    }
}

type UserRow = (
    Uuid,
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    Option<OffsetDateTime>,
);

fn role_of(s: &str) -> Result<InstanceRole, sqlx::Error> {
    s.parse()
        .map_err(|e: keel_core::domain::ParseError| sqlx::Error::Decode(Box::new(e)))
}

fn user_of(r: UserRow) -> Result<(User, Option<OffsetDateTime>), sqlx::Error> {
    Ok((
        User {
            id: r.0,
            issuer: r.1,
            subject: r.2,
            display_name: r.3,
            email: r.4,
            instance_role: role_of(&r.5)?,
            authz_epoch: r.6,
        },
        r.7,
    ))
}

/// What a role mapping decides for one sign-in (research R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mapped {
    /// No mapping: first-user-is-owner, then users.
    Off,
    /// An OIDC sign-in: this role, applied on create and on every sign-in.
    Role(InstanceRole),
    /// Another provider while a mapping is active: new accounts are users, existing ones keep
    /// their role.
    Unmanaged,
}

fn mapped(who: &SignInIdentity<'_>, mapping: Option<&RoleMapping>) -> Result<Mapped, SignInError> {
    let Some(m) = mapping else {
        return Ok(Mapped::Off);
    };
    if who.provider != "oidc" {
        return if m.required {
            Err(SignInError::NotMapped)
        } else {
            Ok(Mapped::Unmanaged)
        };
    }
    match m.role_for(who.groups) {
        Some(role) => Ok(Mapped::Role(role)),
        None if m.required => Err(SignInError::NotMapped),
        None => Ok(Mapped::Role(InstanceRole::User)),
    }
}

/// Records a sign-in: creates or refreshes the user and audits it, in one transaction.
///
/// `context` goes into the audit row (provider, client address); it must hold no secrets.
/// `mapping` is the OIDC role mapping, when one is configured. Returns the user and whether
/// the sign-in bumped their epoch (a mapped role change): the caller then tells the sockets.
///
/// # Errors
///
/// [`SignInError::Disabled`] for a disabled account (audited `denied`),
/// [`SignInError::NotMapped`] when the mapping refuses the identity (nothing written), or a
/// database error.
pub async fn sign_in(
    pool: &PgPool,
    who: &SignInIdentity<'_>,
    mapping: Option<&RoleMapping>,
    context: Value,
) -> Result<(User, bool), SignInError> {
    let mapped = mapped(who, mapping)?;
    let mut bumped = false;
    let mut tx = pool.begin().await?;
    sqlx::query("select pg_advisory_xact_lock($1)")
        .bind(SIGN_IN_LOCK)
        .execute(&mut *tx)
        .await?;
    let existing: Option<UserRow> = sqlx::query_as(
        "select id, issuer, subject, display_name, email, instance_role, authz_epoch, disabled_at \
         from users where issuer = $1 and subject = $2",
    )
    .bind(who.issuer)
    .bind(who.subject)
    .fetch_optional(&mut *tx)
    .await?;

    let user = match existing {
        Some(row) => {
            let (mut user, disabled_at) = user_of(row)?;
            if disabled_at.is_some() {
                audit::record(
                    &mut tx,
                    Entry {
                        actor: &user.actor(),
                        action: "sign_in",
                        workspace: None,
                        target: json!({}),
                        decision: Decision::Denied,
                        reason: Some("account_disabled"),
                        context,
                    },
                )
                .await?;
                tx.commit().await?;
                return Err(SignInError::Disabled);
            }
            let email = who.email.map(str::to_owned);
            if user.display_name != who.display_name || user.email != email {
                sqlx::query("update users set display_name = $2, email = $3 where id = $1")
                    .bind(user.id)
                    .bind(who.display_name)
                    .bind(&email)
                    .execute(&mut *tx)
                    .await?;
                who.display_name.clone_into(&mut user.display_name);
                user.email = email;
            }
            if let Mapped::Role(role) = mapped {
                bumped = apply_mapped_role(&mut tx, &mut user, role, &context).await?;
            }
            user
        }
        None => {
            let (count,): (i64,) = sqlx::query_as("select count(*) from users")
                .fetch_one(&mut *tx)
                .await?;
            let (role, reason) = match mapped {
                Mapped::Off if count == 0 => (InstanceRole::Owner, Some("first_user_is_owner")),
                Mapped::Off | Mapped::Unmanaged => (InstanceRole::User, None),
                Mapped::Role(role) => (role, Some("identity_provider")),
            };
            let idp_role = matches!(mapped, Mapped::Role(_)).then_some(role.as_str());
            let user = User {
                id: Uuid::now_v7(),
                issuer: who.issuer.to_owned(),
                subject: who.subject.to_owned(),
                display_name: who.display_name.to_owned(),
                email: who.email.map(str::to_owned),
                instance_role: role,
                authz_epoch: 1,
            };
            sqlx::query(
                "insert into users (id, issuer, subject, display_name, email, instance_role, \
                 authz_epoch, role_source, idp_role) values ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(user.id)
            .bind(&user.issuer)
            .bind(&user.subject)
            .bind(&user.display_name)
            .bind(&user.email)
            .bind(role.as_str())
            .bind(user.authz_epoch)
            .bind(if idp_role.is_some() { "idp" } else { "keel" })
            .bind(idp_role)
            .execute(&mut *tx)
            .await?;
            audit::record(
                &mut tx,
                Entry {
                    actor: &user.actor(),
                    action: "user_create",
                    workspace: None,
                    target: json!({ "user": user.id, "instance_role": role.as_str() }),
                    decision: Decision::Allowed,
                    reason,
                    context: context.clone(),
                },
            )
            .await?;
            user
        }
    };
    audit::record(
        &mut tx,
        Entry {
            actor: &user.actor(),
            action: "sign_in",
            workspace: None,
            target: json!({}),
            decision: Decision::Allowed,
            reason: None,
            context,
        },
    )
    .await?;
    tx.commit().await?;
    Ok((user, bumped))
}

/// Applies the role the mapping produced to an existing, enabled user, inside the sign-in
/// transaction (research R7):
///
/// - The user is marked `role_source = 'idp'` and `idp_role` records `role`.
/// - A changed role bumps the epoch, ending sessions issued under the old one (FR-027), and is
///   audited as `user_role` with reason `identity_provider`.
/// - Demoting the last active owner keeps them owner instead, audited with reason
///   `idp_names_no_owner` (FR-028).
///
/// Returns whether the epoch was bumped.
async fn apply_mapped_role(
    tx: &mut PgConnection,
    user: &mut User,
    role: InstanceRole,
    context: &Value,
) -> Result<bool, sqlx::Error> {
    let from = user.instance_role;
    let mut to = role;
    let mut reason = "identity_provider";
    if from == InstanceRole::Owner && role != InstanceRole::Owner {
        // Every sign-in holds SIGN_IN_LOCK, but role changes in governance don't: lock the
        // owner rows so the count can't move under us.
        let owners: Vec<Uuid> = sqlx::query_scalar(
            "select id from users where instance_role = 'owner' and disabled_at is null \
             order by id for update",
        )
        .fetch_all(&mut *tx)
        .await?;
        if owners.len() <= 1 {
            to = InstanceRole::Owner;
            reason = "idp_names_no_owner";
        }
    }
    sqlx::query(
        "update users set instance_role = $2, role_source = 'idp', idp_role = $3 \
         where id = $1 and (instance_role <> $2 or role_source <> 'idp' \
         or idp_role is distinct from $3)",
    )
    .bind(user.id)
    .bind(to.as_str())
    .bind(role.as_str())
    .execute(&mut *tx)
    .await?;
    if from != to {
        user.instance_role = to;
        user.authz_epoch = bump_epoch(&mut *tx, user.id).await?;
    }
    if from != to || to != role {
        audit::record(
            &mut *tx,
            Entry {
                actor: &user.actor(),
                action: "user_role",
                workspace: None,
                target: json!({
                    "user": user.id, "from": from.as_str(), "to": to.as_str(),
                    "mapped": role.as_str(), "epoch_bumped": from != to,
                }),
                decision: Decision::Allowed,
                reason: Some(reason),
                context: context.clone(),
            },
        )
        .await?;
    }
    Ok(from != to)
}

/// Whether no active user's last mapped role is owner: System Settings warns "the identity
/// provider names no owner" while a mapping is active and this holds (research R7). Callers
/// check that a mapping is active ([`crate::serve::AppState::idp_names_no_owner`] does).
///
/// # Errors
///
/// A database error.
pub async fn idp_names_no_owner(conn: impl sqlx::PgExecutor<'_>) -> Result<bool, sqlx::Error> {
    let named: bool = sqlx::query_scalar(
        "select exists (select 1 from users where idp_role = 'owner' and disabled_at is null)",
    )
    .fetch_one(conn)
    .await?;
    Ok(!named)
}

/// Stores a session row for `user` at its current epoch.
///
/// # Errors
///
/// A database error.
pub async fn open_session(
    pool: &PgPool,
    session: &str,
    user: &User,
    expires_at: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into sessions (id, user_id, authz_epoch, expires_at) values ($1, $2, $3, $4)",
    )
    .bind(session)
    .bind(user.id)
    .bind(user.authz_epoch)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Deletes a session row (sign-out, revocation).
///
/// # Errors
///
/// A database error.
pub async fn close_session(pool: &PgPool, session: &str) -> Result<(), sqlx::Error> {
    sqlx::query("delete from sessions where id = $1")
        .bind(session)
        .execute(pool)
        .await?;
    Ok(())
}

/// Increments a user's `authz_epoch` (any narrowing change), inside the caller's transaction.
/// After commit, tell the sockets with [`crate::serve::AppState::epoch_changed`].
///
/// # Errors
///
/// A database error.
pub async fn bump_epoch(conn: &mut PgConnection, user: Uuid) -> Result<i64, sqlx::Error> {
    let (epoch,): (i64,) = sqlx::query_as(
        "update users set authz_epoch = authz_epoch + 1 where id = $1 returning authz_epoch",
    )
    .bind(user)
    .fetch_one(conn)
    .await?;
    Ok(epoch)
}

/// What an API token narrows its owner to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenScope {
    pub id: Uuid,
    pub workspace: WorkspaceId,
    /// Command names (`Cmd::name`) the token may run.
    pub actions: Vec<String>,
}

/// Who is asking, re-read from the database for every command.
#[derive(Debug, Clone)]
pub struct Caller {
    pub user: User,
    /// A passkey check within the step-up window counts from here (sessions only).
    pub verified_at: Option<OffsetDateTime>,
    /// The session redeemed a recovery code and has not enrolled a passkey since.
    pub recovering: bool,
    pub grants: Vec<Grant>,
    /// Set when the caller is an API token rather than a session.
    pub token: Option<TokenScope>,
    /// The owner-only System workspace as of this command (spec 060), from
    /// `AppState::system_workspace`. The loaders leave it `None`; the transports fill it.
    pub system_workspace: Option<WorkspaceId>,
}

impl Caller {
    #[must_use]
    pub fn actor(&self) -> Actor {
        self.user.actor()
    }

    /// The key previews are bound to: the session handle, or the token.
    #[must_use]
    pub fn preview_key(&self, session: Option<&str>) -> String {
        match (&self.token, session) {
            (Some(t), _) => format!("token:{}", t.id),
            (None, Some(s)) => s.to_owned(),
            (None, None) => String::new(),
        }
    }
}

async fn grants_of(pool: &PgPool, user: Uuid) -> Result<Vec<(Grant, String)>, sqlx::Error> {
    let rows: Vec<(Uuid, String, String, Vec<String>)> = sqlx::query_as(
        "select g.workspace_id, w.slug, g.role, g.profiles from grants g \
         join team_members m on m.team_id = g.team_id \
         join workspaces w on w.id = g.workspace_id \
         where m.user_id = $1 and w.archived_at is null \
         order by w.slug, g.role",
    )
    .bind(user)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(ws, slug, role, profiles)| {
            let role: TeamRole = role
                .parse()
                .map_err(|e: keel_core::domain::ParseError| sqlx::Error::Decode(Box::new(e)))?;
            Ok((
                Grant {
                    workspace: WorkspaceId(ws),
                    role,
                    profiles,
                },
                slug,
            ))
        })
        .collect()
}

/// A [`UserRow`] followed by `sessions.verified_at` and `sessions.recovering`.
type SessionRow = (
    Uuid,
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
    bool,
);

/// Token id, workspace and actions, then the owner's [`UserRow`].
type TokenRow = (
    Uuid,
    Uuid,
    Vec<String>,
    Uuid,
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    Option<OffsetDateTime>,
);

/// The caller behind a session, or `None` when the session is gone, expired, stale (its epoch
/// is behind the user's) or the user is disabled. Touches `last_seen_at`.
///
/// # Errors
///
/// A database error.
pub async fn load_session(
    pool: &PgPool,
    session: &str,
    now: OffsetDateTime,
) -> Result<Option<Caller>, sqlx::Error> {
    let row: Option<SessionRow> = sqlx::query_as(
        "update sessions s set last_seen_at = $2 from users u \
         where s.id = $1 and u.id = s.user_id and s.authz_epoch = u.authz_epoch \
           and u.disabled_at is null and s.expires_at > $2 \
         returning u.id, u.issuer, u.subject, u.display_name, u.email, u.instance_role, \
           u.authz_epoch, u.disabled_at, s.verified_at, s.recovering",
    )
    .bind(session)
    .bind(now)
    .fetch_optional(pool)
    .await?;
    let Some(r) = row else {
        return Ok(None);
    };
    let (verified_at, recovering) = (r.8, r.9);
    let (user, _) = user_of((r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7))?;
    let grants = grants_of(pool, user.id)
        .await?
        .into_iter()
        .map(|(g, _)| g)
        .collect();
    Ok(Some(Caller {
        user,
        verified_at,
        recovering,
        grants,
        token: None,
        system_workspace: None,
    }))
}

/// The SHA-256 an API token is stored under.
#[must_use]
pub fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// The caller behind an API token (`Authorization: Bearer`), or `None` when it is unknown,
/// revoked, expired, or its owner is disabled. Touches `last_used_at`.
///
/// # Errors
///
/// A database error.
pub async fn load_token(
    pool: &PgPool,
    token: &str,
    now: OffsetDateTime,
) -> Result<Option<Caller>, sqlx::Error> {
    let row: Option<TokenRow> = sqlx::query_as(
        "update api_tokens t set last_used_at = $2 from users u \
         where t.hash = $1 and u.id = t.owner_user_id and t.revoked_at is null \
           and t.expires_at > $2 and u.disabled_at is null \
         returning t.id, t.workspace_id, t.actions, u.id, u.issuer, u.subject, u.display_name, \
           u.email, u.instance_role, u.authz_epoch, u.disabled_at",
    )
    .bind(token_hash(token).as_slice())
    .bind(now)
    .fetch_optional(pool)
    .await?;
    let Some(r) = row else {
        return Ok(None);
    };
    let scope = TokenScope {
        id: r.0,
        workspace: WorkspaceId(r.1),
        actions: r.2,
    };
    let (user, _) = user_of((r.3, r.4, r.5, r.6, r.7, r.8, r.9, r.10))?;
    // A token never reaches past its workspace, whatever its owner holds elsewhere.
    let grants = grants_of(pool, user.id)
        .await?
        .into_iter()
        .map(|(g, _)| g)
        .filter(|g| g.workspace == scope.workspace)
        .collect();
    Ok(Some(Caller {
        user,
        verified_at: None,
        recovering: false,
        grants,
        token: Some(scope),
        system_workspace: None,
    }))
}

/// `Me` for the caller.
///
/// # Errors
///
/// A database error.
pub async fn me(
    pool: &PgPool,
    caller: &Caller,
    window: Duration,
    now: OffsetDateTime,
) -> Result<Me, sqlx::Error> {
    let grants = grants_of(pool, caller.user.id)
        .await?
        .into_iter()
        .filter(|(g, _)| {
            caller
                .token
                .as_ref()
                .is_none_or(|t| t.workspace == g.workspace)
        })
        .map(|(g, slug)| MeGrant {
            workspace: g.workspace.0,
            workspace_slug: slug,
            role: g.role.into(),
            profiles: g.profiles,
        })
        .collect();
    Ok(Me {
        id: caller.user.id,
        display_name: caller.user.display_name.clone(),
        email: caller.user.email.clone(),
        instance_role: caller.user.instance_role.into(),
        grants,
        verification: verification(pool, caller, window, now).await?,
        // Filled by the update notice (spec 061 US1); empty until then.
        keel_version: None,
        update_dot_dismissed: None,
    })
}

/// Where the caller's session stands on passkeys.
///
/// # Errors
///
/// A database error.
pub async fn verification(
    pool: &PgPool,
    caller: &Caller,
    window: Duration,
    now: OffsetDateTime,
) -> Result<Verification, sqlx::Error> {
    let (enrolled,): (bool,) =
        sqlx::query_as("select exists (select 1 from passkeys where user_id = $1)")
            .bind(caller.user.id)
            .fetch_one(pool)
            .await?;
    Ok(Verification {
        verified: is_recent(caller.verified_at, window, now),
        at: caller.verified_at.and_then(|t| t.format(&Rfc3339).ok()),
        window_secs: u32::try_from(window.whole_seconds()).unwrap_or(u32::MAX),
        enrolled,
        // Every role verifies with a passkey before step-up actions (R11).
        required: true,
        recovering: caller.recovering,
    })
}

/// Whether a passkey check at `at` still covers `now`.
#[must_use]
pub fn is_recent(at: Option<OffsetDateTime>, window: Duration, now: OffsetDateTime) -> bool {
    at.is_some_and(|at| at <= now && now - at <= window)
}

/// The user signed in as `(issuer, subject)`, disabled or not.
///
/// # Errors
///
/// A database error.
pub async fn find_user(
    pool: &PgPool,
    issuer: &str,
    subject: &str,
) -> Result<Option<User>, sqlx::Error> {
    let row: Option<UserRow> = sqlx::query_as(
        "select id, issuer, subject, display_name, email, instance_role, authz_epoch, disabled_at \
         from users where issuer = $1 and subject = $2",
    )
    .bind(issuer)
    .bind(subject)
    .fetch_optional(pool)
    .await?;
    row.map(|r| user_of(r).map(|(u, _)| u)).transpose()
}
