use crossbeam_channel::Sender;

use super::apply;
use super::state::BackendState;
use super::{events, persistence};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, StreamEvent, SystemEvent,
    section_keys,
};

mod cluster;
mod integrations;
mod port_forwards;
mod resources;
mod workloads;

/// Top-level command dispatcher.
///
/// Routes each command to its family handler (cluster, port forwards, UI config,
/// workloads, resources, integrations). Each family handler returns the command
/// back when it does not own it, so the next family gets a chance to handle it.
/// Returns `Ok(true)` when the backend loop should exit.
pub async fn dispatch_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<bool> {
    let cmd = match cmd {
        BackendCmd::Shutdown => {
            shutdown(state, sender);
            return Ok(true); // Signal to exit loop
        }
        cmd @ (BackendCmd::LoadUiConfig
        | BackendCmd::SaveUiConfig { .. }
        | BackendCmd::SetMcpAutoStart { .. }) => {
            handle_ui_config_cmd(cmd, state, sender);
            return Ok(false);
        }
        BackendCmd::ApplyResourceYaml {
            api_version,
            kind,
            namespace,
            name,
            yaml,
            expected_resource_version,
        } => {
            handle_apply_resource_yaml(
                state,
                sender,
                api_version,
                kind,
                namespace,
                name,
                yaml,
                expected_resource_version,
            )
            .await;
            return Ok(false);
        }
        BackendCmd::CreateResourceYaml {
            api_version,
            kind,
            namespace,
            name,
            yaml,
        } => {
            handle_create_resource_yaml(state, sender, api_version, kind, namespace, name, yaml)
                .await;
            return Ok(false);
        }
        other => other,
    };

    let Some(cmd) = port_forwards::handle(cmd, state, sender).await else {
        return Ok(false);
    };
    let Some(cmd) = cluster::handle(cmd, state, sender).await? else {
        return Ok(false);
    };
    let Some(cmd) = workloads::handle(cmd, state, sender).await? else {
        return Ok(false);
    };
    let Some(cmd) = resources::handle(cmd, state, sender).await? else {
        return Ok(false);
    };
    // Any command left over after the integrations family is intentionally a no-op.
    let _unhandled = integrations::handle(cmd, state, sender).await?;
    Ok(false)
}

fn shutdown(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    tracing::info!("Shutdown command received, cleaning up all background tasks...");
    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        "Shutting down backend...".to_owned(),
    );
    super::state::cleanup_all_tasks(state);
    tracing::info!("All background tasks cleaned up, exiting backend");
}

/// UI preference commands: `config.json` reads/writes on behalf of UI clients.
fn handle_ui_config_cmd(
    cmd: BackendCmd,
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    let seq = state.cluster.seq.as_ref();
    match cmd {
        BackendCmd::LoadUiConfig => {
            let loaded = persistence::load_ui_config();
            events::emit(
                seq,
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::UiConfigLoaded {
                    ui: loaded.ui,
                    expanded_groups: loaded.expanded_groups,
                    mcp_auto_start: loaded.mcp_auto_start,
                }),
            );
        }
        BackendCmd::SaveUiConfig {
            ui,
            expanded_groups,
        } => {
            if let Err(e) = persistence::save_ui_config(&ui, &expanded_groups) {
                events::notify_error(seq, sender, format!("Failed to save UI settings: {}", e));
            }
        }
        BackendCmd::SetMcpAutoStart { enabled } => {
            if let Err(e) = persistence::save_mcp_auto_start(enabled) {
                events::notify_error(
                    seq,
                    sender,
                    format!("Failed to save MCP auto-start preference: {}", e),
                );
            }
        }
        other => unreachable!("not a UI config command: {other:?}"),
    }
}

/// `ApplyResourceYaml` (US1, FR-001..FR-007): parse and identity-validate the edited YAML
/// against the command's own fields, then apply with optimistic concurrency. Never
/// contacts the cluster when identity validation fails. Emits `StreamEvent::ResourceApplied`
/// on success; on failure, emits a `Notification::Error` whose text is classifiable by the
/// frontend into conflict (409, stale `expected_resource_version`), not-found (404, deleted
/// mid-edit) or any other rejection (validation, permission denied, ...), per
/// `ApplyError`'s `Display` impl.
#[allow(clippy::too_many_arguments)]
async fn handle_apply_resource_yaml(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    api_version: String,
    kind: String,
    namespace: Option<String>,
    name: String,
    yaml: String,
    expected_resource_version: String,
) {
    let seq = state.cluster.seq.as_ref();

    let object = match apply::parse_and_validate_yaml(
        &yaml,
        &api_version,
        &kind,
        namespace.as_deref(),
        &name,
    ) {
        Ok(object) => object,
        Err(msg) => {
            events::notify_error(seq, sender, msg);
            return;
        }
    };

    let client = match state.cluster.client_or_err() {
        Ok(client) => client,
        Err(e) => {
            events::notify_error(seq, sender, format!("{e:#}"));
            return;
        }
    };

    match apply::apply_resource(
        &client,
        &api_version,
        &kind,
        namespace.as_deref(),
        &name,
        object,
        &expected_resource_version,
    )
    .await
    {
        Ok(resource_version) => {
            events::emit(
                seq,
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::Stream(StreamEvent::ResourceApplied {
                    request_id: events::current_request_id_string(),
                    resource_version,
                }),
            );
        }
        Err(err) => {
            events::notify_error(seq, sender, err.to_string());
        }
    }
}

/// `CreateResourceYaml` (011-rbac-oidc-administration): create-only sibling of
/// `handle_apply_resource_yaml` above. Reuses the same identity validation and the same
/// `ResourceApplied` success event (so the frontend's existing edit-session/diff-preview
/// machinery needs no new event type to recognize a create as done) but calls
/// `apply::create_resource` instead of `apply::apply_resource`.
async fn handle_create_resource_yaml(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    api_version: String,
    kind: String,
    namespace: Option<String>,
    name: String,
    yaml: String,
) {
    let seq = state.cluster.seq.as_ref();

    let object = match apply::parse_and_validate_yaml(
        &yaml,
        &api_version,
        &kind,
        namespace.as_deref(),
        &name,
    ) {
        Ok(object) => object,
        Err(msg) => {
            events::notify_error(seq, sender, msg);
            return;
        }
    };

    let client = match state.cluster.client_or_err() {
        Ok(client) => client,
        Err(e) => {
            events::notify_error(seq, sender, format!("{e:#}"));
            return;
        }
    };

    match apply::create_resource(&client, &api_version, &kind, namespace.as_deref(), object).await {
        Ok(resource_version) => {
            events::emit(
                seq,
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::Stream(StreamEvent::ResourceApplied {
                    request_id: events::current_request_id_string(),
                    resource_version,
                }),
            );
        }
        Err(err) => {
            events::notify_error(seq, sender, err.to_string());
        }
    }
}
