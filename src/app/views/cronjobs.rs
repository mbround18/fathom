use super::super::components::Table;
use super::super::components::table::Column;
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, CronJobDetails, CronJobSummary};

/// CronJob-specific state and rendering for the CronJobs view
pub struct CronJobsView {
    pub cronjobs: Vec<CronJobSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<CronJobDetails>,
    pub table: Table<CronJobSummary>,
}

impl Default for CronJobsView {
    fn default() -> Self {
        Self::new()
    }
}

impl CronJobsView {
    pub fn new() -> Self {
        Self {
            cronjobs: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

/// Render the cronjobs grid with Table component
fn render_cronjobs_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<CronJobSummary>,
    _cronjobs: &[CronJobSummary],
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    // Search bar
    ui.horizontal(|ui| {
        ui.label("🔍 Search:");
        ui.text_edit_singleline(&mut table.state.search_text);
    });
    ui.add_space(8.0);

    let columns = vec![
        Column {
            header: "Namespace".to_string(),
            width: 100.0,
            sort_value: |c: &CronJobSummary| c.namespace.clone(),
            render: |c: &CronJobSummary| c.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 150.0,
            sort_value: |c: &CronJobSummary| c.name.clone(),
            render: |c: &CronJobSummary| c.name.clone(),
        },
        Column {
            header: "Schedule".to_string(),
            width: 120.0,
            sort_value: |c: &CronJobSummary| c.schedule.clone(),
            render: |c: &CronJobSummary| c.schedule.clone(),
        },
        Column {
            header: "Suspend".to_string(),
            width: 70.0,
            sort_value: |c: &CronJobSummary| if c.suspend { "Yes" } else { "No" }.to_string(),
            render: |c: &CronJobSummary| if c.suspend { "Yes" } else { "No" }.to_string(),
        },
        Column {
            header: "Active".to_string(),
            width: 60.0,
            sort_value: |c: &CronJobSummary| c.active.to_string(),
            render: |c: &CronJobSummary| c.active.to_string(),
        },
        Column {
            header: "Last Schedule".to_string(),
            width: 150.0,
            sort_value: |c: &CronJobSummary| c.last_schedule.clone().unwrap_or_default(),
            render: |c: &CronJobSummary| c.last_schedule.as_deref().unwrap_or("-").to_string(),
        },
        Column {
            header: "Age".to_string(),
            width: 80.0,
            sort_value: |c: &CronJobSummary| c.age.clone(),
            render: |c: &CronJobSummary| c.age.clone(),
        },
    ];

    let mut newly_selected: Option<(String, String)> = None;

    table.render_table(
        ui,
        &columns,
        |cj: &CronJobSummary, search: &str| {
            cj.namespace.to_lowercase().contains(search)
                || cj.name.to_lowercase().contains(search)
                || cj.schedule.to_lowercase().contains(search)
        },
        None,
    );

    // Check if a row was selected and return it
    if let Some(selected_idx) = table.selected {
        let filtered = table.filtered_items(|cj: &CronJobSummary, search: &str| {
            cj.namespace.to_lowercase().contains(search)
                || cj.name.to_lowercase().contains(search)
                || cj.schedule.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, cj)) = filtered.get(selected_idx) {
            newly_selected = Some((cj.namespace.clone(), cj.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for CronJobsView {
    fn id(&self) -> &'static str {
        "workloads_cronjobs"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        // Update table data
        self.table.set_items(self.cronjobs.clone());

        let newly_selected = render_cronjobs_grid(ui, "CronJobs", &mut self.table, &self.cronjobs);

        if let Some((ns, name)) = newly_selected {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectCronJob {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(crate::protocol::ResourceListEvent::CronJobsUpdated(
                cronjobs,
            )) => {
                self.cronjobs = cronjobs.clone();
            }
            BackendEvent::ResourceRead(
                crate::protocol::ResourceReadEvent::CronJobDetailsUpdated(details),
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
            ui.heading("CronJob");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteCronJob(
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
                // 📅 Schedule
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📅 Schedule");
                        ui.separator();
                        ui.monospace(format!("Schedule: {}", details.schedule));
                        ui.monospace(format!(
                            "Suspended: {}",
                            if details.suspend { "Yes" } else { "No" }
                        ));
                        ui.monospace(format!(
                            "Concurrency Policy: {}",
                            details.concurrency_policy
                        ));
                    });
                });
                ui.add_space(8.0);

                // ⏱️ Timing
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("⏱️ Timing");
                        ui.separator();
                        if let Some(last_schedule) = &details.last_schedule_time {
                            ui.monospace(format!("Last Schedule: {}", last_schedule));
                        } else {
                            ui.monospace("Last Schedule: Never");
                        }
                        ui.monospace(format!("Active Jobs: {}", details.active));
                    });
                });
                ui.add_space(8.0);

                // 🔧 Configuration
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("🔧 Configuration");
                        ui.separator();
                        if let Some(deadline) = details.starting_deadline_seconds {
                            ui.monospace(format!("Starting Deadline: {} seconds", deadline));
                        }
                        if let Some(successful_limit) = details.successful_jobs_history_limit {
                            ui.monospace(format!(
                                "Successful Jobs History Limit: {}",
                                successful_limit
                            ));
                        }
                        if let Some(failed_limit) = details.failed_jobs_history_limit {
                            ui.monospace(format!("Failed Jobs History Limit: {}", failed_limit));
                        }
                    });
                });
                ui.add_space(8.0);

                // 🏷️ Labels & Annotations
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
            } else {
                ui.label("Loading cronjob details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshCronJobs)]
    }
}
