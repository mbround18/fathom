//! Typed host queries (spec 067 US5, T042): read-only questions put to a host's runner over its
//! session, direct or relayed.
//!
//! - `AskHost` (workspace `Operate`) stores a `host_queries` row and sends `Query` to the
//!   host's runner. A host that is not connected gets `disconnected`, one whose runner did not
//!   advertise `query-v1` gets `no_capability`, both at once and without sending anything.
//! - The runner's `QueryAnswer` or `QueryRefused`, or Keel's timeout (15 s), or the session
//!   ending, settles the row once and pushes `HostQueryAnswered` to the workspace.
//! - `ListHostQueries` (View) lists a host's newest queries.
//!
//! The runner decides what it answers from its own `queries.toml`; Keel's parameter checks only
//! catch mistakes early. Every query is audited once, when it settles, as `host.query` with its
//! kind, params and outcome and the asker as actor; never the answer. The newest 200 rows per
//! host are kept, older ones deleted in the same transaction as each insert. Each settled query
//! is also a `host.query` span (kind, outcome).

use std::collections::BTreeMap;

use keel_core::WorkspaceId;
use keel_core::authz::Capability;
use keel_exec::frames::{CAP_QUERY_V1, Frame, MAX_QUERY_ANSWER};
use keel_protocol::{Cmd, Event, HostQuery, QueryKind, QueryOutcome};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::bindings::{db_failed, enum_from, enum_str, internal, not_found, op_error, require, ts};
use crate::dispatch::Request;
use crate::serve::AppState;

/// Rows kept per host.
pub const KEEP_PER_HOST: i64 = 200;
/// `ListHostQueries` without a limit.
const DEFAULT_LIST: u32 = 50;
/// The longest unit name (as the runner checks it).
const MAX_UNIT: usize = 256;
/// The longest path (as the runner checks it).
const MAX_PATH: usize = 4096;

/// Handles `AskHost` and `ListHostQueries`.
pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::AskHost { host, kind, params } => ask(app, req, host, kind, params).await,
        Cmd::ListHostQueries { host, limit } => list(app, req, host, limit).await,
        _ => internal(),
    }
}

/// The one parameter `kind` takes, if any.
#[must_use]
pub const fn param_of(kind: QueryKind) -> Option<&'static str> {
    match kind {
        QueryKind::UnitState => Some("unit"),
        QueryKind::PathFree => Some("path"),
        QueryKind::Facts | QueryKind::AppliedRevision | QueryKind::DiskHealth => None,
    }
}

/// A unit name as the runner accepts it: `^[A-Za-z0-9@._:-]{1,256}$`.
#[must_use]
pub fn valid_unit(unit: &str) -> bool {
    !unit.is_empty()
        && unit.len() <= MAX_UNIT
        && unit
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'@' | b'.' | b'_' | b':' | b'-'))
}

/// A path as the runner accepts it: absolute, normalised (no empty, `.` or `..` components, no
/// trailing slash but on `/`), no NUL, at most 4096 bytes.
#[must_use]
pub fn valid_path(path: &str) -> bool {
    if path == "/" {
        return true;
    }
    path.len() <= MAX_PATH
        && !path.contains('\0')
        && path.strip_prefix('/').is_some_and(|rest| {
            rest.split('/')
                .all(|c| !c.is_empty() && c != "." && c != "..")
        })
}

/// Keel's early check of `params` for `kind`: exactly the kind's one parameter, well formed.
fn check_params(kind: QueryKind, params: &BTreeMap<String, String>) -> Result<(), &'static str> {
    match param_of(kind) {
        None if params.is_empty() => Ok(()),
        None => Err("This query takes no parameters."),
        Some(key) => {
            let (Some(value), 1) = (params.get(key), params.len()) else {
                return Err(match key {
                    "unit" => "Give the unit to ask about, and nothing else.",
                    _ => "Give the path to ask about, and nothing else.",
                });
            };
            let ok = match key {
                "unit" => valid_unit(value),
                _ => valid_path(value),
            };
            if ok {
                Ok(())
            } else if key == "unit" {
                Err("That is not a systemd unit name.")
            } else {
                Err("Give an absolute path without `.`, `..` or a trailing slash.")
            }
        }
    }
}

