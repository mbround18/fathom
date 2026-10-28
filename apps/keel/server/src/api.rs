//! `POST /api/v1/cmd` (spec 055 T026): one non-streaming command over HTTP.
//!
//! - `Authorization: Bearer <token>`: an API token, narrowed to its workspace and actions.
//! - Otherwise the session cookie, with the same checks as any browser mutation: `Origin`,
//!   `Sec-Fetch-Site` and `X-CSRF-Token` (403 `{"error":"csrf"}`).
//!
//! The caller is re-read from the database first (a stale `authz_epoch` is 401). The body is a
//! `CmdEnvelope`; the reply is the JSON array of `EventEnvelope`s the command produced.
//! Streaming commands answer `OpError{use_websocket}`.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use keel_protocol::{CmdEnvelope, Event, EventEnvelope};
use web_auth::session::EndReason;

use crate::dispatch::{self, Channel, Request};
use crate::identity;
use crate::serve::{AppState, error};
// Governance (US6)
use crate::governance;
use crate::preview::Draft;
use keel_core::WorkspaceId;
use keel_core::authz::Capability;
use keel_protocol::{
    AdminApiTokenInfo, ApiTokenInfo, ApiTokenState, Label, MAX_TOKEN_DAYS, TOKEN_ACTIONS,
    TokenActions, TokenSecret,
};
use serde_json::json;

/// The header the client echoes the session's CSRF token in.
pub const CSRF_HEADER: &str = "x-csrf-token";

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|t| !t.is_empty())
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

pub async fn cmd(State(app): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let now = app.clock.now();
    let (caller, session) = if headers.contains_key(header::AUTHORIZATION) {
        let Some(token) = bearer(&headers) else {
            return app.auth.unauthenticated();
        };
        match identity::load_token(&app.db, token, now).await {
            Ok(Some(c)) => (c, None),
            Ok(None) => return app.auth.unauthenticated(),
            Err(e) => {
                tracing::error!(error = %e, "token lookup failed");
                return error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable");
            }
        }
    } else {
        let Some(session) = app.auth.session(&headers) else {
            return app.auth.unauthenticated();
        };
        if !web_auth::csrf::check_mutation(
            &app.origin,
            header_str(&headers, header::ORIGIN.as_str()),
            header_str(&headers, "sec-fetch-site"),
            &session.csrf,
            header_str(&headers, CSRF_HEADER),
        ) {
            return error(StatusCode::FORBIDDEN, "csrf");
        }
        match identity::load_session(&app.db, &session.id, now).await {
            Ok(Some(c)) => (c, Some(session.id)),
            Ok(None) => {
                // Stale epoch, disabled or expired: the session is over everywhere.
                app.auth.sessions.end(&session.id, EndReason::Revoked);
                app.previews.forget_session(&session.id);
                return app.auth.unauthenticated();
            }
            Err(e) => {
                tracing::error!(error = %e, "session lookup failed");
                return error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable");
            }
        }
    };
    let mut caller = caller;
    caller.system_workspace = app.system_workspace().map(WorkspaceId);
    let Ok(env) = serde_json::from_slice::<CmdEnvelope>(&body) else {
        return error(StatusCode::BAD_REQUEST, "bad_request");
    };
    if env.cmd.is_streaming() {
        let reply = EventEnvelope::reply(
            env.id,
            Event::op_error(
                "use_websocket",
                "This command streams. Send it over /api/v1/ws.",
            ),
        );
        return (StatusCode::BAD_REQUEST, Json(vec![reply])).into_response();
    }
    let req = Request {
        caller: &caller,
        session: session.as_deref(),
        channel: Channel::Api,
    };
    Json(dispatch::dispatch(&app, &req, env).await).into_response()
}

// Governance (US6): API tokens (spec 055 T037).
//
// - The secret is made at `Confirm`, shown once in `ApiTokenCreated`, and stored as its SHA-256
//   ([`identity::token_hash`]); it never reaches the audit log or the logs.
// - A token is scoped to one workspace (which its owner must at least see) and to a set of
//   command names. It acts as its owner, narrowed: [`identity::load_token`] keeps only the
//   owner's grants there and `dispatch::authorize` refuses any command not in `actions`.
// - Only commands on [`TOKEN_ACTIONS`] may be named; `authorize` also refuses anything else.
// - Tokens are managed from a session only (`authorize` refuses tokens with `session_required`).

type TokenRow = (
    uuid::Uuid,
    String,
    uuid::Uuid,
    uuid::Uuid,
    String,
    Vec<String>,
    time::OffsetDateTime,
    Option<time::OffsetDateTime>,
    bool,
);

