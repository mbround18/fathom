//! Passkey commands (spec 055 T028, the 054 shapes): registration, verification, removal and
//! recovery, and the step-up verifier `Confirm` uses.
//!
//! - `crates/web-passkeys` runs the ceremonies; passkeys and recovery codes live in PostgreSQL
//!   ([`PgStore`]). Ceremony state stays in memory: single-use, bound to the session and its
//!   purpose, and void after two minutes. Anything else is refused as `expired`.
//! - The first passkey registers without a check; later ones need a check within the step-up
//!   window, or a session that redeemed a recovery code. Registering or verifying marks the
//!   session verified.
//! - Removing a passkey (a step-up preview) and redeeming a recovery code bump the user's
//!   `authz_epoch`, so every other session ends; the session that did it is carried forward.
//! - Every refusal and every passkey change is audited. Credentials, challenges, recovery codes
//!   and session ids never are, and are never logged.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use keel_core::ports::Clock;
use keel_protocol::{
    AssertionCredential, AttestationCredential, B64Url, CeremonyId, Event, PasskeyAssertion,
    PasskeyCredential, PasskeyInfo, PasskeyPurpose, PasskeyRefusedReason, PasskeyRequest,
    PasskeyResponse, RecoveryCode,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sqlx::types::Json;
use sqlx::{PgConnection, PgExecutor, PgPool};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use web_passkeys::recovery::{self, RecoveryError, RecoveryRecord};
use web_passkeys::{Ceremonies, CeremonyError, PasskeyStore, PasskeyUser, StoredPasskey};
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, WebauthnError,
};

use crate::audit::{self, Actor, Decision, Entry};
use crate::config::Config;
use crate::dispatch::{PendingOp, Request};
use crate::identity::{self, Caller};
use crate::preview::{ConfirmError, Draft, StepUpVerifier};
use crate::serve::AppState;

/// How long a ceremony can be answered.
pub const CEREMONY_TTL: Duration = Duration::minutes(2);

/// The most open ceremonies one session may hold; beginning another drops its oldest.
pub const MAX_CEREMONIES_PER_SESSION: usize = 8;

/// The relying-party name browsers show.
const RP_NAME: &str = "Keel";

fn rfc3339(t: OffsetDateTime) -> Result<String, sqlx::Error> {
    t.format(&Rfc3339)
        .map_err(|e| sqlx::Error::Protocol(format!("timestamp: {e}")))
}

fn parse_rfc3339(s: &str) -> Result<OffsetDateTime, sqlx::Error> {
    OffsetDateTime::parse(s, &Rfc3339).map_err(|e| sqlx::Error::Protocol(format!("timestamp: {e}")))
}

fn user_id(user: &str) -> Result<Uuid, sqlx::Error> {
    Uuid::parse_str(user).map_err(|_| sqlx::Error::Protocol("passkey user is not a uuid".into()))
}

type PasskeyRow = (
    String,
    String,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Json<Passkey>,
);

/// A user's passkeys, oldest first.
async fn stored<'e, E: PgExecutor<'e>>(
    e: E,
    user: Uuid,
) -> Result<Vec<StoredPasskey>, sqlx::Error> {
    let rows: Vec<PasskeyRow> = sqlx::query_as(
        "select id, name, created_at, last_used_at, credential from passkeys \
         where user_id = $1 order by created_at, id",
    )
    .bind(user)
    .fetch_all(e)
    .await?;
    rows.into_iter()
        .map(|(id, name, created, last_used, credential)| {
            Ok(StoredPasskey {
                id,
                name,
                created: rfc3339(created)?,
                last_used: last_used.map(rfc3339).transpose()?,
                credential: credential.0,
            })
        })
        .collect()
}

fn info(p: &StoredPasskey) -> PasskeyInfo {
    PasskeyInfo {
        id: p.id.clone(),
        name: p.name.clone(),
        created: p.created.clone(),
        last_used: p.last_used.clone(),
    }
}

/// Whether `user` has at least one passkey.
///
/// # Errors
///
/// A database error.
pub async fn enrolled(pool: &PgPool, user: Uuid) -> Result<bool, sqlx::Error> {
    let (enrolled,): (bool,) =
        sqlx::query_as("select exists (select 1 from passkeys where user_id = $1)")
            .bind(user)
            .fetch_one(pool)
            .await?;
    Ok(enrolled)
}

