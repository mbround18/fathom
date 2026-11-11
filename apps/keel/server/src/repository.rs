//! A workspace's primary repository: its catalog, job drafts, the forge connection and the
//! changes Keel proposes (spec 066).
//!
//! - `PreviewWorkspacePrimary` and `PreviewWorkspaceRepository` (US2) set which binding the
//!   catalog is built from. The second creates the binding on the `default` project through the
//!   binding create flow, so it validates and test-fetches exactly as `PreviewBindingCreate`.
//! - The forge connection (FR-008) names a provider, an API base and a `forge_token`
//!   credential. `ProbeForge` asks the forge what the token may do, through keel-forge, in a
//!   `forge.probe` span; the token never leaves the server or reaches the audit log.
//!
//! The catalog, drafts and proposals (Phase 3) are refused `not_implemented` until they land.

use keel_core::authz::Capability;
use keel_forge::{Forge, ForgeClient, ForgeConfig, Provider};
use keel_protocol::RepositorySpec;
use keel_protocol::{ApiBase, BindingSpec, Cmd, Event, ForgeConnectionInfo, ForgeProvider};
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use time::OffsetDateTime;
use tracing::Instrument;
use uuid::Uuid;

use crate::bindings::{self, CreateAs, enum_from, enum_str};
use crate::dispatch::Request;
use crate::governance::{
    self, Fail, Step, check_ws, deny, live_in_tx, live_workspace, origin, pending, record, settle,
};
use crate::preview::Draft;
use crate::serve::AppState;

/// Handles the spec 066 commands.
pub async fn handle(app: &AppState, req: &Request<'_>, cmd: Cmd) -> Vec<Event> {
    match cmd {
        Cmd::PreviewWorkspacePrimary { workspace, binding } => {
            primary(app, req, workspace, binding).await
        }
        Cmd::PreviewWorkspaceRepository { workspace, spec } => {
            repository(app, req, workspace, spec).await
        }
        Cmd::GetForgeConnection { workspace } => get_forge(app, req, workspace).await,
        Cmd::PreviewForgeConnection {
            workspace,
            provider,
            api_base,
            credential,
        } => forge_set(app, req, workspace, provider, api_base, credential).await,
        Cmd::PreviewForgeRemove { workspace } => forge_remove(app, req, workspace).await,
        Cmd::ProbeForge { workspace } => probe(app, req, workspace).await,
        other => vec![Event::op_error(
            "not_implemented",
            format!("{} is not available yet.", other.name()),
        )],
    }
}

fn draft(action: &str, summary: String, affected: Vec<String>) -> Draft {
    Draft {
        action: action.to_owned(),
        summary,
        affected,
        step_up: false,
    }
}

fn op(code: &'static str, message: &str) -> Vec<Event> {
    vec![Event::op_error(code, message)]
}

fn no_primary() -> Vec<Event> {
    op(
        "no_primary_repository",
        "This workspace has no primary repository yet. Set one in Settings → Workspace.",
    )
}

fn not_configured() -> Vec<Event> {
    op(
        "forge_not_configured",
        "This workspace has no forge connection.",
    )
}

// ---- The primary repository ------------------------------------------------------------------

/// The binding's workspace, URL and branch.
async fn binding_row(
    app: &AppState,
    binding: Uuid,
) -> Result<Option<(Uuid, String, String)>, sqlx::Error> {
    sqlx::query_as(
        "select p.workspace_id, b.url, b.branch from repo_bindings b \
         join projects p on p.id = b.project_id where b.id = $1",
    )
    .bind(binding)
    .fetch_optional(&app.db)
    .await
}

/// The URL of the workspace's primary binding, if it has one.
async fn primary_url(app: &AppState, ws: Uuid) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as(
        "select b.url from workspaces w join repo_bindings b on b.id = w.primary_binding \
         where w.id = $1",
    )
    .bind(ws)
    .fetch_optional(&app.db)
    .await?;
    Ok(row.map(|r| r.0))
}

