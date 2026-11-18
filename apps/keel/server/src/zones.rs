//! Zones, relays and the relay-link key over the protocol (spec 067 US3, T030).
//!
//! Commands:
//! - `ListZones`, `ListRelays`: `View` on the workspace (an API token may hold them).
//! - `PreviewZoneCreate`, `PreviewZoneUpdate`, `PreviewZoneRemove`, `PreviewHostZone`:
//!   `Administer`. A zone that any host or relay (revoked ones included) points at cannot be
//!   removed: the preview lists them and confirming refuses with `zone_in_use`.
//! - `PreviewRelayRegister`, `PreviewRelayUpdate`, `PreviewRelayRelease`: `Administer` with a
//!   fresh passkey. `PreviewRelayRevoke`: `Administer`; revoking is the fast way to cut a
//!   compromised relay off, so it asks for no step-up.
//! - `GetRelayLinkKeys`, `PreviewRelayLinkKeyRotate`, `…Activate`, `…Retire`: instance admin,
//!   the previews with a fresh passkey.
//!
//! Every confirmed change is audited in its own transaction (`zone.create`, `zone.update`,
//! `zone.remove`, `host.zone_set`, `relay.register`, `relay.update`, `relay.revoke`,
//! `relay.release`, `relay_link_key.create`, `relay_link_key.activate`,
//! `relay_link_key.retire`) and pushed to the workspace's sockets.

use keel_core::WorkspaceId;
use keel_core::authz::Capability;
use keel_protocol::{Cmd, Event, ZoneInfo, ZoneName};
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::audit::{self, Decision, Entry};
use crate::bindings::{db_failed, internal, not_found, op_error, require, ts};
use crate::dispatch::Request;
use crate::governance::{check_admin, deny};
use crate::hosts::{pending, preview, recheck};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::relays::{self, LinkKeyError, RegisterError, RelayActionError, RelayRegistration};
use crate::serve::AppState;

/// Handles the zone, relay and relay-link key commands.
pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::ListZones { workspace } => {
            if let Err(d) = require(app, req, "list_zones", Capability::View, workspace).await {
                return d;
            }
            zones_event(app, workspace).await
        }
        Cmd::PreviewZoneCreate {
            workspace,
            name,
            description,
        } => preview_zone_create(app, req, workspace, name, description.as_str().to_owned()).await,
        Cmd::PreviewZoneUpdate {
            zone,
            name,
            description,
        } => preview_zone_update(app, req, zone, name, description.as_str().to_owned()).await,
        Cmd::PreviewZoneRemove { zone } => preview_zone_remove(app, req, zone).await,
        Cmd::PreviewHostZone { host, zone } => preview_host_zone(app, req, host, zone).await,
        Cmd::ListRelays { workspace } => {
            if let Err(d) = require(app, req, "list_relays", Capability::View, workspace).await {
                return d;
            }
            relays_event(app, workspace).await
        }
        Cmd::PreviewRelayRegister {
            workspace,
            zone,
            name,
            dial_address,
            host_key,
            host,
        } => {
            let edit = RelayEdit {
                zone,
                name: name.as_str().to_owned(),
                dial_address,
                host_key,
                host,
            };
            preview_relay_edit(app, req, workspace, None, edit).await
        }
        Cmd::PreviewRelayUpdate {
            relay,
            zone,
            name,
            dial_address,
            host_key,
            host,
        } => {
            let ws = match relay_row(app, relay).await {
                Ok(Some(r)) => r.workspace,
                Ok(None) => return not_found("relay"),
                Err(e) => return db_failed(&e),
            };
            let edit = RelayEdit {
                zone,
                name: name.as_str().to_owned(),
                dial_address,
                host_key,
                host,
            };
            preview_relay_edit(app, req, ws, Some(relay), edit).await
        }
        Cmd::PreviewRelayRevoke { relay } => preview_relay_revoke(app, req, relay).await,
        Cmd::PreviewRelayRelease { relay } => preview_relay_release(app, req, relay).await,
        Cmd::GetRelayLinkKeys => {
            if let Err(reason) = check_admin(req.caller) {
                return deny_admin(app, req, "get_relay_link_keys", reason).await;
            }
            link_keys_event(app).await
        }
        Cmd::PreviewRelayLinkKeyRotate => preview_key_rotate(app, req).await,
        Cmd::PreviewRelayLinkKeyActivate { fingerprint } => {
            preview_key_step(app, req, fingerprint, true).await
        }
        Cmd::PreviewRelayLinkKeyRetire { fingerprint } => {
            preview_key_step(app, req, fingerprint, false).await
        }
        _ => internal(),
    }
}

