//! Sign-in and sign-out (spec 055 T025), on the shared `web-auth` providers.
//!
//! - `GET /auth/login?provider=` or `/auth/login/{provider}` starts a sign-in (one configured
//!   provider needs no name).
//! - `GET /auth/callback/{provider}` finishes it: the user is created or refreshed in PostgreSQL
//!   (the first user becomes `owner`), a session is opened, and the browser goes to `/`. A
//!   refused callback is audited `denied` and answered with a fixed page that echoes nothing.
//! - `POST /auth/logout` (behind the CSRF layer) ends the session and returns `endSessionUrl`.
//!
//! The session cookie is `HttpOnly`. A second, readable cookie carries the session's CSRF token
//! for `Hello.csrf_token` and `X-CSRF-Token`. Neither value, nor the session handle, is ever
//! logged or audited.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use web_auth::extract::SessionUser;
use web_auth::provider::ProviderError;
use web_auth::session::{EndReason, Identity, SessionError, Tokens};

use crate::audit::{self, Actor, Decision, Entry};
use crate::identity::{self, SignInError, SignInIdentity, User};
use crate::serve::{AppState, Peer, error};

#[derive(Deserialize)]
pub struct LoginQuery {
    provider: Option<String>,
}

pub async fn login_query(State(app): State<AppState>, Query(q): Query<LoginQuery>) -> Response {
    let id = match q.provider {
        Some(p) => p,
        None if app.providers.len() == 1 => app.providers[0].0.to_owned(),
        None => return error(StatusCode::BAD_REQUEST, "provider_required"),
    };
    start_login(&app, &id).await
}

pub async fn login_path(State(app): State<AppState>, Path(provider): Path<String>) -> Response {
    start_login(&app, &provider).await
}

async fn start_login(app: &AppState, id: &str) -> Response {
    let Some(provider) = app.provider(id) else {
        return error(StatusCode::NOT_FOUND, "unknown_provider");
    };
    let Ok(redirect) = web_auth::oauth::callback_url(&app.cfg.public_url, provider.id()) else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    };
    match provider.authorize(&redirect).await {
        Ok(ar) => {
            let cookie = app.logins.begin(ar.pending, "/".to_owned());
            let mut r = Redirect::to(ar.url.as_str()).into_response();
            r.headers_mut().append(header::SET_COOKIE, cookie);
            r
        }
        Err(e) => {
            tracing::warn!(
                provider = id,
                error = category(&e),
                "sign-in could not start"
            );
            error(StatusCode::BAD_GATEWAY, "provider_unavailable")
        }
    }
}

/// A fixed page for a failed callback: nothing from the request is echoed.
fn sign_in_failed(status: StatusCode, clear: HeaderValue, msg: &'static str) -> Response {
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Sign-in \
         failed</title></head><body><h1>Sign-in failed</h1><p>{msg}</p><p><a href=\"/\">Start \
         again</a></p></body></html>"
    );
    let mut r = (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response();
    r.headers_mut().append(header::SET_COOKIE, clear);
    r
}

fn category(e: &ProviderError) -> &'static str {
    match e {
        ProviderError::StateMismatch => "state_mismatch",
        ProviderError::Refused(_) => "refused",
        ProviderError::Rejected(_) => "rejected",
        ProviderError::Transient(_) => "transient",
        ProviderError::RateLimited(_) => "rate_limited",
        ProviderError::Failed(_) => "failed",
    }
}

/// Audit context for a request: the provider and the client address. No secrets.
fn context(provider: &str, peer: Peer) -> Value {
    let mut c = json!({ "provider": provider });
    if let Some(a) = peer.0 {
        c["ip"] = json!(a.ip().to_string());
    }
    c
}

/// Audits a refused sign-in. Nobody is known yet, so the actor is the system.
async fn refused(app: &AppState, reason: &'static str, context: Value) {
    record(
        app,
        Entry {
            actor: &Actor::System,
            action: "sign_in",
            workspace: None,
            target: json!({}),
            decision: Decision::Denied,
            reason: Some(reason),
            context,
        },
    )
    .await;
}

/// Writes one audit entry in its own transaction; a failed write is logged, never shown.
async fn record(app: &AppState, entry: Entry<'_>) {
    let result = async {
        let mut tx = app.db.begin().await?;
        audit::record(&mut tx, entry).await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "audit write failed");
    }
}

/// A signed-in session, ready to hand to the browser.
pub struct Established {
    pub user: User,
    /// The web-auth handle (what the database row and `Ended` use). Never logged.
    pub session: String,
    /// The CSRF token the client must echo.
    pub csrf: String,
    /// `Set-Cookie` values: the session cookie and the CSRF cookie.
    pub cookies: [HeaderValue; 2],
}

/// Why a verified identity could not be signed in.
#[derive(Debug)]
pub enum EstablishError {
    /// The account is disabled (audited by [`identity::sign_in`]).
    Disabled,
    /// The session store is full.
    Full,
    Internal,
}

