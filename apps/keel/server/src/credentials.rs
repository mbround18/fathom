//! Credentials (spec 055 T047, data-model `credentials`, research R12).
//!
//! - Kinds: `vault_password`, `env`, `secret_vars`, `git_https_token`, `git_ssh_key`.
//! - Values are sealed at rest under the master key (AAD `credential:<id>`) and never returned:
//!   lists show the name, kind and a fingerprint (SHA-256 of the plaintext, truncated).
//! - Create, rotate and grant need step-up. Without a master key they fail closed.
//! - A manifest's credential name resolves only through a grant to the binding.
//! - Removing a credential a binding fetches with is refused. (Runs arrive with US2; the
//!   "pending or queued run needs it" check joins then.)

use keel_core::authz::Capability;
use keel_core::domain::WorkspaceId;
use keel_protocol::{
    CredentialGrantInfo, CredentialInfo, CredentialKind, Event, ObjectName, SecretValue,
};
use secrecy::SecretSlice;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
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

/// Why a credential could not be opened. Never carries the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    NotFound,
    /// No master key, or the sealed value does not open.
    Sealing,
    Database,
}

/// The AAD a credential's value is sealed under.
fn aad(id: Uuid) -> Vec<u8> {
    format!("credential:{id}").into_bytes()
}

/// Lowercase hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// `sha256:` and the first 16 bytes of SHA-256 over the plaintext, in hex.
#[must_use]
pub fn fingerprint(value: &[u8]) -> String {
    let d = Sha256::digest(value);
    format!("sha256:{}", hex(&d[..16]))
}

/// Opens a credential: its kind and plaintext. The caller holds it only while it needs it.
///
/// # Errors
///
/// Missing, no master key, or it does not open.
pub async fn open(app: &AppState, id: Uuid) -> Result<(String, SecretSlice<u8>), OpenError> {
    let row: Option<(String, Vec<u8>)> =
        sqlx::query_as("select kind, sealed from credentials where id = $1")
            .bind(id)
            .fetch_optional(&app.db)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "credential lookup failed");
                OpenError::Database
            })?;
    let (kind, sealed) = row.ok_or(OpenError::NotFound)?;
    let key = app.repos.master_key.as_ref().ok_or(OpenError::Sealing)?;
    let value = key
        .open(&aad(id), &sealed)
        .map_err(|_| OpenError::Sealing)?;
    Ok((kind, value))
}

fn sealing_unavailable() -> Vec<Event> {
    op_error(
        "sealing_unavailable",
        "Keel has no master key loaded, so it cannot store or use credentials.",
    )
}

/// Refuses values that cannot be what their kind says. Messages never echo the value.
fn check_value(kind: CredentialKind, value: &str) -> Result<(), Vec<Event>> {
    let bad = |m: &str| Err(op_error("credential_invalid", m));
    match kind {
        CredentialKind::GitSshKey => {
            if !(value.contains("-----BEGIN") && value.contains("PRIVATE KEY-----")) {
                return bad("A git_ssh_key is an OpenSSH or PEM private key.");
            }
        }
        CredentialKind::GitHttpsToken => {
            if value.trim().is_empty() || value.trim().contains(char::is_whitespace) {
                return bad("A git_https_token is one token, or username:token.");
            }
        }
        CredentialKind::Env => {
            for line in value.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let ok = line.split_once('=').is_some_and(|(k, _)| {
                    !k.is_empty()
                        && !k.starts_with(|c: char| c.is_ascii_digit())
                        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                });
                if !ok {
                    return bad("An env credential is NAME=value lines.");
                }
            }
        }
        CredentialKind::SecretVars => {
            if !matches!(serde_json::from_str::<Value>(value), Ok(Value::Object(_))) {
                return bad("secret_vars is a JSON object of variable names to values.");
            }
        }
        CredentialKind::VaultPassword => {}
    }
    Ok(())
}

