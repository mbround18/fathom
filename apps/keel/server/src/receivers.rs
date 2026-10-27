//! Receivers (spec 055 T049, data-model `receivers`, contracts/receivers.md).
//!
//! A receiver is the public webhook endpoint a source-control host calls. Its secret is a fresh
//! 256-bit token, sealed at rest (AAD `receiver:<id>`) and shown once, after create or rotate.
//! Rotating keeps the previous secret verifying until `previous_expires_at`. Create, rotate and
//! bind need step-up; everything but reading needs `Administer`.

use keel_core::authz::Capability;
use keel_core::domain::WorkspaceId;
use keel_protocol::{
    DeliveryDecision, DeliveryInfo, Event, Instant, ObjectName, ReceiverInfo, ReceiverProvider,
};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::audit::{self, Decision, Entry};
use crate::bindings::{
    binding_workspace, channel, db_failed, enum_from, enum_str, not_found, op_error, preview,
    require, ts,
};
use crate::dispatch::{PendingOp, Request};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::serve::AppState;

/// How long the previous secret keeps verifying after a rotation, unless the command says.
pub const DEFAULT_PREVIOUS_VALID_FOR: u32 = 86_400;
/// The longest a previous secret may keep verifying.
pub const MAX_PREVIOUS_VALID_FOR: u32 = 7 * 86_400;
/// The most deliveries one `ListDeliveries` returns.
pub const DELIVERY_PAGE: i64 = 200;

/// The AAD a receiver's secrets are sealed under.
pub(crate) fn aad(id: Uuid) -> Vec<u8> {
    format!("receiver:{id}").into_bytes()
}

/// The URL to configure at the source-control host.
pub(crate) fn hook_url(app: &AppState, id: Uuid) -> String {
    format!(
        "{}/hooks/{id}",
        app.cfg.public_url.as_str().trim_end_matches('/')
    )
}

fn sealing_unavailable() -> Vec<Event> {
    op_error(
        "sealing_unavailable",
        "Keel has no master key loaded, so it cannot create or rotate receiver secrets.",
    )
}

/// The receiver's workspace and name.
async fn locate(app: &AppState, id: Uuid) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as("select workspace_id, name from receivers where id = $1")
        .bind(id)
        .fetch_optional(&app.db)
        .await
}

/// Finds the receiver and requires `Administer` on its workspace.
async fn administer(
    app: &AppState,
    req: &Request<'_>,
    action: &str,
    receiver: Uuid,
) -> Result<(Uuid, String), Vec<Event>> {
    let (ws, name) = match locate(app, receiver).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(not_found("receiver")),
        Err(e) => return Err(db_failed(&e)),
    };
    require(app, req, action, Capability::Administer, ws).await?;
    Ok((ws, name))
}

pub async fn list(app: &AppState, req: &Request<'_>, workspace: Uuid) -> Vec<Event> {
    if let Err(e) = require(app, req, "list_receivers", Capability::View, workspace).await {
        return e;
    }
    items(app, workspace).await
}

