//! Inventories (spec 055 US4, data-model.md "Inventory").
//!
//! [`pin_latest`] is the seam runs use (US2, T069): it picks the revision a new run is pinned to
//! and the hosts that revision lists. US4's commands (T058) live alongside it.

use keel_core::authz::Capability;
use keel_core::domain::WorkspaceId;
use keel_core::inventory::{self as core_inv, Checked, Format, InventoryIssue};
use keel_protocol::{
    Event, InventoryChange, InventoryChangeKind, InventoryHostInfo, InventoryInfo,
    InventoryProblem, InventoryRevisionInfo, InventoryRevisionSummary, InventoryText, ObjectName,
};
use serde_json::{Value, json};
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::audit::{self, Decision, Entry};
use crate::bindings::{channel, db_failed, enum_from, not_found, op_error, preview, require, ts};
use crate::dispatch::{PendingOp, Request};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::serve::AppState;

/// A host as one inventory revision lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryHost {
    pub host: String,
    pub groups: Vec<String>,
}

/// The revision a run is pinned to, and its hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    pub revision: Uuid,
    pub hosts: Vec<InventoryHost>,
}

/// Why no revision could be pinned. Each is a run `state_reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinRefusal {
    /// No inventory by that name in the workspace, or it has no revisions yet.
    NotFound,
    /// The inventory is deprecated and accepts no new runs.
    Deprecated,
    /// The newest revision did not validate (`inventory_invalid`).
    Invalid,
}

impl PinRefusal {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotFound => "inventory_not_found",
            Self::Deprecated => "inventory_deprecated",
            Self::Invalid => "inventory_invalid",
        }
    }
}

/// Pins the newest revision of `name` in `workspace`. An invalid newest revision is refused, not
/// skipped: running against an older one would hide the error.
///
/// # Errors
///
/// The database failed; a refusal is `Ok(Err(_))`.
pub async fn pin_latest(
    conn: &mut PgConnection,
    workspace: Uuid,
    name: &str,
) -> Result<Result<Pinned, PinRefusal>, sqlx::Error> {
    let row: Option<(String, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "select i.status, r.id, r.status from inventories i \
         left join lateral (select id, status from inventory_revisions \
           where inventory_id = i.id order by seq desc limit 1) r on true \
         where i.workspace_id = $1 and i.name = $2",
    )
    .bind(workspace)
    .bind(name)
    .fetch_optional(&mut *conn)
    .await?;
    let revision = match row {
        None | Some((_, None, _)) => return Ok(Err(PinRefusal::NotFound)),
        Some((status, _, _)) if status == "deprecated" => return Ok(Err(PinRefusal::Deprecated)),
        Some((_, Some(_), Some(rev_status))) if rev_status != "valid" => {
            return Ok(Err(PinRefusal::Invalid));
        }
        Some((_, Some(id), _)) => id,
    };
    let hosts: Vec<(String, Vec<String>)> = sqlx::query_as(
        "select host, groups from inventory_hosts where revision_id = $1 order by host",
    )
    .bind(revision)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Ok(Pinned {
        revision,
        hosts: hosts
            .into_iter()
            .map(|(host, groups)| InventoryHost { host, groups })
            .collect(),
    }))
}

// ---- Commands (spec 055 US4, T058) ----
//
// Reads need View on the inventory's workspace; create, edit, restore and deprecate need
// Administer. Revisions are immutable: every change inserts one under a lock on the inventory
// row, and a writer that lost the race answers `stale_base`.

/// One revision to insert.
pub(crate) struct NewRevision<'a> {
    pub checked: &'a Checked,
    pub source_commit: Option<&'a str>,
    pub author: Option<Uuid>,
    pub restored_from: Option<Uuid>,
}

/// The newest revision: id, seq and content hash.
pub(crate) async fn head(
    conn: &mut PgConnection,
    inventory: Uuid,
) -> Result<Option<(Uuid, i32, String)>, sqlx::Error> {
    sqlx::query_as(
        "select id, seq, content_hash from inventory_revisions where inventory_id = $1 \
         order by seq desc limit 1",
    )
    .bind(inventory)
    .fetch_optional(&mut *conn)
    .await
}