/// Passkeys and recovery codes in PostgreSQL. Users are keyed by their id as a string.
#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PasskeyStore for PgStore {
    type Error = sqlx::Error;

    async fn passkeys(&self, user: &str) -> Result<Vec<StoredPasskey>, Self::Error> {
        stored(&self.pool, user_id(user)?).await
    }

    async fn add_passkey(&self, user: &str, passkey: StoredPasskey) -> Result<(), Self::Error> {
        sqlx::query(
            "insert into passkeys (id, user_id, name, credential, created_at, last_used_at) \
             values ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&passkey.id)
        .bind(user_id(user)?)
        .bind(&passkey.name)
        .bind(Json(&passkey.credential))
        .bind(parse_rfc3339(&passkey.created)?)
        .bind(
            passkey
                .last_used
                .as_deref()
                .map(parse_rfc3339)
                .transpose()?,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_passkey(&self, user: &str, passkey: &StoredPasskey) -> Result<(), Self::Error> {
        sqlx::query(
            "update passkeys set credential = $3, last_used_at = $4 \
             where user_id = $1 and id = $2",
        )
        .bind(user_id(user)?)
        .bind(&passkey.id)
        .bind(Json(&passkey.credential))
        .bind(
            passkey
                .last_used
                .as_deref()
                .map(parse_rfc3339)
                .transpose()?,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove_passkey(&self, user: &str, id: &str) -> Result<bool, Self::Error> {
        let done = sqlx::query("delete from passkeys where user_id = $1 and id = $2")
            .bind(user_id(user)?)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn remove_all_passkeys(&self, user: &str) -> Result<usize, Self::Error> {
        let done = sqlx::query("delete from passkeys where user_id = $1")
            .bind(user_id(user)?)
            .execute(&self.pool)
            .await?;
        Ok(usize::try_from(done.rows_affected()).unwrap_or(usize::MAX))
    }

    async fn put_recovery(&self, record: RecoveryRecord) -> Result<(), Self::Error> {
        sqlx::query(
            "insert into passkey_recovery (user_id, code_sha256, expires_at, used_at) \
             values ($1, decode($2, 'hex'), to_timestamp($3::bigint), case when $4 then now() end) \
             on conflict (user_id) do update set code_sha256 = excluded.code_sha256, \
               expires_at = excluded.expires_at, used_at = excluded.used_at",
        )
        .bind(user_id(&record.user)?)
        .bind(&record.code_sha256)
        .bind(record.expires)
        .bind(record.used)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn recovery(&self, user: &str) -> Result<Option<RecoveryRecord>, Self::Error> {
        let row: Option<(String, i64, bool)> = sqlx::query_as(
            "select encode(code_sha256, 'hex'), extract(epoch from expires_at)::bigint, \
               used_at is not null from passkey_recovery where user_id = $1",
        )
        .bind(user_id(user)?)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(code_sha256, expires, used)| RecoveryRecord {
            user: user.to_owned(),
            code_sha256,
            expires,
            used,
        }))
    }

    async fn consume_recovery(&self, user: &str, code_sha256: &str) -> Result<bool, Self::Error> {
        let done = sqlx::query(
            "update passkey_recovery set used_at = now() \
             where user_id = $1 and code_sha256 = decode($2, 'hex') and used_at is null",
        )
        .bind(user_id(user)?)
        .bind(code_sha256)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }
}

enum Kind {
    Register {
        name: String,
        state: PasskeyRegistration,
    },
    Verify {
        state: PasskeyAuthentication,
    },
    /// An assertion over a fixed challenge (an override or baseline, spec 055 US8). The
    /// challenge is Keel's own 32 bytes, never a random one; it is never logged.
    Bound {
        challenge: Vec<u8>,
    },
}

struct Pending {
    session: String,
    user: Uuid,
    actor: Actor,
    created: OffsetDateTime,
    kind: Kind,
}

/// Why finishing a ceremony did not work.
enum Fail {
    Refused(PasskeyRefusedReason),
    /// The database or the relying party failed; logged, never shown.
    Internal,
}

impl From<PasskeyRefusedReason> for Fail {
    fn from(r: PasskeyRefusedReason) -> Self {
        Fail::Refused(r)
    }
}

impl From<sqlx::Error> for Fail {
    fn from(e: sqlx::Error) -> Self {
        tracing::error!(error = %e, "passkey database error");
        Fail::Internal
    }
}

/// The refusal a failed ceremony maps to, or `None` for a server-side failure.
fn reason_of<E: std::error::Error>(e: &CeremonyError<E>) -> Option<PasskeyRefusedReason> {
    Some(match e {
        CeremonyError::Webauthn(w) => match w {
            WebauthnError::InvalidRPOrigin
            | WebauthnError::InvalidRPIDHash
            | WebauthnError::CredentialCrossOrigin => PasskeyRefusedReason::WrongOrigin,
            WebauthnError::CredentialPossibleCompromise
            | WebauthnError::CredentialCounterUpdateFailure => PasskeyRefusedReason::Replayed,
            WebauthnError::CredentialNotFound => PasskeyRefusedReason::UnknownCredential,
            _ => PasskeyRefusedReason::Invalid,
        },
        CeremonyError::UnknownCredential => PasskeyRefusedReason::UnknownCredential,
        CeremonyError::InvalidName
        | CeremonyError::UnsupportedKey
        | CeremonyError::Assertion(_) => PasskeyRefusedReason::Invalid,
        CeremonyError::Config | CeremonyError::Store(_) => return None,
    })
}

fn fail_of<E: std::error::Error>(e: &CeremonyError<E>) -> Fail {
    if let Some(r) = reason_of(e) {
        return Fail::Refused(r);
    }
    tracing::error!(error = %e, "passkey ceremony failed");
    Fail::Internal
}

/// A browser credential as webauthn-rs takes it. Transport hints and extension results are
/// dropped first: nothing is decided on them, and an unknown transport would not parse.
fn to_webauthn<T: DeserializeOwned>(credential: &impl Serialize) -> Option<T> {
    let mut v = serde_json::to_value(credential).ok()?;
    let obj = v.as_object_mut()?;
    obj.remove("extensions");
    if let Some(r) = obj.get_mut("response").and_then(Value::as_object_mut) {
        r.remove("transports");
    }
    serde_json::from_value(v).ok()
}

/// The inner `publicKey` options of a challenge, as the browser API takes them.
fn public_key(challenge: &impl Serialize) -> Option<Value> {
    serde_json::to_value(challenge)
        .ok()?
        .get_mut("publicKey")
        .map(Value::take)
}

/// The passkey service: the relying party and the open ceremonies.
pub struct Passkeys {
    /// `None` when `--public-url` cannot be a relying party; every ceremony is then refused.
    ceremonies: Option<Ceremonies>,
    db: PgPool,
    clock: Arc<dyn Clock>,
    pending: Mutex<HashMap<CeremonyId, Pending>>,
}

impl Passkeys {
    /// A relying party for `--public-url` (its host is the RP id, its origin the only one).
    #[must_use]
    pub fn new(cfg: &Config, db: PgPool, clock: Arc<dyn Clock>) -> Self {
        let ceremonies = cfg.public_url.host_str().and_then(|host| {
            Ceremonies::new(host, &cfg.origin(), RP_NAME)
                .inspect_err(|e| tracing::warn!(error = %e, "passkeys unavailable"))
                .ok()
        });
        Self {
            ceremonies,
            db,
            clock,
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<CeremonyId, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn open(&self, p: Pending) -> CeremonyId {
        let id = CeremonyId(Uuid::now_v7());
        let now = self.clock.now();
        let mut map = self.lock();
        map.retain(|_, c| now - c.created < CEREMONY_TTL);
        let mut mine: Vec<(CeremonyId, OffsetDateTime)> = map
            .iter()
            .filter(|(_, c)| c.session == p.session)
            .map(|(k, c)| (*k, c.created))
            .collect();
        if mine.len() >= MAX_CEREMONIES_PER_SESSION {
            // v7 ids sort by creation, which breaks ties between equal clock readings.
            mine.sort_by_key(|(k, created)| (*created, k.0));
            for (old, _) in mine
                .iter()
                .take(mine.len() + 1 - MAX_CEREMONIES_PER_SESSION)
            {
                map.remove(old);
            }
        }
        map.insert(id, p);
        id
    }

    /// Takes the ceremony: it must be this session's and live. Taking it uses it up.
    fn take(&self, session: &str, id: CeremonyId) -> Result<Pending, PasskeyRefusedReason> {
        let now = self.clock.now();
        let mut map = self.lock();
        // Another session's ceremony is left alone, so it cannot be spoiled from outside.
        if map.get(&id).is_none_or(|p| p.session != session) {
            return Err(PasskeyRefusedReason::Expired);
        }
        let p = map.remove(&id).ok_or(PasskeyRefusedReason::Expired)?;
        if now - p.created >= CEREMONY_TTL {
            return Err(PasskeyRefusedReason::Expired);
        }
        Ok(p)
    }

    /// Drops every ceremony of `session` (sign-out).
    pub fn forget_session(&self, session: &str) {
        self.lock().retain(|_, p| p.session != session);
    }

    /// Checks an assertion for a `verify` ceremony; on success the session is verified and the
    /// check is audited.
    async fn verify_assertion(
        &self,
        session: &str,
        p: Pending,
        credential: &AssertionCredential,
    ) -> Result<(), Fail> {
        let Kind::Verify { state } = &p.kind else {
            return Err(PasskeyRefusedReason::Expired.into());
        };
        let Some(ceremonies) = &self.ceremonies else {
            return Err(Fail::Internal);
        };
        let response: PublicKeyCredential =
            to_webauthn(credential).ok_or(PasskeyRefusedReason::Invalid)?;
        let now = self.clock.now();
        let result = ceremonies
            .finish_authentication(
                &PgStore::new(self.db.clone()),
                &p.user.to_string(),
                &response,
                state,
                &rfc3339(now)?,
            )
            .await
            .map_err(|e| fail_of(&e))?;
        let mut tx = self.db.begin().await?;
        sqlx::query("update sessions set verified_at = $2 where id = $1 and user_id = $3")
            .bind(session)
            .bind(now)
            .bind(p.user)
            .execute(&mut *tx)
            .await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &p.actor,
                action: "passkey_verified",
                workspace: None,
                target: json!({ "id": web_passkeys::ceremony::credential_id_string(result.cred_id()) }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({}),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// The parts of a bound assertion a host verifies (contracts/runner.md `Approvals`), base64url.
/// Public material only: the authenticator signs, the private key never leaves it.
pub(crate) struct BoundAssertion {
    pub credential_id: String,
    pub authenticator_data: String,
    pub client_data_json: String,
    pub signature: String,
}

impl Passkeys {
    /// Opens a ceremony whose challenge is `challenge` (spec 055 US8: the override challenge),
    /// for `caller`'s passkeys. `Err` carries the events to reply with.
    pub(crate) async fn bound_begin(
        &self,
        session: &str,
        caller: &Caller,
        challenge: &[u8],
    ) -> Result<PasskeyRequest, Vec<Event>> {
        let Some(ceremonies) = &self.ceremonies else {
            return Err(vec![Event::op_error(
                "passkeys_unavailable",
                "Passkeys need an https public URL.",
            )]);
        };
        let existing = stored(&self.db, caller.user.id).await.map_err(|e| {
            tracing::error!(error = %e, "load passkeys failed");
            internal()
        })?;
        if existing.is_empty() {
            return Err(vec![Event::PasskeyEnrolmentRequired]);
        }
        let options = ceremonies
            .bound_request(&existing, challenge)
            .ok()
            .and_then(|r| public_key(&r))
            .ok_or_else(|| {
                tracing::error!("start a bound passkey assertion failed");
                internal()
            })?;
        let ceremony = self.open(Pending {
            session: session.to_owned(),
            user: caller.user.id,
            actor: caller.actor(),
            created: self.clock.now(),
            kind: Kind::Bound {
                challenge: challenge.to_vec(),
            },
        });
        Ok(PasskeyRequest { ceremony, options })
    }

    /// Checks an answer to a [`Self::bound_begin`] ceremony: it must be this session's, live,
    /// `user`'s, opened over exactly `challenge`, and signed by one of `user`'s passkeys with
    /// user verification. Spends the ceremony. Fails closed.
    pub(crate) async fn verify_bound(
        &self,
        session: &str,
        user: Uuid,
        assertion: &PasskeyAssertion,
        challenge: &[u8],
    ) -> Result<BoundAssertion, PasskeyRefusedReason> {
        let p = self.take(session, assertion.ceremony)?;
        let Kind::Bound { challenge: opened } = &p.kind else {
            return Err(PasskeyRefusedReason::Expired);
        };
        if p.user != user || opened.as_slice() != challenge {
            return Err(PasskeyRefusedReason::Expired);
        }
        let ceremonies = self
            .ceremonies
            .as_ref()
            .ok_or(PasskeyRefusedReason::Invalid)?;
        let existing = stored(&self.db, user).await.map_err(|e| {
            tracing::error!(error = %e, "load passkeys failed");
            PasskeyRefusedReason::Invalid
        })?;
        let response: PublicKeyCredential =
            to_webauthn(&assertion.credential).ok_or(PasskeyRefusedReason::Invalid)?;
        let used = ceremonies
            .verify_bound_assertion(&existing, &response, challenge)
            .map_err(|e| reason_of(&e).unwrap_or(PasskeyRefusedReason::Invalid))?;
        let text = |v: &B64Url| -> Result<String, PasskeyRefusedReason> {
            serde_json::to_value(v)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .ok_or(PasskeyRefusedReason::Invalid)
        };
        let r = &assertion.credential.response;
        Ok(BoundAssertion {
            credential_id: used.id.clone(),
            authenticator_data: text(&r.authenticator_data)?,
            client_data_json: text(&r.client_data_json)?,
            signature: text(&r.signature)?,
        })
    }
}

impl StepUpVerifier for Passkeys {
    fn verify<'a>(
        &'a self,
        session: &'a str,
        assertion: &'a PasskeyAssertion,
    ) -> Pin<Box<dyn Future<Output = Result<(), PasskeyRefusedReason>> + Send + 'a>> {
        Box::pin(async move {
            let p = self.take(session, assertion.ceremony)?;
            match self
                .verify_assertion(session, p, &assertion.credential)
                .await
            {
                Ok(()) => Ok(()),
                Err(Fail::Refused(r)) => Err(r),
                // Fail closed.
                Err(Fail::Internal) => Err(PasskeyRefusedReason::Invalid),
            }
        })
    }
}

fn window(app: &AppState) -> Duration {
    Duration::try_from(app.cfg.step_up_window).unwrap_or(Duration::minutes(5))
}

fn internal() -> Vec<Event> {
    vec![Event::op_error(
        "internal",
        "Keel could not complete that. Try again.",
    )]
}

fn step_up_required() -> Event {
    Event::op_error(
        "step_up_required",
        "Verify with a passkey you already have first.",
    )
}

fn context(req: &Request<'_>) -> Value {
    json!({ "channel": req.channel.as_str() })
}

/// Audits a denial; a failed write is logged, never shown to the caller.
async fn audit_denied(app: &AppState, actor: &Actor, action: &str, reason: &str, context: Value) {
    let result = async {
        let mut tx = app.db.begin().await?;
        audit::record(
            &mut tx,
            Entry {
                actor,
                action,
                workspace: None,
                target: json!({}),
                decision: Decision::Denied,
                reason: Some(reason),
                context,
            },
        )
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "audit write failed");
    }
}

/// Audits the refusal under `action` and replies `PasskeyRefused`.
pub async fn refuse(
    app: &AppState,
    actor: &Actor,
    action: &str,
    reason: PasskeyRefusedReason,
    context: Value,
) -> Vec<Event> {
    audit_denied(app, actor, action, reason.as_str(), context).await;
    vec![Event::PasskeyRefused { reason }]
}

/// The session handle, which every passkey command needs (`authorize` refuses tokens).
fn session<'a>(req: &Request<'a>) -> Option<&'a str> {
    req.session.filter(|_| req.caller.token.is_none())
}

fn session_required(action: &str) -> Vec<Event> {
    vec![Event::Denied {
        action: action.to_owned(),
        reason: "session_required".to_owned(),
    }]
}

/// `Verification` for `caller` as of now.
async fn verification(app: &AppState, caller: &Caller) -> Vec<Event> {
    match identity::verification(&app.db, caller, window(app), app.clock.now()).await {
        Ok(v) => vec![Event::Verification(v)],
        Err(e) => {
            tracing::error!(error = %e, "verification failed");
            internal()
        }
    }
}

/// `GetVerification`.
pub async fn get_verification(app: &AppState, req: &Request<'_>) -> Vec<Event> {
    verification(app, req.caller).await
}

/// `ListPasskeys`.
pub async fn list(app: &AppState, caller: &Caller) -> Vec<Event> {
    match stored(&app.db, caller.user.id).await {
        Ok(items) => vec![Event::Passkeys {
            items: items.iter().map(info).collect(),
        }],
        Err(e) => {
            tracing::error!(error = %e, "list passkeys failed");
            internal()
        }
    }
}

/// `PasskeyBegin`.
pub async fn begin(app: &AppState, req: &Request<'_>, purpose: PasskeyPurpose) -> Vec<Event> {
    let Some(session) = session(req) else {
        return session_required("passkey_begin");
    };
    let Some(ceremonies) = &app.passkeys.ceremonies else {
        return vec![Event::op_error(
            "passkeys_unavailable",
            "Passkeys need an https public URL.",
        )];
    };
    let caller = req.caller;
    let existing = match stored(&app.db, caller.user.id).await {
        Ok(existing) => existing,
        Err(e) => {
            tracing::error!(error = %e, "load passkeys failed");
            return internal();
        }
    };
    let (kind, options) = match purpose {
        PasskeyPurpose::Register { name } => {
            let recent = identity::is_recent(caller.verified_at, window(app), app.clock.now());
            if !(existing.is_empty() || recent || caller.recovering) {
                audit_denied(
                    app,
                    &caller.actor(),
                    "passkey_begin",
                    "step_up_required",
                    context(req),
                )
                .await;
                return vec![step_up_required()];
            }
            let user = PasskeyUser {
                handle: caller.user.id,
                name: caller.user.display_name.clone(),
                display_name: caller.user.display_name.clone(),
            };
            let Ok((challenge, state)) = ceremonies.start_registration(&user, &existing) else {
                tracing::error!("start passkey registration failed");
                return internal();
            };
            let kind = Kind::Register {
                name: name.as_str().to_owned(),
                state,
            };
            (kind, public_key(&challenge))
        }
        PasskeyPurpose::Verify => {
            if existing.is_empty() {
                return vec![Event::PasskeyEnrolmentRequired];
            }
            let Ok((challenge, state)) = ceremonies.start_authentication(&existing) else {
                tracing::error!("start passkey authentication failed");
                return internal();
            };
            (Kind::Verify { state }, public_key(&challenge))
        }
    };
    let Some(options) = options else {
        tracing::error!("passkey options did not serialise");
        return internal();
    };
    let ceremony = app.passkeys.open(Pending {
        session: session.to_owned(),
        user: caller.user.id,
        actor: caller.actor(),
        created: app.clock.now(),
        kind,
    });
    vec![Event::PasskeyChallenge(PasskeyRequest {
        ceremony,
        options,
    })]
}

/// `PasskeyFinish`.
pub async fn finish(app: &AppState, req: &Request<'_>, response: PasskeyResponse) -> Vec<Event> {
    const ACTION: &str = "passkey_finish";
    let Some(session) = session(req) else {
        return session_required(ACTION);
    };
    let actor = req.caller.actor();
    let p = match app.passkeys.take(session, response.ceremony) {
        Ok(p) if p.user == req.caller.user.id => p,
        Ok(_) => {
            return refuse(
                app,
                &actor,
                ACTION,
                PasskeyRefusedReason::Expired,
                context(req),
            )
            .await;
        }
        Err(r) => return refuse(app, &actor, ACTION, r, context(req)).await,
    };
    let result = match (&p.kind, &response.credential) {
        (Kind::Register { .. }, PasskeyCredential::Attestation(c)) => {
            register(app, req, session, p, c).await
        }
        (Kind::Verify { .. }, PasskeyCredential::Assertion(c)) => {
            match app.passkeys.verify_assertion(session, p, c).await {
                Ok(()) => {
                    let mut caller = req.caller.clone();
                    caller.verified_at = Some(app.clock.now());
                    Ok(verification(app, &caller).await)
                }
                Err(f) => Err(f),
            }
        }
        // An answer of the wrong kind spends the ceremony.
        _ => Err(PasskeyRefusedReason::Expired.into()),
    };
    match result {
        Ok(events) => events,
        Err(Fail::Refused(r)) => refuse(app, &actor, ACTION, r, context(req)).await,
        Err(Fail::Internal) => internal(),
    }
}

/// Finishes a registration: re-checks, inside a transaction holding the user, that this
/// session may add a passkey, stores it, verifies the session and ends its recovery.
async fn register(
    app: &AppState,
    req: &Request<'_>,
    session: &str,
    p: Pending,
    credential: &AttestationCredential,
) -> Result<Vec<Event>, Fail> {
    let Kind::Register { name, state } = &p.kind else {
        return Err(PasskeyRefusedReason::Expired.into());
    };
    let Some(ceremonies) = &app.passkeys.ceremonies else {
        return Err(Fail::Internal);
    };
    let response: RegisterPublicKeyCredential =
        to_webauthn(credential).ok_or(PasskeyRefusedReason::Invalid)?;
    let now = app.clock.now();
    let mut tx = app.db.begin().await?;
    lock_user(&mut tx, p.user).await?;
    let Some((verified_at, recovering)) = sqlx::query_as::<_, (Option<OffsetDateTime>, bool)>(
        "select verified_at, recovering from sessions where id = $1 and user_id = $2 for update",
    )
    .bind(session)
    .bind(p.user)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Err(PasskeyRefusedReason::Expired.into());
    };
    let existing = stored(&mut *tx, p.user).await?;
    if !(existing.is_empty() || recovering || identity::is_recent(verified_at, window(app), now)) {
        drop(tx);
        audit_denied(
            app,
            &p.actor,
            "passkey_finish",
            "step_up_required",
            context(req),
        )
        .await;
        return Ok(vec![step_up_required()]);
    }
    let passkey = ceremonies
        .finish_registration(&response, state, name, &existing, &rfc3339(now)?)
        .map_err(|e| fail_of(&e))?;
    let created: Option<(OffsetDateTime,)> = sqlx::query_as(
        "insert into passkeys (id, user_id, name, credential, created_at) \
         values ($1, $2, $3, $4, $5) on conflict do nothing returning created_at",
    )
    .bind(&passkey.id)
    .bind(p.user)
    .bind(&passkey.name)
    .bind(Json(&passkey.credential))
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    // The same credential registered twice, or a name taken concurrently.
    let Some((created,)) = created else {
        return Err(PasskeyRefusedReason::Invalid.into());
    };
    sqlx::query("update sessions set verified_at = $2, recovering = false where id = $1")
        .bind(session)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        Entry {
            actor: &p.actor,
            action: "passkey_registered",
            workspace: None,
            target: json!({ "id": passkey.id, "name": passkey.name }),
            decision: Decision::Allowed,
            reason: None,
            context: context(req),
        },
    )
    .await?;
    tx.commit().await?;
    let registered = PasskeyInfo {
        id: passkey.id,
        name: passkey.name,
        created: rfc3339(created)?,
        last_used: None,
    };
    let mut caller = req.caller.clone();
    caller.verified_at = Some(now);
    caller.recovering = false;
    let mut events = vec![Event::PasskeyRegistered(registered)];
    events.extend(verification(app, &caller).await);
    Ok(events)
}

/// Serialises passkey changes for one user (so "not the last one" holds when it is removed).
async fn lock_user(conn: &mut PgConnection, user: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("select 1 from users where id = $1 for update")
        .bind(user)
        .execute(conn)
        .await?;
    Ok(())
}

/// `PreviewPasskeyRemove`: a step-up preview; confirming it removes the passkey and ends the
/// person's other sessions.
pub async fn preview_remove(app: &AppState, req: &Request<'_>, id: B64Url) -> Vec<Event> {
    const ACTION: &str = "preview_passkey_remove";
    let Some(session) = session(req) else {
        return session_required(ACTION);
    };
    let caller = req.caller;
    let existing = match stored(&app.db, caller.user.id).await {
        Ok(existing) => existing,
        Err(e) => {
            tracing::error!(error = %e, "load passkeys failed");
            return internal();
        }
    };
    let Some(target) = existing.iter().find(|p| p.id == id.as_str()) else {
        let reason = PasskeyRefusedReason::UnknownCredential;
        return refuse(app, &caller.actor(), ACTION, reason, context(req)).await;
    };
    if existing.len() <= 1 {
        let reason = PasskeyRefusedReason::LastPasskey;
        return refuse(app, &caller.actor(), ACTION, reason, context(req)).await;
    }
    let name = target.name.clone();
    let (passkey, owner) = (target.id.clone(), session.to_owned());
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move { remove(&app, &caller, &owner, &passkey).await })
    });
    let draft = Draft {
        action: "passkey_remove".to_owned(),
        summary: format!("Remove the passkey \"{name}\". Your other sessions end."),
        affected: vec![name],
        step_up: true,
    };
    let key = caller.preview_key(Some(session));
    vec![Event::Preview(app.previews.create(&key, draft, op))]
}

