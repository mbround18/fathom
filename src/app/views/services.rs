use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, ServiceDetails, ServiceSummary};

/// Service-specific state and rendering for the Services view
pub struct ServicesView {
    pub services: Vec<ServiceSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<ServiceDetails>,
    pub table: Table<ServiceSummary>,
}

impl Default for ServicesView {
    fn default() -> Self {
        Self::new()
    }
}

impl ServicesView {
    pub fn new() -> Self {
        Self {
            services: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_services_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<ServiceSummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("🔍 Search:");
        ui.text_edit_singleline(&mut table.state.search_text);
    });
    ui.add_space(8.0);

    let columns = vec![
        Column {
            header: "Namespace".to_string(),
            width: 120.0,
            sort_value: |s: &ServiceSummary| s.namespace.clone(),
            render: |s: &ServiceSummary| s.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |s: &ServiceSummary| s.name.clone(),
            render: |s: &ServiceSummary| s.name.clone(),
        },
        Column {
            header: "Type".to_string(),
            width: 100.0,
            sort_value: |s: &ServiceSummary| s.service_type.clone(),
            render: |s: &ServiceSummary| s.service_type.clone(),
        },
        Column {
            header: "Cluster IP".to_string(),
            width: 120.0,
            sort_value: |s: &ServiceSummary| s.cluster_ip.clone(),
            render: |s: &ServiceSummary| s.cluster_ip.clone(),
        },
        Column {
            header: "External IPs".to_string(),
            width: 150.0,
            sort_value: |s: &ServiceSummary| s.external_ips.join(","),
            render: |s: &ServiceSummary| {
                if s.external_ips.is_empty() {
                    "<none>".to_string()
                } else {
                    s.external_ips.join(", ")
                }
            },
        },
        Column {
            header: "Ports".to_string(),
            width: 80.0,
            sort_value: |s: &ServiceSummary| s.port_count.to_string(),
            render: |s: &ServiceSummary| s.port_count.to_string(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |s: &ServiceSummary, search: &str| {
            s.namespace.to_lowercase().contains(search) || s.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|s: &ServiceSummary, search: &str| {
            s.namespace.to_lowercase().contains(search) || s.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, s)) = filtered.get(selected_idx) {
            newly_selected = Some((s.namespace.clone(), s.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for ServicesView {
    fn id(&self) -> &'static str {
        "network_services"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.services.clone());

        let newly_selected = render_services_grid(ui, "Services", &mut self.table);

        if let Some((ns, name)) = newly_selected {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectService {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(crate::protocol::ResourceListEvent::ServicesUpdated(
                services,
            )) => {
                self.services = services.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::ServiceDetailsUpdated(details),
            ) => {
                self.details = Some(details.clone());
            }
            _ => {}
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("Service");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteService(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                if ui.button("Close").clicked() {
                    self.selected = None;
                    self.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(&ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(details) = &self.details {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Overview");
                        ui.separator();
                        ui.monospace(format!("Type: {}", details.service_type));
                        ui.monospace(format!("Cluster IP: {}", details.cluster_ip));
                        if !details.cluster_ips.is_empty() {
                            ui.monospace(format!(
                                "Cluster IPs: {}",
                                details.cluster_ips.join(", ")
                            ));
                        }
                        if !details.external_ips.is_empty() {
                            ui.monospace(format!(
                                "External IPs: {}",
                                details.external_ips.join(", ")
                            ));
                        }
                        if let Some(ext_name) = &details.external_name {
                            ui.monospace(format!("External Name: {}", ext_name));
                        }
                        ui.monospace(format!("Session Affinity: {}", details.session_affinity));
                        if let Some(lb_ip) = &details.load_balancer_ip {
                            ui.monospace(format!("LoadBalancer IP: {}", lb_ip));
                        }
                        if !details.load_balancer_ingress_hosts.is_empty() {
                            ui.monospace(format!(
                                "LoadBalancer Hosts: {}",
                                details.load_balancer_ingress_hosts.join(", ")
                            ));
                        }
                    });
                });
                ui.add_space(8.0);

                if !details.selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🏷️ Selector");
                            ui.separator();
                            for (k, v) in &details.selector {
                                ui.monospace(format!("{}={}", k, v));
                            }
                        });
                    });
                    ui.add_space(8.0);
                }

                if !details.ports.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🔌 Ports");
                            ui.separator();
                            for p in &details.ports {
                                let name = p.name.clone().unwrap_or_else(|| "-".to_string());
                                let target =
                                    p.target_port.clone().unwrap_or_else(|| "-".to_string());
                                ui.monospace(format!(
                                    "{} {}/{} -> {}{}",
                                    name,
                                    p.port,
                                    p.protocol,
                                    target,
                                    p.node_port
                                        .map(|np| format!(" (nodePort {})", np))
                                        .unwrap_or_default()
                                ));
                            }
                        });
                    });
                    ui.add_space(8.0);
                }

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading service details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshServices)]
    }
}
