use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{
    BackendCmd, BackendEvent, PersistentVolumeClaimDetails, PersistentVolumeClaimSummary,
};

/// PersistentVolumeClaim-specific state and rendering for the PersistentVolumeClaims view
pub struct PersistentVolumeClaimsView {
    pub claims: Vec<PersistentVolumeClaimSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<PersistentVolumeClaimDetails>,
    pub table: Table<PersistentVolumeClaimSummary>,
}

impl Default for PersistentVolumeClaimsView {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistentVolumeClaimsView {
    pub fn new() -> Self {
        Self {
            claims: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_claims_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<PersistentVolumeClaimSummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    let columns = vec![
        Column {
            header: "Namespace".to_string(),
            width: 120.0,
            sort_value: |c: &PersistentVolumeClaimSummary| c.namespace.clone(),
            render: |c: &PersistentVolumeClaimSummary| c.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 180.0,
            sort_value: |c: &PersistentVolumeClaimSummary| c.name.clone(),
            render: |c: &PersistentVolumeClaimSummary| c.name.clone(),
        },
        Column {
            header: "Status".to_string(),
            width: 90.0,
            sort_value: |c: &PersistentVolumeClaimSummary| c.status.clone(),
            render: |c: &PersistentVolumeClaimSummary| c.status.clone(),
        },
        Column {
            header: "Volume".to_string(),
            width: 180.0,
            sort_value: |c: &PersistentVolumeClaimSummary| {
                c.volume_name.clone().unwrap_or_default()
            },
            render: |c: &PersistentVolumeClaimSummary| c.volume_name.clone().unwrap_or_default(),
        },
        Column {
            header: "Capacity".to_string(),
            width: 90.0,
            sort_value: |c: &PersistentVolumeClaimSummary| c.capacity.clone().unwrap_or_default(),
            render: |c: &PersistentVolumeClaimSummary| c.capacity.clone().unwrap_or_default(),
        },
        Column {
            header: "Access Modes".to_string(),
            width: 150.0,
            sort_value: |c: &PersistentVolumeClaimSummary| c.access_modes.join(","),
            render: |c: &PersistentVolumeClaimSummary| c.access_modes.join(", "),
        },
        Column {
            header: "Storage Class".to_string(),
            width: 140.0,
            sort_value: |c: &PersistentVolumeClaimSummary| {
                c.storage_class.clone().unwrap_or_default()
            },
            render: |c: &PersistentVolumeClaimSummary| c.storage_class.clone().unwrap_or_default(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |c: &PersistentVolumeClaimSummary, search: &str| {
            c.namespace.to_lowercase().contains(search) || c.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|c: &PersistentVolumeClaimSummary, search: &str| {
            c.namespace.to_lowercase().contains(search) || c.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, c)) = filtered.get(selected_idx) {
            newly_selected = Some((c.namespace.clone(), c.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for PersistentVolumeClaimsView {
    fn id(&self) -> &'static str {
        "storage_pvc"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.claims.clone());

        let newly_selected = render_claims_grid(ui, "Persistent Volume Claims", &mut self.table);

        if let Some((ns, name)) = newly_selected
            && self.selected.as_ref() != Some(&(ns.clone(), name.clone()))
        {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(
                BackendCmd::SelectPersistentVolumeClaim {
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
                crate::protocol::ResourceListEvent::PersistentVolumeClaimsUpdated(claims),
            ) => {
                self.claims = claims.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::PersistentVolumeClaimDetailsUpdated(details),
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
            ui.heading("Persistent Volume Claim");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(
                        ConfirmAction::DeletePersistentVolumeClaim(ns.clone(), name.clone()),
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
                        ui.monospace(format!("Status: {}", details.status));
                        ui.monospace(format!(
                            "Volume: {}",
                            details.volume_name.clone().unwrap_or_default()
                        ));
                        ui.monospace(format!(
                            "Capacity: {}",
                            details.capacity.clone().unwrap_or_default()
                        ));
                        ui.monospace(format!(
                            "Requested Storage: {}",
                            details.requested_storage.clone().unwrap_or_default()
                        ));
                        ui.monospace(format!("Access Modes: {}", details.access_modes.join(", ")));
                        ui.monospace(format!(
                            "Storage Class: {}",
                            details.storage_class.clone().unwrap_or_default()
                        ));
                    });
                });
                ui.add_space(8.0);

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading persistent volume claim details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(
            BackendCmd::RefreshPersistentVolumeClaims,
        )]
    }
}