// ---- shared -----------------------------------------------------------------------------------

async fn deny_admin(app: &AppState, req: &Request<'_>, action: &str, reason: &str) -> Vec<Event> {
    deny(
        app,
        req.caller,
        action,
        None,
        reason,
        crate::bindings::channel(req),
    )
    .await
}

fn admin_recheck(caller: &Caller, action: &str) -> Result<(), Vec<Event>> {
    check_admin(caller).map_err(|reason| {
        vec![Event::Denied {
            action: action.to_owned(),
            reason: reason.to_owned(),
        }]
    })
}

/// Adds `warnings` to the preview in `events`.
fn with_warnings(mut events: Vec<Event>, warnings: &[String]) -> Vec<Event> {
    for e in &mut events {
        if let Event::Preview(p) = e {
            p.warnings.extend(warnings.iter().cloned());
        }
    }
    events
}

async fn audit_tx(
    tx: &mut sqlx::PgConnection,
    caller: &Caller,
    action: &str,
    ws: Uuid,
    target: serde_json::Value,
    context: serde_json::Value,
) -> Result<(), sqlx::Error> {
    audit::record(
        tx,
        Entry {
            actor: &caller.actor(),
            action,
            workspace: Some(WorkspaceId(ws)),
            target,
            decision: Decision::Allowed,
            reason: None,
            context,
        },
    )
    .await
    .map(|_| ())
}

fn is_unique(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .is_some_and(|d| d.is_unique_violation())
}

fn is_foreign_key(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .is_some_and(|d| d.is_foreign_key_violation())
}

// ---- zones ------------------------------------------------------------------------------------

async fn zones(app: &AppState, ws: Uuid) -> Result<Vec<ZoneInfo>, sqlx::Error> {
    let rows: Vec<(Uuid, String, String, i64, i64)> = sqlx::query_as(
        "select z.id, z.name, z.description, \
           (select count(*) from hosts h where h.zone_id = z.id and h.state <> 'revoked'), \
           (select count(*) from relays r where r.zone_id = z.id and r.revoked_at is null) \
         from zones z where z.workspace_id = $1 order by z.name",
    )
    .bind(ws)
    .fetch_all(&app.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, name, description, hosts, relays)| {
            Some(ZoneInfo {
                id,
                workspace: ws,
                name: ZoneName::new(name).ok()?,
                description,
                hosts: u32::try_from(hosts).unwrap_or(u32::MAX),
                relays: u32::try_from(relays).unwrap_or(u32::MAX),
            })
        })
        .collect())
}

async fn zones_event(app: &AppState, ws: Uuid) -> Vec<Event> {
    match zones(app, ws).await {
        Ok(items) => vec![Event::Zones { items }],
        Err(e) => db_failed(&e),
    }
}

/// Replies with the workspace's zones and pushes them to its sockets.
async fn zones_changed(app: &AppState, ws: Uuid) -> Vec<Event> {
    let events = zones_event(app, ws).await;
    if let Some(e @ Event::Zones { .. }) = events.first() {
        app.repos.push(ws, e.clone());
    }
    events
}

/// A zone's workspace, name and description.
async fn zone_row(app: &AppState, zone: Uuid) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as("select workspace_id, name from zones where id = $1")
        .bind(zone)
        .fetch_optional(&app.db)
        .await
}

async fn zone_name_taken(
    app: &AppState,
    ws: Uuid,
    name: &str,
    except: Option<Uuid>,
) -> Result<bool, sqlx::Error> {
    let (taken,): (bool,) = sqlx::query_as(
        "select exists (select 1 from zones where workspace_id = $1 and name = $2 \
         and ($3::uuid is null or id <> $3))",
    )
    .bind(ws)
    .bind(name)
    .bind(except)
    .fetch_one(&app.db)
    .await?;
    Ok(taken)
}

fn zone_exists(name: &str) -> Vec<Event> {
    op_error(
        "zone_exists",
        &format!("A zone named {name} exists in this workspace."),
    )
}

