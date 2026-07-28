use super::super::components::Table;
use super::super::components::table::Column;
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{
    BackendCmd, BackendEvent, ClusterRoleBindingDetails, ClusterRoleBindingSummary,
};

/// ClusterRoleBinding-specific state and rendering for the ClusterRoleBindings view
pub struct ClusterRoleBindingsView {
    pub bindings: Vec<ClusterRoleBindingSummary>,
    pub selected: Option<String>,
    pub details: Option<ClusterRoleBindingDetails>,
    pub table: Table<ClusterRoleBindingSummary>,
}

impl Default for ClusterRoleBindingsView {
    fn default() -> Self {
        Self::new()
    }
}

impl ClusterRoleBindingsView {
    pub fn new() -> Self {
        Self {
            bindings: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_bindings_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<ClusterRoleBindingSummary>,
) -> Option<String> {
    ui.heading(title);
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("🔍 Search:");
        ui.text_edit_singleline(&mut table.state.search_text);
    });
    ui.add_space(8.0);

    let columns = vec![
        Column {
            header: "Name".to_string(),
            width: 200.0,
            sort_value: |b: &ClusterRoleBindingSummary| b.name.clone(),
            render: |b: &ClusterRoleBindingSummary| b.name.clone(),
        },
        Column {
            header: "Role Ref".to_string(),
            width: 220.0,
            sort_value: |b: &ClusterRoleBindingSummary| b.role_ref.clone(),
            render: |b: &ClusterRoleBindingSummary| b.role_ref.clone(),
        },
        Column {
            header: "Subjects".to_string(),
            width: 80.0,
            sort_value: |b: &ClusterRoleBindingSummary| b.subjects_count.to_string(),
            render: |b: &ClusterRoleBindingSummary| b.subjects_count.to_string(),
        },
    ];

    let mut newly_selected: Option<String> = None;

    table.render_table(
        ui,
        &columns,
        |b: &ClusterRoleBindingSummary, search: &str| b.name.to_lowercase().contains(search),
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|b: &ClusterRoleBindingSummary, search: &str| {
            b.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, b)) = filtered.get(selected_idx) {
            newly_selected = Some(b.name.clone());
        }
    }

    newly_selected
}

impl ViewComponent for ClusterRoleBindingsView {
    fn id(&self) -> &'static str {
        "access_clusterrolebindings"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.bindings.clone());

        let newly_selected = render_bindings_grid(ui, "Cluster Role Bindings", &mut self.table);

        if let Some(name) = newly_selected {
            self.selected = Some(name.clone());
            self.details = None;
            actions.push(ViewAction::SendCommand(
                BackendCmd::SelectClusterRoleBinding { name },
            ));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::ClusterRoleBindingsUpdated(bindings),
            ) => {
                self.bindings = bindings.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::ClusterRoleBindingDetailsUpdated(details),
            ) => {
                self.details = Some(details.clone());
            }
            _ => {}
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let name = self.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("Cluster Role Binding");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(
                        ConfirmAction::DeleteClusterRoleBinding(name.clone()),
                    ));
                }
                if ui.button("Close").clicked() {
                    self.selected = None;
                    self.details = None;
                }
            });
        });

        ui.label(&name);

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(details) = &self.details {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Overview");
                        ui.separator();
                        ui.monospace(format!("Role Ref: {}", details.role_ref));
                    });
                });
                ui.add_space(8.0);

                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("👥 Subjects");
                        ui.separator();
                        if details.subjects.is_empty() {
                            ui.label("(none)");
                        } else {
                            for subject in &details.subjects {
                                ui.monospace(format!(
                                    "{}: {}{}",
                                    subject.kind,
                                    subject
                                        .namespace
                                        .clone()
                                        .map(|n| format!("{}/", n))
                                        .unwrap_or_default(),
                                    subject.name
                                ));
                            }
                        }
                    });
                });
                ui.add_space(8.0);

                ui.collapsing("🏷️ Labels", |ui| {
                    if details.labels.is_empty() {
                        ui.label("(none)");
                    } else {
                        for (k, v) in &details.labels {
                            ui.monospace(format!("{}={}", k, v));
                        }
                    }
                });
                ui.collapsing("📝 Annotations", |ui| {
                    if details.annotations.is_empty() {
                        ui.label("(none)");
                    } else {
                        for (k, v) in &details.annotations {
                            ui.monospace(format!("{}={}", k, v));
                        }
                    }
                });
                ui.collapsing("Raw JSON", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(250.0)
                        .show(ui, |ui| {
                            ui.monospace(&details.raw_json);
                        });
                });
            } else {
                ui.label("Loading cluster role binding details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(
            BackendCmd::RefreshClusterRoleBindings,
        )]
    }
}
