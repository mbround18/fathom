//! The one command dispatcher behind both transports (spec 055 T026).
//!
//! `ws.rs` and `api.rs` authenticate the caller and re-read it from the database (so the
//! session's `authz_epoch` is current) before every command; this module authorizes the
//! command, runs it and returns the reply events. A refusal replies `Denied` and is audited.

use std::future::Future;
use std::pin::Pin;

use keel_protocol::{Cmd, CmdEnvelope, Event, EventEnvelope};
use serde_json::json;

use crate::audit::{self, Decision, Entry};
use crate::identity::{self, Caller};
use crate::passkeys;
use crate::preview::Confirmation;
use crate::serve::AppState;

/// What a confirmed preview runs: it gets the app and the caller as of the `Confirm`, and
/// returns the command's own result events (after `Confirmed`).
pub type PendingOp =
    Box<dyn FnOnce(AppState, Caller) -> Pin<Box<dyn Future<Output = Vec<Event>> + Send>> + Send>;

/// Which transport a command came in on (audited as context).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Socket,
    Api,
}

impl Channel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Channel::Socket => "ws",
            Channel::Api => "api",
        }
    }
}

/// Everything about the request except the command.
pub struct Request<'a> {
    pub caller: &'a Caller,
    /// The session handle, for session callers.
    pub session: Option<&'a str>,
    pub channel: Channel,
}

/// Why authorization refused a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused {
    /// Stable code, sent as `Denied.reason` and audited.
    pub code: &'static str,
}

/// Per-command authorization (deny by default for anything a token was not scoped to).
///
/// # Errors
///
/// The command is not allowed for this caller.
pub fn authorize(caller: &Caller, cmd: &Cmd) -> Result<(), Refused> {
    if let Some(t) = &caller.token
        && !t.actions.iter().any(|a| a == cmd.name())
    {
        return Err(Refused {
            code: "token_scope",
        });
    }
    match cmd {
        // Everyone signed in may read themselves. A `Confirm` was authorized when its preview
        // was made, and the operation re-checks what it needs when it runs.
        Cmd::GetMe | Cmd::Confirm { .. } => Ok(()),
        // Passkeys belong to a person at a browser: a session, never a token.
        Cmd::GetVerification
        | Cmd::PasskeyBegin { .. }
        | Cmd::PasskeyFinish(_)
        | Cmd::ListPasskeys
        | Cmd::PreviewPasskeyRemove { .. }
        | Cmd::RecoverPasskeys { .. } => {
            if caller.token.is_some() {
                Err(Refused {
                    code: "session_required",
                })
            } else {
                Ok(())
            }
        }
        // Governance (US6). The handlers check grants; managing API tokens needs a session.
        Cmd::ListApiTokens
        | Cmd::PreviewApiTokenCreate { .. }
        | Cmd::PreviewApiTokenRevoke { .. } => {
            if caller.token.is_some() {
                Err(Refused {
                    code: "session_required",
                })
            } else {
                Ok(())
            }
        }
        Cmd::ListWorkspaces
        | Cmd::PreviewWorkspaceCreate { .. }
        | Cmd::PreviewWorkspaceUpdate { .. }
        | Cmd::PreviewWorkspaceArchive { .. }
        | Cmd::PreviewProjectCreate { .. }
        | Cmd::ListTeams
        | Cmd::PreviewTeamCreate { .. }
        | Cmd::PreviewTeamMemberAdd { .. }
        | Cmd::PreviewTeamMemberRemove { .. }
        | Cmd::PreviewGrantAdd { .. }
        | Cmd::PreviewGrantRemove { .. }
        | Cmd::ListUsers
        | Cmd::PreviewUserRole { .. }
        | Cmd::PreviewUserRemove { .. } => Ok(()),
    }
}

