use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{BackendCmd, BackendEvent, JobDetails, JobSummary};

/// Job-specific state and rendering for the Jobs view
pub struct JobsView {
    pub jobs: Vec<JobSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<JobDetails>,
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
        }
    }
}

impl ViewComponent for JobsView {
    fn id(&self) -> &'static str {
        "workloads_jobs"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        ui.heading("Jobs");
        ui.separator();

        let mut newly_selected: Option<(String, String)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("jobs_grid")
                .striped(true)
                .min_col_width(100.0)
                .show(ui, |ui| {
                    ui.label("Namespace");
                    ui.label("Name");
                    ui.label("Completions");
                    ui.label("Duration");
                    ui.label("Active");
                    ui.label("Succeeded");
                    ui.label("Failed");
                    ui.label("Age");
                    ui.end_row();

                    for job in &self.jobs {
                        let is_selected = self
                            .selected
                            .as_ref()
                            .is_some_and(|(ns, name)| ns == &job.namespace && name == &job.name);

                        if ui.selectable_label(is_selected, &job.namespace).clicked() {
                            newly_selected = Some((job.namespace.clone(), job.name.clone()));
                        }
                        ui.label(&job.name);
                        ui.label(&job.completions);
                        ui.label(&job.duration);
                        ui.label(job.active.to_string());
                        ui.label(job.succeeded.to_string());
                        ui.label(job.failed.to_string());
                        ui.label(&job.age);
                        ui.end_row();
                    }
                });
        });

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
                    actions.push(ViewAction::NotifyStatus(format!(
                        "Delete job {}/{}",
                        ns, name
                    )));
                }
                if ui.button("Close").clicked() {
                    self.selected = None;
                    self.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(details) = &self.details {
                ui.label(format!("Completions: {}", details.completions.unwrap_or(0)));
                ui.label(format!("Duration: {}", details.duration));
                ui.label(format!("Active: {}", details.active));
                ui.label(format!("Succeeded: {}", details.succeeded));
                ui.label(format!("Failed: {}", details.failed));
                ui.add_space(10.0);
                ui.collapsing("Labels", |ui| {
                    for (k, v) in &details.labels {
                        ui.monospace(format!("{}={}", k, v));
                    }
                });
                ui.collapsing("Annotations", |ui| {
                    for (k, v) in &details.annotations {
                        ui.monospace(format!("{}={}", k, v));
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
