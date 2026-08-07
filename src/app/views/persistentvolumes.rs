use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, PersistentVolumeDetails, PersistentVolumeSummary};

/// PersistentVolume-specific state and rendering for the PersistentVolumes view
pub struct PersistentVolumesView {
    pub volumes: Vec<PersistentVolumeSummary>,
    pub selected: Option<String>,
    pub details: Option<PersistentVolumeDetails>,
    pub table: Table<PersistentVolumeSummary>,
}

impl Default for PersistentVolumesView {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistentVolumesView {
    pub fn new() -> Self {
        Self {
            volumes: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_volumes_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<PersistentVolumeSummary>,
) -> Option<String> {
    ui.heading(title);
    ui.separator();

    let columns = vec![
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |v: &PersistentVolumeSummary| v.name.clone(),
            render: |v: &PersistentVolumeSummary| v.name.clone(),
        },
        Column {
            header: "Capacity".to_string(),
            width: 90.0,
            sort_value: |v: &PersistentVolumeSummary| v.capacity.clone(),
            render: |v: &PersistentVolumeSummary| v.capacity.clone(),
        },
        Column {
            header: "Access Modes".to_string(),
            width: 150.0,
            sort_value: |v: &PersistentVolumeSummary| v.access_modes.join(","),
            render: |v: &PersistentVolumeSummary| v.access_modes.join(", "),
        },
        Column {
            header: "Status".to_string(),
            width: 100.0,
            sort_value: |v: &PersistentVolumeSummary| v.status.clone(),
            render: |v: &PersistentVolumeSummary| v.status.clone(),
        },
        Column {
            header: "Storage Class".to_string(),
            width: 140.0,
            sort_value: |v: &PersistentVolumeSummary| v.storage_class.clone().unwrap_or_default(),
            render: |v: &PersistentVolumeSummary| v.storage_class.clone().unwrap_or_default(),
        },
        Column {
            header: "Reclaim Policy".to_string(),
            width: 110.0,
            sort_value: |v: &PersistentVolumeSummary| v.reclaim_policy.clone(),
            render: |v: &PersistentVolumeSummary| v.reclaim_policy.clone(),
        },
        Column {
            header: "Claim".to_string(),
            width: 200.0,
            sort_value: |v: &PersistentVolumeSummary| v.claim_ref.clone().unwrap_or_default(),
            render: |v: &PersistentVolumeSummary| v.claim_ref.clone().unwrap_or_default(),
        },
    ];

    let mut newly_selected: Option<String> = None;

    table.render_table(
        ui,
        &columns,
        |v: &PersistentVolumeSummary, search: &str| v.name.to_lowercase().contains(search),
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|v: &PersistentVolumeSummary, search: &str| {
            v.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, v)) = filtered.get(selected_idx) {
            newly_selected = Some(v.name.clone());
        }
    }

    newly_selected
}

impl ViewComponent for PersistentVolumesView {
    fn id(&self) -> &'static str {
        "storage_pv"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.volumes.clone());

        let newly_selected = render_volumes_grid(ui, "Persistent Volumes", &mut self.table);

        if let Some(name) = newly_selected
            && self.selected.as_ref() != Some(&name)
        {
            self.selected = Some(name.clone());
            self.details = None;
            actions.push(ViewAction::SendCommand(
                BackendCmd::SelectPersistentVolume { name },
            ));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::PersistentVolumesUpdated(volumes),
            ) => {
                self.volumes = volumes.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::PersistentVolumeDetailsUpdated(details),
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
            ui.heading("Persistent Volume");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(
                        ConfirmAction::DeletePersistentVolume(name.clone()),
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
                        ui.monospace(format!("Capacity: {}", details.capacity));
                        ui.monospace(format!("Access Modes: {}", details.access_modes.join(", ")));
                        ui.monospace(format!("Status: {}", details.status));
                        ui.monospace(format!(
                            "Storage Class: {}",
                            details.storage_class.clone().unwrap_or_default()
                        ));
                        ui.monospace(format!("Reclaim Policy: {}", details.reclaim_policy));
                        ui.monospace(format!(
                            "Claim: {}",
                            details.claim_ref.clone().unwrap_or_default()
                        ));
                        ui.monospace(format!(
                            "Volume Mode: {}",
                            details.volume_mode.clone().unwrap_or_default()
                        ));
                        if let Some(driver) = &details.csi_driver {
                            ui.monospace(format!("CSI Driver: {}", driver));
                        }
                    });
                });
                ui.add_space(8.0);

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading persistent volume details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(
            BackendCmd::RefreshPersistentVolumes,
        )]
    }
}
