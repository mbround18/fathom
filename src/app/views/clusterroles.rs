use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, ClusterRoleDetails, ClusterRoleSummary};

/// ClusterRole-specific state and rendering for the ClusterRoles view
pub struct ClusterRolesView {
    pub roles: Vec<ClusterRoleSummary>,
    pub selected: Option<String>,
    pub details: Option<ClusterRoleDetails>,
    pub table: Table<ClusterRoleSummary>,
}

impl Default for ClusterRolesView {
    fn default() -> Self {
        Self::new()
    }
}

impl ClusterRolesView {
    pub fn new() -> Self {
        Self {
            roles: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_cluster_roles_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<ClusterRoleSummary>,
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
            width: 220.0,
            sort_value: |c: &ClusterRoleSummary| c.name.clone(),
            render: |c: &ClusterRoleSummary| c.name.clone(),
        },
        Column {
            header: "Rules".to_string(),
            width: 80.0,
            sort_value: |c: &ClusterRoleSummary| c.rules_count.to_string(),
            render: |c: &ClusterRoleSummary| c.rules_count.to_string(),
        },
    ];

    let mut newly_selected: Option<String> = None;

    table.render_table(
        ui,
        &columns,
        |c: &ClusterRoleSummary, search: &str| c.name.to_lowercase().contains(search),
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|c: &ClusterRoleSummary, search: &str| {
            c.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, c)) = filtered.get(selected_idx) {
            newly_selected = Some(c.name.clone());
        }
    }

    newly_selected
}

impl ViewComponent for ClusterRolesView {
    fn id(&self) -> &'static str {
        "access_clusterroles"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.roles.clone());

        let newly_selected = render_cluster_roles_grid(ui, "Cluster Roles", &mut self.table);

        if let Some(name) = newly_selected {
            self.selected = Some(name.clone());
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectClusterRole {
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::ClusterRolesUpdated(roles),
            ) => {
                self.roles = roles.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::ClusterRoleDetailsUpdated(details),
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
            ui.heading("Cluster Role");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteClusterRole(
                        name.clone(),
                    )));
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

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading cluster role details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshClusterRoles)]
    }
}
