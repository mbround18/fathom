//! Runs outside Keel (spec 058 amendment A2). A host's runner reports what its
//! `/var/lib/keel-outside/runs.jsonl` says was run on it without Keel; Keel stores those
//! entries as the host's claims and keeps `hosts.changed_outside`:
//!
//! - set by a stored entry that was not a `--check` run;
//! - cleared by a baseline approval for the host ([`clear`], from the review), or by a
//!   succeeded run of every job assigned to the host that started after the newest such entry
//!   ([`after_finished`]).
//!
//! The entries are what the host says. Nothing here trusts them beyond storing and showing them.

use keel_core::WorkspaceId;
use keel_core::manifest::JobDefinition;
use keel_exec::frames::OutsideRun;
use keel_protocol::{Event, OutsideRunInfo};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::audit::{self, Actor, Decision, Entry};
use crate::bindings::ts;
use crate::serve::AppState;

/// How many entries `GetHost` shows, newest first.
pub const SHOWN: i64 = 20;

/// The host's newest entries, newest first.
pub async fn newest(db: &PgPool, host: Uuid) -> Result<Vec<OutsideRunInfo>, sqlx::Error> {
    type Row = (
        OffsetDateTime,
        String,
        String,
        Option<String>,
        bool,
        bool,
        OffsetDateTime,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "select at, \"user\", playbook, commit, dirty, \"check\", received_at \
         from host_outside_runs where host_id = $1 \
         order by at desc, received_at desc limit $2",
    )
    .bind(host)
    .bind(SHOWN)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(at, user, playbook, commit, dirty, check, received_at)| OutsideRunInfo {
                at: ts(at),
                user,
                playbook,
                commit,
                dirty,
                check,
                received_at: ts(received_at),
            },
        )
        .collect())
}

/// What one batch did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ingested {
    /// Entries stored for the first time.
    pub new: u64,
    /// Entries the runner skipped plus those refused here.
    pub skipped: u64,
    /// Whether this batch set `changed_outside`.
    pub flagged: bool,
}

