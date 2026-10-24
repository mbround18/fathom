//! Spec 052 HTTP routes: the person's own sessions and networks, the home/public answer, and
//! the admin view of anyone's sessions and networks with the whitelist.
//!
//! | Method | Path | Who | From an untrusted network |
//! |---|---|---|---|
//! | GET | `/api/v1/networks` | self | yes |
//! | POST | `/api/v1/networks/answer` | self | yes (never grants trust) |
//! | GET | `/api/v1/sessions` | self | yes |
//! | DELETE | `/api/v1/sessions/{id_hint}` | self | yes |
//! | POST | `/api/v1/sessions/end-others` | self | yes |
//! | GET | `/api/v1/users/{id}/sessions`, `/api/v1/users/{id}/networks` | admin | yes |
//! | DELETE | `/api/v1/users/{id}/sessions` | admin | yes |
//! | POST | `/api/v1/users/{id}/networks/whitelist` | admin | **no** |
//! | DELETE | `/api/v1/users/{id}/networks/{*key}` | admin | **no** |
//!
//! Every state change passes the CSRF layer and is audited. The admin routes check the role
//! (`manage_users`) first, then the gate.

use std::net::IpAddr;
use tokio::time::Instant;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use chrono::{DateTime, Utc};
use depths_core::network::{Answer, KnownNetwork, NetworkKey, Removed, Source};
use depths_core::role::{self, Action, CommandKind};
use depths_protocol::{
    AnswerNetwork, AuditAction, AuditNetwork, AuditOutcome, NetworkAnswer, NetworkSource,
    NetworkView, SessionView, SessionsEnded, WhitelistNetwork,
};
use http::StatusCode;
use web_auth::extract::SessionUser;
use web_auth::session::{EndReason, Session};

use crate::http::{AppState, ClientAddrExt, error, untrusted};
use crate::networks::NetworkError;
use crate::session_meta::id_hint;
use crate::trust::{rfc3339, wire, wire_reason};
use crate::users::SignIn;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/networks", get(my_networks))
        .route("/api/v1/networks/answer", post(answer))
        .route("/api/v1/sessions", get(my_sessions))
        .route("/api/v1/sessions/end-others", post(end_others))
        .route("/api/v1/sessions/{hint}", delete(end_one))
        .route(
            "/api/v1/users/{id}/sessions",
            get(user_sessions).delete(end_user_sessions),
        )
        .route("/api/v1/users/{id}/networks", get(user_networks))
        .route("/api/v1/users/{id}/networks/whitelist", post(whitelist))
        .route("/api/v1/users/{id}/networks/{*key}", delete(remove_network))
}

fn user_of(s: &Session) -> String {
    SignIn::from_identity(&s.identity).id()
}

/// The wall-clock time of a session `Instant`.
fn wall(at: Instant, now: DateTime<Utc>) -> DateTime<Utc> {
    now - chrono::Duration::from_std(Instant::now().saturating_duration_since(at))
        .unwrap_or_default()
}

fn source(s: Source) -> NetworkSource {
    match s {
        Source::First => NetworkSource::First,
        Source::Answer => NetworkSource::Answer,
        Source::Whitelist => NetworkSource::Whitelist,
        Source::Seen => NetworkSource::Seen,
    }
}

fn answer_wire(a: Answer) -> NetworkAnswer {
    match a {
        Answer::Home => NetworkAnswer::Home,
        Answer::Public => NetworkAnswer::Public,
        Answer::None => NetworkAnswer::None,
    }
}

fn answer_core(a: NetworkAnswer) -> Answer {
    match a {
        NetworkAnswer::Home => Answer::Home,
        NetworkAnswer::Public => Answer::Public,
        NetworkAnswer::None => Answer::None,
    }
}

/// One person's networks, newest use first. `current` marks the row holding `ip`, when given.
fn networks_of(app: &AppState, user: &str, ip: Option<IpAddr>) -> Vec<NetworkView> {
    let now = Utc::now();
    let current = ip.map(|ip| app.trust.networks.key_of(ip));
    let mut rows: Vec<KnownNetwork> = app.trust.networks.list(user);
    rows.sort_by_key(|n| std::cmp::Reverse(n.last_used));
    rows.into_iter()
        .map(|n| {
            let d = app.trust.networks.trust_key(user, &n.key, now);
            NetworkView {
                key: n.key.to_string(),
                first_used: rfc3339(n.first_used),
                last_used: rfc3339(n.last_used),
                days: n
                    .days
                    .iter()
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .collect(),
                source: source(n.source),
                answer: answer_wire(n.answer),
                whitelisted_by: n.whitelisted_by.clone(),
                at: n.at.map(rfc3339),
                label: n.label.clone(),
                trusted: d.trusted,
                reason: wire_reason(d.reason),
                location: app.trust.geo.locate(n.key.net()),
                current: !n.is_whitelist() && current == Some(n.key),
            }
        })
        .collect()
}

