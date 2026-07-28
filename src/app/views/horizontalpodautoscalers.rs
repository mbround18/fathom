use super::super::components::Table;
use super::super::components::table::Column;
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{
    BackendCmd, BackendEvent, HorizontalPodAutoscalerDetails, HorizontalPodAutoscalerSummary,
};

/// HorizontalPodAutoscaler-specific state and rendering for the HorizontalPodAutoscalers view
pub struct HorizontalPodAutoscalersView {
    pub hpas: Vec<HorizontalPodAutoscalerSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<HorizontalPodAutoscalerDetails>,
    pub table: Table<HorizontalPodAutoscalerSummary>,
}

impl Default for HorizontalPodAutoscalersView {
    fn default() -> Self {
        Self::new()
    }
}

impl HorizontalPodAutoscalersView {
    pub fn new() -> Self {
        Self {
            hpas: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn opt_i32(v: Option<i32>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string())
}

fn render_hpas_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<HorizontalPodAutoscalerSummary>,
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
            sort_value: |h: &HorizontalPodAutoscalerSummary| h.namespace.clone(),
            render: |h: &HorizontalPodAutoscalerSummary| h.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |h: &HorizontalPodAutoscalerSummary| h.name.clone(),
            render: |h: &HorizontalPodAutoscalerSummary| h.name.clone(),
        },
        Column {
            header: "Target".to_string(),
            width: 200.0,
            sort_value: |h: &HorizontalPodAutoscalerSummary| h.target_ref.clone(),
            render: |h: &HorizontalPodAutoscalerSummary| h.target_ref.clone(),
        },
        Column {
            header: "Min".to_string(),
            width: 60.0,
            sort_value: |h: &HorizontalPodAutoscalerSummary| opt_i32(h.min_replicas),
            render: |h: &HorizontalPodAutoscalerSummary| opt_i32(h.min_replicas),
        },
        Column {
            header: "Max".to_string(),
            width: 60.0,
            sort_value: |h: &HorizontalPodAutoscalerSummary| h.max_replicas.to_string(),
            render: |h: &HorizontalPodAutoscalerSummary| h.max_replicas.to_string(),
        },
        Column {
            header: "Current".to_string(),
            width: 80.0,
            sort_value: |h: &HorizontalPodAutoscalerSummary| opt_i32(h.current_replicas),
            render: |h: &HorizontalPodAutoscalerSummary| opt_i32(h.current_replicas),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |h: &HorizontalPodAutoscalerSummary, search: &str| {
            h.namespace.to_lowercase().contains(search) || h.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|h: &HorizontalPodAutoscalerSummary, search: &str| {
            h.namespace.to_lowercase().contains(search) || h.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, h)) = filtered.get(selected_idx) {
            newly_selected = Some((h.namespace.clone(), h.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for HorizontalPodAutoscalersView {
    fn id(&self) -> &'static str {
        "config_horizontalpodautoscalers"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.hpas.clone());

        let newly_selected =
            render_hpas_grid(ui, "Horizontal Pod Autoscalers", &mut self.table);

        if let Some((ns, name)) = newly_selected {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(
                BackendCmd::SelectHorizontalPodAutoscaler {
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
                crate::protocol::ResourceListEvent::HorizontalPodAutoscalersUpdated(hpas),
            ) => {
                self.hpas = hpas.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::HorizontalPodAutoscalerDetailsUpdated(
                    details,
                ),
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
            ui.heading("Horizontal Pod Autoscaler");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(
                        ConfirmAction::DeleteHorizontalPodAutoscaler(ns.clone(), name.clone()),
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
                        ui.monospace(format!("Target: {}", details.target_ref));
                        ui.monospace(format!(
                            "Min: {}  Max: {}",
                            opt_i32(details.min_replicas),
                            details.max_replicas
                        ));
                        ui.monospace(format!(
                            "Current: {}  Desired: {}",
                            opt_i32(details.current_replicas),
                            details.desired_replicas
                        ));
                    });
                });
                ui.add_space(8.0);

                if !details.conditions.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("⚙️ Conditions");
                            ui.separator();
                            for c in &details.conditions {
                                ui.monospace(c);
                            }
                        });
                    });
                    ui.add_space(8.0);
                }

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
                ui.label("Loading horizontal pod autoscaler details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(
            BackendCmd::RefreshHorizontalPodAutoscalers,
        )]
    }
}
