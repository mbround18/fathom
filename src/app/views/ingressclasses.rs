use super::super::components::Table;
use super::super::components::table::Column;
use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{BackendCmd, BackendEvent, IngressClassDetails, IngressClassSummary};

/// IngressClass-specific state and rendering for the IngressClasses view
pub struct IngressClassesView {
    pub classes: Vec<IngressClassSummary>,
    pub selected: Option<String>,
    pub details: Option<IngressClassDetails>,
    pub table: Table<IngressClassSummary>,
}

impl Default for IngressClassesView {
    fn default() -> Self {
        Self::new()
    }
}

impl IngressClassesView {
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
    table: &mut Table<IngressClassSummary>,
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
            sort_value: |c: &IngressClassSummary| c.name.clone(),
            render: |c: &IngressClassSummary| c.name.clone(),
        },
        Column {
            header: "Controller".to_string(),
            width: 250.0,
            sort_value: |c: &IngressClassSummary| c.controller.clone(),
            render: |c: &IngressClassSummary| c.controller.clone(),
        },
    ];

    let mut newly_selected: Option<String> = None;

    table.render_table(
        ui,
        &columns,
        |c: &IngressClassSummary, search: &str| c.name.to_lowercase().contains(search),
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table
            .filtered_items(|c: &IngressClassSummary, search: &str| {
                c.name.to_lowercase().contains(search)
            });
        if let Some((_orig_idx, c)) = filtered.get(selected_idx) {
            newly_selected = Some(c.name.clone());
        }
    }

    newly_selected
}

impl ViewComponent for IngressClassesView {
    fn id(&self) -> &'static str {
        "network_ingressclasses"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.classes.clone());

        let newly_selected = render_classes_grid(ui, "Ingress Classes", &mut self.table);

        if let Some(name) = newly_selected {
            self.selected = Some(name.clone());
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectIngressClass {
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::IngressClassesUpdated(classes),
            ) => {
                self.classes = classes.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::IngressClassDetailsUpdated(details),
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

        let actions = Vec::new();
        let name = self.selected.as_ref().unwrap().clone();

        ui.horizontal(|ui| {
            ui.heading("Ingress Class");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                        ui.monospace(format!("Controller: {}", details.controller));
                    });
                });
                ui.add_space(8.0);

                if let Some(params) = &details.parameters {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("⚙️ Parameters");
                            ui.separator();
                            ui.monospace(format!(
                                "Kind: {}{}",
                                params
                                    .api_group
                                    .clone()
                                    .map(|g| format!("{}/", g))
                                    .unwrap_or_default(),
                                params.kind
                            ));
                            ui.monospace(format!(
                                "Name: {}{}",
                                params
                                    .namespace
                                    .clone()
                                    .map(|n| format!("{}/", n))
                                    .unwrap_or_default(),
                                params.name
                            ));
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
                ui.label("Loading ingress class details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshIngressClasses)]
    }
}