fn issues_json(issues: &[InventoryIssue]) -> Value {
    Value::Array(
        issues
            .iter()
            .map(|i| json!({ "at": i.at, "message": i.message }))
            .collect(),
    )
}

/// Inserts the revision after `base` and its `inventory_hosts` rows. Returns its id and seq.
pub(crate) async fn insert_revision(
    conn: &mut PgConnection,
    inventory: Uuid,
    base: Option<(Uuid, i32)>,
    rev: NewRevision<'_>,
) -> Result<(Uuid, i32), sqlx::Error> {
    let id = Uuid::now_v7();
    let seq = base.map_or(1, |b| b.1 + 1);
    let status = if rev.checked.is_valid() {
        "valid"
    } else {
        "invalid"
    };
    sqlx::query(
        "insert into inventory_revisions (id, inventory_id, seq, parent_id, content, \
         content_hash, source_commit, author_user_id, restored_from, status, errors, warnings) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(id)
    .bind(inventory)
    .bind(seq)
    .bind(base.map(|b| b.0))
    .bind(&rev.checked.content)
    .bind(&rev.checked.hash)
    .bind(rev.source_commit)
    .bind(rev.author)
    .bind(rev.restored_from)
    .bind(status)
    .bind(issues_json(&rev.checked.errors))
    .bind(issues_json(&rev.checked.warnings))
    .execute(&mut *conn)
    .await?;
    let hosts: Vec<Value> = rev
        .checked
        .hosts
        .iter()
        .map(|h| json!({ "host": h.host, "groups": h.groups }))
        .collect();
    sqlx::query(
        "insert into inventory_hosts (revision_id, host, groups) \
         select $1, e->>'host', array(select jsonb_array_elements_text(e->'groups')) \
         from jsonb_array_elements($2) e",
    )
    .bind(id)
    .bind(Value::Array(hosts))
    .execute(&mut *conn)
    .await?;
    Ok((id, seq))
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505")
}

fn stale_base() -> Vec<Event> {
    op_error(
        "stale_base",
        "The inventory changed since you started editing. Reload it and try again.",
    )
}

fn no_change() -> Vec<Event> {
    op_error(
        "no_change",
        "That is the same content as the newest revision.",
    )
}

fn invalid(checked: &Checked) -> Vec<Event> {
    let first = checked
        .errors
        .first()
        .map(|i| format!(" First: {}: {}", i.at, i.message))
        .unwrap_or_default();
    op_error(
        "inventory_invalid",
        &format!(
            "The inventory has {} error(s); fix them before saving.{first}",
            checked.errors.len()
        ),
    )
}

fn from_repo() -> Vec<Event> {
    op_error(
        "inventory_from_repo",
        "This inventory comes from a repository. Change it there; sync makes the revision.",
    )
}

fn problems(v: Value) -> Vec<InventoryProblem> {
    serde_json::from_value(v).unwrap_or_default()
}

fn to_problems(issues: &[InventoryIssue]) -> Vec<InventoryProblem> {
    issues
        .iter()
        .map(|i| InventoryProblem {
            at: i.at.clone(),
            message: i.message.clone(),
        })
        .collect()
}

fn u32_of(n: i32) -> u32 {
    u32::try_from(n).unwrap_or(0)
}

fn i32_of(n: u32) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn count(n: i64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// An inventory's row as the commands need it.
struct Row {
    workspace: Uuid,
    name: String,
    source: String,
    status: String,
    var_schema: Option<Value>,
}

/// Finds the inventory and requires `cap` on its workspace.
async fn authorized(
    app: &AppState,
    req: &Request<'_>,
    action: &str,
    cap: Capability,
    id: Uuid,
) -> Result<Row, Vec<Event>> {
    type Found = (Uuid, String, String, String, Option<Value>);
    let row: Result<Option<Found>, _> = sqlx::query_as(
        "select workspace_id, name, source, status, var_schema from inventories where id = $1",
    )
    .bind(id)
    .fetch_optional(&app.db)
    .await;
    let row = match row {
        Ok(Some((workspace, name, source, status, var_schema))) => Row {
            workspace,
            name,
            source,
            status,
            var_schema,
        },
        Ok(None) => return Err(not_found("inventory")),
        Err(e) => return Err(db_failed(&e)),
    };
    require(app, req, action, cap, row.workspace).await?;
    Ok(row)
}

/// The head revision's id, seq and hash, outside a transaction (previews).
async fn current_head(
    app: &AppState,
    inventory: Uuid,
) -> Result<Option<(Uuid, i32, String)>, sqlx::Error> {
    let mut conn = app.db.acquire().await?;
    head(&mut conn, inventory).await
}

type InfoRow = (
    Uuid,
    Uuid,
    String,
    String,
    Option<Uuid>,
    Option<String>,
    String,
    Option<i32>,
    Option<String>,
    Option<i64>,
    bool,
    OffsetDateTime,
);

const INFO_SQL: &str = "select i.id, i.workspace_id, i.name, i.source, i.binding_id, i.path, \
     i.status, r.seq, r.status, \
     (select count(*) from inventory_hosts h where h.revision_id = r.id), \
     i.var_schema is not null, i.created_at \
     from inventories i left join lateral (select id, seq, status from inventory_revisions \
       where inventory_id = i.id order by seq desc limit 1) r on true \
     where ($1::uuid is null or i.workspace_id = $1) and ($2::uuid is null or i.id = $2) \
     order by i.name";

fn info(row: InfoRow) -> Option<InventoryInfo> {
    let (id, workspace, name, source, binding, path, status, seq, head_status, n, schema, at) = row;
    Some(InventoryInfo {
        id,
        workspace,
        name,
        source: enum_from(&source)?,
        binding,
        path,
        status: enum_from(&status)?,
        head_seq: seq.map(u32_of),
        head_status: head_status.as_deref().and_then(enum_from),
        host_count: count(n.unwrap_or(0)),
        has_schema: schema,
        created_at: ts(at),
    })
}

/// The workspace's inventories, as `Inventories`.
async fn items(app: &AppState, workspace: Uuid) -> Vec<Event> {
    let rows: Result<Vec<InfoRow>, _> = sqlx::query_as(INFO_SQL)
        .bind(Some(workspace))
        .bind(None::<Uuid>)
        .fetch_all(&app.db)
        .await;
    match rows {
        Ok(rows) => vec![Event::Inventories {
            items: rows.into_iter().filter_map(info).collect(),
        }],
        Err(e) => db_failed(&e),
    }
}

pub async fn list(app: &AppState, req: &Request<'_>, workspace: Uuid) -> Vec<Event> {
    if let Err(e) = require(app, req, "list_inventories", Capability::View, workspace).await {
        return e;
    }
    items(app, workspace).await
}

type SummaryRow = (
    Uuid,
    i32,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<i32>,
    i64,
    OffsetDateTime,
);

const SUMMARY_SQL: &str = "select r.id, r.seq, r.status, r.content_hash, r.source_commit, \
     (select display_name from users u where u.id = r.author_user_id), \
     (select x.seq from inventory_revisions x where x.id = r.restored_from), \
     (select count(*) from inventory_hosts h where h.revision_id = r.id), r.created_at \
     from inventory_revisions r where r.inventory_id = $1 order by r.seq desc";

fn summary(row: SummaryRow) -> Option<InventoryRevisionSummary> {
    let (_, seq, status, content_hash, source_commit, author, restored, n, at) = row;
    Some(InventoryRevisionSummary {
        seq: u32_of(seq),
        status: enum_from(&status)?,
        content_hash,
        source_commit,
        author,
        restored_from: restored.map(u32_of),
        host_count: count(n),
        created_at: ts(at),
    })
}

/// One revision (the newest when `seq` is `None`) and the history, as `InventoryRevision`.
async fn revision_event(app: &AppState, inventory: Uuid, seq: Option<u32>) -> Vec<Event> {
    let result: Result<Option<Event>, sqlx::Error> = async {
        let Some(inv) = sqlx::query_as::<_, InfoRow>(INFO_SQL)
            .bind(None::<Uuid>)
            .bind(Some(inventory))
            .fetch_optional(&app.db)
            .await?
            .and_then(info)
        else {
            return Ok(None);
        };
        let history: Vec<SummaryRow> = sqlx::query_as(SUMMARY_SQL)
            .bind(inventory)
            .fetch_all(&app.db)
            .await?;
        let want = seq.map(i32_of).or_else(|| history.first().map(|h| h.1));
        let Some(row) = history.iter().find(|h| Some(h.1) == want).cloned() else {
            return Ok(None);
        };
        let rev_id = row.0;
        let (content, errors, warnings): (Value, Value, Value) = sqlx::query_as(
            "select content, errors, warnings from inventory_revisions where id = $1",
        )
        .bind(rev_id)
        .fetch_one(&app.db)
        .await?;
        let hosts: Vec<(String, Vec<String>)> = sqlx::query_as(
            "select host, groups from inventory_hosts where revision_id = $1 order by host",
        )
        .bind(rev_id)
        .fetch_all(&app.db)
        .await?;
        let source = core_inv::source_of(&content).map(str::to_owned);
        let mut shown = content;
        if let Value::Object(m) = &mut shown {
            m.remove("source");
        }
        let Some(summary_now) = summary(row) else {
            return Ok(None);
        };
        Ok(Some(Event::InventoryRevision {
            inventory: inv,
            revision: InventoryRevisionInfo {
                summary: summary_now,
                source,
                content: serde_json::to_string_pretty(&shown).unwrap_or_default(),
                hosts: hosts
                    .into_iter()
                    .map(|(host, groups)| InventoryHostInfo { host, groups })
                    .collect(),
                errors: problems(errors),
                warnings: problems(warnings),
            },
            history: history.into_iter().filter_map(summary).collect(),
        }))
    }
    .await;
    match result {
        Ok(Some(e)) => vec![e],
        Ok(None) => not_found("inventory revision"),
        Err(e) => db_failed(&e),
    }
}

pub async fn get_revision(
    app: &AppState,
    req: &Request<'_>,
    inventory: Uuid,
    seq: Option<u32>,
) -> Vec<Event> {
    const ACTION: &str = "get_inventory_revision";
    if let Err(e) = authorized(app, req, ACTION, Capability::View, inventory).await {
        return e;
    }
    revision_event(app, inventory, seq).await
}

async fn content_at(
    app: &AppState,
    inventory: Uuid,
    seq: u32,
) -> Result<Option<Value>, sqlx::Error> {
    let row: Option<(Value,)> = sqlx::query_as(
        "select content from inventory_revisions where inventory_id = $1 and seq = $2",
    )
    .bind(inventory)
    .bind(i32_of(seq))
    .fetch_optional(&app.db)
    .await?;
    Ok(row.map(|r| r.0))
}

pub async fn diff(
    app: &AppState,
    req: &Request<'_>,
    inventory: Uuid,
    from: u32,
    to: u32,
) -> Vec<Event> {
    if let Err(e) = authorized(app, req, "diff_inventory", Capability::View, inventory).await {
        return e;
    }
    let (a, b) = match (
        content_at(app, inventory, from).await,
        content_at(app, inventory, to).await,
    ) {
        (Ok(Some(a)), Ok(Some(b))) => (a, b),
        (Err(e), _) | (_, Err(e)) => return db_failed(&e),
        _ => return not_found("inventory revision"),
    };
    let text = |v: Option<Value>| v.map(|v| serde_json::to_string(&v).unwrap_or_default());
    let changes = core_inv::diff(&a, &b)
        .into_iter()
        .map(|c| InventoryChange {
            path: c.path,
            kind: match c.kind {
                core_inv::ChangeKind::Added => InventoryChangeKind::Added,
                core_inv::ChangeKind::Removed => InventoryChangeKind::Removed,
                core_inv::ChangeKind::Changed => InventoryChangeKind::Changed,
            },
            before: text(c.before),
            after: text(c.after),
        })
        .collect();
    vec![Event::InventoryDiff {
        inventory,
        from,
        to,
        changes,
    }]
}

pub async fn validate(
    app: &AppState,
    req: &Request<'_>,
    workspace: Uuid,
    content: InventoryText,
    inventory: Option<Uuid>,
) -> Vec<Event> {
    const ACTION: &str = "validate_inventory";
    let schema = match inventory {
        Some(id) => match authorized(app, req, ACTION, Capability::View, id).await {
            Ok(row) if row.workspace == workspace => row.var_schema,
            Ok(_) => return not_found("inventory"),
            Err(e) => return e,
        },
        None => {
            if let Err(e) = require(app, req, ACTION, Capability::View, workspace).await {
                return e;
            }
            None
        }
    };
    let checked = core_inv::check_text(content.as_str(), Format::Yaml, schema.as_ref());
    vec![Event::InventoryValidation {
        content_hash: checked.hash.clone(),
        hosts: checked
            .hosts
            .iter()
            .map(|h| InventoryHostInfo {
                host: h.host.clone(),
                groups: h.groups.clone(),
            })
            .collect(),
        errors: to_problems(&checked.errors),
        warnings: to_problems(&checked.warnings),
    }]
}

/// Audits one inventory write in `tx`.
async fn audit_write(
    tx: &mut PgConnection,
    caller: &Caller,
    workspace: Uuid,
    action: &'static str,
    target: Value,
    ctx: Value,
) -> Result<(), sqlx::Error> {
    audit::record(
        tx,
        Entry {
            actor: &caller.actor(),
            action,
            workspace: Some(WorkspaceId(workspace)),
            target,
            decision: Decision::Allowed,
            reason: None,
            context: ctx,
        },
    )
    .await
    .map(|_| ())
}

pub async fn preview_create(
    app: &AppState,
    req: &Request<'_>,
    workspace: Uuid,
    name: ObjectName,
    content: InventoryText,
    var_schema: Option<InventoryText>,
) -> Vec<Event> {
    const ACTION: &str = "preview_inventory_create";
    if let Err(e) = require(app, req, ACTION, Capability::Administer, workspace).await {
        return e;
    }
    let schema = match var_schema
        .as_ref()
        .map(|s| serde_json::from_str::<Value>(s.as_str()))
    {
        None => None,
        Some(Ok(v @ Value::Object(_))) => Some(v),
        Some(_) => return op_error("schema_invalid", "The var schema is not a JSON object."),
    };
    let checked = core_inv::check_text(content.as_str(), Format::Yaml, schema.as_ref());
    if !checked.is_valid() {
        return invalid(&checked);
    }
    let taken: Result<Option<(Uuid,)>, _> =
        sqlx::query_as("select id from inventories where workspace_id = $1 and name = $2")
            .bind(workspace)
            .bind(name.as_str())
            .fetch_optional(&app.db)
            .await;
    match taken {
        Ok(Some(_)) => return op_error("name_taken", "An inventory with that name exists."),
        Ok(None) => {}
        Err(e) => return db_failed(&e),
    }
    let summary = format!(
        "Create the managed inventory \"{}\" with {} host(s).",
        name.as_str(),
        checked.hosts.len()
    );
    let affected = vec![name.as_str().to_owned()];
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            let id = Uuid::now_v7();
            let result: Result<bool, sqlx::Error> = async {
                let mut tx = app.db.begin().await?;
                let n = sqlx::query(
                    "insert into inventories (id, workspace_id, name, source, var_schema) \
                     values ($1, $2, $3, 'managed', $4) on conflict (workspace_id, name) do nothing",
                )
                .bind(id)
                .bind(workspace)
                .bind(name.as_str())
                .bind(&schema)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if n == 0 {
                    return Ok(false);
                }
                let rev = NewRevision {
                    checked: &checked,
                    source_commit: None,
                    author: Some(caller.user.id),
                    restored_from: None,
                };
                insert_revision(&mut tx, id, None, rev).await?;
                audit_write(
                    &mut tx,
                    &caller,
                    workspace,
                    "inventory_created",
                    json!({ "inventory": id, "name": name.as_str() }),
                    json!({ "seq": 1, "content_hash": checked.hash,
                            "hosts": checked.hosts.len(), "request": ctx }),
                )
                .await?;
                tx.commit().await?;
                Ok(true)
            }
            .await;
            match result {
                Ok(true) => revision_event(&app, id, None).await,
                Ok(false) => op_error("name_taken", "An inventory with that name exists."),
                Err(e) => db_failed(&e),
            }
        })
    });
    preview(
        app,
        req,
        Draft {
            action: "inventory_create".to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

/// What a confirmed revise targets.
struct Target {
    workspace: Uuid,
    inventory: Uuid,
    base_seq: i32,
    action: &'static str,
}

/// Inserts a revision after `base_seq` under a lock on the inventory row, and audits it.
/// `Ok(None)` when another writer moved the head first.
async fn revise(
    app: &AppState,
    caller: &Caller,
    t: Target,
    rev: NewRevision<'_>,
    ctx: Value,
) -> Result<Option<i32>, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    sqlx::query("select 1 from inventories where id = $1 for update")
        .bind(t.inventory)
        .execute(&mut *tx)
        .await?;
    let Some((head_id, head_seq, _)) = head(&mut tx, t.inventory).await? else {
        return Ok(None);
    };
    if head_seq != t.base_seq {
        return Ok(None);
    }
    let hash = rev.checked.hash.clone();
    let hosts = rev.checked.hosts.len();
    let restored = rev.restored_from;
    let seq = match insert_revision(&mut tx, t.inventory, Some((head_id, head_seq)), rev).await {
        Ok((_, seq)) => seq,
        Err(e) if is_unique_violation(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    audit_write(
        &mut tx,
        caller,
        t.workspace,
        t.action,
        json!({ "inventory": t.inventory }),
        json!({ "seq": seq, "content_hash": hash, "hosts": hosts,
                "restored_from": restored, "request": ctx }),
    )
    .await?;
    tx.commit().await?;
    Ok(Some(seq))
}

/// The confirmed half of edit and restore.
fn revise_op(t: Target, checked: Checked, restored_from: Option<Uuid>, ctx: Value) -> PendingOp {
    Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            let inventory = t.inventory;
            let rev = NewRevision {
                checked: &checked,
                source_commit: None,
                author: Some(caller.user.id),
                restored_from,
            };
            match revise(&app, &caller, t, rev, ctx).await {
                Ok(Some(_)) => revision_event(&app, inventory, None).await,
                Ok(None) => stale_base(),
                Err(e) => db_failed(&e),
            }
        })
    })
}