/// Signs a provider-verified identity in: upserts the user (audited), opens the in-memory
/// session and its database row, and builds the cookies. The callback and the tests use it.
///
/// # Errors
///
/// See [`EstablishError`].
pub async fn establish(
    app: &AppState,
    who: Identity,
    tokens: Tokens,
    context: Value,
) -> Result<Established, EstablishError> {
    let sign_in = SignInIdentity {
        provider: who.provider.as_str(),
        issuer: &who.issuer,
        subject: &who.sub,
        display_name: &who.username,
        email: who.email.as_deref(),
    };
    let user = match identity::sign_in(&app.db, &sign_in, context).await {
        Ok(u) => u,
        Err(SignInError::Disabled) => return Err(EstablishError::Disabled),
        Err(SignInError::Db(e)) => {
            tracing::error!(error = %e, "sign-in write failed");
            return Err(EstablishError::Internal);
        }
    };
    let new = match app.auth.sessions.create(who, tokens) {
        Ok(s) => s,
        Err(SessionError::GlobalLimit(_)) => return Err(EstablishError::Full),
    };
    let absolute = app.session_cfg.absolute;
    let expires_at =
        app.clock.now() + time::Duration::try_from(absolute).unwrap_or(time::Duration::hours(8));
    if let Err(e) = identity::open_session(&app.db, &new.id, &user, expires_at).await {
        tracing::error!(error = %e, "session write failed");
        app.auth.sessions.end(&new.id, EndReason::Revoked);
        return Err(EstablishError::Internal);
    }
    tracing::info!(user = %user.id, role = user.instance_role.as_str(), "signed in");
    let max_age = absolute.as_secs();
    Ok(Established {
        cookies: [
            app.auth.cookie.set(&new.cookie, max_age),
            app.cookies.set_csrf(&new.csrf, max_age),
        ],
        session: new.id.clone(),
        csrf: new.csrf.clone(),
        user,
    })
}

pub async fn callback(
    State(app): State<AppState>,
    Path(provider_id): Path<String>,
    peer: Peer,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let clear = app.logins.clear_cookie();
    let Some(provider) = app.provider(&provider_id).cloned() else {
        return error(StatusCode::NOT_FOUND, "unknown_provider");
    };
    let ctx = context(provider.id(), peer);
    let state = q.get("state").map(String::as_str).unwrap_or_default();
    // Consumed whatever happens next: a login is single-use.
    let pending = match app.logins.finish(&headers, state) {
        Ok((pending, _)) => pending,
        Err(why) => {
            tracing::warn!(provider = provider.id(), reason = %why, "sign-in callback refused");
            refused(&app, "login_state", ctx).await;
            return sign_in_failed(
                StatusCode::BAD_REQUEST,
                clear,
                "The sign-in attempt is missing, expired or was started elsewhere.",
            );
        }
    };
    if q.contains_key("error") {
        refused(&app, "provider_denied", ctx).await;
        return sign_in_failed(
            StatusCode::UNAUTHORIZED,
            clear,
            "The provider did not grant the sign-in.",
        );
    }
    let Some(code) = q.get("code") else {
        return sign_in_failed(
            StatusCode::BAD_REQUEST,
            clear,
            "The callback is missing its parameters.",
        );
    };
    let completed = match provider.complete(pending, code, state).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                provider = provider.id(),
                error = category(&e),
                "sign-in failed"
            );
            let status = if e.is_unreachable() {
                StatusCode::BAD_GATEWAY
            } else {
                refused(&app, category(&e), ctx).await;
                StatusCode::UNAUTHORIZED
            };
            return sign_in_failed(
                status,
                clear,
                "The provider's answer could not be accepted.",
            );
        }
    };
    match establish(&app, completed.identity, completed.tokens, ctx).await {
        Ok(est) => {
            // Only ever `/`: no return path from the request is followed.
            let mut r = Redirect::to("/").into_response();
            let h = r.headers_mut();
            for c in est.cookies {
                h.append(header::SET_COOKIE, c);
            }
            h.append(header::SET_COOKIE, clear);
            r
        }
        Err(EstablishError::Disabled) => sign_in_failed(
            StatusCode::FORBIDDEN,
            clear,
            "This account is disabled. Ask an owner of this Keel instance.",
        ),
        Err(EstablishError::Full) => sign_in_failed(
            StatusCode::SERVICE_UNAVAILABLE,
            clear,
            "Too many people are signed in. Try again later.",
        ),
        Err(EstablishError::Internal) => sign_in_failed(
            StatusCode::INTERNAL_SERVER_ERROR,
            clear,
            "Keel could not record the sign-in.",
        ),
    }
}

/// Behind the CSRF layer: session, Origin and token are already checked.
pub async fn logout(
    State(app): State<AppState>,
    peer: Peer,
    SessionUser(session): SessionUser,
) -> Response {
    let provider = session.identity.provider.as_str();
    match identity::find_user(&app.db, &session.identity.issuer, &session.identity.sub).await {
        Ok(Some(user)) => {
            record(
                &app,
                Entry {
                    actor: &user.actor(),
                    action: "sign_out",
                    workspace: None,
                    target: json!({}),
                    decision: Decision::Allowed,
                    reason: None,
                    context: context(provider, peer),
                },
            )
            .await;
        }
        Ok(None) => {}
        Err(e) => tracing::error!(error = %e, "sign-out: user lookup failed"),
    }
    if let Err(e) = identity::close_session(&app.db, &session.id).await {
        tracing::error!(error = %e, "sign-out: session delete failed");
    }
    app.previews.forget_session(&session.id);
    let end = app
        .provider(provider)
        .and_then(|p| p.end_session_url(&session))
        .map(|u| u.to_string());
    app.auth.sessions.end(&session.id, EndReason::SignedOut);
    let mut r = Json(json!({ "endSessionUrl": end })).into_response();
    let h = r.headers_mut();
    h.append(header::SET_COOKIE, app.auth.cookie.clear());
    h.append(header::SET_COOKIE, app.cookies.clear_csrf());
    r
}
