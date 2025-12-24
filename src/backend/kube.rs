use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};
use crossbeam_channel::Sender;
use kube::{
    Client, Config,
    config::{KubeConfigOptions, Kubeconfig},
};
use std::sync::atomic::AtomicU64;

use super::{events, state::BackendState};

pub(super) fn load_and_send_contexts(
    seq: &AtomicU64,
    selected_context: Option<String>,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    // Best-effort: if kubeconfig isn't present (e.g. in-cluster), just report no contexts.
    let kubeconfig = match Kubeconfig::read() {
        Ok(kc) => kc,
        Err(_) => {
            events::emit(
                seq,
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::ContextsUpdated {
                    contexts: Vec::new(),
                    current: selected_context,
                }),
            );
            return Ok(());
        }
    };

    let mut contexts: Vec<String> = kubeconfig.contexts.iter().map(|c| c.name.clone()).collect();
    contexts.sort();

    let current = selected_context.or_else(|| kubeconfig.current_context.clone());
    events::emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::ContextsUpdated { contexts, current }),
    );
    Ok(())
}

pub(super) async fn connect_client_for_current_context(
    state: &BackendState,
) -> anyhow::Result<Client> {
    // If we have a selected context and kubeconfig exists, use it; otherwise fall back.
    if let Some(ctx) = state.current_context.clone() {
        if let Ok(kubeconfig) = Kubeconfig::read() {
            let opts = KubeConfigOptions {
                context: Some(ctx),
                ..Default::default()
            };
            let config = Config::from_custom_kubeconfig(kubeconfig, &opts).await?;
            return Ok(Client::try_from(config)?);
        }
    }

    Ok(Client::try_default().await?)
}

pub(super) fn read_kubeconfig_current_context() -> Option<String> {
    Kubeconfig::read().ok().and_then(|kc| kc.current_context)
}
