use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, PriorityClassDetails, PriorityClassSummary};

/// PriorityClass-specific state and rendering for the PriorityClasses view
pub struct PriorityClassesView {
    pub classes: Vec<PriorityClassSummary>,
    pub selected: Option<String>,
    pub details: Option<PriorityClassDetails>,
    pub table: Table<PriorityClassSummary>,
}

impl Default for PriorityClassesView {
    fn default() -> Self {
        Self::new()
    }
}

impl PriorityClassesView {
    pub fn new() -> Self {
        Self {
            classes: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn opt_str(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "-".to_string())
}

fn render_classes_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<PriorityClassSummary>,
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
            width: 180.0,
            sort_value: |c: &PriorityClassSummary| c.name.clone(),
            render: |c: &PriorityClassSummary| c.name.clone(),
        },
        Column {
            header: "Value".to_string(),
            width: 100.0,
            sort_value: |c: &PriorityClassSummary| format!("{:010}", c.value),
            render: |c: &PriorityClassSummary| c.value.to_string(),
        },
        Column {
            header: "Global Default".to_string(),
            width: 110.0,
            sort_value: |c: &PriorityClassSummary| c.global_default.to_string(),
            render: |c: &PriorityClassSummary| c.global_default.to_string(),
        },
        Column {
            header: "Description".to_string(),
            width: 250.0,
            sort_value: |c: &PriorityClassSummary| opt_str(&c.description),
            render: |c: &PriorityClassSummary| opt_str(&c.description),
        },
    ];

    let mut newly_selected: Option<String> = None;

    table.render_table(
        ui,
        &columns,
        |c: &PriorityClassSummary, search: &str| c.name.to_lowercase().contains(search),
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|c: &PriorityClassSummary, search: &str| {
            c.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, c)) = filtered.get(selected_idx) {
            newly_selected = Some(c.name.clone());
        }
    }

    newly_selected
}

impl ViewComponent for PriorityClassesView {
    fn id(&self) -> &'static str {
        "config_priorityclasses"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.classes.clone());

        let newly_selected = render_classes_grid(ui, "Priority Classes", &mut self.table);

        if let Some(name) = newly_selected {
            self.selected = Some(name.clone());
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectPriorityClass {
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::PriorityClassesUpdated(classes),
            ) => {
                self.classes = classes.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::PriorityClassDetailsUpdated(details),
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
            ui.heading("Priority Class");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeletePriorityClass(
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
                        ui.label("📊 Overview");
                        ui.separator();
                        ui.monospace(format!("Value: {}", details.value));
                        ui.monospace(format!("Global Default: {}", details.global_default));
                        ui.monospace(format!("Description: {}", opt_str(&details.description)));
                        ui.monospace(format!(
                            "Preemption Policy: {}",
                            opt_str(&details.preemption_policy)
                        ));
                    });
                });
                ui.add_space(8.0);

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading priority class details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshPriorityClasses)]
    }
}