/// `ApiTokens`: the caller's own, newest first.
pub async fn list_tokens(app: &AppState, req: &Request<'_>) -> Vec<Event> {
    tokens(app, req.caller.user.id).await
}

async fn tokens(app: &AppState, owner: uuid::Uuid) -> Vec<Event> {
    let rows: Result<Vec<TokenRow>, sqlx::Error> = sqlx::query_as(
        "select t.id, t.name, t.owner_user_id, t.workspace_id, w.slug, t.actions, t.expires_at, \
         t.last_used_at, t.revoked_at is not null from api_tokens t \
         join workspaces w on w.id = t.workspace_id where t.owner_user_id = $1 order by t.id desc",
    )
    .bind(owner)
    .fetch_all(&app.db)
    .await;
    match rows {
        Ok(rows) => vec![Event::ApiTokens {
            items: rows.into_iter().map(token_info).collect(),
        }],
        Err(e) => {
            tracing::error!(error = %e, "list api tokens failed");
            vec![governance::internal()]
        }
    }
}

fn token_info(r: TokenRow) -> ApiTokenInfo {
    ApiTokenInfo {
        id: r.0,
        name: r.1,
        owner: r.2,
        workspace: r.3,
        workspace_slug: r.4,
        actions: r.5,
        expires_at: governance::ts(r.6),
        last_used_at: r.7.map(governance::ts),
        revoked: r.8,
    }
}

/// `ListAllApiTokens` (spec 060 US4): every holder's tokens, for instance admins.
pub async fn list_all_tokens(
    app: &AppState,
    req: &Request<'_>,
    holder: Option<uuid::Uuid>,
    state: Option<ApiTokenState>,
) -> Vec<Event> {
    const CMD: &str = "list_all_api_tokens";
    if let Err(code) = governance::check_admin(req.caller) {
        let context = governance::origin(req).context;
        return governance::deny(app, req.caller, CMD, None, code, context).await;
    }
    all_tokens(app, req.caller, holder, state).await
}

/// The state of `api_tokens t`, as SQL: revoked first, then expired.
const TOKEN_STATE_SQL: &str = "case when t.revoked_at is not null then 'revoked' \
     when t.expires_at <= now() then 'expired' else 'active' end";

async fn all_tokens(
    app: &AppState,
    caller: &identity::Caller,
    holder: Option<uuid::Uuid>,
    state: Option<ApiTokenState>,
) -> Vec<Event> {
    type Row = (
        uuid::Uuid,
        String,
        uuid::Uuid,
        uuid::Uuid,
        String,
        Vec<String>,
        time::OffsetDateTime,
        Option<time::OffsetDateTime>,
        bool,
        String,
        String,
    );
    // The filters are one `where` clause, so a visibility rule can join it.
    let rows: Result<Vec<Row>, sqlx::Error> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select t.id, t.name, t.owner_user_id, t.workspace_id, w.slug, t.actions, \
         t.expires_at, t.last_used_at, t.revoked_at is not null, u.display_name, \
         {TOKEN_STATE_SQL} from api_tokens t join workspaces w on w.id = t.workspace_id \
         join users u on u.id = t.owner_user_id \
         where ($1::uuid is null or t.owner_user_id = $1) \
         and ($2::text is null or {TOKEN_STATE_SQL} = $2) \
         and ($3::uuid is null or t.workspace_id is distinct from $3) order by t.id desc"
    )))
    .bind(holder)
    .bind(state.as_ref().map(crate::bindings::enum_str))
    // The System workspace is left out for non-owners (spec 060 US7).
    .bind(crate::system::hide_system(caller))
    .fetch_all(&app.db)
    .await;
    match rows {
        Ok(rows) => vec![Event::AllApiTokens {
            items: rows
                .into_iter()
                .map(|r| {
                    let (holder_name, state) = (r.9, r.10);
                    let token = token_info((r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7, r.8));
                    AdminApiTokenInfo {
                        holder: token.owner,
                        workspace_slug: token.workspace_slug.clone(),
                        holder_name,
                        // An unknown state reads as revoked: fail closed.
                        state: crate::bindings::enum_from(&state).unwrap_or(ApiTokenState::Revoked),
                        token,
                    }
                })
                .collect(),
        }],
        Err(e) => {
            tracing::error!(error = %e, "list all api tokens failed");
            vec![governance::internal()]
        }
    }
}