/// The workspace's receivers, as `Receivers`.
async fn items(app: &AppState, workspace: Uuid) -> Vec<Event> {
    type Row = (
        Uuid,
        String,
        String,
        String,
        Option<OffsetDateTime>,
        OffsetDateTime,
        Option<OffsetDateTime>,
    );
    let rows: Result<Vec<Row>, _> = sqlx::query_as(
        "select id, name, provider, status, previous_expires_at, created_at, rotated_at \
         from receivers where workspace_id = $1 order by name",
    )
    .bind(workspace)
    .fetch_all(&app.db)
    .await;
    let rows = match rows {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let links: Result<Vec<(Uuid, Uuid)>, _> = sqlx::query_as(
        "select rb.receiver_id, rb.binding_id from receiver_bindings rb \
         join receivers r on r.id = rb.receiver_id where r.workspace_id = $1 \
         order by rb.created_at",
    )
    .bind(workspace)
    .fetch_all(&app.db)
    .await;
    let links = match links {
        Ok(l) => l,
        Err(e) => return db_failed(&e),
    };
    let now = app.clock.now();
    let items = rows
        .into_iter()
        .filter_map(|(id, name, provider, status, prev, created, rotated)| {
            Some(ReceiverInfo {
                id,
                workspace,
                name,
                provider: enum_from(&provider)?,
                status: enum_from(&status)?,
                url: hook_url(app, id),
                previous_expires_at: prev.filter(|p| *p > now).map(ts),
                bindings: links.iter().filter(|l| l.0 == id).map(|l| l.1).collect(),
                created_at: ts(created),
                rotated_at: rotated.map(ts),
            })
        })
        .collect();
    vec![Event::Receivers { items }]
}

pub async fn preview_create(
    app: &AppState,
    req: &Request<'_>,
    workspace: Uuid,
    name: ObjectName,
    provider: ReceiverProvider,
) -> Vec<Event> {
    const ACTION: &str = "preview_receiver_create";
    if let Err(e) = require(app, req, ACTION, Capability::Administer, workspace).await {
        return e;
    }
    if app.repos.master_key.is_none() {
        return sealing_unavailable();
    }
    let taken: Result<Option<(Uuid,)>, _> =
        sqlx::query_as("select id from receivers where workspace_id = $1 and name = $2")
            .bind(workspace)
            .bind(name.as_str())
            .fetch_optional(&app.db)
            .await;
    match taken {
        Ok(Some(_)) => return op_error("name_taken", "A receiver with that name exists."),
        Ok(None) => {}
        Err(e) => return db_failed(&e),
    }
    let summary = format!(
        "Create the {} receiver \"{}\". Its URL and secret are shown once, after you confirm.",
        enum_str(&provider),
        name.as_str()
    );
    let affected = vec![name.as_str().to_owned()];
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move { create(&app, &caller, workspace, &name, provider, ctx).await })
    });
    preview(
        app,
        req,
        Draft {
            action: "receiver_create".to_owned(),
            summary,
            affected,
            step_up: true,
        },
        op,
    )
}

async fn create(
    app: &AppState,
    caller: &Caller,
    workspace: Uuid,
    name: &ObjectName,
    provider: ReceiverProvider,
    ctx: Value,
) -> Vec<Event> {
    let Some(key) = app.repos.master_key.clone() else {
        return sealing_unavailable();
    };
    let id = Uuid::now_v7();
    let secret = web_auth::csrf::new_token();
    let Ok(sealed) = key.seal(&aad(id), secret.as_bytes()) else {
        return sealing_unavailable();
    };
    let result: Result<bool, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let n = sqlx::query(
            "insert into receivers (id, workspace_id, name, provider, secret_current) \
             values ($1, $2, $3, $4, $5) on conflict (workspace_id, name) do nothing",
        )
        .bind(id)
        .bind(workspace)
        .bind(name.as_str())
        .bind(enum_str(&provider))
        .bind(&sealed)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 0 {
            return Ok(false);
        }
        audit::record(
            &mut tx,
            Entry {
                actor: &caller.actor(),
                action: "receiver_created",
                workspace: Some(WorkspaceId(workspace)),
                target: json!({ "receiver": id, "name": name.as_str() }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({ "provider": enum_str(&provider), "request": ctx }),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }
    .await;
    match result {
        Ok(true) => {
            let mut out = vec![Event::ReceiverSecret {
                url: hook_url(app, id),
                secret,
            }];
            out.extend(items(app, workspace).await);
            out
        }
        Ok(false) => op_error("name_taken", "A receiver with that name exists."),
        Err(e) => db_failed(&e),
    }
}

pub async fn preview_rotate(
    app: &AppState,
    req: &Request<'_>,
    receiver: Uuid,
    previous_valid_for: Option<u32>,
) -> Vec<Event> {
    let (ws, name) = match administer(app, req, "preview_receiver_rotate", receiver).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    if app.repos.master_key.is_none() {
        return sealing_unavailable();
    }
    let keep = previous_valid_for.unwrap_or(DEFAULT_PREVIOUS_VALID_FOR);
    if keep > MAX_PREVIOUS_VALID_FOR {
        return op_error(
            "invalid_argument",
            "The previous secret can keep working for at most 7 days.",
        );
    }
    let summary = if keep == 0 {
        format!("Rotate the secret of \"{name}\". The current secret stops working at once.")
    } else {
        format!(
            "Rotate the secret of \"{name}\". The current secret keeps working for {} more, \
             so the source-control host can be updated.",
            human_seconds(keep)
        )
    };
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move { rotate(&app, &caller, ws, receiver, keep, ctx).await })
    });
    preview(
        app,
        req,
        Draft {
            action: "receiver_rotate".to_owned(),
            summary,
            affected: vec![name],
            step_up: true,
        },
        op,
    )
}

