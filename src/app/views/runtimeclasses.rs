use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, RuntimeClassDetails, RuntimeClassSummary};

/// RuntimeClass-specific state and rendering for the RuntimeClasses view
pub struct RuntimeClassesView {
    pub classes: Vec<RuntimeClassSummary>,
    pub selected: Option<String>,
    pub details: Option<RuntimeClassDetails>,
    pub table: Table<RuntimeClassSummary>,
}

impl Default for RuntimeClassesView {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeClassesView {
    pub fn new() -> Self {
        Self {
            classes: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_classes_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<RuntimeClassSummary>,
) -> Option<String> {
    ui.heading(title);
    ui.separator();

    let columns = vec![
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |c: &RuntimeClassSummary| c.name.clone(),
            render: |c: &RuntimeClassSummary| c.name.clone(),
        },
        Column {
            header: "Handler".to_string(),
            width: 200.0,
            sort_value: |c: &RuntimeClassSummary| c.handler.clone(),
            render: |c: &RuntimeClassSummary| c.handler.clone(),
        },
    ];

    let mut newly_selected: Option<String> = None;

    table.render_table(
        ui,
        &columns,
        |c: &RuntimeClassSummary, search: &str| c.name.to_lowercase().contains(search),
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|c: &RuntimeClassSummary, search: &str| {
            c.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, c)) = filtered.get(selected_idx) {
            newly_selected = Some(c.name.clone());
        }
    }

    newly_selected
}

impl ViewComponent for RuntimeClassesView {
    fn id(&self) -> &'static str {
        "config_runtimeclasses"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.classes.clone());

        let newly_selected = render_classes_grid(ui, "Runtime Classes", &mut self.table);

        if let Some(name) = newly_selected
            && self.selected.as_ref() != Some(&name)
        {
            self.selected = Some(name.clone());
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectRuntimeClass {
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::RuntimeClassesUpdated(classes),
            ) => {
                self.classes = classes.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::RuntimeClassDetailsUpdated(details),
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
        let name = self.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("Runtime Class");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteRuntimeClass(
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
                        ui.monospace(format!("Handler: {}", details.handler));
                        if let Some(scheduling) = &details.scheduling {
                            ui.monospace(format!("Scheduling: {}", scheduling));
                        }
                    });
                });
                ui.add_space(8.0);

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading runtime class details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshRuntimeClasses)]
    }
}
