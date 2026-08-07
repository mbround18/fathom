use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, ConfigMapDetails, ConfigMapSummary};

/// ConfigMap-specific state and rendering for the ConfigMaps view
pub struct ConfigMapsView {
    pub configmaps: Vec<ConfigMapSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<ConfigMapDetails>,
    pub table: Table<ConfigMapSummary>,
}

impl Default for ConfigMapsView {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigMapsView {
    pub fn new() -> Self {
        Self {
            configmaps: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_configmaps_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<ConfigMapSummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    let columns = vec![
        Column {
            header: "Namespace".to_string(),
            width: 120.0,
            sort_value: |c: &ConfigMapSummary| c.namespace.clone(),
            render: |c: &ConfigMapSummary| c.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 200.0,
            sort_value: |c: &ConfigMapSummary| c.name.clone(),
            render: |c: &ConfigMapSummary| c.name.clone(),
        },
        Column {
            header: "Keys".to_string(),
            width: 80.0,
            sort_value: |c: &ConfigMapSummary| format!("{:010}", c.keys_count),
            render: |c: &ConfigMapSummary| c.keys_count.to_string(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |c: &ConfigMapSummary, search: &str| {
            c.namespace.to_lowercase().contains(search) || c.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|c: &ConfigMapSummary, search: &str| {
            c.namespace.to_lowercase().contains(search) || c.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, c)) = filtered.get(selected_idx) {
            newly_selected = Some((c.namespace.clone(), c.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for ConfigMapsView {
    fn id(&self) -> &'static str {
        "config_configmaps"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.configmaps.clone());

        let newly_selected = render_configmaps_grid(ui, "ConfigMaps", &mut self.table);

        if let Some((ns, name)) = newly_selected
            && self.selected.as_ref() != Some(&(ns.clone(), name.clone()))
        {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectConfigMap {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(crate::protocol::ResourceListEvent::ConfigMapsUpdated(
                configmaps,
            )) => {
                self.configmaps = configmaps.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::ConfigMapDetailsUpdated(details),
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
            ui.heading("ConfigMap");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteConfigMap(
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
                        ui.label("🔑 Data");
                        ui.separator();
                        if details.data.is_empty() {
                            ui.label("(none)");
                        } else {
                            for (k, v) in &details.data {
                                ui.collapsing(k, |ui| {
                                    ui.monospace(v);
                                });
                            }
                        }
                    });
                });
                ui.add_space(8.0);

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading configmap details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshConfigMaps)]
    }
}
