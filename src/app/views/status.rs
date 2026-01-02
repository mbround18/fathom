use super::traits::{ViewAction, ViewComponent};
use crate::protocol::BackendEvent;

/// Status log view - displays the bottom status panel
///
/// Manages the status log and auto-scroll setting. Other components
/// push messages to this view via FathomApp's helper methods.
pub struct StatusView {
    pub log: Vec<String>,
    pub auto_scroll: bool,
}

impl StatusView {
    pub fn new() -> Self {
        Self {
            log: vec!["System Ready.".to_owned()],
            auto_scroll: true,
        }
    }

    pub fn push(&mut self, msg: impl Into<String>) {
        self.log.push(format!("> {}", msg.into()));
        if self.log.len() > 500 {
            let drain = self.log.len() - 500;
            self.log.drain(0..drain);
        }
    }
}

impl ViewComponent for StatusView {
    fn id(&self) -> &'static str {
        "status"
    }

    fn render(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> Vec<ViewAction> {
        ui.horizontal(|ui| {
            ui.label("Status Log");
            if ui.button("Clear").clicked() {
                self.log.clear();
            }
            if ui.button("Copy").clicked() {
                let text = self.log.join("\n");
                ctx.output_mut(|o| {
                    o.commands.push(egui::output::OutputCommand::CopyText(text));
                });
            }
            let toggle = ui.selectable_label(self.auto_scroll, "Auto-scroll");
            if toggle.clicked() {
                self.auto_scroll = !self.auto_scroll;
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .stick_to_bottom(self.auto_scroll)
            .show(ui, |ui| {
                for line in &self.log {
                    ui.monospace(line);
                }
            });

        Vec::new()
    }

    fn on_event(&mut self, _event: &BackendEvent) {
        // Status view doesn't respond to backend events
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        Vec::new()
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        Vec::new()
    }
}