pub async fn preview_edit(
    app: &AppState,
    req: &Request<'_>,
    inventory: Uuid,
    base_seq: u32,
    content: InventoryText,
) -> Vec<Event> {
    const ACTION: &str = "preview_inventory_edit";
    let row = match authorized(app, req, ACTION, Capability::Administer, inventory).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    if row.source != "managed" {
        return from_repo();
    }
    let (head_seq, head_hash) = match current_head(app, inventory).await {
        Ok(Some((_, seq, hash))) if seq == i32_of(base_seq) => (seq, hash),
        Ok(_) => return stale_base(),
        Err(e) => return db_failed(&e),
    };
    let checked = core_inv::check_text(content.as_str(), Format::Yaml, row.var_schema.as_ref());
    if !checked.is_valid() {
        return invalid(&checked);
    }
    if checked.hash == head_hash {
        return no_change();
    }
    let summary = format!(
        "Save revision {} of \"{}\" with {} host(s).",
        head_seq + 1,
        row.name,
        checked.hosts.len()
    );
    let target = Target {
        workspace: row.workspace,
        inventory,
        base_seq: head_seq,
        action: "inventory_revised",
    };
    let op = revise_op(target, checked, None, channel(req));
    preview(
        app,
        req,
        Draft {
            action: "inventory_edit".to_owned(),
            summary,
            affected: vec![row.name],
            step_up: false,
        },
        op,
    )
}