async fn preview_zone_create(
    app: &AppState,
    req: &Request<'_>,
    ws: Uuid,
    name: ZoneName,
    description: String,
) -> Vec<Event> {
    const ACTION: &str = "preview_zone_create";
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    let name = name.as_str().to_owned();
    match zone_name_taken(app, ws, &name, None).await {
        Ok(true) => return zone_exists(&name),
        Ok(false) => {}
        Err(e) => return db_failed(&e),
    }
    let summary = format!(
        "Create zone {name}. Hosts placed in it are reached through its relays; register a \
         relay for it before moving hosts in."
    );
    let affected = vec![format!("zone {name}")];
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        let id = Uuid::now_v7();
        let result = async {
            let mut tx = app.db.begin().await?;
            sqlx::query(
                "insert into zones (id, workspace_id, name, description) values ($1, $2, $3, $4)",
            )
            .bind(id)
            .bind(ws)
            .bind(&name)
            .bind(&description)
            .execute(&mut *tx)
            .await?;
            audit_tx(
                &mut tx,
                &caller,
                "zone.create",
                ws,
                json!({ "zone": id }),
                json!({ "name": name, "description": description }),
            )
            .await?;
            tx.commit().await
        }
        .await;
        match result {
            Ok(()) => zones_changed(&app, ws).await,
            Err(e) if is_unique(&e) => zone_exists(&name),
            Err(e) => db_failed(&e),
        }
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

async fn preview_zone_update(
    app: &AppState,
    req: &Request<'_>,
    zone: Uuid,
    name: ZoneName,
    description: String,
) -> Vec<Event> {
    const ACTION: &str = "preview_zone_update";
    let (ws, old) = match zone_row(app, zone).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("zone"),
        Err(e) => return db_failed(&e),
    };
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    let name = name.as_str().to_owned();
    match zone_name_taken(app, ws, &name, Some(zone)).await {
        Ok(true) => return zone_exists(&name),
        Ok(false) => {}
        Err(e) => return db_failed(&e),
    }
    let summary = if old == name {
        format!("Update zone {name}'s description.")
    } else {
        format!("Rename zone {old} to {name} and update its description.")
    };
    let affected = vec![format!("zone {old}")];
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        let result = async {
            let mut tx = app.db.begin().await?;
            let before: Option<(String, String)> = sqlx::query_as(
                "select name, description from zones where id = $1 and workspace_id = $2 \
                 for update",
            )
            .bind(zone)
            .bind(ws)
            .fetch_optional(&mut *tx)
            .await?;
            let Some((before_name, before_description)) = before else {
                return Ok(false);
            };
            sqlx::query("update zones set name = $2, description = $3 where id = $1")
                .bind(zone)
                .bind(&name)
                .bind(&description)
                .execute(&mut *tx)
                .await?;
            audit_tx(
                &mut tx,
                &caller,
                "zone.update",
                ws,
                json!({ "zone": zone }),
                json!({
                    "before": { "name": before_name, "description": before_description },
                    "after": { "name": name, "description": description },
                }),
            )
            .await?;
            tx.commit().await?;
            Ok(true)
        }
        .await;
        match result {
            Ok(true) => zones_changed(&app, ws).await,
            Ok(false) => not_found("zone"),
            Err(e) if is_unique(&e) => zone_exists(&name),
            Err(e) => db_failed(&e),
        }
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

/// What keeps a zone from being removed: its hosts and relays (revoked ones included: their
/// rows keep pointing at it).
async fn zone_blockers(
    conn: &mut sqlx::PgConnection,
    zone: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "select 'host ' || inventory_name from hosts where zone_id = $1 \
         union all \
         select 'relay ' || name || case when revoked_at is null then '' else ' (revoked)' end \
         from relays where zone_id = $1 order by 1",
    )
    .bind(zone)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

fn zone_in_use(blockers: &[String]) -> Vec<Event> {
    op_error(
        "zone_in_use",
        &format!(
            "The zone still has {}. Move its hosts out first; a zone that ever had a relay \
             cannot be removed.",
            blockers.join(", ")
        ),
    )
}

async fn preview_zone_remove(app: &AppState, req: &Request<'_>, zone: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_zone_remove";
    let (ws, name) = match zone_row(app, zone).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("zone"),
        Err(e) => return db_failed(&e),
    };
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    let blockers = match app.db.acquire().await {
        Ok(mut conn) => match zone_blockers(&mut conn, zone).await {
            Ok(b) => b,
            Err(e) => return db_failed(&e),
        },
        Err(e) => return db_failed(&e),
    };
    let summary = if blockers.is_empty() {
        format!("Remove zone {name}.")
    } else {
        format!(
            "Zone {name} cannot be removed while hosts or relays point at it. Confirming will \
             be refused."
        )
    };
    let mut affected = vec![format!("zone {name}")];
    affected.extend(blockers.iter().cloned());
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        let result = async {
            let mut tx = app.db.begin().await?;
            let locked: Option<(String,)> = sqlx::query_as(
                "select name from zones where id = $1 and workspace_id = $2 for update",
            )
            .bind(zone)
            .bind(ws)
            .fetch_optional(&mut *tx)
            .await?;
            let Some((name,)) = locked else {
                return Ok(Err(not_found("zone")));
            };
            let blockers = zone_blockers(&mut tx, zone).await?;
            if !blockers.is_empty() {
                return Ok(Err(zone_in_use(&blockers)));
            }
            sqlx::query("delete from zones where id = $1")
                .bind(zone)
                .execute(&mut *tx)
                .await?;
            audit_tx(
                &mut tx,
                &caller,
                "zone.remove",
                ws,
                json!({ "zone": zone }),
                json!({ "name": name }),
            )
            .await?;
            tx.commit().await?;
            Ok(Ok(()))
        }
        .await;
        match result {
            Ok(Ok(())) => zones_changed(&app, ws).await,
            Ok(Err(refused)) => refused,
            // A host moved in between the check and the delete.
            Err(e) if is_foreign_key(&e) => zone_in_use(&["hosts or relays".to_owned()]),
            Err(e) => db_failed(&e),
        }
    });
    let warnings = if blockers.is_empty() {
        Vec::new()
    } else {
        vec![format!("In use by {}.", blockers.join(", "))]
    };
    with_warnings(
        preview(
            app,
            req,
            Draft {
                action: ACTION.to_owned(),
                summary,
                affected,
                step_up: false,
            },
            op,
        ),
        &warnings,
    )
}

