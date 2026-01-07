use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{BackendCmd, BackendEvent, ResourceListEvent};
use crate::sections::network::{PortForward, PortForwardStatus};

pub struct PortForwardsView {
    forwards: Vec<PortForward>,
}

impl PortForwardsView {
    pub fn new() -> Self {
        Self {
            forwards: Vec::new(),
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
        match event {
            BackendEvent::ResourceList(ResourceListEvent::PortForwardsUpdated(list)) => {
                self.forwards = list.clone();
            }
            _ => {}
        }
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        ui.heading("Port Forwards");
        ui.separator();

        if self.forwards.is_empty() {
            ui.label("No active port forwards.");
            ui.add_space(8.0);
            ui.label("💡 Tip: Click the ⚡ Forward button next to a port in the pod details panel to create a port forward.");
            return actions;
        }

        // Render simple list of active port forwards
        ui.label(format!("Active port forwards: {}", self.forwards.len()));
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for forward in &self.forwards {
                ui.horizontal(|ui| {
                    // Status indicator
                    let status_text = match forward.status {
                        PortForwardStatus::Connecting => "🔄",
                        PortForwardStatus::Active => "✅",
                        PortForwardStatus::Error => "❌",
                        PortForwardStatus::Closed => "⭕",
                    };
                    ui.label(status_text);

                    // Port forward info
                    ui.monospace(format!(
                        "localhost:{} → {}/{} (port {})",
                        forward.local_port, forward.namespace, forward.pod, forward.remote_port
                    ));

                    // Stop button
                    if matches!(
                        forward.status,
                        PortForwardStatus::Active | PortForwardStatus::Error | PortForwardStatus::Closed
                    ) {
                        if ui.small_button("🗑 Stop").clicked() {
                            actions.push(ViewAction::SendCommand(BackendCmd::StopPortForward {
                                tunnel_id: forward.id.clone(),
                            }));
                        }
                    }
                });
                ui.separator();
            }
        });

        actions
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        // No details panel for port forwards
        (false, vec![])
    }
}
