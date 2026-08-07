use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, LeaseDetails, LeaseSummary};

/// Lease-specific state and rendering for the Leases view
pub struct LeasesView {
    pub leases: Vec<LeaseSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<LeaseDetails>,
    pub table: Table<LeaseSummary>,
}

impl Default for LeasesView {
    fn default() -> Self {
        Self::new()
    }
}

impl LeasesView {
    pub fn new() -> Self {
        Self {
            leases: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_leases_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<LeaseSummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    let columns = vec![
        Column {
            header: "Namespace".to_string(),
            width: 120.0,
            sort_value: |l: &LeaseSummary| l.namespace.clone(),
            render: |l: &LeaseSummary| l.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 200.0,
            sort_value: |l: &LeaseSummary| l.name.clone(),
            render: |l: &LeaseSummary| l.name.clone(),
        },
        Column {
            header: "Holder Identity".to_string(),
            width: 220.0,
            sort_value: |l: &LeaseSummary| l.holder_identity.clone().unwrap_or_default(),
            render: |l: &LeaseSummary| l.holder_identity.clone().unwrap_or_default(),
        },
        Column {
            header: "Renew Time".to_string(),
            width: 200.0,
            sort_value: |l: &LeaseSummary| l.renew_time.clone().unwrap_or_default(),
            render: |l: &LeaseSummary| l.renew_time.clone().unwrap_or_default(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |l: &LeaseSummary, search: &str| {
            l.namespace.to_lowercase().contains(search) || l.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|l: &LeaseSummary, search: &str| {
            l.namespace.to_lowercase().contains(search) || l.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, l)) = filtered.get(selected_idx) {
            newly_selected = Some((l.namespace.clone(), l.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for LeasesView {
    fn id(&self) -> &'static str {
        "config_leases"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.leases.clone());

        let newly_selected = render_leases_grid(ui, "Leases", &mut self.table);

        if let Some((ns, name)) = newly_selected
            && self.selected.as_ref() != Some(&(ns.clone(), name.clone()))
        {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectLease {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(crate::protocol::ResourceListEvent::LeasesUpdated(
                leases,
            )) => {
                self.leases = leases.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::LeaseDetailsUpdated(details),
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
            ui.heading("Lease");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteLease(
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
                        ui.monospace(format!(
                            "Holder Identity: {}",
                            details.holder_identity.clone().unwrap_or_default()
                        ));
                        ui.monospace(format!(
                            "Lease Duration Seconds: {}",
                            details
                                .lease_duration_seconds
                                .map(|s| s.to_string())
                                .unwrap_or_default()
                        ));
                        ui.monospace(format!(
                            "Acquire Time: {}",
                            details.acquire_time.clone().unwrap_or_default()
                        ));
                        ui.monospace(format!(
                            "Renew Time: {}",
                            details.renew_time.clone().unwrap_or_default()
                        ));
                    });
                });
                ui.add_space(8.0);

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading lease details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshLeases)]
    }
}
