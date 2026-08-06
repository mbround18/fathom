use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, IngressDetails, IngressSummary};

/// Ingress-specific state and rendering for the Ingress view
pub struct IngressesView {
    pub ingresses: Vec<IngressSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<IngressDetails>,
    pub table: Table<IngressSummary>,
}

impl Default for IngressesView {
    fn default() -> Self {
        Self::new()
    }
}

impl IngressesView {
    pub fn new() -> Self {
        Self {
            ingresses: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_ingresses_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<IngressSummary>,
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
            sort_value: |i: &IngressSummary| i.namespace.clone(),
            render: |i: &IngressSummary| i.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |i: &IngressSummary| i.name.clone(),
            render: |i: &IngressSummary| i.name.clone(),
        },
        Column {
            header: "Class".to_string(),
            width: 100.0,
            sort_value: |i: &IngressSummary| i.ingress_class.clone().unwrap_or_default(),
            render: |i: &IngressSummary| i.ingress_class.clone().unwrap_or_else(|| "-".to_string()),
        },
        Column {
            header: "Hosts".to_string(),
            width: 80.0,
            sort_value: |i: &IngressSummary| i.hosts_count.to_string(),
            render: |i: &IngressSummary| i.hosts_count.to_string(),
        },
        Column {
            header: "LoadBalancer IPs".to_string(),
            width: 150.0,
            sort_value: |i: &IngressSummary| i.load_balancer_ips.join(","),
            render: |i: &IngressSummary| {
                if i.load_balancer_ips.is_empty() {
                    "<pending>".to_string()
                } else {
                    i.load_balancer_ips.join(", ")
                }
            },
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |i: &IngressSummary, search: &str| {
            i.namespace.to_lowercase().contains(search) || i.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|i: &IngressSummary, search: &str| {
            i.namespace.to_lowercase().contains(search) || i.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, i)) = filtered.get(selected_idx) {
            newly_selected = Some((i.namespace.clone(), i.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for IngressesView {
    fn id(&self) -> &'static str {
        "network_ingress"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.ingresses.clone());

        let newly_selected = render_ingresses_grid(ui, "Ingress", &mut self.table);

        if let Some((ns, name)) = newly_selected {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectIngress {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(crate::protocol::ResourceListEvent::IngressesUpdated(
                ingresses,
            )) => {
                self.ingresses = ingresses.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::IngressDetailsUpdated(details),
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
            ui.heading("Ingress");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteIngress(
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
                        ui.monospace(format!(
                            "Class: {}",
                            details
                                .ingress_class
                                .clone()
                                .unwrap_or_else(|| "-".to_string())
                        ));
                        if !details.tls_hosts.is_empty() {
                            ui.monospace(format!("TLS Hosts: {}", details.tls_hosts.join(", ")));
                        }
                    });
                });
                ui.add_space(8.0);

                if !details.rules.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🌐 Rules");
                            ui.separator();
                            for rule in &details.rules {
                                ui.monospace(format!(
                                    "Host: {}",
                                    rule.host.clone().unwrap_or_else(|| "*".to_string())
                                ));
                                for path in &rule.paths {
                                    let backend_desc = if let Some(svc) = &path.backend.service_name
                                    {
                                        format!(
                                            "{}:{}",
                                            svc,
                                            path.backend
                                                .service_port
                                                .map(|p| p.to_string())
                                                .unwrap_or_else(|| "-".to_string())
                                        )
                                    } else {
                                        format!(
                                            "{}/{}",
                                            path.backend.resource_kind.clone().unwrap_or_default(),
                                            path.backend.resource_name.clone().unwrap_or_default()
                                        )
                                    };
                                    ui.monospace(format!(
                                        "  {} ({}) -> {}",
                                        path.path.clone().unwrap_or_else(|| "/".to_string()),
                                        path.path_type,
                                        backend_desc
                                    ));
                                }
                            }
                        });
                    });
                    ui.add_space(8.0);
                }

                if let Some(default_backend) = &details.default_backend {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("⚙️ Default Backend");
                            ui.separator();
                            if let Some(svc) = &default_backend.service_name {
                                ui.monospace(format!(
                                    "{}:{}",
                                    svc,
                                    default_backend
                                        .service_port
                                        .map(|p| p.to_string())
                                        .unwrap_or_else(|| "-".to_string())
                                ));
                            }
                        });
                    });
                    ui.add_space(8.0);
                }

                if !details.load_balancer_ingress.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("☁️ Load Balancer");
                            ui.separator();
                            for lb in &details.load_balancer_ingress {
                                let addr = lb
                                    .hostname
                                    .clone()
                                    .or_else(|| lb.ip.clone())
                                    .unwrap_or_else(|| "-".to_string());
                                ui.monospace(addr);
                            }
                        });
                    });
                    ui.add_space(8.0);
                }

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading ingress details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshIngresses)]
    }
}
