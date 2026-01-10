use super::*;

impl FathomApp {
    /// Render top bar with context and namespace selection
    pub(super) fn render_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Context selector
                ui.label("Context:");
                if ui
                    .button(self.current_context.as_deref().unwrap_or("(none)"))
                    .clicked()
                {
                    // Show context selector - basic implementation
                }

                ui.separator();

                // Namespace filter
                ui.label("Namespaces:");
                if ui
                    .checkbox(&mut self.select_all_namespaces, "All")
                    .changed()
                {
                    // Changed to select all
                }

                if !self.select_all_namespaces {
                    for ns in &self.namespaces {
                        let mut selected = self.selected_namespaces.contains(ns);
                        if ui.checkbox(&mut selected, ns).changed() {
                            if selected {
                                self.selected_namespaces.insert(ns.clone());
                            } else {
                                self.selected_namespaces.remove(ns);
                            }
                        }
                    }
                }
            });
        });
    }
}