/// Removes a passkey (a confirmed preview): never the last one. Bumps the epoch, so the
/// person's other sessions end; this one is carried forward.
async fn remove(app: &AppState, caller: &Caller, session: &str, id: &str) -> Vec<Event> {
    const ACTION: &str = "passkey_remove";
    let user = caller.user.id;
    let result: Result<Result<(), PasskeyRefusedReason>, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        lock_user(&mut tx, user).await?;
        let existing = stored(&mut *tx, user).await?;
        let Some(target) = existing.iter().find(|p| p.id == id) else {
            return Ok(Err(PasskeyRefusedReason::UnknownCredential));
        };
        if existing.len() <= 1 {
            return Ok(Err(PasskeyRefusedReason::LastPasskey));
        }
        sqlx::query("delete from passkeys where user_id = $1 and id = $2")
            .bind(user)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &caller.actor(),
                action: "passkey_removed",
                workspace: None,
                target: json!({ "id": target.id, "name": target.name }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({}),
            },
        )
        .await?;
        carry_forward(&mut tx, user, session, false).await?;
        tx.commit().await?;
        Ok(Ok(()))
    }
    .await;
    match result {
        Ok(Ok(())) => {
            app.epoch_changed(user);
            list(app, caller).await
        }
        Ok(Err(reason)) => refuse(app, &caller.actor(), ACTION, reason, json!({})).await,
        Err(e) => {
            tracing::error!(error = %e, "remove passkey failed");
            internal()
        }
    }
}

