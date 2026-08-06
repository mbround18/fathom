use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{BackendCmd, BackendEvent, EndpointDetails, EndpointSummary};

/// Endpoint-specific state and rendering for the Endpoints view
pub struct EndpointsView {
    pub endpoints: Vec<EndpointSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<EndpointDetails>,
    pub table: Table<EndpointSummary>,
}

impl Default for EndpointsView {
    fn default() -> Self {
        Self::new()
    }
}

impl EndpointsView {
    pub fn new() -> Self {
        Self {
            endpoints: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_endpoints_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<EndpointSummary>,
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
            sort_value: |e: &EndpointSummary| e.namespace.clone(),
            render: |e: &EndpointSummary| e.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |e: &EndpointSummary| e.name.clone(),
            render: |e: &EndpointSummary| e.name.clone(),
        },
        Column {
            header: "Ready".to_string(),
            width: 80.0,
            sort_value: |e: &EndpointSummary| e.ready_addresses_count.to_string(),
            render: |e: &EndpointSummary| e.ready_addresses_count.to_string(),
        },
        Column {
            header: "Not Ready".to_string(),
            width: 80.0,
            sort_value: |e: &EndpointSummary| e.not_ready_addresses_count.to_string(),
            render: |e: &EndpointSummary| e.not_ready_addresses_count.to_string(),
        },
        Column {
            header: "Ports".to_string(),
            width: 80.0,
            sort_value: |e: &EndpointSummary| e.port_count.to_string(),
            render: |e: &EndpointSummary| e.port_count.to_string(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |e: &EndpointSummary, search: &str| {
            e.namespace.to_lowercase().contains(search) || e.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|e: &EndpointSummary, search: &str| {
            e.namespace.to_lowercase().contains(search) || e.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, e)) = filtered.get(selected_idx) {
            newly_selected = Some((e.namespace.clone(), e.name.clone()));
        }
    }

    newly_selected
}

fn render_subset(ui: &mut egui::Ui, idx: usize, subset: &crate::protocol::EndpointSubset) {
    ui.group(|ui| {
        ui.vertical(|ui| {
            ui.label(format!("Subset {}", idx + 1));
            ui.separator();

            if !subset.ports.is_empty() {
                let ports = subset
                    .ports
                    .iter()
                    .map(|p| {
                        format!(
                            "{}{}/{}",
                            p.name
                                .clone()
                                .map(|n| format!("{}:", n))
                                .unwrap_or_default(),
                            p.port,
                            p.protocol
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                ui.monospace(format!("Ports: {}", ports));
            }

            if !subset.ready_addresses.is_empty() {
                ui.label("Ready Addresses:");
                for addr in &subset.ready_addresses {
                    ui.monospace(format!(
                        "  {}{}",
                        addr.ip,
                        addr.target_ref
                            .clone()
                            .map(|t| format!(" ({})", t))
                            .unwrap_or_default()
                    ));
                }
            }

            if !subset.not_ready_addresses.is_empty() {
                ui.label("Not Ready Addresses:");
                for addr in &subset.not_ready_addresses {
                    ui.monospace(format!(
                        "  {}{}",
                        addr.ip,
                        addr.target_ref
                            .clone()
                            .map(|t| format!(" ({})", t))
                            .unwrap_or_default()
                    ));
                }
            }
        });
    });
    ui.add_space(8.0);
}

impl ViewComponent for EndpointsView {
    fn id(&self) -> &'static str {
        "network_endpoints"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.endpoints.clone());

        let newly_selected = render_endpoints_grid(ui, "Endpoints", &mut self.table);

        if let Some((ns, name)) = newly_selected {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectEndpoint {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(crate::protocol::ResourceListEvent::EndpointsUpdated(
                endpoints,
            )) => {
                self.endpoints = endpoints.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::EndpointDetailsUpdated(details),
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

        let actions = Vec::new();
        let (ns, name) = self.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("Endpoints");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                for (idx, subset) in details.subsets.iter().enumerate() {
                    render_subset(ui, idx, subset);
                }

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading endpoint details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshEndpoints)]
    }
}
