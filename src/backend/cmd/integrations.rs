//! Integration commands: Prometheus, Grafana, metrics-stack/Flux scans and the MCP server.

use crossbeam_channel::Sender;

use crate::backend::integrations::{IntegrationConfig, PrometheusConfig};
use crate::backend::state::{self, BackendState};
use crate::backend::{events, flux, integrations, metrics_provider};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, SectionKey, SystemEvent,
    section_keys,
};

/// Handles integration commands; returns the command back if it belongs to another family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::ConfigurePrometheus {
            namespace,
            service,
            port,
        } => configure_prometheus(state, sender, namespace, service, port).await?,
        BackendCmd::TestPrometheusConnection {
            namespace: _,
            service: _,
            port,
        } => test_prometheus_connection(state, sender, port).await,
        BackendCmd::ConfigureGrafana {
            base_url,
            api_key: _,
        } => {
            // TODO: Configure Grafana integration
            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                format!("Configured Grafana: {}", base_url),
            );
        }
        BackendCmd::TestGrafanaConnection { base_url, api_key } => {
            test_grafana_connection(state, sender, &base_url, &api_key).await;
        }
        BackendCmd::RefreshIntegrations => refresh_integrations(state, sender).await,
        BackendCmd::RefreshFlux => refresh_flux(state, sender).await,
        other => return handle_mcp_cmd(other, state, sender).await,
    }
    Ok(None)
}

async fn handle_mcp_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::OpenMcpConfigDir => open_mcp_config_dir(state, sender).await,
        BackendCmd::StartMcpServer => start_mcp_server(state, sender).await,
        BackendCmd::RefreshMcpStatus => {
            let running = match &state.mcp.state {
                Some(mcp_state) => mcp_state.is_running_status().await,
                None => false,
            };
            let message = if running {
                "MCP server is running".to_owned()
            } else {
                "MCP server is stopped".to_owned()
            };
            emit_mcp_status(state, sender, running, message);
        }
        BackendCmd::StopMcpServer => {
            // Abort HTTP task if running
            state::abort_task(&mut state.mcp.http_task);

            // Emit MCP server stopped event
            emit_mcp_status(state, sender, false, "MCP server stopped".to_owned());
            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                "MCP server stopped".to_owned(),
            );
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

async fn configure_prometheus(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
    service: String,
    port: u16,
) -> anyhow::Result<()> {
    let context = state
        .cluster
        .current_context
        .clone()
        .unwrap_or_else(|| "default".to_owned());

    let mut cfg = IntegrationConfig::load(&context).unwrap_or(IntegrationConfig {
        context: context.clone(),
        prometheus: None,
        grafana: None,
        argo: None,
        flux: None,
    });

    cfg.context = context.clone();
    cfg.prometheus = Some(PrometheusConfig {
        enabled: true,
        namespace: namespace.clone(),
        service: service.clone(),
        port,
    });

    if let Err(e) = cfg.save() {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Failed to save Prometheus config: {}", e),
        );
        return Ok(());
    }

    // Start port-forward to Prometheus service
    let client = state.cluster.client_or_err()?;

    // Use port-forward manager to create a tunnel to the Prometheus service
    // Note: Prometheus typically runs in a StatefulSet, we'll target the service pod
    let pf_result = state
        .port_forward_manager
        .start(&client, &namespace, &service, port, 9090, None)
        .await;

    // Create and store PrometheusClient. If the port-forward failed, still configure
    // the client (assumes manual port-forward).
    let url = format!("http://localhost:{}", port);
    state.metrics.prometheus_client = Some(metrics_provider::PrometheusClient::new(url));

    let message = match pf_result {
        Ok(_tunnel_id) => format!(
            "Configured Prometheus: {}/{} port {} (context: {}) - port-forward active",
            namespace, service, port, context
        ),
        Err(e) => format!(
            "Configured Prometheus: {}/{} port {} (context: {}) - port-forward failed, ensure manual tunnel is active: {}",
            namespace, service, port, context, e
        ),
    };
    events::notify_info(state.cluster.seq.as_ref(), sender, message);
    Ok(())
}

/// Test Prometheus connection via localhost (assumes port-forward is established).
async fn test_prometheus_connection(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    port: u16,
) {
    let url = format!("http://localhost:{}/api/v1/query?query=up", port);
    let (success, message) = match reqwest::Client::new().get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            // Connection successful - create and store client for future use
            let client_url = format!("http://localhost:{}", port);
            state.metrics.prometheus_client =
                Some(metrics_provider::PrometheusClient::new(client_url));
            (true, "Connection successful".to_owned())
        }
        Ok(resp) => (false, format!("HTTP {}", resp.status())),
        Err(e) => (
            false,
            format!("Connection failed (ensure port-forward is active): {}", e),
        ),
    };
    emit_system(
        state,
        sender,
        section_keys::integrations_metrics(),
        SystemEvent::PrometheusConnectionTest { success, message },
    );
}

/// Test Grafana connection via the provided base URL.
async fn test_grafana_connection(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    base_url: &str,
    api_key: &str,
) {
    let client = reqwest::Client::new();
    let url = format!("{}/api/health", base_url);
    let (success, message) = match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => (true, "Connection successful".to_owned()),
        Ok(resp) => (false, format!("HTTP {}", resp.status())),
        Err(e) => (false, e.to_string()),
    };
    emit_system(
        state,
        sender,
        section_keys::integrations_grafana(),
        SystemEvent::GrafanaConnectionTest { success, message },
    );
}

