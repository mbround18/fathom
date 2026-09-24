//! 038-sdk-login-ollama command handlers. All logic lives in `backend::ai::AiRuntime`;
//! these only route. Login polling runs in a spawned task, so nothing here blocks the
//! backend state lock for the length of a device flow.

use crossbeam_channel::Sender;

use crate::backend::ai::Emitter;
use crate::backend::state::BackendState;
use crate::protocol::{BackendCmd, BackendEventEnvelope};

pub(super) fn emitter(state: &BackendState, sender: &Sender<BackendEventEnvelope>) -> Emitter {
    Emitter::new(state.cluster.seq.clone(), sender.clone())
}

/// Handles `Ai*` commands; returns the command back if it belongs to another family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    let em = emitter(state, sender);
    let ai = state.ai.clone();
    match cmd {
        BackendCmd::AiGetStatus => ai.get_status(&em).await,
        BackendCmd::AiProviderLoginStart { provider } => ai.login_start(provider, &em).await,
        BackendCmd::AiProviderSubmitApiKey { provider, key } => {
            ai.submit_api_key(provider, key, &em).await;
        }
        BackendCmd::AiProviderLoginCancel => ai.login_cancel(&em),
        BackendCmd::AiProviderSignOut { provider } => ai.sign_out(provider, &em).await,
        BackendCmd::AiSetActiveProvider { provider } => ai.set_active_provider(provider, &em).await,
        BackendCmd::AiSetOfflineMode { enabled } => ai.set_offline_mode(enabled, &em),
        BackendCmd::AiOllamaProbe { endpoint } => ai.ollama_probe(endpoint, &em).await,
        BackendCmd::AiOllamaSelectModel { model } => ai.ollama_select_model(model, &em),
        BackendCmd::AiSetOllamaEndpoint {
            endpoint,
            allow_lan,
        } => {
            ai.set_ollama_endpoint(endpoint, allow_lan, &em).await;
        }
        BackendCmd::AiEnableFileStore { confirm } => ai.enable_file_store(confirm, &em).await,
        other => return Ok(Some(other)),
    }
    Ok(None)
}