/// The credential's workspace and name.
async fn locate(app: &AppState, id: Uuid) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as("select workspace_id, name from credentials where id = $1")
        .bind(id)
        .fetch_optional(&app.db)
        .await
}

pub async fn list(app: &AppState, req: &Request<'_>, workspace: Uuid) -> Vec<Event> {
    if let Err(e) = require(app, req, "list_credentials", Capability::View, workspace).await {
        return e;
    }
    items(app, workspace).await
}

/// The workspace's credentials, as `Credentials`. Confirmed changes answer with it too.
async fn items(app: &AppState, workspace: Uuid) -> Vec<Event> {
    type Row = (
        Uuid,
        String,
        String,
        String,
        OffsetDateTime,
        Option<OffsetDateTime>,
    );
    let rows: Result<Vec<Row>, _> = sqlx::query_as(
        "select id, name, kind, fingerprint, created_at, rotated_at from credentials \
         where workspace_id = $1 order by name",
    )
    .bind(workspace)
    .fetch_all(&app.db)
    .await;
    let rows = match rows {
        Ok(r) => r,
        Err(e) => return db_failed(&e),
    };
    let grants: Result<Vec<(Uuid, Uuid, Vec<String>)>, _> = sqlx::query_as(
        "select g.credential_id, g.binding_id, g.profiles from credential_grants g \
         join credentials c on c.id = g.credential_id where c.workspace_id = $1 \
         order by g.created_at",
    )
    .bind(workspace)
    .fetch_all(&app.db)
    .await;
    let grants = match grants {
        Ok(g) => g,
        Err(e) => return db_failed(&e),
    };
    let items = rows
        .into_iter()
        .filter_map(|(id, name, kind, fingerprint, created, rotated)| {
            Some(CredentialInfo {
                id,
                workspace,
                name,
                kind: enum_from(&kind)?,
                fingerprint,
                created_at: ts(created),
                rotated_at: rotated.map(ts),
                grants: grants
                    .iter()
                    .filter(|g| g.0 == id)
                    .map(|g| CredentialGrantInfo {
                        binding: g.1,
                        profiles: g.2.clone(),
                    })
                    .collect(),
            })
        })
        .collect();
    vec![Event::Credentials { items }]
}