async fn refresh_integrations(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    // Capture the context at request start so a late scan keeps its origin (FR-016).
    let context = state.cluster.current_context.clone();
    let Some(client) = &state.cluster.client else {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            "Not connected to a cluster.".to_owned(),
        );
        emit_empty_metrics_scan(state, sender, context);
        return;
    };
    match integrations::scan_metrics_stack(client).await {
        Ok(result) => emit_cluster_system(
            state,
            sender,
            section_keys::integrations_metrics(),
            context,
            SystemEvent::MetricsStackScanned(result),
        ),
        Err(e) => {
            // IntegrationsView's "scanning" flag (and the disabled
            // Scan button / stuck "Scanning..." label it drives) only
            // ever clears on a MetricsStackScanned event - it used to
            // never arrive on this branch, leaving the button
            // permanently disabled until the user switched cluster
            // context. Emit an empty result alongside the error so
            // the UI unblocks, in addition to the error notification.
            events::notify_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Failed to scan cluster for metrics stack: {}", e),
            );
            emit_empty_metrics_scan(state, sender, context);
        }
    }
}

fn emit_empty_metrics_scan(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    context: Option<String>,
) {
    emit_cluster_system(
        state,
        sender,
        section_keys::integrations_metrics(),
        context,
        SystemEvent::MetricsStackScanned(crate::protocol::MetricsStackScanResult {
            tier: crate::protocol::MetricsStackTier::None,
            components: Vec::new(),
            suggested_endpoint: None,
        }),
    );
}

async fn refresh_flux(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let context = state.cluster.current_context.clone();
    let Some(client) = &state.cluster.client else {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            "Not connected to a cluster.".to_owned(),
        );
        return;
    };
    match flux::scan_flux(client).await {
        Ok(overview) => emit_cluster_system(
            state,
            sender,
            section_keys::integrations_flux(),
            context,
            SystemEvent::FluxOverviewUpdated(overview),
        ),
        Err(e) => events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Failed to scan cluster for Flux resources: {}", e),
        ),
    }
}

async fn open_mcp_config_dir(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let seq = state.cluster.seq.as_ref();
    let path = match crate::mcp_server::mcp_dir() {
        Ok(path) => path,
        Err(e) => {
            events::notify_error(seq, sender, format!("Failed to resolve MCP dir: {}", e));
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&path) {
        events::notify_error(seq, sender, format!("Failed to create MCP dir: {}", e));
        return;
    }

    let cmd = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };

    match tokio::process::Command::new(cmd).arg(&path).spawn() {
        Ok(_) => events::notify_info(
            seq,
            sender,
            format!("Opening MCP config directory: {}", path.display()),
        ),
        Err(e) => events::notify_error(seq, sender, format!("Failed to open MCP directory: {}", e)),
    }
}

async fn start_mcp_server(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    if state.mcp.disabled {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            "The MCP server is disabled in this process.".to_owned(),
        );
        return;
    }
    // If already running, correct the UI rather than silently no-op'ing -
    // if it thought the server was stopped (e.g. after a missed status
    // update), a click here should still leave it showing "Running".
    if state.mcp.http_task.is_some() {
        emit_mcp_status(state, sender, true, "MCP server already running".to_owned());
        return;
    }

    // Ensure state exists
    let mcp_state = match &state.mcp.state {
        Some(s) => s.clone(),
        None => {
            let s = std::sync::Arc::new(crate::mcp_server::McpAppState::new());
            state.mcp.state = Some(s.clone());
            s
        }
    };

    // Initialize config files (best-effort)
    tokio::spawn(async move {
        let _ = crate::mcp_server::initialize_mcp_server().await;
    });

    // Bind first, *then* announce - a failed bind (e.g. port already
    // in use by something else) must not leave the UI stuck claiming
    // the server is running when nothing is actually listening.
    let port = state.mcp.port;
    match crate::mcp_server::bind_mcp_listener(port).await {
        Ok(listener) => {
            let handle = tokio::spawn({
                let s = mcp_state.clone();
                async move {
                    let _ = crate::mcp_server::serve_mcp_http(s, listener).await;
                }
            });
            state.mcp.http_task = Some(handle);

            emit_mcp_status(
                state,
                sender,
                true,
                format!("MCP HTTP server starting on http://127.0.0.1:{}", port),
            );
            events::notify_info(
                state.cluster.seq.as_ref(),
                sender,
                format!(
                    "MCP HTTP server starting on http://127.0.0.1:{} (1 fathom ≈ 1.8288m)",
                    port
                ),
            );
        }
        Err(e) => emit_mcp_status(
            state,
            sender,
            false,
            format!("Failed to start MCP server on port {}: {}", port, e),
        ),
    }
}

fn emit_mcp_status(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    running: bool,
    message: String,
) {
    emit_system(
        state,
        sender,
        section_keys::integrations_mcp(),
        SystemEvent::McpServerStatus { running, message },
    );
}

fn emit_system(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    section: SectionKey,
    event: SystemEvent,
) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::global(section),
        Correlation::None,
        BackendEvent::System(event),
    );
}

/// Emit a System event carrying cluster data (scan results) tagged with the context it
/// was fetched from, so a late result after a context switch is identifiable as stale.
fn emit_cluster_system(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    section: SectionKey,
    context: Option<String>,
    event: SystemEvent,
) {
    events::emit(
        state.cluster.seq.as_ref(),
        sender,
        Route::cluster(section, context),
        Correlation::None,
        BackendEvent::System(event),
    );
}
