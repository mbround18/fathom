//! 040-manifest-export command family: YAML fetch for export and kubeconfig export.

use crossbeam_channel::Sender;

use crate::backend::state::BackendState;
use crate::backend::{events, kubeconfig_export, manifest_export};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, ManifestFetchError,
    ManifestObjectRef, Route, SystemEvent, section_keys,
};

pub(super) fn handle(
    cmd: BackendCmd,
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> Option<BackendCmd> {
    match cmd {
        BackendCmd::FetchManifests {
            request_id,
            objects,
        } => fetch(state, sender, request_id, objects),
        BackendCmd::PreviewKubeconfigExport { context } => preview(state, sender, &context),
        BackendCmd::ExportKubeconfig {
            context,
            path,
            include_embedded_credentials,
            overwrite,
        } => export(
            state,
            sender,
            &context,
            &path,
            include_embedded_credentials,
            overwrite,
        ),
        other => return Some(other),
    }
    None
}

fn emit_system(
    seq: &std::sync::atomic::AtomicU64,
    sender: &Sender<BackendEventEnvelope>,
    event: SystemEvent,
) {
    events::emit(
        seq,
        sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(event),
    );
}

/// Runs off the command loop: a 500-object bundle must not stall other commands.
fn fetch(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    request_id: String,
    objects: Vec<ManifestObjectRef>,
) {
    let seq = state.cluster.seq.clone();
    let sender = sender.clone();
    let client = match state.cluster.client_or_err() {
        Ok(c) => c,
        Err(e) => {
            let errors = (0..objects.len().max(1))
                .map(|i| ManifestFetchError {
                    index: u32::try_from(i).unwrap_or(u32::MAX),
                    message: format!("{e:#}"),
                })
                .collect();
            emit_system(
                &seq,
                &sender,
                SystemEvent::ManifestsFetched {
                    request_id,
                    manifests: Vec::new(),
                    errors,
                },
            );
            return;
        }
    };
    tokio::spawn(async move {
        let (manifests, errors) = manifest_export::fetch_manifests(&client, &objects).await;
        emit_system(
            &seq,
            &sender,
            SystemEvent::ManifestsFetched {
                request_id,
                manifests,
                errors,
            },
        );
    });
}

fn finished_err(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
    message: String,
) {
    emit_system(
        &state.cluster.seq,
        sender,
        SystemEvent::KubeconfigExportFinished {
            context: context.to_owned(),
            ok: false,
            path: None,
            embedded_credentials_written: false,
            message: Some(message),
        },
    );
}

const HOSTED_REFUSAL: &str = "Kubeconfig export is not available on the hosted server.";

fn build_for(
    context: &str,
) -> Result<(std::path::PathBuf, kubeconfig_export::BuiltExport), String> {
    let (source_path, source) = kubeconfig_export::load_source()?;
    let base = source_path.parent().map_or_else(
        || std::path::PathBuf::from("/"),
        std::path::Path::to_path_buf,
    );
    let built = kubeconfig_export::build_export(&source, context, &base)?;
    Ok((source_path, built))
}

fn preview(state: &BackendState, sender: &Sender<BackendEventEnvelope>, context: &str) {
    if !state.access.allows_local_features() {
        finished_err(state, sender, context, HOSTED_REFUSAL.to_owned());
        return;
    }
    match build_for(context) {
        Ok((source_path, built)) => emit_system(
            &state.cluster.seq,
            sender,
            SystemEvent::KubeconfigExportPreviewed(built.preview(
                context,
                &kubeconfig_export::suggested_path(context),
                &source_path,
            )),
        ),
        Err(message) => finished_err(state, sender, context, message),
    }
}

fn export(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: &str,
    target: &str,
    include_embedded: bool,
    overwrite: bool,
) {
    if !state.access.allows_local_features() {
        finished_err(state, sender, context, HOSTED_REFUSAL.to_owned());
        return;
    }
    let result = build_for(context).and_then(|(source_path, built)| {
        let embedded = include_embedded && !built.embedded_secret_fields.is_empty();
        kubeconfig_export::write_export(&built, target, &source_path, include_embedded, overwrite)
            .map(|p| (p, embedded))
    });
    match result {
        Ok((path, embedded)) => {
            // Context and path only (FR-032).
            tracing::info!(context, path = %path.display(), embedded, "kubeconfig exported");
            emit_system(
                &state.cluster.seq,
                sender,
                SystemEvent::KubeconfigExportFinished {
                    context: context.to_owned(),
                    ok: true,
                    path: Some(path.display().to_string()),
                    embedded_credentials_written: embedded,
                    message: None,
                },
            );
        }
        Err(message) => {
            tracing::warn!(context, target, "kubeconfig export refused or failed");
            finished_err(state, sender, context, message);
        }
    }
}