pub async fn preview_create(
    app: &AppState,
    req: &Request<'_>,
    workspace: Uuid,
    name: ObjectName,
    kind: CredentialKind,
    value: SecretValue,
) -> Vec<Event> {
    const ACTION: &str = "preview_credential_create";
    if let Err(e) = require(app, req, ACTION, Capability::Administer, workspace).await {
        return e;
    }
    if app.repos.master_key.is_none() {
        return sealing_unavailable();
    }
    if let Err(e) = check_value(kind, value.as_str()) {
        return e;
    }
    let taken: Result<Option<(Uuid,)>, _> =
        sqlx::query_as("select id from credentials where workspace_id = $1 and name = $2")
            .bind(workspace)
            .bind(name.as_str())
            .fetch_optional(&app.db)
            .await;
    match taken {
        Ok(Some(_)) => return op_error("name_taken", "A credential with that name exists."),
        Ok(None) => {}
        Err(e) => return db_failed(&e),
    }
    let fp = fingerprint(value.as_str().as_bytes());
    let summary = format!(
        "Create the {} credential \"{}\" ({fp}). Its value is sealed and never shown again.",
        enum_str(&kind),
        name.as_str()
    );
    let affected = vec![name.as_str().to_owned()];
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move { create(&app, &caller, workspace, &name, kind, &value, ctx).await })
    });
    preview(
        app,
        req,
        Draft {
            action: "credential_create".to_owned(),
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
    kind: CredentialKind,
    value: &SecretValue,
    ctx: Value,
) -> Vec<Event> {
    let Some(key) = app.repos.master_key.clone() else {
        return sealing_unavailable();
    };
    let id = Uuid::now_v7();
    let Ok(sealed) = key.seal(&aad(id), value.as_str().as_bytes()) else {
        return sealing_unavailable();
    };
    let fp = fingerprint(value.as_str().as_bytes());
    let result: Result<bool, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let n = sqlx::query(
            "insert into credentials (id, workspace_id, name, kind, sealed, fingerprint, \
             created_by) values ($1, $2, $3, $4, $5, $6, $7) \
             on conflict (workspace_id, name) do nothing",
        )
        .bind(id)
        .bind(workspace)
        .bind(name.as_str())
        .bind(enum_str(&kind))
        .bind(&sealed)
        .bind(&fp)
        .bind(caller.user.id)
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
                action: "credential_created",
                workspace: Some(WorkspaceId(workspace)),
                target: json!({ "credential": id, "name": name.as_str() }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({ "kind": enum_str(&kind), "fingerprint": fp, "request": ctx }),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }
    .await;
    match result {
        Ok(true) => items(app, workspace).await,
        Ok(false) => op_error("name_taken", "A credential with that name exists."),
        Err(e) => db_failed(&e),
    }
}

pub async fn preview_rotate(
    app: &AppState,
    req: &Request<'_>,
    credential: Uuid,
    value: SecretValue,
) -> Vec<Event> {
    const ACTION: &str = "preview_credential_rotate";
    let row: Result<Option<(Uuid, String, String)>, _> =
        sqlx::query_as("select workspace_id, name, kind from credentials where id = $1")
            .bind(credential)
            .fetch_optional(&app.db)
            .await;
    let (ws, name, kind) = match row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("credential"),
        Err(e) => return db_failed(&e),
    };
    if let Err(e) = require(app, req, ACTION, Capability::Administer, ws).await {
        return e;
    }
    if app.repos.master_key.is_none() {
        return sealing_unavailable();
    }
    let Some(kind) = enum_from::<CredentialKind>(&kind) else {
        return bindings_internal();
    };
    if let Err(e) = check_value(kind, value.as_str()) {
        return e;
    }
    let fp = fingerprint(value.as_str().as_bytes());
    let summary =
        format!("Rotate the credential \"{name}\" to a new value ({fp}). The old value is gone.");
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move { rotate(&app, &caller, ws, credential, &value, ctx).await })
    });
    preview(
        app,
        req,
        Draft {
            action: "credential_rotate".to_owned(),
            summary,
            affected: vec![name],
            step_up: true,
        },
        op,
    )
}

fn bindings_internal() -> Vec<Event> {
    crate::bindings::internal()
}

