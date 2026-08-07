use super::super::components::Table;
use super::super::components::table::Column;
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, SecretDetails, SecretSummary};

/// Secret-specific state and rendering for the Secrets view.
///
/// SECURITY: this view must never render decoded/raw secret values — only
/// key names and the secret type are ever displayed.
pub struct SecretsView {
    pub secrets: Vec<SecretSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<SecretDetails>,
    pub table: Table<SecretSummary>,
}

impl Default for SecretsView {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretsView {
    pub fn new() -> Self {
        Self {
            secrets: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_secrets_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<SecretSummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    let columns = vec![
        Column {
            header: "Namespace".to_string(),
            width: 120.0,
            sort_value: |s: &SecretSummary| s.namespace.clone(),
            render: |s: &SecretSummary| s.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 200.0,
            sort_value: |s: &SecretSummary| s.name.clone(),
            render: |s: &SecretSummary| s.name.clone(),
        },
        Column {
            header: "Type".to_string(),
            width: 160.0,
            sort_value: |s: &SecretSummary| s.secret_type.clone(),
            render: |s: &SecretSummary| s.secret_type.clone(),
        },
        Column {
            header: "Keys".to_string(),
            width: 80.0,
            sort_value: |s: &SecretSummary| format!("{:010}", s.keys_count),
            render: |s: &SecretSummary| s.keys_count.to_string(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |s: &SecretSummary, search: &str| {
            s.namespace.to_lowercase().contains(search) || s.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|s: &SecretSummary, search: &str| {
            s.namespace.to_lowercase().contains(search) || s.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, s)) = filtered.get(selected_idx) {
            newly_selected = Some((s.namespace.clone(), s.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for SecretsView {
    fn id(&self) -> &'static str {
        "config_secrets"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.secrets.clone());

        let newly_selected = render_secrets_grid(ui, "Secrets", &mut self.table);

        if let Some((ns, name)) = newly_selected
            && self.selected.as_ref() != Some(&(ns.clone(), name.clone()))
        {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectSecret {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(crate::protocol::ResourceListEvent::SecretsUpdated(
                secrets,
            )) => {
                self.secrets = secrets.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::SecretDetailsUpdated(details),
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
            ui.heading("Secret");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteSecret(
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
                        ui.monospace(format!("Type: {}", details.secret_type));
                    });
                });
                ui.add_space(8.0);

                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("🔑 Keys");
                        ui.small("Secret values are never displayed.");
                        ui.separator();
                        if details.keys.is_empty() {
                            ui.label("(none)");
                        } else {
                            for k in &details.keys {
                                ui.monospace(format!("{} = (value hidden)", k));
                            }
                        }
                    });
                });
                ui.add_space(8.0);

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
                ui.collapsing("Raw JSON (redacted)", |ui| {
                    ui.small("Values are redacted before display.");
                    egui::ScrollArea::vertical()
                        .max_height(250.0)
                        .show(ui, |ui| {
                            ui.monospace(&details.raw_json);
                        });
                });
            } else {
                ui.label("Loading secret details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshSecrets)]
    }
}
