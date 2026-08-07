use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{
    BackendCmd, BackendEvent, PodDisruptionBudgetDetails, PodDisruptionBudgetSummary,
};

/// PodDisruptionBudget-specific state and rendering for the PodDisruptionBudgets view
pub struct PodDisruptionBudgetsView {
    pub pdbs: Vec<PodDisruptionBudgetSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<PodDisruptionBudgetDetails>,
    pub table: Table<PodDisruptionBudgetSummary>,
}

impl Default for PodDisruptionBudgetsView {
    fn default() -> Self {
        Self::new()
    }
}

impl PodDisruptionBudgetsView {
    pub fn new() -> Self {
        Self {
            pdbs: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn opt_str(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "-".to_string())
}

fn opt_i32(v: Option<i32>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string())
}

fn render_pdbs_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<PodDisruptionBudgetSummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    let columns = vec![
        Column {
            header: "Namespace".to_string(),
            width: 120.0,
            sort_value: |p: &PodDisruptionBudgetSummary| p.namespace.clone(),
            render: |p: &PodDisruptionBudgetSummary| p.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |p: &PodDisruptionBudgetSummary| p.name.clone(),
            render: |p: &PodDisruptionBudgetSummary| p.name.clone(),
        },
        Column {
            header: "Min Available".to_string(),
            width: 110.0,
            sort_value: |p: &PodDisruptionBudgetSummary| opt_str(&p.min_available),
            render: |p: &PodDisruptionBudgetSummary| opt_str(&p.min_available),
        },
        Column {
            header: "Max Unavailable".to_string(),
            width: 120.0,
            sort_value: |p: &PodDisruptionBudgetSummary| opt_str(&p.max_unavailable),
            render: |p: &PodDisruptionBudgetSummary| opt_str(&p.max_unavailable),
        },
        Column {
            header: "Current Healthy".to_string(),
            width: 120.0,
            sort_value: |p: &PodDisruptionBudgetSummary| opt_i32(p.current_healthy),
            render: |p: &PodDisruptionBudgetSummary| opt_i32(p.current_healthy),
        },
        Column {
            header: "Desired Healthy".to_string(),
            width: 120.0,
            sort_value: |p: &PodDisruptionBudgetSummary| opt_i32(p.desired_healthy),
            render: |p: &PodDisruptionBudgetSummary| opt_i32(p.desired_healthy),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |p: &PodDisruptionBudgetSummary, search: &str| {
            p.namespace.to_lowercase().contains(search) || p.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|p: &PodDisruptionBudgetSummary, search: &str| {
            p.namespace.to_lowercase().contains(search) || p.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, p)) = filtered.get(selected_idx) {
            newly_selected = Some((p.namespace.clone(), p.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for PodDisruptionBudgetsView {
    fn id(&self) -> &'static str {
        "config_poddisruptionbudgets"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.pdbs.clone());

        let newly_selected = render_pdbs_grid(ui, "Pod Disruption Budgets", &mut self.table);

        if let Some((ns, name)) = newly_selected
            && self.selected.as_ref() != Some(&(ns.clone(), name.clone()))
        {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(
                BackendCmd::SelectPodDisruptionBudget {
                    namespace: ns,
                    name,
                },
            ));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::PodDisruptionBudgetsUpdated(pdbs),
            ) => {
                self.pdbs = pdbs.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::PodDisruptionBudgetDetailsUpdated(details),
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
            ui.heading("Pod Disruption Budget");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(
                        ConfirmAction::DeletePodDisruptionBudget(ns.clone(), name.clone()),
                    ));
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
                        ui.monospace(format!(
                            "Min Available: {}  Max Unavailable: {}",
                            opt_str(&details.min_available),
                            opt_str(&details.max_unavailable)
                        ));
                        ui.monospace(format!(
                            "Current Healthy: {}  Desired Healthy: {}",
                            opt_i32(details.current_healthy),
                            opt_i32(details.desired_healthy)
                        ));
                        ui.monospace(format!(
                            "Disruptions Allowed: {}  Expected Pods: {}",
                            opt_i32(details.disruptions_allowed),
                            opt_i32(details.expected_pods)
                        ));
                    });
                });
                ui.add_space(8.0);

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading pod disruption budget details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(
            BackendCmd::RefreshPodDisruptionBudgets,
        )]
    }
}
