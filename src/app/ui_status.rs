use super::*;

impl FathomApp {
    pub(super) fn render_bottom_status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .resizable(true)
            .default_height(140.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Status Log");
                    if ui.button("Clear").clicked() {
                        self.status_log.clear();
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for line in &self.status_log {
                        ui.monospace(line);
                    }
                });
            });
    }
}