async fn primary(app: &AppState, req: &Request<'_>, ws: Uuid, binding: Option<Uuid>) -> Vec<Event> {
    const CMD: &str = "preview_workspace_primary";
    const ACTION: &str = "workspace_primary_set";
    let o = origin(req);
    if let Err(code) = check_ws(req.caller, ws, Capability::Administer) {
        return deny(app, req.caller, CMD, Some(ws), code, o.context).await;
    }
    let name = match live_workspace(app, ws).await {
        Ok(n) => n,
        Err(e) => return e,
    };
    let summary = match binding {
        Some(b) => match binding_row(app, b).await {
            Ok(Some((w, url, branch))) if w == ws => {
                format!("Make {url} ({branch}) the primary repository of {name}.")
            }
            Ok(_) => return bindings::not_found("binding"),
            Err(e) => return bindings::db_failed(&e),
        },
        None => format!(
            "Clear the primary repository of {name}. The binding stays; Keel stops building its \
             catalog."
        ),
    };
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_ws(&caller, ws, Capability::Administer) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            if let Some(f) = live_in_tx(&mut tx, ws).await? {
                return Ok(Err(f));
            }
            if let Some(b) = binding {
                let ours: bool = sqlx::query_scalar(
                    "select exists(select 1 from repo_bindings b join projects p \
                     on p.id = b.project_id where b.id = $1 and p.workspace_id = $2)",
                )
                .bind(b)
                .bind(ws)
                .fetch_one(&mut *tx)
                .await?;
                if !ours {
                    return Ok(Err(Fail::Op(
                        "not_found",
                        "That binding does not exist.".into(),
                    )));
                }
            }
            sqlx::query(
                "update workspaces set primary_binding = $1, primary_set_by = $2, \
                 primary_set_at = now() where id = $3",
            )
            .bind(binding)
            .bind(caller.user.id)
            .bind(ws)
            .execute(&mut *tx)
            .await?;
            let target = json!({ "binding": binding });
            record(&mut tx, &caller, ACTION, Some(ws), target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, Some(ws), &o, step).await {
            return events;
        }
        governance::list_workspaces(&app, &caller).await
    });
    governance::preview(app, req, draft(ACTION, summary, vec![name]), op)
}

async fn repository(
    app: &AppState,
    req: &Request<'_>,
    ws: Uuid,
    spec: RepositorySpec,
) -> Vec<Event> {
    const CMD: &str = "preview_workspace_repository";
    if let Err(e) = bindings::require(app, req, CMD, Capability::Administer, ws).await {
        return e;
    }
    if let Err(e) = live_workspace(app, ws).await {
        return e;
    }
    // Workspaces made before spec 066 were backfilled a `default` project, but one created
    // directly in the database may lack it: the confirm creates it then (R2).
    let default: Result<Option<(Uuid,)>, _> =
        sqlx::query_as("select id from projects where workspace_id = $1 and slug = 'default'")
            .bind(ws)
            .fetch_optional(&app.db)
            .await;
    let (project, ensure_default) = match default {
        Ok(Some((p,))) => (p, false),
        Ok(None) => (Uuid::now_v7(), true),
        Err(e) => return bindings::db_failed(&e),
    };
    let spec = BindingSpec {
        project,
        url: spec.url,
        branch: spec.branch,
        transport: spec.transport,
        credential: spec.credential,
        known_host: spec.known_host,
        poll_interval: spec.poll_interval,
        allow_force_push: spec.allow_force_push,
        signed_mode: spec.signed_mode,
        override_quorum: spec.override_quorum,
    };
    let create_as = CreateAs {
        primary: true,
        ensure_default,
    };
    bindings::preview_create_in(
        app,
        req,
        ws,
        "default",
        spec,
        "workspace_repository",
        create_as,
    )
    .await
}

// ---- The forge connection --------------------------------------------------------------------

type ForgeRow = (
    String,
    String,
    Uuid,
    Option<OffsetDateTime>,
    Option<String>,
    OffsetDateTime,
);

async fn forge_row(app: &AppState, ws: Uuid) -> Result<Option<ForgeConnectionInfo>, sqlx::Error> {
    let row: Option<ForgeRow> = sqlx::query_as(
        "select provider, api_base, credential_id, probed_at, probe_error, created_at \
         from forge_connections where workspace_id = $1",
    )
    .bind(ws)
    .fetch_optional(&app.db)
    .await?;
    Ok(row.and_then(
        |(provider, api_base, credential, probed_at, probe_error, created)| {
            Some(ForgeConnectionInfo {
                workspace: ws,
                provider: enum_from(&provider)?,
                api_base,
                credential,
                probed_at: probed_at.map(bindings::ts),
                probe_error,
                created_at: bindings::ts(created),
            })
        },
    ))
}