async fn rotate(
    app: &AppState,
    caller: &Caller,
    ws: Uuid,
    credential: Uuid,
    value: &SecretValue,
    ctx: Value,
) -> Vec<Event> {
    let Some(key) = app.repos.master_key.clone() else {
        return sealing_unavailable();
    };
    let Ok(sealed) = key.seal(&aad(credential), value.as_str().as_bytes()) else {
        return sealing_unavailable();
    };
    let fp = fingerprint(value.as_str().as_bytes());
    let now = app.clock.now();
    let result: Result<u64, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let n = sqlx::query(
            "update credentials set sealed = $2, fingerprint = $3, rotated_at = $4 where id = $1",
        )
        .bind(credential)
        .bind(&sealed)
        .bind(&fp)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 1 {
            audit::record(
                &mut tx,
                Entry {
                    actor: &caller.actor(),
                    action: "credential_rotated",
                    workspace: Some(WorkspaceId(ws)),
                    target: json!({ "credential": credential }),
                    decision: Decision::Allowed,
                    reason: None,
                    context: json!({ "fingerprint": fp, "request": ctx }),
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(n)
    }
    .await;
    match result {
        Ok(1) => items(app, ws).await,
        Ok(_) => not_found("credential"),
        Err(e) => db_failed(&e),
    }
}

/// Checks the credential and the binding share a workspace; returns it and the names.
async fn pair(
    app: &AppState,
    credential: Uuid,
    binding: Uuid,
) -> Result<(Uuid, String, String), Vec<Event>> {
    let (ws, name) = match locate(app, credential).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(not_found("credential")),
        Err(e) => return Err(db_failed(&e)),
    };
    let bws = match binding_workspace(app, binding).await {
        Ok(Some(b)) => b,
        Ok(None) => return Err(not_found("binding")),
        Err(e) => return Err(db_failed(&e)),
    };
    if bws != ws {
        return Err(op_error(
            "workspace_mismatch",
            "The credential and the binding are in different workspaces.",
        ));
    }
    let label: Option<(String,)> =
        sqlx::query_as("select url || ' (' || branch || ')' from repo_bindings where id = $1")
            .bind(binding)
            .fetch_optional(&app.db)
            .await
            .map_err(|e| db_failed(&e))?;
    Ok((ws, name, label.map(|l| l.0).unwrap_or_default()))
}

pub async fn preview_grant(
    app: &AppState,
    req: &Request<'_>,
    credential: Uuid,
    binding: Uuid,
    profiles: Vec<ObjectName>,
) -> Vec<Event> {
    const ACTION: &str = "preview_credential_grant";
    let (ws, name, label) = match pair(app, credential, binding).await {
        Ok(p) => p,
        Err(e) => return e,
    };
    if let Err(e) = require(app, req, ACTION, Capability::Administer, ws).await {
        return e;
    }
    let profiles: Vec<String> = profiles.iter().map(|p| p.as_str().to_owned()).collect();
    let scope = if profiles.is_empty() {
        "hosts of every profile".to_owned()
    } else {
        format!("hosts with profile {}", profiles.join(", "))
    };
    let summary = format!(
        "Grant the credential \"{name}\" to jobs of {label}, for {scope}. Jobs naming it stop \
         being blocked at the next sync."
    );
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move { grant(&app, &caller, ws, credential, binding, &profiles, ctx).await })
    });
    preview(
        app,
        req,
        Draft {
            action: "credential_grant".to_owned(),
            summary,
            affected: vec![name, label],
            step_up: true,
        },
        op,
    )
}

async fn grant(
    app: &AppState,
    caller: &Caller,
    ws: Uuid,
    credential: Uuid,
    binding: Uuid,
    profiles: &[String],
    ctx: Value,
) -> Vec<Event> {
    let result: Result<(), sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        sqlx::query(
            "insert into credential_grants (credential_id, binding_id, profiles, created_by) \
             values ($1, $2, $3, $4) on conflict (credential_id, binding_id) \
             do update set profiles = excluded.profiles",
        )
        .bind(credential)
        .bind(binding)
        .bind(profiles)
        .bind(caller.user.id)
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &caller.actor(),
                action: "credential_granted",
                workspace: Some(WorkspaceId(ws)),
                target: json!({ "credential": credential, "binding": binding }),
                decision: Decision::Allowed,
                reason: None,
                context: json!({ "profiles": profiles, "request": ctx }),
            },
        )
        .await?;
        tx.commit().await
    }
    .await;
    match result {
        Ok(()) => items(app, ws).await,
        Err(e) => db_failed(&e),
    }
}

pub async fn preview_revoke(
    app: &AppState,
    req: &Request<'_>,
    credential: Uuid,
    binding: Uuid,
) -> Vec<Event> {
    const ACTION: &str = "preview_credential_revoke";
    let (ws, name, label) = match pair(app, credential, binding).await {
        Ok(p) => p,
        Err(e) => return e,
    };
    if let Err(e) = require(app, req, ACTION, Capability::Administer, ws).await {
        return e;
    }
    let summary = format!(
        "Revoke the credential \"{name}\" from {label}. Jobs naming it are blocked \
         (credential_unavailable) from the next sync."
    );
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            delete(
                &app,
                &caller,
                ws,
                "credential_revoked",
                sqlx::query(
                    "delete from credential_grants where credential_id = $1 and binding_id = $2",
                )
                .bind(credential)
                .bind(binding),
                json!({ "credential": credential, "binding": binding }),
                ctx,
            )
            .await
        })
    });
    preview(
        app,
        req,
        Draft {
            action: "credential_revoke".to_owned(),
            summary,
            affected: vec![name, label],
            step_up: true,
        },
        op,
    )
}