async fn ask(
    app: &AppState,
    req: &Request<'_>,
    host: Uuid,
    kind: QueryKind,
    params: BTreeMap<String, String>,
) -> Vec<Event> {
    let workspace: Option<(Uuid,)> =
        match sqlx::query_as("select workspace_id from hosts where id = $1")
            .bind(host)
            .fetch_optional(&app.db)
            .await
        {
            Ok(r) => r,
            Err(e) => return db_failed(&e),
        };
    let Some((workspace,)) = workspace else {
        return not_found("host");
    };
    if let Err(d) = require(app, req, "ask_host", Capability::Operate, workspace).await {
        return d;
    }
    if let Err(why) = check_params(kind, &params) {
        return op_error("bad_params", why);
    }
    // Settled at once when there is no one to ask.
    let early = if app.runs.path(host).is_none() {
        Some(QueryOutcome::Disconnected)
    } else if !app.runs.has_capability(host, CAP_QUERY_V1) {
        Some(QueryOutcome::NoCapability)
    } else {
        None
    };
    let id = Uuid::now_v7();
    let caller = req.caller;
    let params_json = json!(params);
    let stored = async {
        let mut tx = app.db.begin().await?;
        let times: (OffsetDateTime, Option<OffsetDateTime>) = sqlx::query_as(
            "insert into host_queries (id, workspace_id, host_id, kind, params, asked_by, \
             outcome, answered_at) \
             values ($1, $2, $3, $4, $5, $6, $7, case when $7::text is null then null else now() end) \
             returning asked_at, answered_at",
        )
        .bind(id)
        .bind(workspace)
        .bind(host)
        .bind(enum_str(&kind))
        .bind(&params_json)
        .bind(caller.user.id)
        .bind(early.map(|o| enum_str(&o)))
        .fetch_one(&mut *tx)
        .await?;
        prune(&mut tx, host).await?;
        if let Some(outcome) = early {
            audit::record(
                &mut tx,
                audit_entry(&caller.actor(), workspace, host, id, kind, &params_json, outcome, None),
            )
            .await?;
        }
        tx.commit().await?;
        Ok::<_, sqlx::Error>(times)
    }
    .await;
    let (asked_at, answered_at) = match stored {
        Ok(t) => t,
        Err(e) => return db_failed(&e),
    };
    if let Some(outcome) = early {
        query_span(kind, outcome);
    }
    let mut query = HostQuery {
        id,
        host,
        kind,
        params: params_json,
        asked_by: caller.user.display_name.clone(),
        asked_at: ts(asked_at),
        outcome: early,
        answer: None,
        refusal: None,
        answered_at: answered_at.map(ts),
    };
    if early.is_none() {
        let frame = Frame::Query {
            query_id: id.to_string(),
            kind: enum_str(&kind),
            params,
            asked_by: caller.user.display_name.chars().take(256).collect(),
        };
        if app.runs.send(host, frame) {
            let app = app.clone();
            let timeout = app.runs.settings().query_timeout;
            tokio::spawn(async move {
                tokio::time::sleep(timeout).await;
                settle(&app, id, None, QueryOutcome::Timeout, None, None).await;
            });
        } else {
            // The session ended between the check and the send.
            settle(app, id, None, QueryOutcome::Disconnected, None, None).await;
            query.outcome = Some(QueryOutcome::Disconnected);
            query.answered_at = Some(ts(OffsetDateTime::now_utc()));
        }
    }
    vec![Event::HostQuery(query)]
}

/// Deletes all but the newest [`KEEP_PER_HOST`] rows of `host`.
async fn prune(tx: &mut sqlx::PgConnection, host: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "delete from host_queries where host_id = $1 and id not in \
         (select id from host_queries where host_id = $1 order by asked_at desc, id desc limit $2)",
    )
    .bind(host)
    .bind(KEEP_PER_HOST)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn audit_entry<'a>(
    actor: &'a Actor,
    workspace: Uuid,
    host: Uuid,
    id: Uuid,
    kind: QueryKind,
    params: &Value,
    outcome: QueryOutcome,
    refusal: Option<&str>,
) -> Entry<'a> {
    Entry {
        actor,
        action: "host.query",
        workspace: Some(WorkspaceId(workspace)),
        target: json!({ "host": host, "query": id }),
        decision: Decision::Allowed,
        reason: None,
        // Never the answer.
        context: json!({
            "kind": enum_str(&kind),
            "params": params,
            "outcome": enum_str(&outcome),
            "refusal": refusal,
        }),
    }
}