fn adapter(p: ForgeProvider) -> Provider {
    match p {
        ForgeProvider::Forgejo => Provider::Forgejo,
        ForgeProvider::Gitea => Provider::Gitea,
        ForgeProvider::Github => Provider::Github,
    }
}

async fn get_forge(app: &AppState, req: &Request<'_>, ws: Uuid) -> Vec<Event> {
    const CMD: &str = "get_forge_connection";
    if let Err(e) = bindings::require(app, req, CMD, Capability::View, ws).await {
        return e;
    }
    match forge_row(app, ws).await {
        Ok(info) => vec![Event::ForgeConnection(info)],
        Err(e) => bindings::db_failed(&e),
    }
}

/// The credential must be a `forge_token` in `ws`.
async fn check_token(app: &AppState, ws: Uuid, credential: Uuid) -> Result<String, Vec<Event>> {
    let row: Option<(Uuid, String, String)> =
        sqlx::query_as("select workspace_id, kind, name from credentials where id = $1")
            .bind(credential)
            .fetch_optional(&app.db)
            .await
            .map_err(|e| bindings::db_failed(&e))?;
    match row {
        Some((w, kind, name)) if w == ws && kind == "forge_token" => Ok(name),
        _ => Err(op(
            "credential_unavailable",
            "The credential must be a forge_token in this workspace.",
        )),
    }
}

async fn forge_set(
    app: &AppState,
    req: &Request<'_>,
    ws: Uuid,
    provider: ForgeProvider,
    api_base: Option<ApiBase>,
    credential: Uuid,
) -> Vec<Event> {
    const CMD: &str = "preview_forge_connection";
    const ACTION: &str = "forge_connection_set";
    let o = origin(req);
    if let Err(code) = check_ws(req.caller, ws, Capability::Administer) {
        return deny(app, req.caller, CMD, Some(ws), code, o.context).await;
    }
    let name = match live_workspace(app, ws).await {
        Ok(n) => n,
        Err(e) => return e,
    };
    let token_name = match check_token(app, ws, credential).await {
        Ok(n) => n,
        Err(e) => return e,
    };
    let api_base = match api_base {
        Some(a) => a.as_str().to_owned(),
        None => match primary_url(app, ws).await {
            Err(e) => return bindings::db_failed(&e),
            Ok(None) => return no_primary(),
            Ok(Some(url)) => match keel_forge::default_api_base(adapter(provider), &url) {
                Ok(b) => b,
                Err(_) => {
                    return op(
                        "url_invalid",
                        "Keel cannot tell the forge's API from the primary repository's URL. \
                         Give the API base.",
                    );
                }
            },
        },
    };
    let existing = match forge_row(app, ws).await {
        Ok(r) => r.is_some(),
        Err(e) => return bindings::db_failed(&e),
    };
    let verb = if existing { "Replace" } else { "Set" };
    let summary = format!(
        "{verb} the forge connection of {name}: {} at {api_base}, with the token {token_name}. \
         Keel uses it only to open pull requests on a new branch.",
        enum_str(&provider),
    );
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_ws(&caller, ws, Capability::Administer) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            if let Some(f) = live_in_tx(&mut tx, ws).await? {
                return Ok(Err(f));
            }
            let kind: Option<(Uuid, String)> =
                sqlx::query_as("select workspace_id, kind from credentials where id = $1")
                    .bind(credential)
                    .fetch_optional(&mut *tx)
                    .await?;
            if !matches!(&kind, Some((w, k)) if *w == ws && k == "forge_token") {
                return Ok(Err(Fail::Op(
                    "credential_unavailable",
                    "The credential must be a forge_token in this workspace.".into(),
                )));
            }
            sqlx::query(
                "insert into forge_connections \
                 (workspace_id, provider, api_base, credential_id, created_by) \
                 values ($1, $2, $3, $4, $5) \
                 on conflict (workspace_id) do update set provider = excluded.provider, \
                 api_base = excluded.api_base, credential_id = excluded.credential_id, \
                 probed_at = null, probe_error = null, created_by = excluded.created_by, \
                 created_at = now()",
            )
            .bind(ws)
            .bind(enum_str(&provider))
            .bind(&api_base)
            .bind(credential)
            .bind(caller.user.id)
            .execute(&mut *tx)
            .await?;
            let target = json!({
                "provider": enum_str(&provider), "api_base": api_base, "credential": credential,
            });
            record(&mut tx, &caller, ACTION, Some(ws), target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, Some(ws), &o, step).await {
            return events;
        }
        match forge_row(&app, ws).await {
            Ok(info) => vec![Event::ForgeConnection(info)],
            Err(e) => bindings::db_failed(&e),
        }
    });
    governance::preview(app, req, draft(ACTION, summary, vec![name]), op)
}

