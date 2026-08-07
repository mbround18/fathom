use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, StorageClassDetails, StorageClassSummary};

/// StorageClass-specific state and rendering for the StorageClasses view
pub struct StorageClassesView {
    pub classes: Vec<StorageClassSummary>,
    pub selected: Option<String>,
    pub details: Option<StorageClassDetails>,
    pub table: Table<StorageClassSummary>,
}

impl Default for StorageClassesView {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageClassesView {
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
    table: &mut Table<StorageClassSummary>,
) -> Option<String> {
    ui.heading(title);
    ui.separator();

    let columns = vec![
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |c: &StorageClassSummary| c.name.clone(),
            render: |c: &StorageClassSummary| {
                if c.is_default {
                    format!("{} (default)", c.name)
                } else {
                    c.name.clone()
                }
            },
        },
        Column {
            header: "Provisioner".to_string(),
            width: 220.0,
            sort_value: |c: &StorageClassSummary| c.provisioner.clone(),
            render: |c: &StorageClassSummary| c.provisioner.clone(),
        },
        Column {
            header: "Reclaim Policy".to_string(),
            width: 110.0,
            sort_value: |c: &StorageClassSummary| c.reclaim_policy.clone().unwrap_or_default(),
            render: |c: &StorageClassSummary| c.reclaim_policy.clone().unwrap_or_default(),
        },
        Column {
            header: "Volume Binding Mode".to_string(),
            width: 160.0,
            sort_value: |c: &StorageClassSummary| c.volume_binding_mode.clone().unwrap_or_default(),
            render: |c: &StorageClassSummary| c.volume_binding_mode.clone().unwrap_or_default(),
        },
    ];

    let mut newly_selected: Option<String> = None;

    table.render_table(
        ui,
        &columns,
        |c: &StorageClassSummary, search: &str| c.name.to_lowercase().contains(search),
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|c: &StorageClassSummary, search: &str| {
            c.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, c)) = filtered.get(selected_idx) {
            newly_selected = Some(c.name.clone());
        }
    }

    newly_selected
}

impl ViewComponent for StorageClassesView {
    fn id(&self) -> &'static str {
        "storage_storageclasses"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.classes.clone());

        let newly_selected = render_classes_grid(ui, "Storage Classes", &mut self.table);

        if let Some(name) = newly_selected
            && self.selected.as_ref() != Some(&name)
        {
            self.selected = Some(name.clone());
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectStorageClass {
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::StorageClassesUpdated(classes),
            ) => {
                self.classes = classes.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::StorageClassDetailsUpdated(details),
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
            ui.heading("Storage Class");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteStorageClass(
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
                        ui.monospace(format!("Provisioner: {}", details.provisioner));
                        ui.monospace(format!(
                            "Reclaim Policy: {}",
                            details.reclaim_policy.clone().unwrap_or_default()
                        ));
                        ui.monospace(format!(
                            "Volume Binding Mode: {}",
                            details.volume_binding_mode.clone().unwrap_or_default()
                        ));
                        ui.monospace(format!("Default: {}", details.is_default));
                        ui.monospace(format!(
                            "Allow Volume Expansion: {}",
                            details
                                .allow_volume_expansion
                                .map(|b| b.to_string())
                                .unwrap_or_default()
                        ));
                    });
                });
                ui.add_space(8.0);

                if !details.parameters.is_empty() {
                    ui.collapsing("⚙️ Parameters", |ui| {
                        for (k, v) in &details.parameters {
                            ui.monospace(format!("{}={}", k, v));
                        }
                    });
                }

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading storage class details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshStorageClasses)]
    }
}
