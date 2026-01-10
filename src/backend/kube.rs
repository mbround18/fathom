use crate::protocol::{
    BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};
use crossbeam_channel::Sender;
use kube::{
    Client, Config,
    config::{KubeConfigOptions, Kubeconfig},
};
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;

use super::{events, state::BackendState};

/// Load kubeconfig respecting KUBECONFIG env var and ~/.kube/config default
fn load_kubeconfig() -> anyhow::Result<Kubeconfig> {
    // Try KUBECONFIG environment variable first
    if let Ok(kubeconfig_path) = std::env::var("KUBECONFIG") {
        // On Windows, paths are separated by `;`, on Unix by `:`
        let separator = if cfg!(windows) { ";" } else { ":" };

        for path_str in kubeconfig_path.split(separator) {
            let path = PathBuf::from(path_str.trim());
            if path.exists() {
                tracing::debug!("Trying kubeconfig from KUBECONFIG env: {}", path.display());
                if let Ok(config) = Kubeconfig::read_from(&path) {
                    tracing::info!(
                        "Successfully loaded kubeconfig from KUBECONFIG: {}",
                        path.display()
                    );
                    return Ok(config);
                }
            }
        }
    }

    // Fall back to ~/.kube/config
    if let Some(home_dir) = dirs::home_dir() {
        let default_config_path = home_dir.join(".kube").join("config");
        if default_config_path.exists() {
            tracing::debug!(
                "Trying kubeconfig at default path: {}",
                default_config_path.display()
            );
            if let Ok(config) = Kubeconfig::read_from(&default_config_path) {
                tracing::info!(
                    "Successfully loaded kubeconfig from default path: {}",
                    default_config_path.display()
                );
                return Ok(config);
            }
        }
    }

    // Last resort: let kube-rs try its default logic
    tracing::debug!("Falling back to Kubeconfig::read() default behavior");
    Kubeconfig::read().map_err(|e| anyhow::anyhow!("Failed to load kubeconfig: {}", e))
}

pub(super) fn load_and_send_contexts(
    seq: &AtomicU64,
    selected_context: Option<String>,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    tracing::info!("Loading kubeconfig contexts...");

    // Best-effort: if kubeconfig isn't present (e.g. in-cluster), just report no contexts.
    let kubeconfig = match load_kubeconfig() {
        Ok(kc) => {
            tracing::info!(
                "Kubeconfig loaded successfully, found {} contexts",
                kc.contexts.len()
            );
            kc
        }
        Err(e) => {
            tracing::warn!("Failed to load kubeconfig: {}, reporting empty contexts", e);
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
    tracing::info!("Sending {} contexts to UI: {:?}", contexts.len(), contexts);

    let current = selected_context.or_else(|| kubeconfig.current_context.clone());
    tracing::info!("Current context: {:?}", current);

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
        if let Ok(kubeconfig) = load_kubeconfig() {
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
    load_kubeconfig().ok().and_then(|kc| kc.current_context)
}