fn human_seconds(s: u32) -> String {
    match s {
        s if s % 86_400 == 0 => format!("{} day(s)", s / 86_400),
        s if s % 3_600 == 0 => format!("{} hour(s)", s / 3_600),
        s => format!("{s} second(s)"),
    }
}

async fn rotate(
    app: &AppState,
    caller: &Caller,
    ws: Uuid,
    receiver: Uuid,
    keep: u32,
    ctx: Value,
) -> Vec<Event> {
    let Some(key) = app.repos.master_key.clone() else {
        return sealing_unavailable();
    };
    let secret = web_auth::csrf::new_token();
    let Ok(sealed) = key.seal(&aad(receiver), secret.as_bytes()) else {
        return sealing_unavailable();
    };
    let now = app.clock.now();
    let expires = (keep > 0).then(|| now + Duration::seconds(i64::from(keep)));
    let result: Result<u64, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        // With nothing kept, the previous secret is dropped rather than left to expire.
        let n = sqlx::query(
            "update receivers set \
               secret_previous = case when $3::timestamptz is null then null \
                                      else secret_current end, \
               previous_expires_at = $3, secret_current = $2, rotated_at = $4 \
             where id = $1",
        )
        .bind(receiver)
        .bind(&sealed)
        .bind(expires)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 1 {
            audit::record(
                &mut tx,
                Entry {
                    actor: &caller.actor(),
                    action: "receiver_rotated",
                    workspace: Some(WorkspaceId(ws)),
                    target: json!({ "receiver": receiver }),
                    decision: Decision::Allowed,
                    reason: None,
                    context: json!({ "previous_valid_for": keep, "request": ctx }),
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(n)
    }
    .await;
    match result {
        Ok(1) => {
            let mut out = vec![Event::ReceiverSecret {
                url: hook_url(app, receiver),
                secret,
            }];
            out.extend(items(app, ws).await);
            out
        }
        Ok(_) => not_found("receiver"),
        Err(e) => db_failed(&e),
    }
}

pub async fn preview_disable(app: &AppState, req: &Request<'_>, receiver: Uuid) -> Vec<Event> {
    let (ws, name) = match administer(app, req, "preview_receiver_disable", receiver).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let summary = format!(
        "Disable the receiver \"{name}\". Deliveries to it are refused (receiver_disabled) and \
         start no syncs."
    );
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            change(
                &app,
                &caller,
                ws,
                "receiver_disabled",
                Change::Disable(receiver),
                json!({ "receiver": receiver }),
                ctx,
            )
            .await
        })
    });
    preview(
        app,
        req,
        Draft {
            action: "receiver_disable".to_owned(),
            summary,
            affected: vec![name],
            step_up: false,
        },
        op,
    )
}

pub async fn preview_remove(app: &AppState, req: &Request<'_>, receiver: Uuid) -> Vec<Event> {
    let (ws, name) = match administer(app, req, "preview_receiver_remove", receiver).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let summary =
        format!("Remove the receiver \"{name}\" and its delivery history. Its URL stops working.");
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            change(
                &app,
                &caller,
                ws,
                "receiver_removed",
                Change::Remove(receiver),
                json!({ "receiver": receiver }),
                ctx,
            )
            .await
        })
    });
    preview(
        app,
        req,
        Draft {
            action: "receiver_remove".to_owned(),
            summary,
            affected: vec![name],
            step_up: false,
        },
        op,
    )
}

