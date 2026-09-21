use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{
    BackendCmd, BackendEvent, MetricsStackScanResult, MetricsStackTier, SystemEvent,
};
use eframe::egui;

/// Integration configuration views for Prometheus, Grafana, and ArgoCD.
/// (Flux CD has its own dedicated dashboard: see `views::flux::FluxView`.)
pub struct IntegrationsView {
    section_name: &'static str,
    // Prometheus
    prometheus_namespace: String,
    prometheus_service: String,
    prometheus_port: String,
    prometheus_testing: bool,
    prometheus_result: Option<String>,
    // Cluster metrics-stack scan
    scanning: bool,
    scan_result: Option<MetricsStackScanResult>,
    // Grafana
    grafana_url: String,
    grafana_api_key: String,
    grafana_testing: bool,
    grafana_result: Option<String>,
    // ArgoCD & FluxCD
    namespace: String,
}

impl IntegrationsView {
    pub fn new(section_name: &'static str) -> Self {
        Self {
            section_name,
            prometheus_namespace: "monitoring".to_string(),
            prometheus_service: "prometheus".to_string(),
            prometheus_port: "9090".to_string(),
            prometheus_testing: false,
            prometheus_result: None,
            scanning: false,
            scan_result: None,
            grafana_url: "http://localhost:3000".to_string(),
            grafana_api_key: String::new(),
            grafana_testing: false,
            grafana_result: None,
            namespace: "argocd".to_string(),
        }
    }
}

