use super::super::components::Table;
use super::super::components::table::Column;
use super::traits::{ViewAction, ViewComponent};
use crate::app::ConfirmAction;
use crate::protocol::{BackendCmd, BackendEvent, JobDetails, JobSummary};

/// Job-specific state and rendering for the Jobs view
pub struct JobsView {
    pub jobs: Vec<JobSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<JobDetails>,
    pub table: Table<JobSummary>,
}

impl Default for JobsView {
    fn default() -> Self {
        Self::new()
    }
}

impl JobsView {
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            selected: None,
            details: None,
            table: Table::new(Vec::new()),
        }
    }
}

/// Render the jobs grid with Table component
fn render_jobs_grid(ui: &mut egui::Ui, title: &str, table: &mut Table<JobSummary>, _jobs: &[JobSummary]) -> Option<(String, String)> {
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
            sort_value: |j: &JobSummary| j.namespace.clone(),
            render: |j: &JobSummary| j.namespace.clone(),
        },
        Column {
            header: "Name".to_string(),
            width: 150.0,
            sort_value: |j: &JobSummary| j.name.clone(),
            render: |j: &JobSummary| j.name.clone(),
        },
        Column {
            header: "Completions".to_string(),
            width: 100.0,
            sort_value: |j: &JobSummary| j.completions.clone(),
            render: |j: &JobSummary| j.completions.clone(),
        },
        Column {
            header: "Duration".to_string(),
            width: 100.0,
            sort_value: |j: &JobSummary| j.duration.clone(),
            render: |j: &JobSummary| j.duration.clone(),
        },
        Column {
            header: "Active".to_string(),
            width: 60.0,
            sort_value: |j: &JobSummary| j.active.to_string(),
            render: |j: &JobSummary| j.active.to_string(),
        },
        Column {
            header: "Succeeded".to_string(),
            width: 80.0,
            sort_value: |j: &JobSummary| j.succeeded.to_string(),
            render: |j: &JobSummary| j.succeeded.to_string(),
        },
        Column {
            header: "Failed".to_string(),
            width: 60.0,
            sort_value: |j: &JobSummary| j.failed.to_string(),
            render: |j: &JobSummary| j.failed.to_string(),
        },
        Column {
            header: "Age".to_string(),
            width: 80.0,
            sort_value: |j: &JobSummary| j.age.clone(),
            render: |j: &JobSummary| j.age.clone(),
        },
    ];

    // Use a RefCell or mutable wrapper to capture selection in the callback
    let mut newly_selected: Option<(String, String)> = None;
    
    // Need to handle the callback differently since we can't use closures
    // Will handle selection in render method after table renders
    table.render_table(
        ui,
        &columns,
        |job: &JobSummary, search: &str| {
            job.namespace.to_lowercase().contains(search)
                || job.name.to_lowercase().contains(search)
        },
        None, // No selection callback - will use table.selected instead
    );

    // Check if a row was selected and return it
    if let Some(selected_idx) = table.selected {
        // Get the selected job from filtered items
        let filtered = table.filtered_items(|job: &JobSummary, search: &str| {
            job.namespace.to_lowercase().contains(search)
                || job.name.to_lowercase().contains(search)
        });
        if let Some((_orig_idx, job)) = filtered.get(selected_idx) {
            newly_selected = Some((job.namespace.clone(), job.name.clone()));
        }
    }

    newly_selected
}

impl ViewComponent for JobsView {
    fn id(&self) -> &'static str {
        "workloads_jobs"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        // Update table data
        self.table.set_items(self.jobs.clone());

        let newly_selected = render_jobs_grid(ui, "Jobs", &mut self.table, &self.jobs);

        if let Some((ns, name)) = newly_selected {
            self.selected = Some((ns.clone(), name.clone()));
            self.details = None;
            actions.push(ViewAction::SendCommand(BackendCmd::SelectJob {
                namespace: ns,
                name,
            }));
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        match event {
            BackendEvent::ResourceList(crate::protocol::ResourceListEvent::JobsUpdated(jobs)) => {
                self.jobs = jobs.clone();
            }
            BackendEvent::ResourceRead(crate::protocol::ResourceReadEvent::JobDetailsUpdated(
                details,
            )) => {
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
            ui.heading("Job");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteJob(ns.clone(), name.clone())));
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
                // 📊 Status
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Status");
                        ui.separator();
                        ui.monospace(format!("Completions: {}/{}", details.succeeded, details.completions.unwrap_or(0)));
                        ui.monospace(format!("Active: {}", details.active));
                        ui.monospace(format!("Succeeded: {}", details.succeeded));
                        ui.monospace(format!("Failed: {}", details.failed));
                        ui.monospace(format!("Parallelism: {}", details.parallelism.unwrap_or(0)));
                    });
                });
                ui.add_space(8.0);

                // ⏱️ Timing
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("⏱️ Timing");
                        ui.separator();
                        if let Some(start) = &details.start_time {
                            ui.monospace(format!("Started: {}", start));
                        }
                        if let Some(completion) = &details.completion_time {
                            ui.monospace(format!("Completed: {}", completion));
                        }
                        ui.monospace(format!("Duration: {}", details.duration));
                    });
                });
                ui.add_space(8.0);

                // 🏷️ Selector
                if !details.selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🏷️ Pod Selector");
                            ui.separator();
                            ui.monospace(&details.selector);
                        });
                    });
                    ui.add_space(8.0);
                }

                // 📋 Conditions
                if !details.conditions.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("📋 Conditions");
                            ui.separator();
                            for condition in &details.conditions {
                                ui.monospace(format!("{}: {}", condition.condition_type, condition.status));
                                if let Some(reason) = &condition.reason {
                                    ui.monospace(format!("  Reason: {}", reason));
                                }
                                if let Some(msg) = &condition.message {
                                    ui.monospace(format!("  Message: {}", msg));
                                }
                            }
                        });
                    });
                    ui.add_space(8.0);
                }

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
                ui.label("Loading job details...");
            }
        });

        ctx.request_repaint();
        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshJobs)]
    }
}