fn query_span(kind: QueryKind, outcome: QueryOutcome) {
    let span = tracing::info_span!(
        "host.query",
        host.query.kind = %enum_str(&kind),
        host.query.outcome = %enum_str(&outcome),
    );
    span.in_scope(|| tracing::debug!("host query settled"));
}

/// The runner's `QueryAnswer` or `QueryRefused` from `host`'s session. A reply for a query that
/// is not `host`'s, or already settled, is ignored.
pub async fn on_reply(app: &AppState, host: Uuid, frame: Frame) {
    let (query_id, outcome, answer, refusal) = match frame {
        Frame::QueryAnswer { query_id, answer } => {
            let size = serde_json::to_vec(&answer).map_or(usize::MAX, |v| v.len());
            if !answer.is_object() || size > MAX_QUERY_ANSWER {
                // A runner that breaks the contract gets its query settled as refused.
                let why = if answer.is_object() {
                    "too_large"
                } else {
                    "invalid_answer"
                };
                (query_id, QueryOutcome::Refused, None, Some(why.to_owned()))
            } else {
                (query_id, QueryOutcome::Answered, Some(answer), None)
            }
        }
        Frame::QueryRefused { query_id, reason } => (
            query_id,
            QueryOutcome::Refused,
            None,
            Some(reason.as_str().to_owned()),
        ),
        _ => return,
    };
    let Ok(id) = Uuid::parse_str(&query_id) else {
        return;
    };
    settle(app, id, Some(host), outcome, answer, refusal).await;
}

/// `host`'s session ended: once no session of it is left, its unsettled queries are
/// `disconnected`.
pub async fn session_ended(app: &AppState, host: Uuid) {
    if app.runs.path(host).is_some() {
        return;
    }
    let ids: Vec<(Uuid,)> =
        match sqlx::query_as("select id from host_queries where host_id = $1 and outcome is null")
            .bind(host)
            .fetch_all(&app.db)
            .await
        {
            Ok(ids) => ids,
            Err(e) => {
                tracing::error!(error = %e, "listing unsettled host queries failed");
                return;
            }
        };
    for (id,) in ids {
        settle(app, id, Some(host), QueryOutcome::Disconnected, None, None).await;
    }
}