/// `PreviewApiTokenCreate` (step-up). `Confirm` replies `ApiTokenCreated`, then `ApiTokens`.
pub async fn token_create(
    app: &AppState,
    req: &Request<'_>,
    name: Label,
    ws: uuid::Uuid,
    actions: TokenActions,
    days: u16,
) -> Vec<Event> {
    const CMD: &str = "preview_api_token_create";
    const ACTION: &str = "api_token_create";
    let o = governance::origin(req);
    if req.caller.token.is_some() {
        return governance::deny(app, req.caller, CMD, None, "session_required", o.context).await;
    }
    if let Err(code) = governance::check_ws(req.caller, ws, Capability::View) {
        return governance::deny(app, req.caller, CMD, Some(ws), code, o.context).await;
    }
    // The System workspace takes no API tokens (spec 060).
    if governance::is_system(req.caller, ws) {
        return governance::deny(app, req.caller, CMD, Some(ws), "owner_only", o.context).await;
    }
    if !(1..=MAX_TOKEN_DAYS).contains(&days) {
        return vec![Event::op_error(
            "invalid_expiry",
            format!("A token lives 1 to {MAX_TOKEN_DAYS} days."),
        )];
    }
    let actions: Vec<String> = actions.0.iter().map(|a| a.as_str().to_owned()).collect();
    if actions.is_empty() || !actions.iter().all(|a| TOKEN_ACTIONS.contains(&a.as_str())) {
        return vec![Event::op_error(
            "action_not_allowed",
            format!(
                "Name at least one action, from: {}. Passkey and token commands need a session.",
                TOKEN_ACTIONS.join(", ")
            ),
        )];
    }
    let ws_row: Result<Option<(String, bool)>, sqlx::Error> =
        sqlx::query_as("select slug, archived_at is not null from workspaces where id = $1")
            .bind(ws)
            .fetch_optional(&app.db)
            .await;
    let slug = match ws_row {
        Ok(Some((slug, false))) => slug,
        Ok(Some((_, true))) => {
            return vec![Event::op_error(
                "workspace_archived",
                "That workspace is archived.",
            )];
        }
        Ok(None) => return vec![governance::not_found("workspace")],
        Err(e) => {
            tracing::error!(error = %e, "token workspace lookup failed");
            return vec![governance::internal()];
        }
    };
    let name = name.as_str().to_owned();
    let summary = format!(
        "Create the API token \"{name}\" for {slug}, valid {days} days. It acts as you, limited \
         to: {}. The secret is shown once.",
        actions.join(", ")
    );
    let op = governance::pending(move |app, caller| async move {
        if caller.token.is_some() {
            return governance::deny(&app, &caller, ACTION, None, "session_required", o.context)
                .await;
        }
        let secret = format!("kt_{}", web_auth::session::random_token());
        let now = app.clock.now();
        let expires = now + time::Duration::days(i64::from(days));
        let id = uuid::Uuid::now_v7();
        let step: Result<Result<(), governance::Fail>, sqlx::Error> = async {
            if let Err(code) = governance::check_ws(&caller, ws, Capability::View) {
                return Ok(Err(governance::Fail::Denied(code)));
            }
            let mut tx = app.db.begin().await?;
            let live: Option<bool> = sqlx::query_scalar(
                "select archived_at is null from workspaces where id = $1 for update",
            )
            .bind(ws)
            .fetch_optional(&mut *tx)
            .await?;
            if live != Some(true) {
                return Ok(Err(governance::Fail::Op(
                    "workspace_archived",
                    "That workspace is archived.".into(),
                )));
            }
            sqlx::query(
                "insert into api_tokens (id, name, owner_user_id, workspace_id, actions, hash, \
                 expires_at) values ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(id)
            .bind(&name)
            .bind(caller.user.id)
            .bind(ws)
            .bind(&actions)
            .bind(identity::token_hash(&secret).as_slice())
            .bind(expires)
            .execute(&mut *tx)
            .await?;
            let target = json!({
                "token": id, "name": name, "actions": actions,
                "expires_at": governance::ts(expires),
            });
            governance::record(&mut tx, &caller, ACTION, Some(ws), target, &o.context).await?;
            tx.commit().await?;
            Ok(Ok(()))
        }
        .await;
        let step = step.map(|r| r.map(|()| vec![]));
        if let Err(events) = governance::settle(&app, &caller, ACTION, Some(ws), &o, step).await {
            return events;
        }
        let token = ApiTokenInfo {
            id,
            name,
            owner: caller.user.id,
            workspace: ws,
            workspace_slug: slug,
            actions,
            expires_at: governance::ts(expires),
            last_used_at: None,
            revoked: false,
        };
        let mut out = vec![Event::ApiTokenCreated {
            token,
            secret: TokenSecret(secret),
        }];
        out.extend(tokens(&app, caller.user.id).await);
        out
    });
    let draft = Draft {
        action: ACTION.to_owned(),
        summary,
        affected: vec![],
        step_up: true,
    };
    governance::preview(app, req, draft, op)
}

/// `PreviewApiTokenRevoke`: the token's owner, or an instance admin. An admin revoking someone
/// else's token needs step-up, is audited with `by_admin: true`, and gets `AllApiTokens` back;
/// revoking your own is unchanged and replies `ApiTokens`.
pub async fn token_revoke(app: &AppState, req: &Request<'_>, token: uuid::Uuid) -> Vec<Event> {
    const CMD: &str = "preview_api_token_revoke";
    const ACTION: &str = "api_token_revoke";
    let o = governance::origin(req);
    if req.caller.token.is_some() {
        return governance::deny(app, req.caller, CMD, None, "session_required", o.context).await;
    }
    type Row = (String, uuid::Uuid, uuid::Uuid, bool, String);
    let row: Result<Option<Row>, sqlx::Error> = sqlx::query_as(
        "select t.name, t.owner_user_id, t.workspace_id, t.revoked_at is not null, \
         u.display_name from api_tokens t join users u on u.id = t.owner_user_id where t.id = $1",
    )
    .bind(token)
    .fetch_optional(&app.db)
    .await;
    let may = |c: &identity::Caller, owner: uuid::Uuid| {
        c.user.id == owner || governance::check_admin(c).is_ok()
    };
    let (name, ws, holder, holder_name) = match row {
        Err(e) => {
            tracing::error!(error = %e, "token lookup failed");
            return vec![governance::internal()];
        }
        // A System-workspace token is invisible to non-owners (spec 060 US7).
        Ok(Some((name, owner, ws, revoked, holder_name)))
            if may(req.caller, owner) && crate::system::hide_system(req.caller) != Some(ws) =>
        {
            if revoked {
                return vec![Event::op_error(
                    "already_revoked",
                    "That token is already revoked.",
                )];
            }
            (name, ws, owner, holder_name)
        }
        // Someone else's token, or none: the same refusal, so nothing leaks.
        Ok(_) => {
            return governance::deny(app, req.caller, CMD, None, "not_owner", o.context).await;
        }
    };
    // Someone else's token (an admin acting): step-up, and the audit row says so (spec 060 US4).
    let by_admin = holder != req.caller.user.id;
    let summary = if by_admin {
        format!("Revoke {holder_name}'s API token \"{name}\". Anything using it stops working.")
    } else {
        format!("Revoke the API token \"{name}\". Anything using it stops working.")
    };
    let op = governance::pending(move |app, caller| async move {
        let by_admin = holder != caller.user.id;
        let step: governance::Step = async {
            let mut tx = app.db.begin().await?;
            let owner: Option<uuid::Uuid> =
                sqlx::query_scalar("select owner_user_id from api_tokens where id = $1 for update")
                    .bind(token)
                    .fetch_optional(&mut *tx)
                    .await?;
            match owner {
                Some(owner) if may(&caller, owner) => {}
                _ => return Ok(Err(governance::Fail::Denied("not_owner"))),
            }
            sqlx::query(
                "update api_tokens set revoked_at = coalesce(revoked_at, now()) where id = $1",
            )
            .bind(token)
            .execute(&mut *tx)
            .await?;
            let mut context = o.context.clone();
            if by_admin {
                context["by_admin"] = json!(true);
            }
            governance::record(
                &mut tx,
                &caller,
                ACTION,
                Some(ws),
                json!({ "token": token, "name": name }),
                &context,
            )
            .await?;
            tx.commit().await?;
            Ok(Ok(vec![]))
        }
        .await;
        if let Err(events) = governance::settle(&app, &caller, ACTION, Some(ws), &o, step).await {
            return events;
        }
        if by_admin {
            all_tokens(&app, &caller, None, None).await
        } else {
            tokens(&app, caller.user.id).await
        }
    });
    let draft = Draft {
        action: ACTION.to_owned(),
        summary,
        affected: if by_admin { vec![holder_name] } else { vec![] },
        step_up: by_admin,
    };
    governance::preview(app, req, draft, op)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_parsing() {
        let mut h = HeaderMap::new();
        assert_eq!(bearer(&h), None);
        h.insert(header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        assert_eq!(bearer(&h), Some("abc"));
        h.insert(header::AUTHORIZATION, "bearer  abc ".parse().unwrap());
        assert_eq!(bearer(&h), Some("abc"));
        h.insert(header::AUTHORIZATION, "Basic abc".parse().unwrap());
        assert_eq!(bearer(&h), None);
        h.insert(header::AUTHORIZATION, "Bearer ".parse().unwrap());
        assert_eq!(bearer(&h), None);
    }
}
