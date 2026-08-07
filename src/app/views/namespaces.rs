/// Namespaces section: displays namespace list with status and resource counts.
use egui::{RichText, ScrollArea, Ui};

use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{BackendCmd, BackendEvent, NamespaceSummary};

#[derive(Debug, Clone, Default)]
pub struct NamespacesState {
    pub namespaces: Vec<NamespaceSummary>,
    pub selected_namespace: Option<String>,
}

pub struct NamespacesView {
    pub state: NamespacesState,
}

impl NamespacesView {
    pub fn new() -> Self {
        Self {
            state: NamespacesState::default(),
        }
    }
}

impl Default for NamespacesView {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewComponent for NamespacesView {
    fn id(&self) -> &'static str {
        "namespaces"
    }

    fn render(&mut self, ui: &mut Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        ui.heading("Namespaces");
        ui.separator();

        if self.state.namespaces.is_empty() {
            ui.label(RichText::new("No namespaces found").weak());
        } else {
            ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    egui::Grid::new("namespaces_grid")
                        .striped(true)
                        .min_col_width(100.0)
                        .show(ui, |ui| {
                            // Headers
                            ui.label(RichText::new("Name").strong());
                            ui.label(RichText::new("Status").strong());
                            ui.label(RichText::new("Pods").strong());
                            ui.label(RichText::new("Deployments").strong());
                            ui.end_row();

                            // Rows
                            for ns in &self.state.namespaces {
                                let is_selected = self
                                    .state
                                    .selected_namespace
                                    .as_ref()
                                    .is_some_and(|s| s == &ns.name);

                                if ui.selectable_label(is_selected, &ns.name).clicked() {
                                    self.state.selected_namespace = Some(ns.name.clone());
                                }
                                ui.label(&ns.status);
                                ui.label(ns.pod_count.to_string());
                                ui.label(ns.deployment_count.to_string());
                                ui.end_row();
                            }
                        });
                });
        }

        Vec::new()
    }

    fn on_event(&mut self, event: &BackendEvent) {
        use crate::protocol::ResourceListEvent;
        if let BackendEvent::ResourceList(ResourceListEvent::NamespaceSummariesUpdated(summaries)) =
            event
        {
            self.state.namespaces = summaries.clone();
        }
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshNamespaces)]
    }
}
