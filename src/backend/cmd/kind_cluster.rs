//! Local kind cluster provisioning commands (spec 030).

use crossbeam_channel::Sender;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use crate::backend::state::BackendState;
use crate::backend::{events, kind, kube};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, KindClusterOperationKind,
    KindClusterProfileId, KindClusterSummary, KindPrerequisites, Route, StreamEvent, SystemEvent,
    section_keys,
};

/// Handles kind commands; returns the command back if it belongs to another family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> Option<BackendCmd> {
    match cmd {
        BackendCmd::RefreshKindPrerequisites => {
            let prerequisites = kind::detect_prerequisites().await;
            emit_stream(
                state.cluster.seq.as_ref(),
                sender,
                StreamEvent::KindPrerequisitesUpdated { prerequisites },
            );
        }
        BackendCmd::RefreshKindClusters => refresh_clusters(state, sender).await,
        BackendCmd::CreateKindCluster {
            request_id,
            name,
            profile,
            custom_yaml,
        } => create(state, sender, request_id, name, profile, custom_yaml).await,
        BackendCmd::CancelCreateKindCluster { request_id } => {
            if state.kind_actions.cancel(&request_id).is_some() {
                finish(
                    state.cluster.seq.as_ref(),
                    sender,
                    request_id,
                    false,
                    Some("cancelled".to_owned()),
                );
                refresh_clusters(state, sender).await;
            }
        }
        BackendCmd::DeleteKindCluster { request_id, name } => {
            delete(state, sender, request_id, name).await;
        }
        other => return Some(other),
    }
    None
}

fn emit_stream(seq: &AtomicU64, sender: &Sender<BackendEventEnvelope>, ev: StreamEvent) {
    events::emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::Stream(ev),
    );
}

fn finish(
    seq: &AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    request_id: String,
    ok: bool,
    error: Option<String>,
) {
    emit_stream(
        seq,
        sender,
        StreamEvent::KindClusterActionFinished {
            request_id,
            ok,
            error,
        },
    );
}

async fn current_summaries(state: &BackendState) -> Result<Vec<KindClusterSummary>, String> {
    kind::list_clusters(&state.kind_actions).await
}

async fn refresh_clusters(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    match current_summaries(state).await {
        Ok(clusters) => emit_stream(
            state.cluster.seq.as_ref(),
            sender,
            StreamEvent::KindClustersUpdated { clusters },
        ),
        Err(e) => events::notify_error(state.cluster.seq.as_ref(), sender, e),
    }
}

/// Re-validates prerequisites at the point of action (spec edge case).
async fn require_ready() -> Result<KindPrerequisites, String> {
    let p = kind::detect_prerequisites().await;
    if !p.kind_installed {
        return Err("kind is not installed.".into());
    }
    if !p.runtime_installed {
        return Err("No container runtime (Docker or Podman) is installed.".into());
    }
    if !p.runtime_running {
        return Err("The container runtime is installed but not running.".into());
    }
    Ok(p)
}

async fn create(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    request_id: String,
    name: String,
    profile: KindClusterProfileId,
    custom_yaml: Option<String>,
) {
    let seq = state.cluster.seq.clone();
    let reject = |msg: String| {
        events::notify_error(seq.as_ref(), sender, msg.clone());
        finish(seq.as_ref(), sender, request_id.clone(), false, Some(msg));
    };

    let prep = async {
        let existing = kind::list_cluster_names().await?;
        kind::validate_new_cluster_name(&name, &existing)?;
        let config = kind::config_for_profile(profile, custom_yaml.as_deref())?;
        let prereq = require_ready().await?;
        Ok::<_, String>((config, prereq))
    }
    .await;
    let (config, prereq) = match prep {
        Ok(v) => v,
        Err(e) => return reject(e),
    };
    if let Err(e) =
        state
            .kind_actions
            .try_register(&request_id, &name, KindClusterOperationKind::Create)
    {
        return reject(e);
    }

    let registry = state.kind_actions.clone();
    let sender = sender.clone();
    let selected = state.cluster.current_context.clone();
    let rid = request_id.clone();
    let task = tokio::spawn(async move {
        let progress_sender = sender.clone();
        let progress_seq = seq.clone();
        let progress_rid = rid.clone();
        let result = kind::run_create(&name, &config, prereq.runtime_kind, move |stage| {
            emit_stream(
                progress_seq.as_ref(),
                &progress_sender,
                StreamEvent::KindProvisionProgress {
                    request_id: progress_rid.clone(),
                    stage,
                },
            );
        })
        .await;
        registry.finish(&rid);
        after_action(&seq, &sender, selected, &registry, rid, result).await;
    });
    state.kind_actions.attach(&request_id, task.abort_handle());
}

