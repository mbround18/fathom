use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, ResourceQuotaDetails, ResourceQuotaSummary};

/// ResourceQuota-specific state and rendering for the ResourceQuotas view
pub struct ResourceQuotasView {
    pub quotas: Vec<ResourceQuotaSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<ResourceQuotaDetails>,
    pub table: Table<ResourceQuotaSummary>,
}

impl Default for ResourceQuotasView {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceQuotasView {
    pub fn new() -> Self {
        Self {
            quotas: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_quotas_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<ResourceQuotaSummary>,
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
            sort_value: |q: &ResourceQuotaSummary| q.namespace.clone(),
            render: |q: &ResourceQuotaSummary| q.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 200.0,
            sort_value: |q: &ResourceQuotaSummary| q.name.clone(),
            render: |q: &ResourceQuotaSummary| q.name.clone(),
        },
        Column {
            header: "Hard Limits".to_string(),
            width: 100.0,
            sort_value: |q: &ResourceQuotaSummary| format!("{:010}", q.hard_limits_count),
            render: |q: &ResourceQuotaSummary| q.hard_limits_count.to_string(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |q: &ResourceQuotaSummary, search: &str| {
            q.namespace.to_lowercase().contains(search) || q.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|q: &ResourceQuotaSummary, search: &str| {
            q.namespace.to_lowercase().contains(search) || q.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, q)) = filtered.get(selected_idx) {
            newly_selected = Some((q.namespace.clone(), q.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for ResourceQuotasView {
    fn id(&self) -> &'static str {
        "config_resourcequotas"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.quotas.clone());

        let newly_selected = render_quotas_grid(ui, "Resource Quotas", &mut self.table);

        if let Some((ns, name)) = newly_selected {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectResourceQuota {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::ResourceQuotasUpdated(quotas),
            ) => {
                self.quotas = quotas.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::ResourceQuotaDetailsUpdated(details),
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
            ui.heading("Resource Quota");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteResourceQuota(
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
                        ui.label("🔒 Hard Limits");
                        ui.separator();
                        if details.hard.is_empty() {
                            ui.label("(none)");
                        } else {
                            for (k, v) in &details.hard {
                                ui.monospace(format!("{} = {}", k, v));
                            }
                        }
                    });
                });
                ui.add_space(8.0);

                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📈 Used");
                        ui.separator();
                        if details.used.is_empty() {
                            ui.label("(none)");
                        } else {
                            for (k, v) in &details.used {
                                ui.monospace(format!("{} = {}", k, v));
                            }
                        }
                    });
                });
                ui.add_space(8.0);

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading resource quota details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshResourceQuotas)]
    }
}
