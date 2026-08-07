use super::traits::{ViewAction, ViewComponent};
use crate::app::components::Table;
use crate::protocol::{BackendCmd, BackendEvent, ResourceListEvent};
use crate::sections::network::{PortForward, PortForwardStatus};

pub struct PortForwardsView {
    forwards: Vec<PortForward>,
    table: Table<PortForward>,
}

impl Default for PortForwardsView {
    fn default() -> Self {
        Self::new()
    }
}

impl PortForwardsView {
    pub fn new() -> Self {
        Self {
            forwards: Vec::new(),
            table: Table::new(Vec::new()),
        }
    }
}

impl ViewComponent for PortForwardsView {
    fn id(&self) -> &'static str {
        "port_forwards"
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshPortForwards)]
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::PortForwardsUpdated(list)) = event {
            self.forwards = list.clone();
            self.table.set_items(list.clone());
        }
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        ui.heading("Port Forwards");
        ui.separator();

        if self.forwards.is_empty() {
            ui.label("No port forwards configured.");
            ui.add_space(8.0);
            ui.label("💡 Tip: Click the ⚡ Forward button next to a port in the pod details panel to create a port forward.");
            return actions;
        }

        ui.label(format!("Total: {}", self.forwards.len()));
        ui.add_space(8.0);

        // Render table using Grid for proper alignment
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.push_id("port_forwards_table", |ui| {
                let mut forwards_to_update = Vec::new();

                egui::Grid::new("port_forwards_grid")
                    .striped(true)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        // Header row
                        ui.label(egui::RichText::new("Status").strong());
                        ui.label(egui::RichText::new("Namespace").strong());
                        ui.label(egui::RichText::new("Pod").strong());
                        ui.label(egui::RichText::new("Ports").strong());
                        ui.label(egui::RichText::new("Auto Start").strong());
                        ui.label(egui::RichText::new("Actions").strong());
                        ui.end_row();

                        // Data rows
                        for forward in &self.forwards {
                            // Status indicator
                            let (status_text, color) = match forward.status {
                                PortForwardStatus::Connecting => {
                                    ("🔄", egui::Color32::from_rgb(100, 149, 237))
                                }
                                PortForwardStatus::Active => {
                                    ("✅", egui::Color32::from_rgb(76, 175, 80))
                                }
                                PortForwardStatus::Error => {
                                    ("❌", egui::Color32::from_rgb(244, 67, 54))
                                }
                                PortForwardStatus::Closed => ("⏸", egui::Color32::GRAY),
                            };
                            ui.label(egui::RichText::new(status_text).color(color));

                            // Namespace
                            ui.label(&forward.namespace);

                            // Pod
                            ui.label(&forward.pod);

                            // Ports
                            ui.monospace(format!("{}→{}", forward.local_port, forward.remote_port));

                            // Auto Start checkbox
                            let mut auto_start = forward.auto_start;
                            if ui.checkbox(&mut auto_start, "").changed() {
                                forwards_to_update.push((
                                    forward.id.clone(),
                                    forward.local_port,
                                    forward.remote_port,
                                    auto_start,
                                ));
                            }

                            // Action buttons
                            ui.horizontal(|ui| match forward.status {
                                PortForwardStatus::Active => {
                                    if ui.small_button("🌐 Open").clicked() {
                                        actions.push(ViewAction::OpenUrl(format!(
                                            "http://localhost:{}",
                                            forward.local_port
                                        )));
                                    }
                                    if ui.small_button("⏸ Stop").clicked() {
                                        actions.push(ViewAction::SendCommand(
                                            BackendCmd::StopPortForward {
                                                tunnel_id: forward.id.clone(),
                                            },
                                        ));
                                    }
                                    if ui.small_button("✏ Edit").clicked() {
                                        actions.push(ViewAction::OpenModal(
                                            crate::app::ConfirmAction::EditPortForward {
                                                tunnel_id: forward.id.clone(),
                                                namespace: forward.namespace.clone(),
                                                pod: forward.pod.clone(),
                                                local_port: forward.local_port,
                                                remote_port: forward.remote_port,
                                                container: forward
                                                    .container
                                                    .clone()
                                                    .unwrap_or_default(),
                                            },
                                        ));
                                    }
                                    if ui.small_button("🗑 Delete").clicked() {
                                        actions.push(ViewAction::SendCommand(
                                            BackendCmd::DeletePortForward {
                                                tunnel_id: forward.id.clone(),
                                            },
                                        ));
                                    }
                                }
                                PortForwardStatus::Closed | PortForwardStatus::Error => {
                                    if ui.small_button("▶ Start").clicked() {
                                        actions.push(ViewAction::SendCommand(
                                            BackendCmd::StartPortForward {
                                                namespace: forward.namespace.clone(),
                                                pod: forward.pod.clone(),
                                                local_port: forward.local_port,
                                                remote_port: forward.remote_port,
                                                container: forward.container.clone(),
                                            },
                                        ));
                                    }
                                    if ui.small_button("✏ Edit").clicked() {
                                        actions.push(ViewAction::OpenModal(
                                            crate::app::ConfirmAction::EditPortForward {
                                                tunnel_id: forward.id.clone(),
                                                namespace: forward.namespace.clone(),
                                                pod: forward.pod.clone(),
                                                local_port: forward.local_port,
                                                remote_port: forward.remote_port,
                                                container: forward
                                                    .container
                                                    .clone()
                                                    .unwrap_or_default(),
                                            },
                                        ));
                                    }
                                    if ui.small_button("🗑 Delete").clicked() {
                                        actions.push(ViewAction::SendCommand(
                                            BackendCmd::DeletePortForward {
                                                tunnel_id: forward.id.clone(),
                                            },
                                        ));
                                    }
                                }
                                PortForwardStatus::Connecting => {
                                    ui.label("Connecting...");
                                }
                            });

                            ui.end_row();
                        }
                    });

                // Process auto_start updates
                for (tunnel_id, local_port, remote_port, auto_start) in forwards_to_update {
                    actions.push(ViewAction::SendCommand(BackendCmd::UpdatePortForward {
                        tunnel_id,
                        local_port,
                        remote_port,
                        auto_start,
                    }));
                }
            });
        });

        actions
    }

    fn render_details(
        &mut self,
        _ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        // No details panel for port forwards
        (false, vec![])
    }
}
