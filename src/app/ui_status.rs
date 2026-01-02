use super::*;

impl FathomApp {
    /// Render status panel using view component
    ///
    /// Uses StatusView to manage and display the status log.
    pub(super) fn render_bottom_status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .resizable(true)
            .default_height(140.0)
            .show(ctx, |ui| {
                // Get the status view
                let status_view = self.get_or_create_view(&section_keys::status());

                // Render status and collect actions
                let actions = status_view.render(ui, ctx);

                // Process any actions (status view typically emits none)
                for action in actions {
                    match action {
                        ViewAction::SendCommand(cmd) => {
                            self.send(cmd);
                        }
                        ViewAction::NotifyStatus(msg) => {
                            self.push_status(msg);
                        }
                        ViewAction::Navigate(_) => {
                            // Status view shouldn't emit navigation
                        }
                    }
                }
            });
    }
}