pub async fn preview_restore(
    app: &AppState,
    req: &Request<'_>,
    inventory: Uuid,
    seq: u32,
) -> Vec<Event> {
    const ACTION: &str = "preview_inventory_restore";
    let row = match authorized(app, req, ACTION, Capability::Administer, inventory).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    if row.source != "managed" {
        return from_repo();
    }
    let old: Result<Option<(Uuid, Value, String)>, _> = sqlx::query_as(
        "select id, content, content_hash from inventory_revisions \
         where inventory_id = $1 and seq = $2",
    )
    .bind(inventory)
    .bind(i32_of(seq))
    .fetch_optional(&app.db)
    .await;
    let (old_id, old_content, old_hash) = match old {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("inventory revision"),
        Err(e) => return db_failed(&e),
    };
    let (head_seq, head_hash) = match current_head(app, inventory).await {
        Ok(Some((_, seq, hash))) => (seq, hash),
        Ok(None) => return not_found("inventory revision"),
        Err(e) => return db_failed(&e),
    };
    if old_hash == head_hash {
        return no_change();
    }
    // Re-checked against today's schema: a restore is a new revision and must validate.
    let Some(source) = core_inv::source_of(&old_content) else {
        return op_error(
            "inventory_invalid",
            "That revision has no source to restore from.",
        );
    };
    let checked = core_inv::check_text(source, Format::Yaml, row.var_schema.as_ref());
    if !checked.is_valid() {
        return invalid(&checked);
    }
    let summary = format!(
        "Restore revision {seq} of \"{}\" as revision {}.",
        row.name,
        head_seq + 1
    );
    let target = Target {
        workspace: row.workspace,
        inventory,
        base_seq: head_seq,
        action: "inventory_restored",
    };
    let op = revise_op(target, checked, Some(old_id), channel(req));
    preview(
        app,
        req,
        Draft {
            action: "inventory_restore".to_owned(),
            summary,
            affected: vec![row.name],
            step_up: false,
        },
        op,
    )
}