/// Bumps `user`'s epoch and moves `session` to it, so only this session survives. A
/// recovering session also loses its check until it enrols again.
async fn carry_forward(
    conn: &mut PgConnection,
    user: Uuid,
    session: &str,
    recovering: bool,
) -> Result<(), sqlx::Error> {
    let epoch = identity::bump_epoch(&mut *conn, user).await?;
    let q = if recovering {
        "update sessions set authz_epoch = $3, recovering = true, verified_at = null \
         where id = $1 and user_id = $2"
    } else {
        "update sessions set authz_epoch = $3 where id = $1 and user_id = $2"
    };
    sqlx::query(q)
        .bind(session)
        .bind(user)
        .bind(epoch)
        .execute(conn)
        .await?;
    Ok(())
}

/// `RecoverPasskeys`: redeems the person's one-time code, removes every passkey, ends the other
/// sessions and lets this one enrol without a check.
pub async fn recover(app: &AppState, req: &Request<'_>, code: &RecoveryCode) -> Vec<Event> {
    const ACTION: &str = "recover_passkeys";
    let Some(session) = session(req) else {
        return session_required(ACTION);
    };
    let caller = req.caller;
    let user = caller.user.id;
    let store = PgStore::new(app.db.clone());
    let now = app.clock.now().unix_timestamp();
    match recovery::redeem(&store, &user.to_string(), code.as_str(), now).await {
        Ok(()) => {}
        Err(RecoveryError::Invalid) => {
            let reason = PasskeyRefusedReason::RecoveryInvalid;
            return refuse(app, &caller.actor(), ACTION, reason, context(req)).await;
        }
        Err(e) => {
            tracing::error!(error = %e, "recovery failed");
            return internal();
        }
    }
    let result: Result<(), sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        lock_user(&mut tx, user).await?;
        let removed = sqlx::query("delete from passkeys where user_id = $1")
            .bind(user)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        audit::record(
            &mut tx,
            Entry {
                actor: &caller.actor(),
                action: "passkey_recovered",
                workspace: None,
                target: json!({ "removed": removed }),
                decision: Decision::Allowed,
                reason: None,
                context: context(req),
            },
        )
        .await?;
        carry_forward(&mut tx, user, session, true).await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "recovery failed");
        return internal();
    }
    app.epoch_changed(user);
    let mut caller = caller.clone();
    caller.verified_at = None;
    caller.recovering = true;
    verification(app, &caller).await
}

/// The reply to a refused `Confirm`, when passkeys have a better answer than `OpError`: the
/// assertion's refusal (audited), or enrolment when there is nothing to verify with.
pub async fn confirm_refused(
    app: &AppState,
    req: &Request<'_>,
    e: &ConfirmError,
) -> Option<Vec<Event>> {
    match e {
        ConfirmError::StepUpFailed(reason) => {
            Some(refuse(app, &req.caller.actor(), "confirm", *reason, context(req)).await)
        }
        ConfirmError::StepUpRequired => match enrolled(&app.db, req.caller.user.id).await {
            Ok(false) => Some(vec![Event::PasskeyEnrolmentRequired]),
            Ok(true) => None,
            Err(e) => {
                tracing::error!(error = %e, "load passkeys failed");
                Some(internal())
            }
        },
        ConfirmError::Unknown | ConfirmError::Expired => None,
    }
}
