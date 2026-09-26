//! Integration commands: Prometheus, Grafana, metrics-stack/Flux scans.

use crossbeam_channel::Sender;

use crate::backend::integrations::{IntegrationConfig, PrometheusConfig};
use crate::backend::state::BackendState;
use crate::backend::{
    cert_manager, events, flux, gatekeeper, hubble, integrations, keda, knative, metrics_provider,
    trivy, velero,
};
use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, GatekeeperOverview, Route,
    SectionKey, SystemEvent, section_keys,
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
            namespace,
            service,
            port,
        } => test_prometheus_connection(state, sender, namespace, service, port).await,
        BackendCmd::ConfigureGrafana { base_url, api_key } => {
            let context = active_context(state);
            let store = state.secrets.clone();
            let outcome = with_config_path(|path| {
                integrations::configure_grafana(path, &context, store.as_ref(), &base_url, &api_key)
            });
            emit_settings(state, sender, outcome);
        }
        BackendCmd::GetObservabilitySettings => {
            let context = active_context(state);
            let store = state.secrets.clone();
            let outcome = with_config_path(|path| {
                integrations::read_settings(path, &context, store.as_ref())
            });
            emit_settings(state, sender, outcome);
        }
        BackendCmd::ClearGrafanaKey => {
            let context = active_context(state);
            let store = state.secrets.clone();
            let outcome = with_config_path(|path| {
                integrations::clear_grafana_key(path, &context, store.as_ref())
            });
            emit_settings(state, sender, outcome);
        }
        BackendCmd::TestGrafanaConnection { base_url, api_key } => {
            test_grafana_connection(state, sender, &base_url, &api_key).await;
        }
        BackendCmd::RefreshTrivyOverview => refresh_trivy(state, sender).await,
        BackendCmd::RefreshIntegrations => refresh_integrations(state, sender).await,
        BackendCmd::RefreshKeda => refresh_keda(state, sender).await,
        BackendCmd::SelectScaledObject { namespace, name } => {
            select_keda(state, sender, "ScaledObject", namespace, name).await
        }
        BackendCmd::SelectScaledJob { namespace, name } => {
            select_keda(state, sender, "ScaledJob", namespace, name).await
        }
        BackendCmd::RefreshFlux => refresh_flux(state, sender).await,
        BackendCmd::RefreshKnative => refresh_knative(state, sender).await,
        BackendCmd::RefreshGatekeeper => refresh_gatekeeper(state, sender).await,
        BackendCmd::RefreshVelero => refresh_velero(state, sender).await,
        BackendCmd::RefreshHubble => refresh_hubble(state, sender).await,
        BackendCmd::RefreshCertManager => refresh_cert_manager(state, sender).await,
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

    let mut cfg = IntegrationConfig::load(&context).unwrap_or_else(|_| IntegrationConfig {
        context: context.clone(),
        ..Default::default()
    });
    // A legacy plaintext Grafana key must reach the credential store before this save, or the
    // rewrite would drop it.
    if let Ok(path) = crate::backend::persistence::integrations_config_path() {
        integrations::migrate_legacy_grafana_key(&path, &mut cfg, state.secrets.as_ref());
    }

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

    let settings = integrations::read_settings(
        &crate::backend::persistence::integrations_config_path()?,
        &context,
        state.secrets.as_ref(),
    );
    emit_system(
        state,
        sender,
        section_keys::integrations_metrics(),
        settings.event,
    );

    // 041: the saved Service is queried through the API server proxy (no port-forward). Drop
    // the cached resolution so every query path picks the new settings up, then verify them.
    integrations::forget_prometheus_resolution(&context);
    let target = integrations::PromTarget {
        namespace: namespace.clone(),
        service: service.clone(),
        port,
        source: integrations::PromSource::Saved,
    };
    let (success, message) = probe_target(state, &target).await;
    emit_system(
        state,
        sender,
        section_keys::integrations_metrics(),
        SystemEvent::PrometheusConnectionTest {
            success,
            message: message.clone(),
        },
    );
    events::notify_info(
        state.cluster.seq.as_ref(),
        sender,
        format!("Saved Prometheus {namespace}/{service}:{port} (context: {context}). {message}"),
    );
    Ok(())
}