/// Stores one `OutsideRuns` batch from `host`: an entry already stored is ignored, a malformed
/// one is counted as skipped. Audits the batch and, when it sets the flag, pushes
/// `HostChangedOutside`. An error leaves nothing stored, so the runner sends the batch again.
pub async fn ingest(
    app: &AppState,
    host: Uuid,
    entries: &[OutsideRun],
    skipped: u32,
) -> Result<Ingested, sqlx::Error> {
    let mut tx = app.db.begin().await?;
    let Some((workspace,)): Option<(Uuid,)> =
        sqlx::query_as("select workspace_id from hosts where id = $1 and state <> 'revoked'")
            .bind(host)
            .fetch_optional(&mut *tx)
            .await?
    else {
        return Ok(Ingested::default());
    };
    let mut done = Ingested {
        skipped: u64::from(skipped),
        ..Ingested::default()
    };
    let mut unchecked = false;
    for e in entries {
        let Some(at) = e
            .check()
            .ok()
            .and_then(|()| OffsetDateTime::parse(&e.at, &Rfc3339).ok())
        else {
            done.skipped += 1;
            continue;
        };
        let r = sqlx::query(
            "insert into host_outside_runs (host_id, at, \"user\", playbook, commit, dirty, \"check\") \
             values ($1, $2, $3, $4, $5, $6, $7) on conflict do nothing",
        )
        .bind(host)
        .bind(at)
        .bind(&e.user)
        .bind(&e.playbook)
        .bind(&e.commit)
        .bind(e.dirty)
        .bind(e.check)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() > 0 {
            done.new += 1;
            unchecked |= !e.check;
        }
    }
    if unchecked {
        done.flagged = sqlx::query(
            "update hosts set changed_outside = true where id = $1 and not changed_outside",
        )
        .bind(host)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
    }
    // A batch that adds nothing (a re-send) is not audited again.
    if done.new > 0 || done.skipped > 0 {
        audit::record(
            &mut tx,
            Entry {
                actor: &Actor::Host(host),
                action: "host_outside_runs_reported",
                workspace: Some(WorkspaceId(workspace)),
                target: json!({ "host": host }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({
                    "received": entries.len(),
                    "new": done.new,
                    "skipped": done.skipped,
                    "changed_outside": unchecked,
                }),
            },
        )
        .await?;
    }
    tx.commit().await?;
    if done.flagged {
        app.repos
            .push(workspace, Event::HostChangedOutside { host });
    }
    Ok(done)
}

/// Clears `changed_outside` on `hosts` (inventory names in `workspace`) for a baseline approval,
/// in the caller's transaction. Returns the hosts cleared; push [`pushed`] for them after commit.
pub async fn clear(
    tx: &mut PgConnection,
    workspace: Uuid,
    hosts: &[String],
    why: &str,
    actor: &Actor,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let cleared: Vec<(Uuid,)> = sqlx::query_as(
        "update hosts set changed_outside = false \
         where workspace_id = $1 and inventory_name = any($2) and changed_outside returning id",
    )
    .bind(workspace)
    .bind(hosts)
    .fetch_all(&mut *tx)
    .await?;
    let cleared: Vec<Uuid> = cleared.into_iter().map(|(h,)| h).collect();
    for host in &cleared {
        audit::record(
            &mut *tx,
            Entry {
                actor,
                action: "host_changed_outside_cleared",
                workspace: Some(WorkspaceId(workspace)),
                target: json!({ "host": host }),
                decision: Decision::Allowed,
                reason: Some(why),
                context: Value::Null,
            },
        )
        .await?;
    }
    Ok(cleared)
}

/// Pushes `HostChangedOutside` for each cleared host.
pub fn pushed(app: &AppState, workspace: Uuid, hosts: &[Uuid]) {
    for &host in hosts {
        app.repos
            .push(workspace, Event::HostChangedOutside { host });
    }
}

/// After a run finished on `host`: clears the flag when every job assigned to the host has a
/// succeeded run on it that started after the newest entry without `--check`. A host no job
/// reaches stays flagged until a baseline approval.
pub async fn after_finished(app: &AppState, host: Uuid) -> Result<bool, sqlx::Error> {
    let mut conn = app.db.acquire().await?;
    let row: Option<(Uuid, Option<OffsetDateTime>)> = sqlx::query_as(
        "select h.workspace_id, (select max(o.at) from host_outside_runs o \
            where o.host_id = h.id and not o.\"check\") \
         from hosts h where h.id = $1 and h.changed_outside",
    )
    .bind(host)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((workspace, newest)) = row else {
        return Ok(false);
    };
    let jobs: Vec<(Uuid, String, Value)> = sqlx::query_as(
        "select j.id, j.name, j.definition from jobs j \
         join repo_bindings b on b.id = j.binding_id join projects p on p.id = b.project_id \
         where p.workspace_id = $1 and j.removed_at_commit is null and j.blocked_reason is null",
    )
    .bind(workspace)
    .fetch_all(&mut *conn)
    .await?;
    let mut assigned = 0usize;
    for (job, name, def) in jobs {
        let Ok(def) = serde_json::from_value::<JobDefinition>(def) else {
            continue;
        };
        let Some((hosts, _, _)) =
            crate::runs::plan_hosts(&mut conn, workspace, &name, &def).await?
        else {
            continue;
        };
        if !hosts.iter().any(|(id, _)| *id == host) {
            continue;
        }
        assigned += 1;
        let (ran,): (bool,) = sqlx::query_as(
            "select exists (select 1 from run_hosts rh join runs r on r.id = rh.run_id \
             where r.job_id = $1 and rh.host_id = $2 and rh.state = 'succeeded' \
             and ($3::timestamptz is null or rh.started_at > $3))",
        )
        .bind(job)
        .bind(host)
        .bind(newest)
        .fetch_one(&mut *conn)
        .await?;
        if !ran {
            return Ok(false);
        }
    }
    if assigned == 0 {
        return Ok(false);
    }
    let mut tx = app.db.begin().await?;
    let cleared =
        sqlx::query("update hosts set changed_outside = false where id = $1 and changed_outside")
            .bind(host)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
    if cleared {
        audit::record(
            &mut tx,
            Entry {
                actor: &Actor::System,
                action: "host_changed_outside_cleared",
                workspace: Some(WorkspaceId(workspace)),
                target: json!({ "host": host }),
                decision: Decision::Allowed,
                reason: Some("every assigned job ran after the newest outside run"),
                context: json!({ "jobs": assigned }),
            },
        )
        .await?;
    }
    tx.commit().await?;
    drop(conn);
    if cleared {
        pushed(app, workspace, &[host]);
    }
    Ok(cleared)
}