/// One person's live sessions, newest use first. `current` marks the session `handle`.
fn sessions_of(app: &AppState, user: &str, handle: Option<&str>) -> Vec<SessionView> {
    let now = Utc::now();
    let mut live: Vec<Session> = app
        .auth
        .sessions
        .live()
        .into_iter()
        .filter(|s| user_of(s) == user)
        .collect();
    live.sort_by_key(|s| std::cmp::Reverse(s.last_seen));
    live.iter()
        .map(|s| {
            let meta = app.trust.sessions.get(&s.id);
            let (address, browser) = meta.map_or_else(
                || (None, "Unknown browser".to_owned()),
                |m| (Some(m.address), m.browser),
            );
            let (network, trusted, location) = match address {
                Some(ip) => {
                    let d = app.trust.networks.trust(user, ip, now);
                    (d.network.to_string(), d.trusted, app.trust.geo.locate(ip))
                }
                None => (String::new(), false, crate::geoip::UNKNOWN.to_owned()),
            };
            SessionView {
                id_hint: id_hint(&s.id),
                created: rfc3339(wall(s.created_at, now)),
                last_used: rfc3339(wall(s.last_seen, now)),
                address: address.map_or_else(|| "unknown".to_owned(), |a| a.to_string()),
                network,
                trusted,
                location,
                browser: if browser.is_empty() {
                    "Unknown browser".to_owned()
                } else {
                    browser
                },
                current: handle == Some(s.id.as_str()),
            }
        })
        .collect()
}

/// Ends `user`'s sessions that `pick` selects; how many.
fn end_where(app: &AppState, user: &str, pick: impl Fn(&Session) -> bool) -> u32 {
    let ended = app
        .auth
        .sessions
        .end_matching(|s| user_of(s) == user && pick(s), EndReason::SignedOut);
    for e in &ended {
        app.trust.sessions.remove(&e.id);
    }
    u32::try_from(ended.len()).unwrap_or(u32::MAX)
}

fn audit_session_end(
    app: &AppState,
    session: &Session,
    ip: IpAddr,
    person: Option<String>,
    which: String,
    ended: u32,
) {
    let mut e = crate::audit::entry(
        crate::audit::actor_of(&session.identity),
        AuditAction::SessionEnd,
        ip,
        AuditOutcome::Succeeded,
    );
    e.diff_summary = Some(format!("{ended} ended"));
    e.network = Some(AuditNetwork {
        session: Some(which),
        person,
        ..AuditNetwork::default()
    });
    app.trust.audit(&e);
}

// ---- self ---------------------------------------------------------------------------------

async fn my_networks(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
) -> Response {
    Json(networks_of(&app, &user_of(&session), Some(ip))).into_response()
}

