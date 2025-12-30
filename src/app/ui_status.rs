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
                    if ui.button("Copy").clicked() {
                        let text = self.status_log.join("\n");
                        ctx.output_mut(|o| {
                            o.commands.push(egui::output::OutputCommand::CopyText(text));
                        });
                    }
                    let toggle = ui.selectable_label(self.status_auto_scroll, "Auto-scroll");
                    if toggle.clicked() {
                        self.status_auto_scroll = !self.status_auto_scroll;
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .stick_to_bottom(self.status_auto_scroll)
                    .show(ui, |ui| {
                        for line in &self.status_log {
                            ui.monospace(line);
                        }
                    });
            });
    }
}