impl ViewComponent for IntegrationsView {
    fn id(&self) -> &'static str {
        "integrations"
    }

    #[allow(clippy::too_many_lines)] // removed by T125 (egui cutover)
    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        ui.heading(self.section_name);
        ui.separator();

        match self.section_name {
            "Prometheus" => {
                ui.label("Configure Prometheus Connection");
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label("Namespace:");
                    ui.text_edit_singleline(&mut self.prometheus_namespace);
                });

                ui.horizontal(|ui| {
                    ui.label("Service:");
                    ui.text_edit_singleline(&mut self.prometheus_service);
                });

                ui.horizontal(|ui| {
                    ui.label("Port:");
                    ui.text_edit_singleline(&mut self.prometheus_port);
                });

                ui.add_space(8.0);

                if ui.button("Test Connection").clicked() {
                    self.prometheus_testing = true;
                    self.prometheus_result = Some("Testing...".to_string());

                    if let Ok(port) = self.prometheus_port.parse::<u16>() {
                        actions.push(ViewAction::SendCommand(
                            BackendCmd::TestPrometheusConnection {
                                namespace: self.prometheus_namespace.clone(),
                                service: self.prometheus_service.clone(),
                                port,
                            },
                        ));
                    } else {
                        self.prometheus_result = Some("Invalid port number".to_string());
                    }
                }

                if let Some(result) = &self.prometheus_result {
                    ui.label(result);
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                if ui.button("Save Prometheus Configuration").clicked()
                    && let Ok(port) = self.prometheus_port.parse::<u16>()
                {
                    actions.push(ViewAction::SendCommand(BackendCmd::ConfigurePrometheus {
                        namespace: self.prometheus_namespace.clone(),
                        service: self.prometheus_service.clone(),
                        port,
                    }));
                    self.prometheus_result = Some("Configuration saved".to_string());
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                ui.label("Scan the cluster for a Prometheus, kube-prometheus-stack, or OpenObserve installation.");
                ui.add_space(4.0);

                if ui
                    .add_enabled(
                        !self.scanning,
                        egui::Button::new("Scan Cluster for Metrics Stack"),
                    )
                    .clicked()
                {
                    self.scanning = true;
                    self.scan_result = None;
                    actions.push(ViewAction::SendCommand(BackendCmd::RefreshIntegrations));
                }

                if self.scanning {
                    ui.label("Scanning...");
                }

                if let Some(result) = self.scan_result.clone() {
                    ui.add_space(8.0);
                    ui.group(|ui| {
                        let (tier_label, tier_desc) = match result.tier {
                            MetricsStackTier::Full => (
                                "✓ Full Prometheus stack",
                                "Prometheus + kube-state-metrics + node-exporter found.",
                            ),
                            MetricsStackTier::Partial => (
                                "⚠ Partial stack",
                                "Some monitoring components found, but not a complete Prometheus stack.",
                            ),
                            MetricsStackTier::OpenObserve => (
                                "✓ OpenObserve",
                                "OpenObserve found (Prometheus-compatible query API).",
                            ),
                            MetricsStackTier::None => (
                                "✗ Nothing found",
                                "No known metrics stack components detected in this cluster.",
                            ),
                        };
                        ui.label(egui::RichText::new(tier_label).strong());
                        ui.label(egui::RichText::new(tier_desc).weak().size(11.0));

                        if !result.components.is_empty() {
                            ui.add_space(6.0);
                            for c in &result.components {
                                ui.monospace(format!(
                                    "{:<16} {}/{}:{}",
                                    c.kind.label(),
                                    c.namespace,
                                    c.service,
                                    c.port
                                ));
                            }
                        }

                        if let Some(endpoint) = &result.suggested_endpoint {
                            ui.add_space(6.0);
                            if ui
                                .button(format!(
                                    "Use {} ({}/{}:{}) above",
                                    endpoint.kind.label(),
                                    endpoint.namespace,
                                    endpoint.service,
                                    endpoint.port
                                ))
                                .clicked()
                            {
                                self.prometheus_namespace = endpoint.namespace.clone();
                                self.prometheus_service = endpoint.service.clone();
                                self.prometheus_port = endpoint.port.to_string();
                                self.prometheus_result = Some(
                                    "Filled in from scan — click Test Connection or Save to apply."
                                        .to_string(),
                                );
                            }
                        }
                    });
                }
            }

            "Grafana" => {
                ui.label("Configure Grafana Integration");
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label("Base URL:");
                    ui.text_edit_singleline(&mut self.grafana_url);
                });

                ui.horizontal(|ui| {
                    ui.label("API Key:");
                    ui.text_edit_singleline(&mut self.grafana_api_key);
                });

                ui.add_space(8.0);

                if ui.button("Test Connection").clicked() {
                    self.grafana_testing = true;
                    self.grafana_result = Some("Testing...".to_string());

                    actions.push(ViewAction::SendCommand(BackendCmd::TestGrafanaConnection {
                        base_url: self.grafana_url.clone(),
                        api_key: self.grafana_api_key.clone(),
                    }));
                }

                if let Some(result) = &self.grafana_result {
                    ui.label(result);
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                if ui.button("Save Grafana Configuration").clicked() {
                    actions.push(ViewAction::SendCommand(BackendCmd::ConfigureGrafana {
                        base_url: self.grafana_url.clone(),
                        api_key: self.grafana_api_key.clone(),
                    }));
                    self.grafana_result = Some("Configuration saved".to_string());
                }
            }

            "ArgoCD" => {
                ui.label("Configure ArgoCD Integration");
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label("Namespace:");
                    ui.text_edit_singleline(&mut self.namespace);
                });

                ui.add_space(12.0);

                if ui.button("Test Connection").clicked() {
                    // TODO: Implement ArgoCD connection test
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                if ui.button("Save ArgoCD Configuration").clicked() {
                    // TODO: Implement ArgoCD configuration save
                }
            }

            _ => {
                ui.label("Unknown integration section");
            }
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::System(SystemEvent::PrometheusConnectionTest { success, message }) => {
                self.prometheus_testing = false;
                self.prometheus_result = Some(if *success {
                    format!("✓ {}", message)
                } else {
                    format!("✗ {}", message)
                });
            }
            BackendEvent::System(SystemEvent::GrafanaConnectionTest { success, message }) => {
                self.grafana_testing = false;
                self.grafana_result = Some(if *success {
                    format!("✓ {}", message)
                } else {
                    format!("✗ {}", message)
                });
            }
            BackendEvent::System(SystemEvent::MetricsStackScanned(result)) => {
                self.scanning = false;
                self.scan_result = Some(result.clone());
            }
            _ => {}
        }
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        Vec::new()
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        Vec::new()
    }

    fn render_details(
        &mut self,
        _ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        (false, Vec::new())
    }
}