pub async fn preview_deprecate(app: &AppState, req: &Request<'_>, inventory: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_inventory_deprecate";
    let row = match authorized(app, req, ACTION, Capability::Administer, inventory).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    if row.status == "deprecated" {
        return op_error("no_change", "That inventory is already deprecated.");
    }
    let summary = format!(
        "Deprecate \"{}\". Runs already pinned to it continue; new runs are refused.",
        row.name
    );
    let ws = row.workspace;
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            let result: Result<u64, sqlx::Error> = async {
                let mut tx = app.db.begin().await?;
                let n = sqlx::query(
                    "update inventories set status = 'deprecated' \
                     where id = $1 and status = 'active'",
                )
                .bind(inventory)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if n > 0 {
                    audit_write(
                        &mut tx,
                        &caller,
                        ws,
                        "inventory_deprecated",
                        json!({ "inventory": inventory }),
                        ctx,
                    )
                    .await?;
                }
                tx.commit().await?;
                Ok(n)
            }
            .await;
            match result {
                Ok(0) => op_error("no_change", "That inventory is already deprecated."),
                Ok(_) => items(&app, ws).await,
                Err(e) => db_failed(&e),
            }
        })
    });
    preview(
        app,
        req,
        Draft {
            action: "inventory_deprecate".to_owned(),
            summary,
            affected: vec![row.name],
            step_up: false,
        },
        op,
    )
}
