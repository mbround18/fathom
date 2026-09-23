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

/// Load kubeconfig respecting KUBECONFIG env var and ~/.kube/config default.
///
/// `pub(super)` so `context_status.rs` can build short-lived clients for non-active contexts
/// (012-incluster-daemon-federation P1) using the same kubeconfig-resolution path as the active
/// connection.
pub(super) fn load_kubeconfig() -> anyhow::Result<Kubeconfig> {
    // Try KUBECONFIG environment variable first
    if let Some(config) = load_kubeconfig_from_env() {
        return Ok(config);
    }

    // Fall back to ~/.kube/config
    if let Some(config) = load_kubeconfig_from_home() {
        return Ok(config);
    }

    // Last resort: let kube-rs try its default logic
    tracing::debug!("Falling back to Kubeconfig::read() default behavior");
    Kubeconfig::read().map_err(|e| anyhow::anyhow!("Failed to load kubeconfig: {}", e))
}

/// First readable kubeconfig listed in the KUBECONFIG env var, if any.
fn load_kubeconfig_from_env() -> Option<Kubeconfig> {
    let kubeconfig_path = std::env::var("KUBECONFIG").ok()?;
    // On Windows, paths are separated by `;`, on Unix by `:`
    let separator = if cfg!(windows) { ";" } else { ":" };

    kubeconfig_path.split(separator).find_map(|path_str| {
        let path = PathBuf::from(path_str.trim());
        if !path.exists() {
            return None;
        }
        tracing::debug!("Trying kubeconfig from KUBECONFIG env: {}", path.display());
        let config = Kubeconfig::read_from(&path).ok()?;
        tracing::info!(
            "Successfully loaded kubeconfig from KUBECONFIG: {}",
            path.display()
        );
        Some(config)
    })
}

/// Kubeconfig at the default `~/.kube/config` path, if present and readable.
fn load_kubeconfig_from_home() -> Option<Kubeconfig> {
    let default_config_path = dirs::home_dir()?.join(".kube").join("config");
    if !default_config_path.exists() {
        return None;
    }
    tracing::debug!(
        "Trying kubeconfig at default path: {}",
        default_config_path.display()
    );
    let config = Kubeconfig::read_from(&default_config_path).ok()?;
    tracing::info!(
        "Successfully loaded kubeconfig from default path: {}",
        default_config_path.display()
    );
    Some(config)
}

/// Loads and emits `ContextsUpdated`, returning the `(contexts, current)` tuple it computed so
/// callers can also trigger per-context status probes (012-incluster-daemon-federation P1)
/// without re-parsing the kubeconfig.
pub(super) fn load_and_send_contexts(
    seq: &AtomicU64,
    selected_context: Option<String>,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<(Vec<String>, Option<String>)> {
    tracing::info!("Loading kubeconfig contexts...");

    // Best-effort: if kubeconfig isn't present (e.g. in-cluster), just report no contexts.
    let (contexts, current) = match load_kubeconfig() {
        Ok(kc) => contexts_from_kubeconfig(&kc, selected_context),
        Err(e) => {
            tracing::warn!("Failed to load kubeconfig: {}, reporting empty contexts", e);
            (Vec::new(), selected_context)
        }
    };

    emit_contexts(seq, sender, contexts.clone(), current.clone());
    Ok((contexts, current))
}

/// Sorted context names plus the effective current context (explicit selection wins
/// over kubeconfig's `current-context`).
fn contexts_from_kubeconfig(
    kubeconfig: &Kubeconfig,
    selected_context: Option<String>,
) -> (Vec<String>, Option<String>) {
    tracing::info!(
        "Kubeconfig loaded successfully, found {} contexts",
        kubeconfig.contexts.len()
    );

    let mut contexts: Vec<String> = kubeconfig.contexts.iter().map(|c| c.name.clone()).collect();
    contexts.sort();
    tracing::info!("Sending {} contexts to UI: {:?}", contexts.len(), contexts);

    let current = selected_context.or_else(|| kubeconfig.current_context.clone());
    tracing::info!("Current context: {:?}", current);
    (contexts, current)
}

fn emit_contexts(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    contexts: Vec<String>,
    current: Option<String>,
) {
    events::emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::ContextsUpdated { contexts, current }),
    );
}

pub(super) async fn connect_client_for_current_context(
    state: &BackendState,
) -> anyhow::Result<Client> {
    // If we have a selected context and kubeconfig exists, use it; otherwise fall back.
    if let Some(ctx) = state.cluster.current_context.clone()
        && let Ok(kubeconfig) = load_kubeconfig()
    {
        let opts = KubeConfigOptions {
            context: Some(ctx),
            ..Default::default()
        };
        let config = Config::from_custom_kubeconfig(kubeconfig, &opts).await?;
        return Ok(Client::try_from(config)?);
    }

    Ok(Client::try_default().await?)
}

pub(super) fn read_kubeconfig_current_context() -> Option<String> {
    load_kubeconfig().ok().and_then(|kc| kc.current_context)
}