async fn answer(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
    body: Result<Json<AnswerNetwork>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, "invalid_body");
    };
    let Some(key) = NetworkKey::parse(&body.network) else {
        return error(StatusCode::BAD_REQUEST, "invalid_network");
    };
    let user = user_of(&session);
    match app
        .trust
        .networks
        .answer(&user, &key, answer_core(body.answer))
    {
        Ok(()) => {}
        Err(NetworkError::NotKnown) => return error(StatusCode::NOT_FOUND, "unknown_network"),
        Err(NetworkError::NoAnswer) => return error(StatusCode::BAD_REQUEST, "invalid_answer"),
        Err(NetworkError::Storage(e)) => {
            tracing::error!(error = %e, "network registry write failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    }
    let mut e = crate::audit::entry(
        crate::audit::actor_of(&session.identity),
        AuditAction::NetworkAnswer,
        ip,
        AuditOutcome::Succeeded,
    );
    e.network = Some(AuditNetwork {
        network: Some(key.to_string()),
        answer: Some(body.answer),
        ..AuditNetwork::default()
    });
    app.trust.audit(&e);
    // The decision for this request's network, which an answer never makes trusted at once.
    Json(wire(&app.trust.decide(&user, ip))).into_response()
}

async fn my_sessions(State(app): State<AppState>, SessionUser(session): SessionUser) -> Response {
    Json(sessions_of(&app, &user_of(&session), Some(&session.id))).into_response()
}

async fn end_one(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
    Path(hint): Path<String>,
) -> Response {
    let user = user_of(&session);
    let ended = end_where(&app, &user, |s| id_hint(&s.id) == hint);
    if ended == 0 {
        return error(StatusCode::NOT_FOUND, "unknown_session");
    }
    audit_session_end(&app, &session, ip, None, hint, ended);
    Json(SessionsEnded { ended }).into_response()
}

async fn end_others(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
) -> Response {
    let user = user_of(&session);
    let ended = end_where(&app, &user, |s| s.id != session.id);
    audit_session_end(&app, &session, ip, None, "others".into(), ended);
    Json(SessionsEnded { ended }).into_response()
}

// ---- admin --------------------------------------------------------------------------------

/// The role check for the admin routes: `manage_users`, audited when refused.
fn admin(app: &AppState, session: &Session, ip: IpAddr, what: &str) -> Result<(), Box<Response>> {
    if role::permits(
        &app.users.access(&user_of(session)),
        None,
        Action::ManageUsers,
    ) {
        return Ok(());
    }
    let _ = app.audit.record(&crate::audit::denied(
        crate::audit::actor_of(&session.identity),
        ip,
        None,
        format!("{what} needs manage_users"),
    ));
    Err(Box::new(error(StatusCode::FORBIDDEN, "forbidden")))
}

async fn user_sessions(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = admin(&app, &session, ip, "listing a person's sessions") {
        return *r;
    }
    Json(sessions_of(&app, &id, Some(&session.id))).into_response()
}

async fn user_networks(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = admin(&app, &session, ip, "listing a person's networks") {
        return *r;
    }
    let own = (id == user_of(&session)).then_some(ip);
    Json(networks_of(&app, &id, own)).into_response()
}

async fn end_user_sessions(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = admin(&app, &session, ip, "ending a person's sessions") {
        return *r;
    }
    // Ending sessions works from any network; an admin ending their own keeps this one.
    let ended = end_where(&app, &id, |s| s.id != session.id);
    audit_session_end(&app, &session, ip, Some(id), "all".into(), ended);
    Json(SessionsEnded { ended }).into_response()
}

async fn whitelist(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
    Path(id): Path<String>,
    body: Result<Json<WhitelistNetwork>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if let Err(r) = admin(&app, &session, ip, "whitelisting a network") {
        return *r;
    }
    let me = user_of(&session);
    let actor = crate::audit::actor_of(&session.identity);
    if let Err(d) = app.trust.gate(
        &me,
        &actor,
        &session.id,
        ip,
        CommandKind::ManageNetworks,
        "whitelist",
    ) {
        return untrusted(&d);
    }
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, "invalid_body");
    };
    let Some(cidr) = NetworkKey::parse(&body.cidr) else {
        return error(StatusCode::BAD_REQUEST, "invalid_network");
    };
    // Belt and braces: never a block that holds the requester's own untrusted network.
    if cidr.contains(ip) && !app.trust.decide(&me, ip).trusted {
        return error(StatusCode::FORBIDDEN, "untrusted_network");
    }
    if app.users.get(&id).is_none() {
        return error(StatusCode::NOT_FOUND, "unknown_user");
    }
    let by = SignIn::from_identity(&session.identity).username;
    if let Err(e) = app
        .trust
        .networks
        .whitelist(&id, cidr, &by, body.label.as_deref(), Utc::now())
    {
        tracing::error!(error = ?e, "network registry write failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    }
    let mut e = crate::audit::entry(
        actor,
        AuditAction::NetworkWhitelistAdd,
        ip,
        AuditOutcome::Succeeded,
    );
    e.network = Some(AuditNetwork {
        network: Some(cidr.to_string()),
        person: Some(id.clone()),
        ..AuditNetwork::default()
    });
    app.trust.audit(&e);
    Json(networks_of(&app, &id, (id == me).then_some(ip))).into_response()
}

async fn remove_network(
    State(app): State<AppState>,
    SessionUser(session): SessionUser,
    ClientAddrExt(ip): ClientAddrExt,
    Path((id, key)): Path<(String, String)>,
) -> Response {
    if let Err(r) = admin(&app, &session, ip, "removing a network") {
        return *r;
    }
    let me = user_of(&session);
    let actor = crate::audit::actor_of(&session.identity);
    if let Err(d) = app.trust.gate(
        &me,
        &actor,
        &session.id,
        ip,
        CommandKind::ManageNetworks,
        "remove network",
    ) {
        return untrusted(&d);
    }
    let Some(key) = NetworkKey::parse(key.trim_start_matches('/')) else {
        return error(StatusCode::BAD_REQUEST, "invalid_network");
    };
    let action = match app.trust.networks.remove(&id, &key) {
        Ok(Removed::Whitelist) => AuditAction::NetworkWhitelistRemove,
        Ok(Removed::Forgotten) => AuditAction::NetworkForget,
        Err(NetworkError::Storage(e)) => {
            tracing::error!(error = %e, "network registry write failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
        Err(_) => return error(StatusCode::NOT_FOUND, "unknown_network"),
    };
    let mut e = crate::audit::entry(actor, action, ip, AuditOutcome::Succeeded);
    e.network = Some(AuditNetwork {
        network: Some(key.to_string()),
        person: Some(id.clone()),
        ..AuditNetwork::default()
    });
    app.trust.audit(&e);
    Json(networks_of(&app, &id, (id == me).then_some(ip))).into_response()
}
