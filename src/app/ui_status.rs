use super::*;

impl FathomApp {
    /// Render status panel at the bottom of the window
    pub(super) fn render_bottom_status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .resizable(true)
            .default_height(140.0)
            .show(ctx, |ui| {
                ui.heading("Status Log");
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        for line in &self.status_log {
                            ui.monospace(line);
                        }
                    });
            });
    }
}