/// Settles query `id` (only if it is `host`'s, when given, and not settled yet): stores the
/// outcome, audits it and pushes `HostQueryAnswered`.
async fn settle(
    app: &AppState,
    id: Uuid,
    host: Option<Uuid>,
    outcome: QueryOutcome,
    answer: Option<Value>,
    refusal: Option<String>,
) {
    let settled = async {
        let mut tx = app.db.begin().await?;
        let row: Option<Row> = sqlx::query_as(
            "update host_queries q set outcome = $3, answer = $4, refusal = $5, \
             answered_at = now() \
             from users u \
             where q.id = $1 and ($2::uuid is null or q.host_id = $2) and q.outcome is null \
             and u.id = q.asked_by \
             returning q.id, q.workspace_id, q.host_id, q.kind, q.params, q.asked_by, \
             u.issuer, u.subject, u.display_name, q.asked_at, q.outcome, q.answer, q.refusal, \
             q.answered_at",
        )
        .bind(id)
        .bind(host)
        .bind(enum_str(&outcome))
        .bind(&answer)
        .bind(
            refusal
                .as_deref()
                .map(|r| r.chars().take(64).collect::<String>()),
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let actor = Actor::User {
            id: row.asked_by,
            issuer: row.issuer.clone(),
            subject: row.subject.clone(),
            display_name: row.display_name.clone(),
        };
        let kind = enum_from::<QueryKind>(&row.kind).unwrap_or(QueryKind::Facts);
        audit::record(
            &mut tx,
            audit_entry(
                &actor,
                row.workspace_id,
                row.host_id,
                row.id,
                kind,
                &row.params,
                outcome,
                row.refusal.as_deref(),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok::<_, sqlx::Error>(Some(row))
    }
    .await;
    match settled {
        Ok(Some(row)) => {
            if let Some(q) = row.query() {
                query_span(q.kind, outcome);
                app.repos
                    .push(row.workspace_id, Event::HostQueryAnswered(q));
            }
        }
        Ok(None) => {}
        Err(e) => tracing::error!(error = %e, query = %id, "settling a host query failed"),
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    workspace_id: Uuid,
    host_id: Uuid,
    kind: String,
    params: Value,
    asked_by: Uuid,
    issuer: String,
    subject: String,
    display_name: String,
    asked_at: OffsetDateTime,
    outcome: Option<String>,
    answer: Option<Value>,
    refusal: Option<String>,
    answered_at: Option<OffsetDateTime>,
}

impl Row {
    fn query(&self) -> Option<HostQuery> {
        Some(HostQuery {
            id: self.id,
            host: self.host_id,
            kind: enum_from(&self.kind)?,
            params: self.params.clone(),
            asked_by: self.display_name.clone(),
            asked_at: ts(self.asked_at),
            outcome: self.outcome.as_deref().and_then(enum_from),
            answer: self.answer.clone(),
            refusal: self.refusal.clone(),
            answered_at: self.answered_at.map(ts),
        })
    }
}

async fn list(app: &AppState, req: &Request<'_>, host: Uuid, limit: Option<u32>) -> Vec<Event> {
    let workspace: Option<(Uuid,)> =
        match sqlx::query_as("select workspace_id from hosts where id = $1")
            .bind(host)
            .fetch_optional(&app.db)
            .await
        {
            Ok(r) => r,
            Err(e) => return db_failed(&e),
        };
    let Some((workspace,)) = workspace else {
        return not_found("host");
    };
    if let Err(d) = require(app, req, "list_host_queries", Capability::View, workspace).await {
        return d;
    }
    let limit = i64::from(limit.unwrap_or(DEFAULT_LIST).clamp(1, 200));
    let rows: Vec<Row> = match sqlx::query_as(
        "select q.id, q.workspace_id, q.host_id, q.kind, q.params, q.asked_by, u.issuer, \
         u.subject, u.display_name, q.asked_at, q.outcome, q.answer, q.refusal, q.answered_at \
         from host_queries q join users u on u.id = q.asked_by \
         where q.host_id = $1 order by q.asked_at desc, q.id desc limit $2",
    )
    .bind(host)
    .bind(limit)
    .fetch_all(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let query_v1 = app
        .runs
        .path(host)
        .map(|_| app.runs.has_capability(host, CAP_QUERY_V1));
    vec![Event::HostQueries {
        host,
        items: rows.iter().filter_map(Row::query).collect(),
        query_v1,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keel_checks_parameters_like_the_runner() {
        let p = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        assert!(check_params(QueryKind::Facts, &p(&[])).is_ok());
        assert!(check_params(QueryKind::Facts, &p(&[("unit", "x")])).is_err());
        assert!(check_params(QueryKind::UnitState, &p(&[("unit", "k3s.service")])).is_ok());
        assert!(check_params(QueryKind::UnitState, &p(&[("unit", "k3s service")])).is_err());
        assert!(check_params(QueryKind::UnitState, &p(&[("path", "/")])).is_err());
        assert!(
            check_params(
                QueryKind::UnitState,
                &p(&[("unit", "a.service"), ("path", "/")])
            )
            .is_err()
        );
        assert!(check_params(QueryKind::PathFree, &p(&[("path", "/")])).is_ok());
        assert!(check_params(QueryKind::PathFree, &p(&[("path", "/var/lib")])).is_ok());
        for bad in [
            "",
            "var",
            "/var/",
            "/var/../etc",
            "/var/./x",
            "/var//x",
            "/a\0b",
        ] {
            assert!(!valid_path(bad), "{bad:?}");
        }
        assert!(!valid_path(&format!("/{}", "a".repeat(4096))));
        assert!(valid_unit(&"a".repeat(256)));
        assert!(!valid_unit(&"a".repeat(257)));
        assert!(!valid_unit("a;b"));
    }
}
