use super::*;

impl FathomApp {
    /// Render right panel (details/tabs)
    ///
    /// Delegates to the active view's render_details() method.
    pub(super) fn render_right_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("right_panel")
            .default_width(400.0)
            .show(ctx, |ui| {
                // Get the active view and render its details panel
                let key = self.active_section.clone();
                let view = self.get_or_create_view(&key);

                let (has_content, actions) = view.render_details(ui, ctx);

                // Show placeholder if view has no content to display
                if !has_content {
                    ui.heading("Details");
                    ui.separator();
                    ui.label("Select a resource to view details.");
                }

                // Process actions returned by details panel
                for action in actions {
                    match action {
                        ViewAction::SendCommand(cmd) => {
                            self.send(cmd);
                        }
                        ViewAction::NotifyStatus(msg) => {
                            self.push_status(msg);
                        }
                        ViewAction::Navigate(section) => {
                            self.active_section = section;
                        }
                    }
                }
            });
    }
}