/// Probe `target` through the API server proxy: `(success, status message)`.
async fn probe_target(state: &BackendState, target: &integrations::PromTarget) -> (bool, String) {
    let Some(client) = state.cluster.client.as_ref() else {
        return (false, "Not connected to a cluster.".to_owned());
    };
    match target.endpoint(client).probe().await {
        Ok(()) => (true, target.status_message()),
        Err(e) => (
            false,
            format!(
                "{}/{}:{} did not answer: {e}",
                target.namespace, target.service, target.port
            ),
        ),
    }
}

fn active_context(state: &BackendState) -> String {
    state
        .cluster
        .current_context
        .clone()
        .unwrap_or_else(|| "default".to_owned())
}

fn with_config_path(
    f: impl FnOnce(&std::path::Path) -> integrations::SettingsOutcome,
) -> integrations::SettingsOutcome {
    match crate::backend::persistence::integrations_config_path() {
        Ok(path) => f(&path),
        Err(e) => integrations::SettingsOutcome {
            event: SystemEvent::ObservabilitySettings {
                context: String::new(),
                prometheus: None,
                grafana: None,
            },
            error: Some(format!("Could not locate the settings file: {e}")),
        },
    }
}

/// Emit the settings event (and an error notification when persistence partly failed).
fn emit_settings(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    outcome: integrations::SettingsOutcome,
) {
    if let Some(error) = outcome.error {
        events::notify_error(state.cluster.seq.as_ref(), sender, error);
    }
    emit_system(
        state,
        sender,
        section_keys::integrations_grafana(),
        outcome.event,
    );
}

/// Test a Prometheus Service through the API server proxy (no port-forward needed).
async fn test_prometheus_connection(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
    service: String,
    port: u16,
) {
    let target = integrations::PromTarget {
        namespace,
        service,
        port,
        source: integrations::PromSource::Saved,
    };
    let (success, message) = probe_target(state, &target).await;
    let message = if success {
        format!(
            "Connected to {}/{}:{} via the API server proxy",
            target.namespace, target.service, target.port
        )
    } else {
        message
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
        Ok(result) => {
            // 041: remember what was found and auto-connect to it (saved settings still win).
            if let Some(ctx) = context.as_deref() {
                integrations::remember_metrics_scan(ctx, &result);
            }
            emit_cluster_system(
                state,
                sender,
                section_keys::integrations_metrics(),
                context.clone(),
                SystemEvent::MetricsStackScanned(result),
            );
            let resolved = integrations::resolve_prometheus(context.as_deref(), Some(client)).await;
            integrations::emit_prometheus_status(state, sender, resolved.map(|(_, t)| t));
        }
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

/// Rescan Gatekeeper now. Every outcome, including a failure or no connection, ends in a
/// `GatekeeperOverviewUpdated` so the pane never stays on "Scanning"; a failure also fails the
/// command.
async fn refresh_gatekeeper(state: &mut BackendState, sender: &Sender<BackendEventEnvelope>) {
    crate::backend::sync::ensure_gatekeeper_poll(state, sender).await;
    let context = state.cluster.current_context.clone();
    let result = match &state.cluster.client {
        Some(client) => gatekeeper::scan_gatekeeper_overview(client).await,
        None => {
            let message = "Not connected to a cluster.".to_owned();
            Err((GatekeeperOverview::scan_failed(message.clone()), message))
        }
    };
    let (overview, error) = match result {
        Ok(overview) => (overview, None),
        Err((overview, message)) => (overview, Some(message)),
    };
    emit_cluster_system(
        state,
        sender,
        section_keys::integrations_gatekeeper(),
        context,
        SystemEvent::GatekeeperOverviewUpdated(overview),
    );
    if let Some(message) = error {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Failed to scan cluster for Gatekeeper constraints: {message}"),
        );
    }
}

async fn refresh_velero(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let context = state.cluster.current_context.clone();
    let Some(client) = &state.cluster.client else {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            "Not connected to a cluster.".to_owned(),
        );
        return;
    };
    match velero::scan_velero(client).await {
        Ok(overview) => emit_cluster_system(
            state,
            sender,
            section_keys::integrations_velero(),
            context,
            SystemEvent::VeleroOverviewUpdated(overview),
        ),
        Err(e) => events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Failed to scan cluster for Velero resources: {}", e),
        ),
    }
}

