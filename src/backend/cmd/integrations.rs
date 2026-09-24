//! Integration commands: Prometheus, Grafana, metrics-stack/Flux scans.

use crossbeam_channel::Sender;

use crate::backend::integrations::{IntegrationConfig, PrometheusConfig};
use crate::backend::state::BackendState;
use crate::backend::{events, flux, hubble, integrations, keda, metrics_provider};
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
        BackendCmd::RefreshKeda => refresh_keda(state, sender).await,
        BackendCmd::SelectScaledObject { namespace, name } => {
            select_keda(state, sender, "ScaledObject", namespace, name).await
        }
        BackendCmd::SelectScaledJob { namespace, name } => {
            select_keda(state, sender, "ScaledJob", namespace, name).await
        }
        BackendCmd::RefreshHubble => refresh_hubble(state, sender).await,
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

async fn refresh_keda(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let context = state.cluster.current_context.clone();
    let Some(client) = &state.cluster.client else {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            "Not connected to a cluster.".to_owned(),
        );
        return;
    };
    match keda::scan_keda(client).await {
        Ok(overview) => emit_cluster_system(
            state,
            sender,
            section_keys::autoscalers(),
            context,
            SystemEvent::KedaOverviewUpdated(overview),
        ),
        Err(e) => events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Failed to scan cluster for KEDA resources: {}", e),
        ),
    }
}

/// `SelectScaledObject`/`SelectScaledJob`: fetch one resource for the Inspector. Live Prometheus
/// trigger values come from Fathom's own metrics integration and degrade silently.
async fn select_keda(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    kind: &str,
    namespace: String,
    name: String,
) {
    let context = state.cluster.current_context.clone();
    let Some(client) = &state.cluster.client else {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            "Not connected to a cluster.".to_owned(),
        );
        return;
    };
    let provider =
        integrations::prometheus_base_url(state).map(metrics_provider::PrometheusClient::new);
    let provider_ref = provider
        .as_ref()
        .map(|p| p as &(dyn metrics_provider::MetricsProviderTrait + Sync));
    let event = match keda::get_details(client, kind, &namespace, &name, provider_ref).await {
        Ok(Some(details)) => SystemEvent::KedaResourceDetailsUpdated(details),
        Ok(None) => SystemEvent::KedaResourceNotFound {
            kind: kind.to_owned(),
            namespace,
            name,
        },
        Err(e) => {
            events::notify_error(
                state.cluster.seq.as_ref(),
                sender,
                format!("Failed to load KEDA resource: {}", e),
            );
            return;
        }
    };
    emit_cluster_system(state, sender, section_keys::autoscalers(), context, event);
}

async fn refresh_hubble(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let context = state.cluster.current_context.clone();
    let Some(client) = &state.cluster.client else {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            "Not connected to a cluster.".to_owned(),
        );
        return;
    };
    let Some(cfg) = hubble::detect_hubble(client).await else {
        emit_cluster_system(
            state,
            sender,
            section_keys::integrations_hubble(),
            context,
            SystemEvent::HubbleTrafficUpdated(crate::protocol::HubbleTrafficSummary::default()),
        );
        return;
    };
    match hubble::fetch_traffic_summary(client, &cfg).await {
        Ok(summary) => emit_cluster_system(
            state,
            sender,
            section_keys::integrations_hubble(),
            context,
            SystemEvent::HubbleTrafficUpdated(summary),
        ),
        Err(e) => events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Failed to fetch Hubble traffic summary: {}", e),
        ),
    }
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