async fn preview_host_zone(
    app: &AppState,
    req: &Request<'_>,
    host: Uuid,
    zone: Option<Uuid>,
) -> Vec<Event> {
    const ACTION: &str = "preview_host_zone";
    type Row = (Uuid, String, Option<Uuid>, Option<String>);
    let row: Option<Row> = match sqlx::query_as(
        "select h.workspace_id, h.inventory_name, h.zone_id, z.name from hosts h \
         left join zones z on z.id = h.zone_id where h.id = $1",
    )
    .bind(host)
    .fetch_optional(&app.db)
    .await
    {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let Some((ws, host_name, from, from_name)) = row else {
        return not_found("host");
    };
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    let to_name = match zone {
        None => "direct".to_owned(),
        Some(z) => match zone_row(app, z).await {
            Ok(Some((zws, n))) if zws == ws => n,
            Ok(_) => return unknown_zone(),
            Err(e) => return db_failed(&e),
        },
    };
    let from_name = from_name.unwrap_or_else(|| "direct".to_owned());
    let summary = if from == zone {
        format!("Host {host_name} is already in zone {to_name}.")
    } else {
        format!(
            "Move host {host_name} from zone {from_name} to zone {to_name}. Its runner keeps \
             its session; the next one must come {}.",
            if zone.is_some() {
                "through one of the new zone's relays"
            } else {
                "straight to Keel"
            }
        )
    };
    let mut warnings = Vec::new();
    if let Some(z) = zone {
        match sqlx::query_as::<_, (i64,)>(
            "select count(*) from relays where zone_id = $1 and revoked_at is null",
        )
        .bind(z)
        .fetch_one(&app.db)
        .await
        {
            Ok((0,)) => warnings.push(format!(
                "Zone {to_name} has no relay: the host will have no path to Keel."
            )),
            Ok(_) => {}
            Err(e) => return db_failed(&e),
        }
    }
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        let result = async {
            let mut tx = app.db.begin().await?;
            let before: Option<(Option<Uuid>,)> = sqlx::query_as(
                "update hosts h set zone_id = $3 from hosts old \
                 where h.id = $1 and h.workspace_id = $2 and old.id = h.id \
                 returning old.zone_id",
            )
            .bind(host)
            .bind(ws)
            .bind(zone)
            .fetch_optional(&mut *tx)
            .await?;
            let Some((before,)) = before else {
                return Ok(false);
            };
            audit_tx(
                &mut tx,
                &caller,
                "host.zone_set",
                ws,
                json!({ "host": host }),
                json!({ "from": before, "to": zone }),
            )
            .await?;
            tx.commit().await?;
            Ok(true)
        }
        .await;
        match result {
            Ok(true) => {}
            Ok(false) => return not_found("host"),
            // The zone was removed, or is another workspace's.
            Err(e) if is_foreign_key(&e) => return unknown_zone(),
            Err(e) => return db_failed(&e),
        }
        relays::push_host_reach(&app, host).await;
        let _ = zones_changed(&app, ws).await;
        crate::hosts::host_event(&app, host).await
    });
    with_warnings(
        preview(
            app,
            req,
            Draft {
                action: ACTION.to_owned(),
                summary,
                affected: vec![host_name],
                step_up: false,
            },
            op,
        ),
        &warnings,
    )
}

