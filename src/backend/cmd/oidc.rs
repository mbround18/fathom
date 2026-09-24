//! OIDC identity commands (033 US1/US2): issuer config, sign-in/out and per-context auth mode.
//!
//! Tokens never appear here: they live in `OidcSessions` and the OS keyring. Every reply is
//! built from the non-secret protocol types.

use std::sync::Arc;

use crossbeam_channel::Sender;

use crate::backend::events;
use crate::backend::persistence;
use crate::backend::state::BackendState;
use crate::protocol::v1::{ContextAuthConfig, ContextAuthMode, OidcConfig, OidcIssuerConfig};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};

/// Handles the 033 identity commands; returns the command back if it belongs to another family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::LoadOidcConfig => {
            bind_sink(state, sender);
            let config = persistence::load_oidc_config();
            emit_config(state, sender, config.clone());
            // Resume previous sessions from the keyring, off the command path.
            for issuer in config.issuers {
                let sessions = state.oidc.clone();
                tokio::spawn(async move { sessions.restore(&issuer).await });
            }
        }
        BackendCmd::OidcSaveIssuer { issuer } => {
            bind_sink(state, sender);
            save_issuer(state, sender, issuer);
        }
        BackendCmd::OidcDeleteIssuer { issuer_id } => {
            bind_sink(state, sender);
            let mut config = persistence::load_oidc_config();
            config.issuers.retain(|i| i.id != issuer_id);
            config
                .contexts
                .retain(|c| c.issuer_id.as_deref() != Some(issuer_id.as_str()));
            state.oidc.forget(&issuer_id);
            persist_and_emit(state, sender, &issuer_id, config);
            emit_identity_cleared(state, sender, &issuer_id);
        }
        BackendCmd::OidcSetRoleMap { role_map } => {
            let mut config = persistence::load_oidc_config();
            config.role_map = role_map;
            persist_and_emit(state, sender, "", config);
        }
        BackendCmd::OidcSignIn { issuer_id, method } => {
            bind_sink(state, sender);
            match find_issuer(&issuer_id) {
                Some(cfg) => state.oidc.sign_in(cfg, method),
                None => emit_error(state, sender, &issuer_id, "config", "unknown issuer"),
            }
        }
        BackendCmd::OidcCancelSignIn { issuer_id } => state.oidc.cancel_sign_in(&issuer_id),
        BackendCmd::OidcSignOut {
            issuer_id,
            end_session,
        } => {
            bind_sink(state, sender);
            match find_issuer(&issuer_id) {
                Some(cfg) => {
                    state.oidc.sign_out(&cfg, end_session).await;
                    // Contexts keep their OIDC opt-in: their client now fails with "sign in
                    // again" rather than silently reverting to kubeconfig credentials.
                }
                None => emit_error(state, sender, &issuer_id, "config", "unknown issuer"),
            }
        }
        BackendCmd::SetContextAuthMode {
            context,
            mode,
            issuer_id,
        } => set_context_auth_mode(state, sender, context, mode, issuer_id).await?,
        other => return Ok(Some(other)),
    }
    Ok(None)
}

fn find_issuer(id: &str) -> Option<OidcIssuerConfig> {
    persistence::load_oidc_config()
        .issuers
        .into_iter()
        .find(|i| i.id == id)
}

/// Route session events to this backend's channel (idempotent).
fn bind_sink(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let seq = state.cluster.seq.clone();
    let tx = sender.clone();
    state.oidc.set_sink(Arc::new(move |event| {
        events::emit(
            seq.as_ref(),
            &tx,
            Route::global(section_keys::system()),
            Correlation::None,
            BackendEvent::System(event),
        );
    }));
}

fn emit_config(state: &BackendState, sender: &Sender<BackendEventEnvelope>, config: OidcConfig) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::OidcConfigLoaded {
            config,
            identities: state.oidc.identities(),
        }),
    );
}

fn emit_identity_cleared(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    issuer_id: &str,
) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::OidcIdentityUpdated {
            issuer_id: issuer_id.to_owned(),
            identity: None,
        }),
    );
}

fn emit_error(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    issuer_id: &str,
    stage: &str,
    message: &str,
) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::OidcError {
            issuer_id: issuer_id.to_owned(),
            stage: stage.to_owned(),
            message: message.to_owned(),
        }),
    );
}

/// Validate + save; an invalid config is refused (never applied) with a verbatim reason.
fn persist_and_emit(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    issuer_id: &str,
    config: OidcConfig,
) -> bool {
    match persistence::save_oidc_config(&config) {
        Ok(()) => {
            emit_config(state, sender, config);
            true
        }
        Err(e) => {
            emit_error(state, sender, issuer_id, "config", &format!("{e:#}"));
            false
        }
    }
}

fn save_issuer(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    issuer: OidcIssuerConfig,
) {
    let mut config = persistence::load_oidc_config();
    let id = issuer.id.clone();
    match config.issuers.iter_mut().find(|i| i.id == id) {
        Some(existing) => *existing = issuer,
        None => config.issuers.push(issuer),
    }
    persist_and_emit(state, sender, &id, config);
}

async fn set_context_auth_mode(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: String,
    mode: ContextAuthMode,
    issuer_id: Option<String>,
) -> anyhow::Result<()> {
    let mut config = persistence::load_oidc_config();
    config.contexts.retain(|c| c.context_name != context);
    if mode == ContextAuthMode::Oidc {
        config.contexts.push(ContextAuthConfig {
            context_name: context.clone(),
            mode,
            issuer_id,
        });
    }
    if !persist_and_emit(state, sender, "", config) {
        return Ok(());
    }
    // Reconnect the context if it is the active one so the new auth mode takes effect now.
    if state.cluster.current_context.as_deref() == Some(context.as_str()) {
        let _ = super::cluster::handle(BackendCmd::SetContext { context }, state, sender).await?;
    }
    Ok(())
}
