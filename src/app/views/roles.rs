use super::super::components::Table;
use super::super::components::table::Column;
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, RoleDetails, RoleSummary};

/// Role-specific state and rendering for the Roles view
pub struct RolesView {
    pub roles: Vec<RoleSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<RoleDetails>,
    pub table: Table<RoleSummary>,
}

impl Default for RolesView {
    fn default() -> Self {
        Self::new()
    }
}

impl RolesView {
    pub fn new() -> Self {
        Self {
            roles: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_roles_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<RoleSummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("🔍 Search:");
        ui.text_edit_singleline(&mut table.state.search_text);
    });
    ui.add_space(8.0);

    let columns = vec![
        Column {
            header: "Namespace".to_string(),
            width: 120.0,
            sort_value: |r: &RoleSummary| r.namespace.clone(),
            render: |r: &RoleSummary| r.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |r: &RoleSummary| r.name.clone(),
            render: |r: &RoleSummary| r.name.clone(),
        },
        Column {
            header: "Rules".to_string(),
            width: 80.0,
            sort_value: |r: &RoleSummary| r.rules_count.to_string(),
            render: |r: &RoleSummary| r.rules_count.to_string(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |r: &RoleSummary, search: &str| {
            r.namespace.to_lowercase().contains(search) || r.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|r: &RoleSummary, search: &str| {
            r.namespace.to_lowercase().contains(search) || r.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, r)) = filtered.get(selected_idx) {
            newly_selected = Some((r.namespace.clone(), r.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for RolesView {
    fn id(&self) -> &'static str {
        "access_roles"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.roles.clone());

        let newly_selected = render_roles_grid(ui, "Roles", &mut self.table);

        if let Some((ns, name)) = newly_selected {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectRole {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(crate::protocol::ResourceListEvent::RolesUpdated(
                roles,
            )) => {
                self.roles = roles.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::RoleDetailsUpdated(details),
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
        let (ns, name) = self.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("Role");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteRole(
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
                        ui.label("📊 Rules");
                        ui.separator();
                        if details.rules.is_empty() {
                            ui.label("(none)");
                        } else {
                            for rule in &details.rules {
                                ui.monospace(format!(
                                    "apiGroups: {}  resources: {}  verbs: {}",
                                    if rule.api_groups.is_empty() {
                                        "*".to_string()
                                    } else {
                                        rule.api_groups.join(", ")
                                    },
                                    rule.resources.join(", "),
                                    rule.verbs.join(", ")
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
                ui.label("Loading role details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshRoles)]
    }
}
