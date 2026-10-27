//! The audit log (spec 055 T022, research R10): an insert-only, hash-chained `audit_event` table.
//!
//! [`record`] writes one event inside the caller's transaction, so the event commits or rolls back
//! with the change it describes. Events are chained in `seq` order:
//! `hash = SHA-256(prev_hash ‖ canonical row)`, with 32 zero bytes before the first event. The
//! canonical row is the event's fields as JSON with every object's keys sorted and no whitespace.
//! A transaction-scoped advisory lock serialises writers so the chain never forks.
//!
//! [`verify`] walks the chain and reports the first event whose link or hash does not match.
//!
//! Never put secrets, session ids, tokens, passkey challenges or recovery codes in an event.

use keel_core::WorkspaceId;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

/// Key of the advisory lock that serialises audit writers ("keel-aud").
const LOCK_KEY: i64 = 0x6b65_656c_2d61_7564;

/// The hash before the first event.
pub const GENESIS: [u8; 32] = [0; 32];

/// Who did it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    User {
        id: Uuid,
        issuer: String,
        subject: String,
        display_name: String,
    },
    System,
    Receiver(Uuid),
    Host(Uuid),
}

impl Actor {
    /// The stable key stored in `actor` and indexed.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Actor::User { id, .. } => format!("user:{id}"),
            Actor::System => "system".into(),
            Actor::Receiver(id) => format!("receiver:{id}"),
            Actor::Host(id) => format!("host:{id}"),
        }
    }

    fn detail(&self) -> Value {
        match self {
            Actor::User {
                issuer,
                subject,
                display_name,
                ..
            } => {
                json!({"issuer": issuer, "subject": subject, "display_name": display_name})
            }
            _ => json!({}),
        }
    }
}

/// Whether the action was allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    Denied,
}

impl Decision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Decision::Allowed => "allowed",
            Decision::Denied => "denied",
        }
    }
}

/// One event to record.
#[derive(Debug, Clone)]
pub struct Entry<'a> {
    pub actor: &'a Actor,
    pub action: &'a str,
    pub workspace: Option<WorkspaceId>,
    pub target: Value,
    pub decision: Decision,
    pub reason: Option<&'a str>,
    pub context: Value,
}

/// The recorded event's identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub id: Uuid,
    pub hash: [u8; 32],
}

/// Records `entry` in the caller's transaction.
pub async fn record(tx: &mut PgConnection, entry: Entry<'_>) -> Result<Recorded, sqlx::Error> {
    sqlx::query("select pg_advisory_xact_lock($1)")
        .bind(LOCK_KEY)
        .execute(&mut *tx)
        .await?;
    let prev: Option<Vec<u8>> =
        sqlx::query_scalar("select hash from audit_event order by seq desc limit 1")
            .fetch_optional(&mut *tx)
            .await?;
    let prev = match prev {
        None => GENESIS,
        Some(bytes) => to_hash(&bytes)
            .ok_or_else(|| sqlx::Error::Protocol("audit_event.hash is not 32 bytes".into()))?,
    };

    let id = Uuid::now_v7();
    let at = truncate_micros(OffsetDateTime::now_utc());
    let row = Canonical {
        id,
        at,
        actor: entry.actor.key(),
        actor_detail: entry.actor.detail(),
        action: entry.action.to_owned(),
        workspace_id: entry.workspace.map(|w| w.0),
        target: entry.target,
        decision: entry.decision.as_str().to_owned(),
        reason: entry.reason.map(str::to_owned),
        context: entry.context,
    };
    let hash = chain_hash(&prev, &row);

    sqlx::query(
        "insert into audit_event \
         (id, at, actor, actor_detail, action, workspace_id, target, decision, reason, context, prev_hash, hash) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(row.id)
    .bind(row.at)
    .bind(&row.actor)
    .bind(&row.actor_detail)
    .bind(&row.action)
    .bind(row.workspace_id)
    .bind(&row.target)
    .bind(&row.decision)
    .bind(&row.reason)
    .bind(&row.context)
    .bind(prev.as_slice())
    .bind(hash.as_slice())
    .execute(&mut *tx)
    .await?;
    Ok(Recorded { id, hash })
}

/// The fields that are hashed, in the shape they are stored.
#[derive(Debug, Clone)]
pub struct Canonical {
    pub id: Uuid,
    pub at: OffsetDateTime,
    pub actor: String,
    pub actor_detail: Value,
    pub action: String,
    pub workspace_id: Option<Uuid>,
    pub target: Value,
    pub decision: String,
    pub reason: Option<String>,
    pub context: Value,
}

impl Canonical {
    /// The canonical bytes: sorted keys, no whitespace.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let at = self.at.format(&Rfc3339).unwrap_or_default();
        let mut obj = Map::new();
        obj.insert("id".into(), Value::String(self.id.to_string()));
        obj.insert("at".into(), Value::String(at));
        obj.insert("actor".into(), Value::String(self.actor.clone()));
        obj.insert("actor_detail".into(), self.actor_detail.clone());
        obj.insert("action".into(), Value::String(self.action.clone()));
        obj.insert(
            "workspace_id".into(),
            self.workspace_id
                .map_or(Value::Null, |w| Value::String(w.to_string())),
        );
        obj.insert("target".into(), self.target.clone());
        obj.insert("decision".into(), Value::String(self.decision.clone()));
        obj.insert(
            "reason".into(),
            self.reason.clone().map_or(Value::Null, Value::String),
        );
        obj.insert("context".into(), self.context.clone());
        let mut out = String::new();
        write_canonical(&Value::Object(obj), &mut out);
        out.into_bytes()
    }
}