/// Checks the receiver and binding share a workspace, and requires `Administer` on it.
async fn link(
    app: &AppState,
    req: &Request<'_>,
    action: &str,
    receiver: Uuid,
    binding: Uuid,
) -> Result<(Uuid, String, String), Vec<Event>> {
    let (ws, name) = administer(app, req, action, receiver).await?;
    match binding_workspace(app, binding).await {
        Ok(Some(b)) if b == ws => {}
        Ok(Some(_)) => {
            return Err(op_error(
                "workspace_mismatch",
                "The receiver and the binding are in different workspaces.",
            ));
        }
        Ok(None) => return Err(not_found("binding")),
        Err(e) => return Err(db_failed(&e)),
    }
    let label: Option<(String,)> =
        sqlx::query_as("select url || ' (' || branch || ')' from repo_bindings where id = $1")
            .bind(binding)
            .fetch_optional(&app.db)
            .await
            .map_err(|e| db_failed(&e))?;
    Ok((ws, name, label.map(|l| l.0).unwrap_or_default()))
}

pub async fn preview_bind(
    app: &AppState,
    req: &Request<'_>,
    receiver: Uuid,
    binding: Uuid,
) -> Vec<Event> {
    let (ws, name, label) = match link(app, req, "preview_receiver_bind", receiver, binding).await {
        Ok(r) => r,
        Err(e) => return e,
    };
    let summary =
        format!("Let deliveries to \"{name}\" sync {label}. A push to that branch starts a fetch.");
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            change(
                &app,
                &caller,
                ws,
                "receiver_bound",
                Change::Bind {
                    receiver,
                    binding,
                    by: caller.user.id,
                },
                json!({ "receiver": receiver, "binding": binding }),
                ctx,
            )
            .await
        })
    });
    preview(
        app,
        req,
        Draft {
            action: "receiver_bind".to_owned(),
            summary,
            affected: vec![name, label],
            step_up: true,
        },
        op,
    )
}

pub async fn preview_unbind(
    app: &AppState,
    req: &Request<'_>,
    receiver: Uuid,
    binding: Uuid,
) -> Vec<Event> {
    let (ws, name, label) = match link(app, req, "preview_receiver_unbind", receiver, binding).await
    {
        Ok(r) => r,
        Err(e) => return e,
    };
    let summary = format!("Stop deliveries to \"{name}\" from syncing {label}.");
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            change(
                &app,
                &caller,
                ws,
                "receiver_unbound",
                Change::Unbind { receiver, binding },
                json!({ "receiver": receiver, "binding": binding }),
                ctx,
            )
            .await
        })
    });
    preview(
        app,
        req,
        Draft {
            action: "receiver_unbind".to_owned(),
            summary,
            affected: vec![name, label],
            step_up: false,
        },
        op,
    )
}

enum Change {
    Disable(Uuid),
    Remove(Uuid),
    Bind {
        receiver: Uuid,
        binding: Uuid,
        by: Uuid,
    },
    Unbind {
        receiver: Uuid,
        binding: Uuid,
    },
}

