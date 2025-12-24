use super::*;

impl FathomApp {
    pub(super) fn render_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Fathom");

                ui.add_space(12.0);

                ui.label("Context:");
                egui::ComboBox::from_id_salt("context_combo")
                    .selected_text(self.context_combo_text())
                    .show_ui(ui, |ui| {
                        if self.contexts.is_empty() {
                            ui.label("(no kubeconfig contexts found)");
                            return;
                        }

                        for ctx_name in self.contexts.clone() {
                            let selected =
                                self.current_context.as_deref() == Some(ctx_name.as_str());
                            if ui.selectable_label(selected, &ctx_name).clicked() {
                                // Reset selection state when switching clusters.
                                self.selected_pod = None;
                                self.pod_details = None;
                                self.pod_metrics = None;
                                self.pod_logs.clear();

                                self.select_all_namespaces = true;
                                self.selected_namespaces.clear();

                                self.send(BackendCmd::SetContext { context: ctx_name });
                                // Backend will refresh namespaces/pods and restart sync.
                            }
                        }
                    });

                if ui.button("Refresh").clicked() {
                    self.send(BackendCmd::RefreshContexts);
                    self.send(BackendCmd::RefreshNamespaces);
                    self.send(BackendCmd::RefreshPods);
                    self.send(BackendCmd::RefreshDeployments);
                }

                ui.add_space(12.0);
                ui.separator();
                ui.label("Namespaces:");

                egui::ComboBox::from_id_salt("namespace_combo")
                    .selected_text(self.namespace_combo_text())
                    .show_ui(ui, |ui| {
                        let enabled_for_section = self.active_scope_kind() == ScopeKind::Namespaced;
                        let mut all = self.select_all_namespaces;
                        if ui
                            .add_enabled(enabled_for_section, egui::Checkbox::new(&mut all, "All"))
                            .changed()
                        {
                            self.select_all_namespaces = all;
                            if all {
                                self.selected_namespaces.clear();
                            }
                            // Wire-up: immediately apply the filter to backend requests.
                            self.send(BackendCmd::SetNamespaceFilter {
                                namespaces: self.effective_namespace_filter(),
                            });
                        }

                        ui.separator();

                        let enabled = enabled_for_section && !self.select_all_namespaces;
                        let namespaces = self.namespaces.clone();
                        let mut changed = false;

                        for ns in namespaces {
                            let mut selected = self.selected_namespaces.contains(&ns);
                            let response =
                                ui.add_enabled(enabled, egui::Checkbox::new(&mut selected, &ns));
                            if enabled && response.changed() {
                                changed = true;
                                if selected {
                                    self.selected_namespaces.insert(ns);
                                } else {
                                    self.selected_namespaces.remove(&ns);
                                }
                            }
                        }

                        if changed {
                            self.send(BackendCmd::SetNamespaceFilter {
                                namespaces: self.effective_namespace_filter(),
                            });
                        }
                    });
            });
        });
    }

    fn context_combo_text(&self) -> String {
        if let Some(ctx) = &self.current_context {
            return ctx.clone();
        }
        if self.contexts.is_empty() {
            return "(default)".to_owned();
        }
        "(select)".to_owned()
    }

    fn namespace_combo_text(&self) -> String {
        if self.select_all_namespaces {
            return "All".to_owned();
        }
        if self.selected_namespaces.is_empty() {
            return "(none)".to_owned();
        }
        if self.selected_namespaces.len() == 1 {
            return self
                .selected_namespaces
                .iter()
                .next()
                .cloned()
                .unwrap_or_else(|| "(none)".to_owned());
        }
        format!("{} selected", self.selected_namespaces.len())
    }
}
