use super::*;

impl FathomApp {
    /// Render left navigation panel using view component
    ///
    /// Uses NavigationView to display sections. Processes Navigate actions
    /// to update active section.
    pub(super) fn render_left_nav(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("left_nav")
            .resizable(false)
            .default_width(170.0)
            .show(ctx, |ui| {
                // Get the navigation view (always the same instance)
                let nav_view = self.get_or_create_view(&section_keys::navigation());

                // Render navigation and collect actions
                let actions = nav_view.render(ui, ctx);

                // Process returned actions
                for action in actions {
                    match action {
                        ViewAction::Navigate(section) => {
                            self.active_section = section;
                            self.send(BackendCmd::SetActiveSection {
                                section: self.active_section.clone(),
                            });
                        }
                        _ => {
                            // Navigation view only emits Navigate actions
                        }
                    }
                }

                ui.separator();
                if ui.button("Simulate Slow Call").clicked() {
                    self.send(BackendCmd::SimulateSlowCall);
                }
            });
    }
}