async fn forge_remove(app: &AppState, req: &Request<'_>, ws: Uuid) -> Vec<Event> {
    const CMD: &str = "preview_forge_remove";
    const ACTION: &str = "forge_connection_remove";
    let o = origin(req);
    if let Err(code) = check_ws(req.caller, ws, Capability::Administer) {
        return deny(app, req.caller, CMD, Some(ws), code, o.context).await;
    }
    let name = match live_workspace(app, ws).await {
        Ok(n) => n,
        Err(e) => return e,
    };
    match forge_row(app, ws).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_configured(),
        Err(e) => return bindings::db_failed(&e),
    }
    let summary = format!(
        "Remove the forge connection of {name}. Proposals then return the diff to apply by \
         hand; the token credential stays."
    );
    let op = pending(move |app, caller| async move {
        let step: Step = async {
            if let Err(code) = check_ws(&caller, ws, Capability::Administer) {
                return Ok(Err(Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            let gone = sqlx::query("delete from forge_connections where workspace_id = $1")
                .bind(ws)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if gone == 0 {
                return Ok(Err(Fail::Op(
                    "forge_not_configured",
                    "This workspace has no forge connection.".into(),
                )));
            }
            record(&mut tx, &caller, ACTION, Some(ws), json!({}), &o.context).await?;
            tx.commit().await?;
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = settle(&app, &caller, ACTION, Some(ws), &o, step).await {
            return events;
        }
        vec![Event::ForgeConnection(None)]
    });
    governance::preview(app, req, draft(ACTION, summary, vec![name]), op)
}

/// `ForgeProbe`: what the token may do on the primary repository. Reads only on the forge; the
/// outcome is kept on the connection so Settings shows it.
async fn probe(app: &AppState, req: &Request<'_>, ws: Uuid) -> Vec<Event> {
    const CMD: &str = "probe_forge";
    // It spends the workspace's token, so it is for those who manage the connection.
    if let Err(e) = bindings::require(app, req, CMD, Capability::Administer, ws).await {
        return e;
    }
    let info = match forge_row(app, ws).await {
        Ok(Some(i)) => i,
        Ok(None) => return not_configured(),
        Err(e) => return bindings::db_failed(&e),
    };
    let url = match primary_url(app, ws).await {
        Ok(Some(u)) => u,
        Ok(None) => return no_primary(),
        Err(e) => return bindings::db_failed(&e),
    };
    let token = match crate::credentials::open(app, info.credential).await {
        Ok((kind, value)) if kind == "forge_token" => {
            match String::from_utf8(value.expose_secret().to_vec()) {
                Ok(t) => SecretString::from(t.trim().to_owned()),
                Err(_) => return op("credential_invalid", "The forge token is not text."),
            }
        }
        _ => {
            return op(
                "credential_unavailable",
                "Keel cannot open the forge token credential.",
            );
        }
    };
    let span = tracing::info_span!(
        "forge.probe",
        provider = %enum_str(&info.provider),
        workspace = %ws,
        ok = tracing::field::Empty,
    );
    let outcome = async {
        let client = ForgeClient::new(ForgeConfig {
            provider: adapter(info.provider),
            repo_url: url,
            api_base: Some(info.api_base.clone()),
            token,
        })?;
        client.probe().await
    }
    .instrument(span.clone())
    .await;
    span.record("ok", outcome.is_ok());
    let error = outcome.as_ref().err().map(ToString::to_string);
    if let Err(e) = sqlx::query(
        "update forge_connections set probed_at = now(), probe_error = $2 where workspace_id = $1",
    )
    .bind(ws)
    .bind(&error)
    .execute(&app.db)
    .await
    {
        tracing::error!(error = %e, "recording the forge probe failed");
    }
    match outcome {
        Ok(access) => vec![Event::ForgeProbe {
            ok: true,
            can_push: access.can_push,
            message: (!access.can_push).then(|| {
                "The token can read the repository but not push to it, so Keel cannot open \
                 pull requests."
                    .to_owned()
            }),
        }],
        Err(_) => vec![Event::ForgeProbe {
            ok: false,
            can_push: false,
            message: error,
        }],
    }
}