/// Applies one change, audits it when it changed something, and answers `Receivers`.
async fn change(
    app: &AppState,
    caller: &Caller,
    ws: Uuid,
    action: &'static str,
    change: Change,
    target: Value,
    ctx: Value,
) -> Vec<Event> {
    let result: Result<u64, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let q = match change {
            Change::Disable(r) => {
                sqlx::query("update receivers set status = 'disabled' where id = $1").bind(r)
            }
            Change::Remove(r) => sqlx::query("delete from receivers where id = $1").bind(r),
            Change::Bind {
                receiver,
                binding,
                by,
            } => sqlx::query(
                "insert into receiver_bindings (receiver_id, binding_id, created_by) \
                 values ($1, $2, $3) on conflict do nothing",
            )
            .bind(receiver)
            .bind(binding)
            .bind(by),
            Change::Unbind { receiver, binding } => sqlx::query(
                "delete from receiver_bindings where receiver_id = $1 and binding_id = $2",
            )
            .bind(receiver)
            .bind(binding),
        };
        let n = q.execute(&mut *tx).await?.rows_affected();
        if n > 0 {
            audit::record(
                &mut tx,
                Entry {
                    actor: &caller.actor(),
                    action,
                    workspace: Some(WorkspaceId(ws)),
                    target,
                    decision: Decision::Allowed,
                    reason: None,
                    context: ctx,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(n)
    }
    .await;
    match result {
        // Binding twice, or disabling a disabled receiver, is not an error.
        Ok(_) => items(app, ws).await,
        Err(e) => db_failed(&e),
    }
}

/// `ListDeliveries`: newest first, at most [`DELIVERY_PAGE`].
#[allow(clippy::too_many_arguments)]
pub async fn list_deliveries(
    app: &AppState,
    req: &Request<'_>,
    workspace: Uuid,
    receiver: Option<Uuid>,
    binding: Option<Uuid>,
    decision: Option<DeliveryDecision>,
    from: Option<Instant>,
    to: Option<Instant>,
) -> Vec<Event> {
    if let Err(e) = require(app, req, "list_deliveries", Capability::View, workspace).await {
        return e;
    }
    let parse = |i: Option<Instant>| -> Result<Option<OffsetDateTime>, ()> {
        i.map(|i| {
            OffsetDateTime::parse(i.as_str(), &time::format_description::well_known::Rfc3339)
                .map_err(|_| ())
        })
        .transpose()
    };
    let (Ok(from), Ok(to)) = (parse(from), parse(to)) else {
        return op_error("invalid_argument", "from and to are RFC 3339 instants.");
    };
    type Row = (
        Uuid,
        Uuid,
        Option<Uuid>,
        String,
        OffsetDateTime,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let rows: Result<Vec<Row>, _> = sqlx::query_as(
        "select d.id, d.receiver_id, d.binding_id, d.provider_delivery_id, d.received_at, \
                d.decision, d.refusal, d.sync_state, d.commit \
         from deliveries d join receivers r on r.id = d.receiver_id \
         where r.workspace_id = $1 \
           and ($2::uuid is null or d.receiver_id = $2) \
           and ($3::uuid is null or d.binding_id = $3) \
           and ($4::text is null or d.decision = $4) \
           and ($5::timestamptz is null or d.received_at >= $5) \
           and ($6::timestamptz is null or d.received_at < $6) \
         order by d.received_at desc, d.id desc limit $7",
    )
    .bind(workspace)
    .bind(receiver)
    .bind(binding)
    .bind(decision.as_ref().map(enum_str))
    .bind(from)
    .bind(to)
    .bind(DELIVERY_PAGE)
    .fetch_all(&app.db)
    .await;
    match rows {
        Ok(rows) => vec![Event::Deliveries {
            items: rows.into_iter().filter_map(delivery_info).collect(),
        }],
        Err(e) => db_failed(&e),
    }
}

#[allow(clippy::type_complexity, clippy::needless_pass_by_value)]
fn delivery_info(
    (id, receiver, binding, provider_delivery_id, received, decision, refusal, sync_state, commit): (
        Uuid,
        Uuid,
        Option<Uuid>,
        String,
        OffsetDateTime,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
) -> Option<DeliveryInfo> {
    Some(DeliveryInfo {
        id,
        receiver,
        binding,
        provider_delivery_id,
        received_at: ts(received),
        decision: enum_from(&decision)?,
        refusal: refusal.as_deref().and_then(enum_from),
        sync_state: sync_state.as_deref().and_then(enum_from),
        commit,
    })
}

/// One delivery as `DeliveryInfo`, for the pushed `DeliveryReceived`.
pub(crate) async fn delivery(app: &AppState, id: Uuid) -> Option<DeliveryInfo> {
    let row = sqlx::query_as(
        "select id, receiver_id, binding_id, provider_delivery_id, received_at, decision, \
                refusal, sync_state, commit from deliveries where id = $1",
    )
    .bind(id)
    .fetch_optional(&app.db)
    .await
    .ok()??;
    delivery_info(row)
}