/// Runs one command and returns its reply events.
pub async fn dispatch(app: &AppState, req: &Request<'_>, env: CmdEnvelope) -> Vec<EventEnvelope> {
    let id = env.id;
    let action = env.cmd.name();
    if let Err(refused) = authorize(req.caller, &env.cmd) {
        deny(app, req, action, refused).await;
        return vec![EventEnvelope::reply(
            id,
            Event::Denied {
                action: action.to_owned(),
                reason: refused.code.to_owned(),
            },
        )];
    }
    let events = match env.cmd {
        Cmd::GetMe => {
            let window = time::Duration::try_from(app.cfg.step_up_window)
                .unwrap_or(time::Duration::minutes(5));
            match identity::me(&app.db, req.caller, window, app.clock.now()).await {
                Ok(me) => vec![Event::Me(me)],
                Err(e) => {
                    tracing::error!(error = %e, "GetMe failed");
                    vec![internal()]
                }
            }
        }
        Cmd::Confirm {
            preview_id,
            step_up,
        } => {
            let key = req.caller.preview_key(req.session);
            let window = time::Duration::try_from(app.cfg.step_up_window)
                .unwrap_or(time::Duration::minutes(5));
            let confirmation = Confirmation {
                session: &key,
                preview_id,
                step_up: step_up.as_ref(),
                verified_at: req.caller.verified_at,
                window,
            };
            match app
                .previews
                .confirm(confirmation, app.step_up.as_ref())
                .await
            {
                Ok(op) => {
                    let mut out = vec![Event::Confirmed { preview_id }];
                    out.extend(op(app.clone(), req.caller.clone()).await);
                    out
                }
                Err(e) => match passkeys::confirm_refused(app, req, &e).await {
                    Some(events) => events,
                    None => vec![Event::op_error(e.code(), e.message())],
                },
            }
        }
        Cmd::GetVerification => passkeys::get_verification(app, req).await,
        Cmd::PasskeyBegin { purpose } => passkeys::begin(app, req, purpose).await,
        Cmd::PasskeyFinish(response) => passkeys::finish(app, req, response).await,
        Cmd::ListPasskeys => passkeys::list(app, req.caller).await,
        Cmd::PreviewPasskeyRemove { id } => passkeys::preview_remove(app, req, id).await,
        Cmd::RecoverPasskeys { code } => passkeys::recover(app, req, &code).await,
        // Governance (US6)
        other @ (Cmd::ListWorkspaces
        | Cmd::PreviewWorkspaceCreate { .. }
        | Cmd::PreviewWorkspaceUpdate { .. }
        | Cmd::PreviewWorkspaceArchive { .. }
        | Cmd::PreviewProjectCreate { .. }
        | Cmd::ListTeams
        | Cmd::PreviewTeamCreate { .. }
        | Cmd::PreviewTeamMemberAdd { .. }
        | Cmd::PreviewTeamMemberRemove { .. }
        | Cmd::PreviewGrantAdd { .. }
        | Cmd::PreviewGrantRemove { .. }
        | Cmd::ListUsers
        | Cmd::PreviewUserRole { .. }
        | Cmd::PreviewUserRemove { .. }
        | Cmd::ListApiTokens
        | Cmd::PreviewApiTokenCreate { .. }
        | Cmd::PreviewApiTokenRevoke { .. }) => crate::governance::handle(app, req, other).await,
    };
    events
        .into_iter()
        .map(|e| EventEnvelope::reply(id, e))
        .collect()
}

fn internal() -> Event {
    Event::op_error("internal", "Keel could not complete that. Try again.")
}

/// Audits a refusal; a failed write is logged, never shown to the caller.
async fn deny(app: &AppState, req: &Request<'_>, action: &str, refused: Refused) {
    let mut context = json!({ "channel": req.channel.as_str() });
    if let Some(t) = &req.caller.token {
        context["token"] = json!(t.id);
    }
    let workspace = req.caller.token.as_ref().map(|t| t.workspace);
    let actor = req.caller.actor();
    let result = async {
        let mut tx = app.db.begin().await?;
        audit::record(
            &mut tx,
            Entry {
                actor: &actor,
                action,
                workspace,
                target: json!({}),
                decision: Decision::Denied,
                reason: Some(refused.code),
                context,
            },
        )
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "audit write failed");
    }
}
