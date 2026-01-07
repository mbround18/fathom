use super::*;

impl FathomApp {
    /// Render center panel using view component dispatch
    ///
    /// New view-based architecture:
    /// 1. Get or create view for active section
    /// 2. Render view in center panel
    /// 3. Process ViewActions returned by render
    pub(super) fn render_center(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Get the view for the active section
            let key = self.active_section.clone();
            let view = self.get_or_create_view(&key);

            // Render view and collect actions
            let actions = view.render(ui, ctx);

            // Process returned actions
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
                    ViewAction::OpenModal(_) => {
                        // Center view modals handled in right panel
                    }
                    ViewAction::OpenScaleModal(_, _, _, _) => {
                        // Center view scale modals handled in right panel
                    }
                    ViewAction::OpenUrl(url) => {
                        if let Err(e) = open::that(&url) {
                            tracing::warn!("Failed to open URL {}: {}", url, e);
                            self.push_status(format!("Failed to open URL: {}", e));
                        }
                    }
                }
            }
        });
    }
}