/// `SHA-256(prev ‖ canonical row)`.
#[must_use]
pub fn chain_hash(prev: &[u8; 32], row: &Canonical) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(prev);
    h.update(row.bytes());
    h.finalize().into()
}

/// Writes `v` as JSON with every object's keys sorted, whatever map order serde_json uses.
fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                write_canonical(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// PostgreSQL keeps microseconds; hash what is stored.
fn truncate_micros(t: OffsetDateTime) -> OffsetDateTime {
    let micros = t.nanosecond() / 1_000 * 1_000;
    t.replace_nanosecond(micros).unwrap_or(t)
}

fn to_hash(bytes: &[u8]) -> Option<[u8; 32]> {
    bytes.try_into().ok()
}

/// What [`verify`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Events checked.
    pub checked: u64,
    /// The first broken event, if any.
    pub broken: Option<Broken>,
}

/// A broken link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broken {
    pub seq: i64,
    pub id: Uuid,
    pub why: &'static str,
}

/// Walks the whole chain in `seq` order.
pub async fn verify(conn: &mut PgConnection) -> Result<Report, sqlx::Error> {
    const PAGE: i64 = 1000;
    let mut prev = GENESIS;
    let mut after = 0_i64;
    let mut checked = 0_u64;
    loop {
        let rows = sqlx::query(
            "select seq, id, at, actor, actor_detail, action, workspace_id, target, decision, reason, \
             context, prev_hash, hash from audit_event where seq > $1 order by seq limit $2",
        )
        .bind(after)
        .bind(PAGE)
        .fetch_all(&mut *conn)
        .await?;
        if rows.is_empty() {
            return Ok(Report {
                checked,
                broken: None,
            });
        }
        for row in &rows {
            let seq: i64 = row.try_get("seq")?;
            let row_prev: Vec<u8> = row.try_get("prev_hash")?;
            let row_hash: Vec<u8> = row.try_get("hash")?;
            let c = Canonical {
                id: row.try_get("id")?,
                at: row.try_get("at")?,
                actor: row.try_get("actor")?,
                actor_detail: row.try_get("actor_detail")?,
                action: row.try_get("action")?,
                workspace_id: row.try_get("workspace_id")?,
                target: row.try_get("target")?,
                decision: row.try_get("decision")?,
                reason: row.try_get("reason")?,
                context: row.try_get("context")?,
            };
            let broken = |why| Report {
                checked,
                broken: Some(Broken { seq, id: c.id, why }),
            };
            if row_prev.as_slice() != prev.as_slice() {
                return Ok(broken("prev_hash does not match the previous event"));
            }
            let want = chain_hash(&prev, &c);
            if row_hash.as_slice() != want.as_slice() {
                return Ok(broken("hash does not match the event's contents"));
            }
            prev = want;
            after = seq;
            checked += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Canonical {
        Canonical {
            id: Uuid::from_u128(1),
            at: OffsetDateTime::UNIX_EPOCH,
            actor: "system".into(),
            actor_detail: json!({}),
            action: "test".into(),
            workspace_id: None,
            target: json!({"b": 1, "a": [{"z": true, "y": null}]}),
            decision: "allowed".into(),
            reason: None,
            context: json!({}),
        }
    }

    #[test]
    fn canonical_bytes_sort_keys_recursively() {
        let text = String::from_utf8(row().bytes()).unwrap();
        assert!(
            text.starts_with(
                "{\"action\":\"test\",\"actor\":\"system\",\"actor_detail\":{},\"at\":"
            )
        );
        assert!(
            text.contains("\"target\":{\"a\":[{\"y\":null,\"z\":true}],\"b\":1}"),
            "{text}"
        );
        assert!(!text.contains(' '));
    }

    #[test]
    fn hash_depends_on_prev_and_contents() {
        let a = chain_hash(&GENESIS, &row());
        assert_eq!(a, chain_hash(&GENESIS, &row()));
        assert_ne!(a, chain_hash(&[1; 32], &row()));
        let mut edited = row();
        edited.decision = "denied".into();
        assert_ne!(a, chain_hash(&GENESIS, &edited));
    }

    #[test]
    fn actor_keys() {
        let id = Uuid::from_u128(5);
        assert_eq!(Actor::Host(id).key(), format!("host:{id}"));
        assert_eq!(Actor::System.key(), "system");
    }
}
