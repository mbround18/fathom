use super::super::components::Table;
use super::super::components::table::Column;
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, LimitRangeDetails, LimitRangeSummary};

/// LimitRange-specific state and rendering for the LimitRanges view
pub struct LimitRangesView {
    pub limitranges: Vec<LimitRangeSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<LimitRangeDetails>,
    pub table: Table<LimitRangeSummary>,
}

impl Default for LimitRangesView {
    fn default() -> Self {
        Self::new()
    }
}

impl LimitRangesView {
    pub fn new() -> Self {
        Self {
            limitranges: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_limitranges_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<LimitRangeSummary>,
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
            sort_value: |l: &LimitRangeSummary| l.namespace.clone(),
            render: |l: &LimitRangeSummary| l.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 200.0,
            sort_value: |l: &LimitRangeSummary| l.name.clone(),
            render: |l: &LimitRangeSummary| l.name.clone(),
        },
        Column {
            header: "Limits".to_string(),
            width: 80.0,
            sort_value: |l: &LimitRangeSummary| format!("{:010}", l.limits_count),
            render: |l: &LimitRangeSummary| l.limits_count.to_string(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |l: &LimitRangeSummary, search: &str| {
            l.namespace.to_lowercase().contains(search) || l.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|l: &LimitRangeSummary, search: &str| {
            l.namespace.to_lowercase().contains(search) || l.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, l)) = filtered.get(selected_idx) {
            newly_selected = Some((l.namespace.clone(), l.name.clone()));
        }
    }

    newly_selected
}

fn render_limit_item(ui: &mut egui::Ui, item: &crate::protocol::LimitRangeItemSummary) {
    ui.group(|ui| {
        ui.vertical(|ui| {
            ui.label(format!("Type: {}", item.limit_type));
            ui.separator();
            if !item.default.is_empty() {
                ui.label("Default:");
                for (k, v) in &item.default {
                    ui.monospace(format!("  {} = {}", k, v));
                }
            }
            if !item.max.is_empty() {
                ui.label("Max:");
                for (k, v) in &item.max {
                    ui.monospace(format!("  {} = {}", k, v));
                }
            }
            if !item.min.is_empty() {
                ui.label("Min:");
                for (k, v) in &item.min {
                    ui.monospace(format!("  {} = {}", k, v));
                }
            }
        });
    });
    ui.add_space(8.0);
}

impl ViewComponent for LimitRangesView {
    fn id(&self) -> &'static str {
        "config_limitranges"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.limitranges.clone());

        let newly_selected = render_limitranges_grid(ui, "Limit Ranges", &mut self.table);

        if let Some((ns, name)) = newly_selected {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectLimitRange {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::LimitRangesUpdated(limitranges),
            ) => {
                self.limitranges = limitranges.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::LimitRangeDetailsUpdated(details),
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
            ui.heading("Limit Range");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteLimitRange(
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
                ui.label("📏 Limits");
                ui.separator();
                if details.limits.is_empty() {
                    ui.label("(none)");
                } else {
                    for item in &details.limits {
                        render_limit_item(ui, item);
                    }
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
                ui.label("Loading limit range details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshLimitRanges)]
    }
}
