use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{BackendCmd, BackendEvent};
use std::collections::BTreeSet;

/// Context selector view - displays the top bar context and namespace selection
pub struct ContextView {
    pub contexts: Vec<String>,
    pub current_context: Option<String>,
    pub namespaces: Vec<String>,
    pub selected_namespaces: BTreeSet<String>,
    pub select_all_namespaces: bool,
}

impl Default for ContextView {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextView {
    pub fn new() -> Self {
        Self {
            contexts: Vec::new(),
            current_context: None,
            namespaces: Vec::new(),
            selected_namespaces: BTreeSet::new(),
            select_all_namespaces: true,
        }
    }

    pub fn context_combo_text(&self) -> String {
        self.current_context
            .as_deref()
            .unwrap_or("(no context)")
            .to_string()
    }

    pub fn namespace_combo_text(&self) -> String {
        if self.select_all_namespaces {
            "All".to_string()
        } else if self.selected_namespaces.is_empty() {
            "(none selected)".to_string()
        } else if self.selected_namespaces.len() == 1 {
            self.selected_namespaces.iter().next().unwrap().clone()
        } else {
            format!("{} namespaces", self.selected_namespaces.len())
        }
    }

    pub fn effective_namespace_filter(&self) -> Vec<String> {
        if self.select_all_namespaces {
            Vec::new() // empty = all namespaces
        } else {
            self.selected_namespaces.iter().cloned().collect()
        }
    }
}

impl ViewComponent for ContextView {
    fn id(&self) -> &'static str {
        "context"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

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
                        let selected = self.current_context.as_deref() == Some(ctx_name.as_str());
                        if ui.selectable_label(selected, &ctx_name).clicked() {
                            self.current_context = Some(ctx_name.clone());
                            self.select_all_namespaces = true;
                            self.selected_namespaces.clear();

                            actions.push(ViewAction::SendCommand(BackendCmd::SetContext {
                                context: ctx_name,
                            }));
                        }
                    }
                });

            if ui.button("Refresh").clicked() {
                actions.push(ViewAction::SendCommand(BackendCmd::RefreshContexts));
                actions.push(ViewAction::SendCommand(BackendCmd::RefreshNamespaces));
                actions.push(ViewAction::SendCommand(BackendCmd::RefreshPods));
                actions.push(ViewAction::SendCommand(BackendCmd::RefreshDeployments));
            }

            ui.add_space(12.0);
            ui.separator();
            ui.label("Namespaces:");

            egui::ComboBox::from_id_salt("namespace_combo")
                .selected_text(self.namespace_combo_text())
                .show_ui(ui, |ui| {
                    let mut all = self.select_all_namespaces;
                    if ui
                        .add_enabled(true, egui::Checkbox::new(&mut all, "All"))
                        .changed()
                    {
                        self.select_all_namespaces = all;
                        if all {
                            self.selected_namespaces.clear();
                            actions.push(ViewAction::SendCommand(BackendCmd::SetNamespaceFilter {
                                namespaces: self.effective_namespace_filter(),
                            }));
                        }
                    }

                    ui.separator();

                    let namespaces = self.namespaces.clone();
                    for ns in namespaces {
                        let mut selected = self.selected_namespaces.contains(&ns);
                        if ui
                            .add_enabled(true, egui::Checkbox::new(&mut selected, &ns))
                            .changed()
                        {
                            if selected {
                                self.selected_namespaces.insert(ns.clone());
                                self.select_all_namespaces = false;
                            } else {
                                self.selected_namespaces.remove(&ns);
                            }
                            actions.push(ViewAction::SendCommand(BackendCmd::SetNamespaceFilter {
                                namespaces: self.effective_namespace_filter(),
                            }));
                        }
                    }
                });
        });

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        use crate::protocol::{BackendEvent, ResourceListEvent, SystemEvent};

        if let BackendEvent::System(SystemEvent::ContextsUpdated { contexts, current }) = event {
            self.contexts = contexts.clone();
            self.current_context = current.clone();
        }

        if let BackendEvent::System(SystemEvent::ContextChanged { current }) = event {
            self.current_context = current.clone();
            self.select_all_namespaces = true;
            self.selected_namespaces.clear();
        }

        if let BackendEvent::System(SystemEvent::NamespaceFilterChanged { namespaces }) = event {
            if namespaces.is_empty() {
                self.select_all_namespaces = true;
                self.selected_namespaces.clear();
            } else {
                self.select_all_namespaces = false;
                self.selected_namespaces = namespaces.iter().cloned().collect();
            }
        }

        if let BackendEvent::ResourceList(ResourceListEvent::NamespacesUpdated(namespaces)) = event
        {
            self.namespaces = namespaces.clone();
        }
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        Vec::new()
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        Vec::new()
    }
}