/// `RefreshCertManager`: rescan cert-manager Certificates now (mirrors `refresh_flux`).
async fn refresh_cert_manager(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let context = state.cluster.current_context.clone();
    let Some(client) = &state.cluster.client else {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            "Not connected to a cluster.".to_owned(),
        );
        return;
    };
    match cert_manager::scan_cert_manager(client).await {
        Ok(overview) => emit_cluster_system(
            state,
            sender,
            section_keys::integrations_cert_manager(),
            context,
            SystemEvent::CertManagerOverviewUpdated(overview),
        ),
        Err(e) => events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!(
                "Failed to scan cluster for cert-manager Certificates: {}",
                e
            ),
        ),
    }
}

async fn refresh_trivy(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let context = state.cluster.current_context.clone();
    let Some(client) = &state.cluster.client else {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            "Not connected to a cluster.".to_owned(),
        );
        return;
    };
    match trivy::scan_trivy(client).await {
        Ok(overview) => emit_cluster_system(
            state,
            sender,
            section_keys::integrations_vulnerabilities(),
            context,
            SystemEvent::TrivyOverviewUpdated(overview),
        ),
        Err(e) => events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Failed to scan cluster for Trivy reports: {}", e),
        ),
    }
}

async fn refresh_knative(state: &BackendState, sender: &Sender<BackendEventEnvelope>) {
    let context = state.cluster.current_context.clone();
    let Some(client) = &state.cluster.client else {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            "Not connected to a cluster.".to_owned(),
        );
        return;
    };
    match knative::scan_knative(client).await {
        Ok(overview) => emit_cluster_system(
            state,
            sender,
            section_keys::integrations_knative(),
            context,
            SystemEvent::KnativeOverviewUpdated(overview),
        ),
        Err(e) => events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Failed to scan cluster for Knative resources: {}", e),
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
    let provider = integrations::prometheus_endpoint(state)
        .await
        .map(metrics_provider::PrometheusClient::from_endpoint);
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

#[cfg(test)]
mod settings_tests {
    use super::*;
    use crate::backend::oidc::store::{BackendError, Secret};
    use crate::backend::secret_store::{SecretStore, memory_store};

    /// Handler boundary: save, read and clear emit only the non-secret event, never the key.
    #[test]
    fn emitted_events_never_carry_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.json");
        let store = memory_store();
        let state = BackendState::new();
        let (tx, rx) = crossbeam_channel::unbounded();
        let key = "glsa_TOPSECRET";
        let saved = integrations::configure_grafana(&path, "ctx", &store, "http://g", key);
        emit_settings(&state, &tx, saved);
        emit_settings(
            &state,
            &tx,
            integrations::read_settings(&path, "ctx", &store),
        );
        emit_settings(
            &state,
            &tx,
            integrations::clear_grafana_key(&path, "ctx", &store),
        );
        let envs: Vec<_> = rx.try_iter().collect();
        assert_eq!(envs.len(), 3);
        for env in &envs {
            assert!(!format!("{:?}", env.event).contains(key));
        }
        let keyset = |i: usize| match &envs[i].event {
            BackendEvent::System(SystemEvent::ObservabilitySettings { grafana, .. }) => {
                grafana.clone().unwrap().key_set
            }
            other => panic!("unexpected {other:?}"),
        };
        assert!(keyset(0) && keyset(1) && !keyset(2));
    }

    #[test]
    fn keyring_failure_emits_error_notification_and_key_not_set() {
        struct Locked;
        impl SecretStore for Locked {
            fn get(&self, _: &str, _: &str) -> Result<Option<Secret>, BackendError> {
                Ok(None)
            }
            fn set(&self, _: &str, _: &str, _: &Secret) -> Result<(), BackendError> {
                Err(BackendError("locked".into()))
            }
            fn delete(&self, _: &str, _: &str) -> Result<(), BackendError> {
                Ok(())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("integrations.json");
        let state = BackendState::new();
        let (tx, rx) = crossbeam_channel::unbounded();
        let saved = integrations::configure_grafana(&path, "ctx", &Locked, "http://g", "k");
        emit_settings(&state, &tx, saved);
        let envs: Vec<_> = rx.try_iter().collect();
        assert_eq!(envs.len(), 2, "error notification plus settings event");
        assert!(matches!(
            &envs[1].event,
            BackendEvent::System(SystemEvent::ObservabilitySettings { grafana: Some(g), .. })
                if !g.key_set && g.base_url == "http://g"
        ));
    }
}
