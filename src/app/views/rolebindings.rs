use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, RoleBindingDetails, RoleBindingSummary};

/// RoleBinding-specific state and rendering for the RoleBindings view
pub struct RoleBindingsView {
    pub bindings: Vec<RoleBindingSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<RoleBindingDetails>,
    pub table: Table<RoleBindingSummary>,
}

impl Default for RoleBindingsView {
    fn default() -> Self {
        Self::new()
    }
}

impl RoleBindingsView {
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
    table: &mut Table<RoleBindingSummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    let columns = vec![
        Column {
            header: "Namespace".to_string(),
            width: 120.0,
            sort_value: |b: &RoleBindingSummary| b.namespace.clone(),
            render: |b: &RoleBindingSummary| b.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |b: &RoleBindingSummary| b.name.clone(),
            render: |b: &RoleBindingSummary| b.name.clone(),
        },
        Column {
            header: "Role Ref".to_string(),
            width: 200.0,
            sort_value: |b: &RoleBindingSummary| b.role_ref.clone(),
            render: |b: &RoleBindingSummary| b.role_ref.clone(),
        },
        Column {
            header: "Subjects".to_string(),
            width: 80.0,
            sort_value: |b: &RoleBindingSummary| b.subjects_count.to_string(),
            render: |b: &RoleBindingSummary| b.subjects_count.to_string(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |b: &RoleBindingSummary, search: &str| {
            b.namespace.to_lowercase().contains(search) || b.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|b: &RoleBindingSummary, search: &str| {
            b.namespace.to_lowercase().contains(search) || b.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, b)) = filtered.get(selected_idx) {
            newly_selected = Some((b.namespace.clone(), b.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for RoleBindingsView {
    fn id(&self) -> &'static str {
        "access_rolebindings"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.bindings.clone());

        let newly_selected = render_bindings_grid(ui, "Role Bindings", &mut self.table);

        if let Some((ns, name)) = newly_selected
            && self.selected.as_ref() != Some(&(ns.clone(), name.clone()))
        {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectRoleBinding {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::RoleBindingsUpdated(bindings),
            ) => {
                self.bindings = bindings.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::RoleBindingDetailsUpdated(details),
            ) => {
                self.details = Some(details.clone());
            }
            _ => {}
        }
    }

    fn has_details(&self) -> bool {
        self.selected.is_some()
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
        let (ns, name) = self.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("Role Binding");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteRoleBinding(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                if ui.button("Close").clicked() {
                    self.selected = None;
                    self.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(&ns);
            ui.label(format!("/{}", name));
        });

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

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading role binding details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshRoleBindings)]
    }
}
