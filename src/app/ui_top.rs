use super::*;

impl FathomApp {
    /// Render top bar using view component
    ///
    /// Uses ContextView to manage context and namespace selection.
    pub(super) fn render_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            // Get the context view
            let context_view = self.get_or_create_view(&section_keys::context());

            // Render context and collect actions
            let actions = context_view.render(ui, ctx);

            // Process returned actions
            for action in actions {
                match action {
                    ViewAction::SendCommand(cmd) => {
                        self.send(cmd);
                    }
                    ViewAction::NotifyStatus(msg) => {
                        self.push_status(msg);
                    }
                    ViewAction::Navigate(_) => {
                        // Context view shouldn't emit navigation
                    }
                    ViewAction::OpenModal(_) => {
                        // Context view shouldn't emit modals
                    }
                    ViewAction::OpenScaleModal(_, _, _, _) => {
                        // Context view shouldn't emit scale modals
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
