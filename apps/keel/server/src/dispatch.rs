//! The one command dispatcher behind both transports (spec 055 T026).
//!
//! `ws.rs` and `api.rs` authenticate the caller and re-read it from the database (so the
//! session's `authz_epoch` is current) before every command; this module authorizes the
//! command, runs it and returns the reply events. A refusal replies `Denied` and is audited.

use std::future::Future;
use std::pin::Pin;

use keel_protocol::{Cmd, CmdEnvelope, Event, EventEnvelope, TOKEN_ACTIONS};
use serde_json::json;

use crate::audit::{self, Decision, Entry};
use crate::identity::{self, Caller};
use crate::passkeys;
use crate::preview::Confirmation;
use crate::serve::AppState;
use crate::{bindings, credentials, inventory, receivers};

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

/// Per-command authorization (deny by default for anything a token was not scoped to, or that
/// is not on [`TOKEN_ACTIONS`]).
///
/// # Errors
///
/// The command is not allowed for this caller.
pub fn authorize(caller: &Caller, cmd: &Cmd) -> Result<(), Refused> {
    if let Some(t) = &caller.token
        && !(TOKEN_ACTIONS.contains(&cmd.name()) && t.actions.iter().any(|a| a == cmd.name()))
    {
        return Err(Refused {
            code: "token_scope",
        });
    }
    match cmd {
        // Everyone signed in may read themselves. A `Confirm` was authorized when its preview
        // was made, and the operation re-checks what it needs when it runs.
        Cmd::GetMe | Cmd::Confirm { .. } => Ok(()),
        // Bindings and receivers (US1): each checks its capability on the object's workspace,
        // which needs the database, so the check runs in the handler.
        Cmd::ListBindings { .. }
        | Cmd::GetBinding { .. }
        | Cmd::PreviewBindingCreate(_)
        | Cmd::PreviewBindingUpdate(_)
        | Cmd::PreviewSignedMode { .. }
        | Cmd::PreviewBindingDisable { .. }
        | Cmd::PreviewBindingEnable { .. }
        | Cmd::PreviewBindingRemove { .. }
        | Cmd::SyncBinding { .. }
        | Cmd::ListReceivers { .. }
        | Cmd::PreviewReceiverCreate { .. }
        | Cmd::PreviewReceiverRotate { .. }
        | Cmd::PreviewReceiverDisable { .. }
        | Cmd::PreviewReceiverRemove { .. }
        | Cmd::PreviewReceiverBind { .. }
        | Cmd::PreviewReceiverUnbind { .. }
        | Cmd::ListDeliveries { .. }
        | Cmd::ListCredentials { .. }
        | Cmd::PreviewCredentialCreate { .. }
        | Cmd::PreviewCredentialRotate { .. }
        | Cmd::PreviewCredentialGrant { .. }
        | Cmd::PreviewCredentialRevoke { .. }
        | Cmd::PreviewCredentialRemove { .. }
        // Inventories (US4): the same, per inventory or workspace.
        | Cmd::ListInventories { .. }
        | Cmd::GetInventoryRevision { .. }
        | Cmd::DiffInventory { .. }
        | Cmd::PreviewInventoryCreate { .. }
        | Cmd::PreviewInventoryEdit { .. }
        | Cmd::PreviewInventoryRestore { .. }
        | Cmd::PreviewInventoryDeprecate { .. }
        | Cmd::ValidateInventory { .. }
        | Cmd::PreviewInventorySchema { .. } => Ok(()),
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
        // Runs (US2): each handler checks the workspace capability.
        Cmd::ListRuns { .. }
        | Cmd::GetRun { .. }
        | Cmd::SubscribeRun { .. }
        | Cmd::UnsubscribeRun { .. }
        | Cmd::PreviewRunStart { .. }
        | Cmd::PreviewCancel { .. }
        | Cmd::ListJobs { .. }
        | Cmd::ListPolicies { .. }
        | Cmd::PreviewPolicyUpsert(_)
        | Cmd::PreviewPolicyRemove { .. }
        | Cmd::ExplainPolicy { .. } => Ok(()),
        // Hosts and the SSH CAs (US7): each handler checks the workspace capability (or, for
        // rotation, instance admin).
        Cmd::ListHosts { .. }
        | Cmd::GetHost { .. }
        | Cmd::PreviewHostEnroll { .. }
        | Cmd::PreviewHostRevoke { .. }
        | Cmd::GetSshCa
        | Cmd::PreviewSshCaRotate { .. } => Ok(()),
        // Signed mode (US8): each handler checks the workspace capability.
        Cmd::ListReviews { .. }
        | Cmd::GetReview { .. }
        | Cmd::GetReviewDiff { .. }
        | Cmd::PreviewOverride { .. }
        | Cmd::PreviewOverrideDeny { .. }
        | Cmd::PreviewBaseline { .. }
        | Cmd::ListSigners { .. }
        | Cmd::PreviewSignerAdd { .. }
        | Cmd::PreviewSignerRemove { .. } => Ok(()),
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
        // Bindings and receivers (US1)
        Cmd::ListBindings { workspace } => bindings::list(app, req, workspace).await,
        Cmd::GetBinding { binding } => bindings::get(app, req, binding).await,
        Cmd::PreviewBindingCreate(spec) => bindings::preview_create(app, req, spec).await,
        Cmd::PreviewBindingUpdate(upd) => bindings::preview_update(app, req, upd).await,
        Cmd::PreviewSignedMode { binding, on } => {
            bindings::preview_signed_mode(app, req, binding, on).await
        }
        Cmd::PreviewBindingDisable { binding } => {
            bindings::preview_status(app, req, binding, false).await
        }
        Cmd::PreviewBindingEnable { binding } => {
            bindings::preview_status(app, req, binding, true).await
        }
        Cmd::PreviewBindingRemove { binding } => bindings::preview_remove(app, req, binding).await,
        Cmd::SyncBinding { binding } => bindings::sync_now(app, req, binding).await,
        Cmd::ListReceivers { workspace } => receivers::list(app, req, workspace).await,
        Cmd::PreviewReceiverCreate {
            workspace,
            name,
            provider,
        } => receivers::preview_create(app, req, workspace, name, provider).await,
        Cmd::PreviewReceiverRotate {
            receiver,
            previous_valid_for,
        } => receivers::preview_rotate(app, req, receiver, previous_valid_for).await,
        Cmd::PreviewReceiverDisable { receiver } => {
            receivers::preview_disable(app, req, receiver).await
        }
        Cmd::PreviewReceiverRemove { receiver } => {
            receivers::preview_remove(app, req, receiver).await
        }
        Cmd::PreviewReceiverBind { receiver, binding } => {
            receivers::preview_bind(app, req, receiver, binding).await
        }
        Cmd::PreviewReceiverUnbind { receiver, binding } => {
            receivers::preview_unbind(app, req, receiver, binding).await
        }
        Cmd::ListDeliveries {
            workspace,
            receiver,
            binding,
            decision,
            from,
            to,
        } => {
            receivers::list_deliveries(app, req, workspace, receiver, binding, decision, from, to)
                .await
        }
        Cmd::ListCredentials { workspace } => credentials::list(app, req, workspace).await,
        Cmd::PreviewCredentialCreate {
            workspace,
            name,
            kind,
            value,
        } => credentials::preview_create(app, req, workspace, name, kind, value).await,
        Cmd::PreviewCredentialRotate { credential, value } => {
            credentials::preview_rotate(app, req, credential, value).await
        }
        Cmd::PreviewCredentialGrant {
            credential,
            binding,
            profiles,
        } => credentials::preview_grant(app, req, credential, binding, profiles).await,
        Cmd::PreviewCredentialRevoke {
            credential,
            binding,
        } => credentials::preview_revoke(app, req, credential, binding).await,
        Cmd::PreviewCredentialRemove { credential } => {
            credentials::preview_remove(app, req, credential).await
        }
        Cmd::ListInventories { workspace } => inventory::list(app, req, workspace).await,
        Cmd::GetInventoryRevision { inventory, seq } => {
            inventory::get_revision(app, req, inventory, seq).await
        }
        Cmd::DiffInventory {
            inventory,
            from,
            to,
        } => inventory::diff(app, req, inventory, from, to).await,
        Cmd::PreviewInventoryCreate {
            workspace,
            name,
            content,
            var_schema,
        } => inventory::preview_create(app, req, workspace, name, content, var_schema).await,
        Cmd::PreviewInventoryEdit {
            inventory,
            base_seq,
            content,
        } => inventory::preview_edit(app, req, inventory, base_seq, content).await,
        Cmd::PreviewInventoryRestore { inventory, seq } => {
            inventory::preview_restore(app, req, inventory, seq).await
        }
        Cmd::PreviewInventoryDeprecate { inventory } => {
            inventory::preview_deprecate(app, req, inventory).await
        }
        Cmd::ValidateInventory {
            workspace,
            content,
            inventory,
            var_schema,
        } => inventory::validate(app, req, workspace, content, inventory, var_schema).await,
        Cmd::PreviewInventorySchema {
            inventory,
            var_schema,
        } => inventory::preview_schema(app, req, inventory, var_schema).await,
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
            // An override or baseline must carry a passkey assertion over its own challenge:
            // a recent step-up does not stand in for it (spec 055 US8, research R20).
            let bound = app.reviews.verifier(&app.passkeys, preview_id);
            let result = match &bound {
                Some(v) => {
                    let confirmation = Confirmation {
                        verified_at: None,
                        ..confirmation
                    };
                    app.previews.confirm(confirmation, v).await
                }
                None => {
                    app.previews
                        .confirm(confirmation, app.step_up.as_ref())
                        .await
                }
            };
            match result {
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
        // Runs (US2).
        other @ (Cmd::ListRuns { .. }
        | Cmd::GetRun { .. }
        | Cmd::SubscribeRun { .. }
        | Cmd::UnsubscribeRun { .. }
        | Cmd::PreviewRunStart { .. }
        | Cmd::PreviewCancel { .. }
        | Cmd::ListJobs { .. }
        | Cmd::ListPolicies { .. }
        | Cmd::PreviewPolicyUpsert(_)
        | Cmd::PreviewPolicyRemove { .. }
        | Cmd::ExplainPolicy { .. }) => crate::runs::handle(app, req, other).await,
        other @ (Cmd::ListHosts { .. }
        | Cmd::GetHost { .. }
        | Cmd::PreviewHostEnroll { .. }
        | Cmd::PreviewHostRevoke { .. }
        | Cmd::GetSshCa
        | Cmd::PreviewSshCaRotate { .. }) => crate::hosts::handle(app, req, other).await,
        other @ (Cmd::ListReviews { .. }
        | Cmd::GetReview { .. }
        | Cmd::GetReviewDiff { .. }
        | Cmd::PreviewOverride { .. }
        | Cmd::PreviewOverrideDeny { .. }
        | Cmd::PreviewBaseline { .. }
        | Cmd::ListSigners { .. }
        | Cmd::PreviewSignerAdd { .. }
        | Cmd::PreviewSignerRemove { .. }) => crate::review::handle(app, req, other).await,
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
