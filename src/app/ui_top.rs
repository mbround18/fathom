use super::*;

impl FathomApp {
    /// Render top bar using view component
    ///
    /// Uses ContextView to manage context and namespace selection.
    pub(super) fn render_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Get the context view
                let context_view = self.get_or_create_view(&section_keys::context());

                // Render context and collect actions
                let actions = context_view.render(ui, ctx);

                // Add namespace filter
                ui.separator();
                ui.label("Filter by namespace:");

                // Create a copy of namespaces for iteration
                let available_ns = self.all_namespaces.clone();

                for namespace in available_ns {
                    let is_selected = self.namespace_matches_filter(&namespace)
                        || self.selected_namespaces.is_empty();
                    let mut checkbox = is_selected && !self.selected_namespaces.is_empty();

                    // If no filter selected, show all as selected but don't check the box
                    if self.selected_namespaces.is_empty() {
                        if ui.selectable_label(false, &namespace).clicked() {
                            self.toggle_namespace_selection(&namespace);
                        }
                    } else if ui
                        .selectable_label(self.selected_namespaces.contains(&namespace), &namespace)
                        .clicked()
                    {
                        self.toggle_namespace_selection(&namespace);
                    }
                }

                // "Show All" button
                if !self.selected_namespaces.is_empty() {
                    if ui.button("Show All").clicked() {
                        self.selected_namespaces.clear();
                    }
                }

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
        });
    }
}
