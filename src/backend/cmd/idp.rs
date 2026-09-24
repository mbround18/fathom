//! IdP directory commands (033 US3): the write-only admin API key and the read-only directory
//! refresh. The key never leaves the credential store; replies carry only presence and data.

use crossbeam_channel::Sender;

use crate::backend::events;
use crate::backend::idp_directory;
use crate::backend::oidc::flow;
use crate::backend::oidc::store::{Secret, SecretKind};
use crate::backend::persistence;
use crate::backend::state::BackendState;
use crate::protocol::v1::{IdpGroup, IdpUser, OidcIdentity};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};

/// The OIDC identity of the active context, or `None` for kubeconfig-only contexts (where the
/// Fathom role map does not apply).
pub(super) fn active_identity(state: &BackendState) -> Option<OidcIdentity> {
    let context = state.cluster.current_context.as_deref()?;
    let issuer = crate::backend::kube::oidc_issuer_for_context(context)?;
    state
        .oidc
        .identities()
        .into_iter()
        .find(|i| i.issuer_id == issuer)
}

/// Handles the IdP directory commands; returns the command back if it belongs to another family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::IdpSetAdminKey { issuer_id, api_key } => {
            let creds = state.oidc.credentials();
            let key = api_key.trim();
            if key.is_empty() {
                creds.delete(SecretKind::AdminApiKey, &issuer_id);
                state.idp.forget(&issuer_id);
            } else {
                creds.set(SecretKind::AdminApiKey, &issuer_id, &Secret::new(key));
            }
            let present = creds.has(SecretKind::AdminApiKey, &issuer_id);
            emit(
                state,
                sender,
                SystemEvent::IdpAdminKeyStatus { issuer_id, present },
            );
        }
        BackendCmd::IdpRefreshDirectory { issuer_id } => {
            let result = fetch(state, &issuer_id).await;
            let snapshot = state.idp.apply(&issuer_id, result);
            emit(state, sender, SystemEvent::IdpDirectoryUpdated(snapshot));
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

fn emit(state: &BackendState, sender: &Sender<BackendEventEnvelope>, event: SystemEvent) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section_keys::team_profiles()),
        Correlation::None,
        BackendEvent::System(event),
    );
}

async fn fetch(
    state: &BackendState,
    issuer_id: &str,
) -> Result<(Vec<IdpUser>, Vec<IdpGroup>), String> {
    let cfg = persistence::load_oidc_config()
        .issuers
        .into_iter()
        .find(|i| i.id == issuer_id)
        .ok_or_else(|| format!("unknown issuer '{issuer_id}'"))?;
    let key = state
        .oidc
        .credentials()
        .get(SecretKind::AdminApiKey, issuer_id)
        .ok_or_else(|| {
            "no IdP admin API key is stored; enter a read-only key (Users:read, Groups:read)"
                .to_owned()
        })?;
    let http = flow::http_client(&cfg)?;
    idp_directory::fetch(&http, &idp_directory::admin_base(&cfg), key.expose()).await
}