fn unknown_zone() -> Vec<Event> {
    op_error("unknown_zone", "No such zone in this workspace.")
}

// ---- relays -----------------------------------------------------------------------------------

struct RelayRowView {
    workspace: Uuid,
    name: String,
    revoked: bool,
    quarantined_at: Option<OffsetDateTime>,
    quarantine_reason: Option<String>,
}

async fn relay_row(app: &AppState, relay: Uuid) -> Result<Option<RelayRowView>, sqlx::Error> {
    type Row = (Uuid, String, bool, Option<OffsetDateTime>, Option<String>);
    let row: Option<Row> = sqlx::query_as(
        "select workspace_id, name, revoked_at is not null, quarantined_at, quarantine_reason \
         from relays where id = $1",
    )
    .bind(relay)
    .fetch_optional(&app.db)
    .await?;
    Ok(row.map(
        |(workspace, name, revoked, quarantined_at, quarantine_reason)| RelayRowView {
            workspace,
            name,
            revoked,
            quarantined_at,
            quarantine_reason,
        },
    ))
}

async fn relays_event(app: &AppState, ws: Uuid) -> Vec<Event> {
    match relays::relay_infos(app, ws).await {
        Ok(items) => vec![Event::Relays { items }],
        Err(e) => db_failed(&e),
    }
}

/// Pushes `RelayChanged` for `relay` and replies with the workspace's relays.
async fn relay_changed(app: &AppState, ws: Uuid, relay: Uuid) -> Vec<Event> {
    match relays::relay_infos(app, ws).await {
        Ok(items) => {
            if let Some(info) = items.iter().find(|r| r.id == relay) {
                app.repos.push(ws, Event::RelayChanged(info.clone()));
            }
            vec![Event::Relays { items }]
        }
        Err(e) => db_failed(&e),
    }
}

fn register_error(e: &RegisterError) -> Vec<Event> {
    match e {
        RegisterError::Name => op_error("invalid_name", &e.to_string()),
        RegisterError::DialAddress(_) => op_error("invalid_dial_address", &e.to_string()),
        RegisterError::HostKey(_) => op_error("invalid_host_key", &e.to_string()),
        RegisterError::Zone => unknown_zone(),
        RegisterError::Host => op_error("unknown_host", "No such host in this workspace."),
        RegisterError::NameTaken => op_error(
            "relay_name_taken",
            "A relay of that name exists in this workspace.",
        ),
        RegisterError::HostKeyTaken => {
            op_error("host_key_taken", "Another relay already has that host key.")
        }
        RegisterError::Relay => not_found("relay"),
        RegisterError::Revoked => relay_revoked(),
        RegisterError::Db(e) => db_failed(e),
    }
}

fn action_error(e: &RelayActionError) -> Vec<Event> {
    match e {
        RelayActionError::NotFound => not_found("relay"),
        RelayActionError::Revoked => relay_revoked(),
        RelayActionError::NotQuarantined => {
            op_error("not_quarantined", "The relay is not quarantined.")
        }
        RelayActionError::Db(e) => db_failed(e),
    }
}

fn relay_revoked() -> Vec<Event> {
    op_error(
        "relay_revoked",
        "The relay is revoked; revocation is final. Register it again under a new name.",
    )
}

/// A register (`relay` = `None`) or update, as entered.
struct RelayEdit {
    zone: Uuid,
    name: String,
    dial_address: String,
    host_key: String,
    host: Option<Uuid>,
}