/// Shared tail: re-read kubeconfig contexts, re-list clusters, emit the terminal event.
async fn after_action(
    seq: &Arc<AtomicU64>,
    sender: &Sender<BackendEventEnvelope>,
    selected_context: Option<String>,
    registry: &kind::KindActionRegistry,
    request_id: String,
    result: Result<(), String>,
) {
    let _ = kube::load_and_send_contexts(seq.as_ref(), selected_context, sender);
    if let Ok(clusters) = kind::list_clusters(registry).await {
        emit_stream(
            seq.as_ref(),
            sender,
            StreamEvent::KindClustersUpdated { clusters },
        );
    }
    match result {
        Ok(()) => finish(seq.as_ref(), sender, request_id, true, None),
        Err(e) => finish(seq.as_ref(), sender, request_id, false, Some(e)),
    }
}

/// True when deleting `deleted_cluster` removes the currently active context.
pub(super) fn deletes_active_context(active: Option<&str>, deleted_cluster: &str) -> bool {
    active == Some(kind::context_name_for(deleted_cluster).as_str())
}

async fn delete(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    request_id: String,
    name: String,
) {
    let seq = state.cluster.seq.clone();
    let prep = async {
        kind::validate_cluster_name(&name)?;
        let existing = kind::list_cluster_names().await?;
        // Only kind clusters can be deleted; never an arbitrary kubeconfig context.
        if !existing.contains(&name) {
            return Err(format!("No kind cluster named '{name}' exists."));
        }
        require_ready().await
    }
    .await;
    let prereq = match prep {
        Ok(p) => p,
        Err(e) => {
            events::notify_error(seq.as_ref(), sender, e.clone());
            finish(seq.as_ref(), sender, request_id, false, Some(e));
            return;
        }
    };
    if let Err(e) =
        state
            .kind_actions
            .try_register(&request_id, &name, KindClusterOperationKind::Delete)
    {
        events::notify_error(seq.as_ref(), sender, e.clone());
        finish(seq.as_ref(), sender, request_id, false, Some(e));
        return;
    }

    // Delete is short; run it inline so the active-context fallback can mutate state.
    let (pseq, psender, prid) = (seq.clone(), sender.clone(), request_id.clone());
    let result = kind::run_delete(&name, prereq.runtime_kind, move |stage| {
        emit_stream(
            pseq.as_ref(),
            &psender,
            StreamEvent::KindProvisionProgress {
                request_id: prid.clone(),
                stage,
            },
        );
    })
    .await;
    state.kind_actions.finish(&request_id);

    let mut selected = state.cluster.current_context.clone();
    if result.is_ok() && deletes_active_context(selected.as_deref(), &name) {
        // The active cluster no longer exists: drop the connection rather than hang on it.
        state.watch_tasks.abort_all();
        state.cluster.client = None;
        state.cluster.current_context = None;
        selected = None;
        events::emit(
            seq.as_ref(),
            sender,
            Route::global(section_keys::system()),
            Correlation::None,
            BackendEvent::System(SystemEvent::ContextChanged { current: None }),
        );
    }
    after_action(
        &seq,
        sender,
        selected,
        &state.kind_actions,
        request_id,
        result,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_context_detection() {
        assert!(deletes_active_context(Some("kind-dev"), "dev"));
        assert!(!deletes_active_context(Some("kind-dev"), "other"));
        assert!(!deletes_active_context(Some("prod"), "prod"));
        assert!(!deletes_active_context(None, "dev"));
    }
}