pub async fn preview_remove(app: &AppState, req: &Request<'_>, credential: Uuid) -> Vec<Event> {
    const ACTION: &str = "preview_credential_remove";
    let (ws, name) = match locate(app, credential).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("credential"),
        Err(e) => return db_failed(&e),
    };
    if let Err(e) = require(app, req, ACTION, Capability::Administer, ws).await {
        return e;
    }
    match in_use(app, credential).await {
        Ok(0) => {}
        Ok(n) => {
            return op_error(
                "credential_in_use",
                &format!("{n} binding(s) fetch with this credential. Change them first."),
            );
        }
        Err(e) => return db_failed(&e),
    }
    let summary = format!(
        "Remove the credential \"{name}\". Its grants go with it, and jobs naming it are blocked \
         from the next sync."
    );
    let ctx = channel(req);
    let op: PendingOp = Box::new(move |app: AppState, caller: Caller| {
        Box::pin(async move {
            // Re-checked: a binding may have started using it since the preview.
            match in_use(&app, credential).await {
                Ok(0) => {}
                Ok(_) => {
                    return op_error(
                        "credential_in_use",
                        "A binding fetches with this credential. Change it first.",
                    );
                }
                Err(e) => return db_failed(&e),
            }
            delete(
                &app,
                &caller,
                ws,
                "credential_removed",
                sqlx::query("delete from credentials where id = $1").bind(credential),
                json!({ "credential": credential }),
                ctx,
            )
            .await
        })
    });
    preview(
        app,
        req,
        Draft {
            action: "credential_remove".to_owned(),
            summary,
            affected: vec![name],
            step_up: false,
        },
        op,
    )
}

async fn in_use(app: &AppState, credential: Uuid) -> Result<i64, sqlx::Error> {
    let (n,): (i64,) =
        sqlx::query_as("select count(*) from repo_bindings where credential_id = $1")
            .bind(credential)
            .fetch_one(&app.db)
            .await?;
    Ok(n)
}

/// Runs one delete and audits it when it removed something.
async fn delete(
    app: &AppState,
    caller: &Caller,
    ws: Uuid,
    action: &'static str,
    query: sqlx::query::Query<'_, sqlx::Postgres, sqlx::postgres::PgArguments>,
    target: Value,
    ctx: Value,
) -> Vec<Event> {
    let result: Result<u64, sqlx::Error> = async {
        let mut tx = app.db.begin().await?;
        let n = query.execute(&mut *tx).await?.rows_affected();
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
        Ok(0) => not_found("credential grant"),
        Ok(_) => items(app, ws).await,
        Err(e) => db_failed(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_are_truncated_sha256() {
        let fp = fingerprint(b"hunter2");
        assert!(fp.starts_with("sha256:"));
        assert_eq!(fp.len(), "sha256:".len() + 32);
        assert!(!fp.contains("hunter2"));
    }

    #[test]
    fn values_are_checked_by_kind() {
        assert!(check_value(CredentialKind::Env, "A=1\n# c\nB_2=x").is_ok());
        assert!(check_value(CredentialKind::Env, "1A=1").is_err());
        assert!(check_value(CredentialKind::SecretVars, r#"{"db_pass":"x"}"#).is_ok());
        assert!(check_value(CredentialKind::SecretVars, "[1]").is_err());
        assert!(check_value(CredentialKind::GitSshKey, "nope").is_err());
        assert!(check_value(CredentialKind::GitHttpsToken, "a b").is_err());
        assert!(check_value(CredentialKind::VaultPassword, "anything").is_ok());
    }
}