impl RelayEdit {
    fn registration(&self, workspace: Uuid) -> RelayRegistration<'_> {
        RelayRegistration {
            workspace,
            zone: self.zone,
            name: &self.name,
            dial_address: &self.dial_address,
            host_key: &self.host_key,
            host: self.host,
        }
    }
}

async fn preview_relay_edit(
    app: &AppState,
    req: &Request<'_>,
    ws: Uuid,
    relay: Option<Uuid>,
    edit: RelayEdit,
) -> Vec<Event> {
    let action = if relay.is_some() {
        "preview_relay_update"
    } else {
        "preview_relay_register"
    };
    if let Err(d) = require(app, req, action, Capability::Administer, ws).await {
        return d;
    }
    // Checked now for early errors, and again in the confirming transaction.
    let checked = async {
        let mut conn = app.db.acquire().await.map_err(RegisterError::Db)?;
        let fingerprint = relays::check_registration(&mut conn, &edit.registration(ws)).await?;
        let before: Option<(bool, String)> = match relay {
            Some(id) => sqlx::query_as(
                "select revoked_at is not null, host_key_fingerprint from relays \
                 where id = $1 and workspace_id = $2",
            )
            .bind(id)
            .bind(ws)
            .fetch_optional(&mut *conn)
            .await
            .map_err(RegisterError::Db)?,
            None => None,
        };
        // Names and host keys are unique (revoked relays included); say so before confirming.
        let (name_taken, key_taken): (bool, bool) = sqlx::query_as(
            "select \
               exists (select 1 from relays where workspace_id = $1 and name = $2 \
                 and ($4::uuid is null or id <> $4)), \
               exists (select 1 from relays where host_key_fingerprint = $3 \
                 and ($4::uuid is null or id <> $4))",
        )
        .bind(ws)
        .bind(&edit.name)
        .bind(&fingerprint)
        .bind(relay)
        .fetch_one(&mut *conn)
        .await
        .map_err(RegisterError::Db)?;
        if name_taken {
            return Err(RegisterError::NameTaken);
        }
        if key_taken {
            return Err(RegisterError::HostKeyTaken);
        }
        Ok::<_, RegisterError>((fingerprint, before))
    }
    .await;
    let (fingerprint, before) = match checked {
        Ok(c) => c,
        Err(e) => return register_error(&e),
    };
    let mut warnings = Vec::new();
    let summary = match (&relay, &before) {
        (Some(_), None) => return not_found("relay"),
        (Some(_), Some((true, _))) => return relay_revoked(),
        (Some(_), Some((false, old_fp))) => {
            if *old_fp != fingerprint {
                warnings.push(format!(
                    "The host key changes from {old_fp} to {fingerprint}: Keel drops the \
                     current link and pins the new key when it redials."
                ));
            }
            format!(
                "Update relay {} ({}, host key {fingerprint}).",
                edit.name, edit.dial_address
            )
        }
        (None, _) => format!(
            "Register relay {} at {}, pinning host key {fingerprint}. Keel dials it at once.",
            edit.name, edit.dial_address
        ),
    };
    match app.relay_link.get() {
        Some(k) => warnings.push(format!(
            "The relay must trust Keel's relay-link key: add this line to its hub_keys: {}",
            k.public_line
        )),
        None => warnings.push(
            "Keel's relay-link key is not loaded, so Keel cannot dial any relay yet.".to_owned(),
        ),
    }
    let affected = vec![format!("relay {}", edit.name)];
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, action, Capability::Administer, ws) {
            return d;
        }
        let result = async {
            let mut tx = app.db.begin().await.map_err(RegisterError::Db)?;
            let r = edit.registration(ws);
            let done = match relay {
                Some(id) => relays::update_relay(&mut tx, &caller.actor(), id, &r).await?,
                None => relays::register_relay(&mut tx, &caller.actor(), &r).await?,
            };
            tx.commit().await.map_err(RegisterError::Db)?;
            Ok::<_, RegisterError>(done)
        }
        .await;
        match result {
            Ok(done) => {
                app.relay_live.resync();
                relay_changed(&app, ws, done.id).await
            }
            Err(e) => register_error(&e),
        }
    });
    with_warnings(
        preview(
            app,
            req,
            Draft {
                action: action.to_owned(),
                summary,
                affected,
                step_up: true,
            },
            op,
        ),
        &warnings,
    )
}

