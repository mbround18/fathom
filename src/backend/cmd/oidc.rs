//! OIDC identity commands (033 US1/US2): issuer config, sign-in/out and per-context auth mode,
//! plus 045's guided path, which probes and signs in against a draft that was never saved.
//!
//! Tokens never appear here: they live in `OidcSessions` and the OS keyring. Every reply is
//! built from the non-secret protocol types.

use std::sync::Arc;

use crossbeam_channel::Sender;

use crate::backend::events;
use crate::backend::oidc::flow;
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
            if let Some(reason) = super::idp::admin_refusal(state) {
                emit_error(state, sender, "", "forbidden", &reason);
                return Ok(None);
            }
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
        BackendCmd::OidcDiscoverDraft {
            issuer_url,
            ca_bundle_path,
        } => {
            // No saved issuer and no cluster: `flow::discover` takes a bare URL, so the probe
            // runs before anything exists to look up (045 R1). A private CA has to be
            // supplyable here, because without it the first probe is the thing that fails
            // (FR-003).
            let http =
                flow::http_client_with_ca(ca_bundle_path.as_deref()).map_err(anyhow::Error::msg)?;
            // The error is returned, not swallowed: `run_cmd` turns it into this command's
            // `CommandFinished { ok: false, error }` with the provider's words and the URL
            // tried, both of which `FlowError` already carries verbatim.
            let discovery = flow::discover(&http, &issuer_url)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            emit_system(
                state,
                sender,
                SystemEvent::OidcDraftDiscovered {
                    issuer_url,
                    discovery,
                },
            );
        }
        BackendCmd::OidcTestSignIn { draft } => {
            bind_sink(state, sender);
            // The config comes from the draft, not `find_issuer`: there is nothing saved to find.
            // `test_sign_in` writes no credential, so abandoning the path leaves nothing behind.
            state.oidc.test_sign_in(draft.config, draft.method);
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
        BackendCmd::OidcSaveDraft {
            draft,
            role_map,
            adopt_test_session,
        } => {
            bind_sink(state, sender);
            save_draft(state, sender, draft, role_map, adopt_test_session).await;
        }
        BackendCmd::ReadApiServerAuth => {
            // Read-only, and deliberately one event per call: the comparison step asks once and
            // the 60 s cache answers every re-entry (045 US3).
            crate::backend::apiserver_auth::handle_read_api_server_auth(state, sender).await?;
        }
        cmd @ BackendCmd::WriteApiServerAuth { .. } => {
            // Inline, and awaited: `audit_summary()` is taken at dispatch (`src/backend.rs`),
            // so a write finished in a spawned task would be audited before the cluster had
            // changed (045 T061).
            write_api_server_auth(state, sender, cmd).await?;
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

/// `WriteApiServerAuth`, destructured away from [`handle`] so the command family stays readable.
async fn write_api_server_auth(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    cmd: BackendCmd,
) -> anyhow::Result<()> {
    let BackendCmd::WriteApiServerAuth {
        cluster,
        style,
        issuer_url,
        client_id,
        username_claim,
        groups_claim,
        username_prefix,
        groups_prefix,
    } = cmd
    else {
        unreachable!("only called for WriteApiServerAuth")
    };
    crate::backend::apiserver_auth::handle_write_api_server_auth(
        state,
        sender,
        cluster,
        style,
        issuer_url,
        client_id,
        username_claim,
        groups_claim,
        username_prefix,
        groups_prefix,
    )
    .await
}

/// Why `draft` cannot be saved, in words for the operator, or `None` when it can.
///
/// An id already in use is refused rather than merged (FR-017): the id keys the credential-store
/// entries, so quietly overwriting one issuer's entry with another's would sign the first out and
/// leave its tokens under a config that no longer describes them. Re-saving the *same* issuer --
/// the same id at the same URL, which is what Review... opens (FR-016) -- is an update, not a
/// collision.
fn save_refusal(
    config: &OidcConfig,
    draft: &OidcIssuerConfig,
    adopt_test_session: bool,
    has_draft_session: bool,
) -> Option<String> {
    if adopt_test_session && !has_draft_session {
        return Some(
            "There is no live test sign-in to adopt. Run the test sign-in again, or save \
             without adopting a session."
                .to_owned(),
        );
    }
    let existing = config.issuers.iter().find(|i| i.id == draft.id)?;
    (existing.issuer_url != draft.issuer_url).then(|| {
        format!(
            "The issuer id \"{}\" is already in use by {}. Choose a different id: the id keys \
             the stored credentials, so reusing it would sign that issuer out.",
            draft.id, existing.issuer_url
        )
    })
}

/// `OidcSaveDraft` (045 T049): one transactional save.
///
/// Validate, write the issuer and the role map in a single `config.json` write, and only then
/// adopt the draft's in-memory tokens. Nothing is adopted if the write was refused, so a failed
/// save leaves no credential behind under an id that was never persisted.
async fn save_draft(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    draft: crate::protocol::IssuerDraft,
    role_map: crate::protocol::FathomRoleMap,
    adopt_test_session: bool,
) {
    let mut config = persistence::load_oidc_config();
    let issuer = draft.config;
    if let Some(reason) = save_refusal(
        &config,
        &issuer,
        adopt_test_session,
        state.oidc.has_draft_session(),
    ) {
        emit_error(state, sender, &issuer.id, "config", &reason);
        return;
    }

    let id = issuer.id.clone();
    match config.issuers.iter_mut().find(|i| i.id == id) {
        Some(existing) => *existing = issuer.clone(),
        None => config.issuers.push(issuer.clone()),
    }
    // The role map travels in the same write: a save that landed the issuer but not the roles
    // would leave the operator an admin of nothing (FR-028).
    config.role_map = role_map;
    if !persist_and_emit(state, sender, &id, config) {
        return;
    }

    if adopt_test_session {
        match state.oidc.adopt_draft(&issuer).await {
            // The config event above already carries the new issuer; `adopt_draft` emits the
            // identity, so the UI needs nothing further to show the operator signed in.
            Ok(()) => {}
            Err(e) => emit_error(state, sender, &id, "session", &e),
        }
    } else {
        // Saved without adopting: the draft's tokens are not a session the operator asked for.
        state.oidc.abandon_draft();
    }
    emit_config(state, sender, persistence::load_oidc_config());
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

/// Identity is global, so every event here goes to the system section.
fn emit_system(state: &BackendState, sender: &Sender<BackendEventEnvelope>, event: SystemEvent) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(event),
    );
}

fn emit_config(state: &BackendState, sender: &Sender<BackendEventEnvelope>, config: OidcConfig) {
    emit_system(
        state,
        sender,
        SystemEvent::OidcConfigLoaded {
            config,
            identities: state.oidc.identities(),
        },
    );
}

fn emit_identity_cleared(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    issuer_id: &str,
) {
    emit_system(
        state,
        sender,
        SystemEvent::OidcIdentityUpdated {
            issuer_id: issuer_id.to_owned(),
            identity: None,
        },
    );
}

fn emit_error(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    issuer_id: &str,
    stage: &str,
    message: &str,
) {
    emit_system(
        state,
        sender,
        SystemEvent::OidcError {
            issuer_id: issuer_id.to_owned(),
            stage: stage.to_owned(),
            message: message.to_owned(),
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::FathomRoleMap;

    fn issuer(id: &str, url: &str) -> OidcIssuerConfig {
        OidcIssuerConfig {
            id: id.to_owned(),
            issuer_url: url.to_owned(),
            client_id: "fathom".to_owned(),
            scopes: vec!["openid".to_owned()],
            ca_bundle_path: None,
            username_claim: "email".to_owned(),
            groups_claim: "groups".to_owned(),
            username_prefix: None,
            groups_prefix: None,
            admin_api_base: None,
        }
    }

    fn saved(issuers: Vec<OidcIssuerConfig>) -> OidcConfig {
        OidcConfig {
            issuers,
            contexts: Vec::new(),
            role_map: FathomRoleMap::default(),
        }
    }

    /// T048 (FR-017): an id already in use is refused with an explanation, not merged.
    #[test]
    fn a_duplicate_issuer_id_is_refused_with_an_explanation() {
        let config = saved(vec![issuer("corp", "https://idp.example.com/realms/corp")]);
        let reason = save_refusal(
            &config,
            &issuer("corp", "https://other.example.com/realms/corp"),
            false,
            false,
        )
        .expect("a second issuer under an id in use is refused");
        // The explanation names the id and what already holds it, because the operator has to
        // choose: rename the draft, or review the issuer that is there.
        assert!(reason.contains("corp"), "{reason}");
        assert!(
            reason.contains("https://idp.example.com/realms/corp"),
            "{reason}"
        );

        // A free id is fine.
        assert_eq!(
            save_refusal(
                &config,
                &issuer("corp-2", "https://other.example.com/realms/corp"),
                false,
                false
            ),
            None
        );
        // And re-saving the same issuer at the same URL is the Review... case (FR-016): an
        // update, not a collision, or an existing issuer could never be edited.
        assert_eq!(
            save_refusal(
                &config,
                &issuer("corp", "https://idp.example.com/realms/corp"),
                false,
                false
            ),
            None
        );
    }

    /// T047: adopting a session that does not exist is refused before anything is written.
    #[test]
    fn adopting_without_a_live_test_sign_in_is_refused() {
        let reason = save_refusal(
            &saved(Vec::new()),
            &issuer("corp", "https://i"),
            true,
            false,
        )
        .expect("there is no session to adopt");
        assert!(reason.contains("test sign-in"), "{reason}");
        // The same save without the adoption is allowed: FR-015's "Save without testing".
        assert_eq!(
            save_refusal(
                &saved(Vec::new()),
                &issuer("corp", "https://i"),
                false,
                false
            ),
            None
        );
        // And with a live draft, adopting is allowed.
        assert_eq!(
            save_refusal(&saved(Vec::new()), &issuer("corp", "https://i"), true, true),
            None
        );
    }
}
