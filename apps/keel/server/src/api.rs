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