async fn preview_relay_revoke(app: &AppState, req: &Request<'_>, relay: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_relay_revoke";
    let row = match relay_row(app, relay).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("relay"),
        Err(e) => return db_failed(&e),
    };
    let ws = row.workspace;
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    if row.revoked {
        return relay_revoked();
    }
    // The hosts whose session this relay carries now: they are cut off.
    let hosts: Vec<(Uuid, String)> = match sqlx::query_as(
        "select id, inventory_name from hosts where workspace_id = $1 and state = 'online' \
         order by inventory_name",
    )
    .bind(ws)
    .fetch_all(&app.db)
    .await
    {
        Ok(h) => h,
        Err(e) => return db_failed(&e),
    };
    let carried: Vec<String> = hosts
        .into_iter()
        .filter(|(id, _)| app.runs.path(*id).and_then(|p| p.relay()) == Some(relay))
        .map(|(_, name)| name)
        .collect();
    let name = row.name;
    let summary = format!(
        "Revoke relay {name}: Keel closes its link now and never dials it again. {} Revocation \
         is final.",
        if carried.is_empty() {
            "It carries no runner session now.".to_owned()
        } else {
            format!(
                "The runner sessions it carries end, and their runs in progress are lost: {}.",
                carried.join(", ")
            )
        }
    );
    let mut affected = vec![format!("relay {name}")];
    affected.extend(carried);
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        match relays::revoke_relay(&app, &caller.actor(), ws, relay).await {
            Ok(()) => relay_changed(&app, ws, relay).await,
            Err(e) => action_error(&e),
        }
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary,
            affected,
            step_up: false,
        },
        op,
    )
}

async fn preview_relay_release(app: &AppState, req: &Request<'_>, relay: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_relay_release";
    let row = match relay_row(app, relay).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("relay"),
        Err(e) => return db_failed(&e),
    };
    let ws = row.workspace;
    if let Err(d) = require(app, req, ACTION, Capability::Administer, ws).await {
        return d;
    }
    if row.revoked {
        return relay_revoked();
    }
    let Some(reason) = row.quarantine_reason else {
        return action_error(&RelayActionError::NotQuarantined);
    };
    // What the relay tried while it was quarantined (coalesced `relay.refused` rows).
    let since = row.quarantined_at.unwrap_or(OffsetDateTime::UNIX_EPOCH);
    let refusals = match sqlx::query_as::<_, (i64,)>(
        "select coalesce(sum(coalesce((context->>'count')::bigint, 1)), 0)::bigint \
         from audit_event where action = $1 and target->>'relay' = $2 and at >= $3",
    )
    .bind(relays::ACTION_REFUSED)
    .bind(relay.to_string())
    .bind(since)
    .fetch_one(&app.db)
    .await
    {
        Ok((n,)) => n,
        Err(e) => return db_failed(&e),
    };
    let name = row.name;
    let summary = format!(
        "Release relay {name} from quarantine and dial it again. It was quarantined at {} for \
         {reason}; {refusals} refused attempt(s) since.",
        row.quarantined_at
            .map(ts)
            .unwrap_or_else(|| "an unknown time".to_owned()),
    );
    let op = pending(move |app, caller| async move {
        if let Err(d) = recheck(&caller, ACTION, Capability::Administer, ws) {
            return d;
        }
        match relays::release_relay(&app, &caller.actor(), ws, relay).await {
            Ok(()) => relay_changed(&app, ws, relay).await,
            Err(e) => action_error(&e),
        }
    });
    with_warnings(
        preview(
            app,
            req,
            Draft {
                action: ACTION.to_owned(),
                summary,
                affected: vec![format!("relay {name}")],
                step_up: true,
            },
            op,
        ),
        &[
            "A relay is quarantined when it breaks the relay protocol. Release it only once \
           you know why it did."
                .to_owned(),
        ],
    )
}

// ---- the relay-link key -----------------------------------------------------------------------

async fn link_keys_event(app: &AppState) -> Vec<Event> {
    match relays::link_keys(&app.db).await {
        Ok(items) => vec![Event::RelayLinkKeys { items }],
        Err(e) => db_failed(&e),
    }
}

