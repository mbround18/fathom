use super::super::components::Table;
use super::super::components::table::Column;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{
    BackendCmd, BackendEvent, ValidatingWebhookConfigurationDetails,
    ValidatingWebhookConfigurationSummary,
};

/// ValidatingWebhookConfiguration-specific state and rendering
pub struct ValidatingWebhooksView {
    pub configs: Vec<ValidatingWebhookConfigurationSummary>,
    pub selected: Option<String>,
    pub details: Option<ValidatingWebhookConfigurationDetails>,
    pub table: Table<ValidatingWebhookConfigurationSummary>,
}

impl Default for ValidatingWebhooksView {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidatingWebhooksView {
    pub fn new() -> Self {
        Self {
            configs: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

fn render_configs_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<ValidatingWebhookConfigurationSummary>,
) -> Option<String> {
    ui.heading(title);
    ui.separator();

    let columns = vec![
        Column {
            header: "Name".to_string(),
            width: 260.0,
            sort_value: |c: &ValidatingWebhookConfigurationSummary| c.name.clone(),
            render: |c: &ValidatingWebhookConfigurationSummary| c.name.clone(),
        },
        Column {
            header: "Webhooks".to_string(),
            width: 100.0,
            sort_value: |c: &ValidatingWebhookConfigurationSummary| c.webhooks_count.to_string(),
            render: |c: &ValidatingWebhookConfigurationSummary| c.webhooks_count.to_string(),
        },
    ];

    let mut newly_selected: Option<String> = None;

    table.render_table(
        ui,
        &columns,
        |c: &ValidatingWebhookConfigurationSummary, search: &str| {
            c.name.to_lowercase().contains(search)
        },
        None,
    );

    if let Some(selected_idx) = table.selected {
        let filtered =
            table.filtered_items(|c: &ValidatingWebhookConfigurationSummary, search: &str| {
                c.name.to_lowercase().contains(search)
            });
        if let Some((_orig_idx, c)) = filtered.get(selected_idx) {
            newly_selected = Some(c.name.clone());
        }
    }

    newly_selected
}

impl ViewComponent for ValidatingWebhooksView {
    fn id(&self) -> &'static str {
        "config_validatingwebhooks"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        self.table.set_items(self.configs.clone());

        let newly_selected =
            render_configs_grid(ui, "Validating Webhook Configurations", &mut self.table);

        if let Some(name) = newly_selected
            && self.selected.as_ref() != Some(&name)
        {
            self.selected = Some(name.clone());
            self.details = None;
            actions.push(ViewAction::SendCommand(
                BackendCmd::SelectValidatingWebhookConfiguration { name },
            ));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(
                crate::protocol::ResourceListEvent::ValidatingWebhookConfigurationsUpdated(configs),
            ) => {
                self.configs = configs.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::ValidatingWebhookConfigurationDetailsUpdated(
                    details,
                ),
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
            ui.heading("Validating Webhook Configuration");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(
                        ConfirmAction::DeleteValidatingWebhookConfiguration(name.clone()),
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
                        ui.label("🪝 Webhooks");
                        ui.separator();
                        if details.webhooks.is_empty() {
                            ui.label("(none)");
                        } else {
                            for wh in &details.webhooks {
                                ui.monospace(format!(
                                    "{}  failurePolicy={}  sideEffects={}",
                                    wh.name,
                                    wh.failure_policy.clone().unwrap_or_default(),
                                    wh.side_effects.clone().unwrap_or_default()
                                ));
                            }
                        }
                    });
                });
                ui.add_space(8.0);

                render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label("Loading validating webhook configuration details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(
            BackendCmd::RefreshValidatingWebhookConfigurations,
        )]
    }
}