fn link_key_error(e: &LinkKeyError) -> Vec<Event> {
    match e {
        LinkKeyError::PendingExists => op_error(
            "link_key_pending",
            "A new relay-link key is already pending. Activate or retire it first.",
        ),
        LinkKeyError::NotPending => op_error(
            "link_key_not_pending",
            "No pending relay-link key has that fingerprint.",
        ),
        LinkKeyError::Key(relays::RelayKeyError::NoMasterKey) => key_unavailable(),
        LinkKeyError::Key(e) => {
            tracing::error!(error = %e, "relay-link key");
            internal()
        }
    }
}

fn key_unavailable() -> Vec<Event> {
    op_error(
        "relay_link_key_unavailable",
        "Keel's relay-link key cannot be changed without the master key.",
    )
}

async fn pending_key(app: &AppState) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> =
        sqlx::query_as("select fingerprint from relay_link_keys where state = 'pending'")
            .fetch_optional(&app.db)
            .await?;
    Ok(row.map(|r| r.0))
}

async fn preview_key_rotate(app: &AppState, req: &Request<'_>) -> Vec<Event> {
    const ACTION: &str = "preview_relay_link_key_rotate";
    if let Err(reason) = check_admin(req.caller) {
        return deny_admin(app, req, ACTION, reason).await;
    }
    if app.repos.master_key.is_none() {
        return key_unavailable();
    }
    match pending_key(app).await {
        Ok(Some(_)) => return link_key_error(&LinkKeyError::PendingExists),
        Ok(None) => {}
        Err(e) => return db_failed(&e),
    }
    let op = pending(move |app, caller| async move {
        if let Err(d) = admin_recheck(&caller, ACTION) {
            return d;
        }
        match relays::rotate_link_key(&app, &caller.actor()).await {
            Ok(_) => link_keys_event(&app).await,
            Err(e) => link_key_error(&e),
        }
    });
    preview(
        app,
        req,
        Draft {
            action: ACTION.to_owned(),
            summary: "Make a new relay-link key, pending. Keel keeps dialling with the active key \
                      until you activate the new one; add its public line to every relay's \
                      hub_keys first."
                .to_owned(),
            affected: vec!["relay-link key".to_owned()],
            step_up: true,
        },
        op,
    )
}

/// Activates (`activate`) or retires the pending key `fingerprint`.
async fn preview_key_step(
    app: &AppState,
    req: &Request<'_>,
    fingerprint: String,
    activate: bool,
) -> Vec<Event> {
    let action = if activate {
        "preview_relay_link_key_activate"
    } else {
        "preview_relay_link_key_retire"
    };
    if let Err(reason) = check_admin(req.caller) {
        return deny_admin(app, req, action, reason).await;
    }
    if activate && app.repos.master_key.is_none() {
        return key_unavailable();
    }
    match pending_key(app).await {
        Ok(Some(fp)) if fp == fingerprint => {}
        Ok(_) => return link_key_error(&LinkKeyError::NotPending),
        Err(e) => return db_failed(&e),
    }
    let (summary, warnings) = if activate {
        let relays: Vec<(String,)> =
            match sqlx::query_as("select name from relays where revoked_at is null order by name")
                .fetch_all(&app.db)
                .await
            {
                Ok(r) => r,
                Err(e) => return db_failed(&e),
            };
        let mut warnings = vec![
            "Every relay must already list the new key in its hub_keys; one that does not \
             refuses Keel's next dial."
                .to_owned(),
        ];
        if !relays.is_empty() {
            warnings.push(format!(
                "Relays: {}.",
                relays
                    .into_iter()
                    .map(|r| r.0)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        (
            format!(
                "Activate relay-link key {fingerprint} and retire the active one. Links up now \
                 stay up; every new dial uses the new key."
            ),
            warnings,
        )
    } else {
        (
            format!("Retire the pending relay-link key {fingerprint}. The active key stays."),
            Vec::new(),
        )
    };
    let op = pending(move |app, caller| async move {
        if let Err(d) = admin_recheck(&caller, action) {
            return d;
        }
        let actor = caller.actor();
        let done = if activate {
            relays::activate_link_key(&app, &actor, &fingerprint).await
        } else {
            relays::retire_link_key(&app, &actor, &fingerprint).await
        };
        match done {
            Ok(()) => link_keys_event(&app).await,
            Err(e) => link_key_error(&e),
        }
    });
    with_warnings(
        preview(
            app,
            req,
            Draft {
                action: action.to_owned(),
                summary,
                affected: vec!["relay-link key".to_owned()],
                step_up: true,
            },
            op,
        ),
        &warnings,
    )
}
