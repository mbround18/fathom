use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{BackendCmd, BackendEvent, CronJobDetails, CronJobSummary};

/// CronJob-specific state and rendering for the CronJobs view
pub struct CronJobsView {
    pub cronjobs: Vec<CronJobSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<CronJobDetails>,
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
        }
    }
}

impl ViewComponent for CronJobsView {
    fn id(&self) -> &'static str {
        "workloads_cronjobs"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        ui.heading("CronJobs");
        ui.separator();

        let mut newly_selected: Option<(String, String)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("cronjobs_grid")
                .striped(true)
                .min_col_width(100.0)
                .show(ui, |ui| {
                    ui.label("Namespace");
                    ui.label("Name");
                    ui.label("Schedule");
                    ui.label("Suspend");
                    ui.label("Active");
                    ui.label("Last Schedule");
                    ui.label("Age");
                    ui.end_row();

                    for cj in &self.cronjobs {
                        let is_selected = self
                            .selected
                            .as_ref()
                            .is_some_and(|(ns, name)| ns == &cj.namespace && name == &cj.name);

                        if ui.selectable_label(is_selected, &cj.namespace).clicked() {
                            newly_selected = Some((cj.namespace.clone(), cj.name.clone()));
                        }
                        ui.label(&cj.name);
                        ui.label(&cj.schedule);
                        ui.label(if cj.suspend { "Yes" } else { "No" });
                        ui.label(cj.active.to_string());
                        ui.label(cj.last_schedule.as_deref().unwrap_or("-"));
                        ui.label(&cj.age);
                        ui.end_row();
                    }
                });
        });

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
                    actions.push(ViewAction::NotifyStatus(format!(
                        "Delete cronjob {}/{}",
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
                ui.label(format!("Schedule: {}", details.schedule));
                ui.label(format!("Suspend: {}", details.suspend));
                ui.label(format!("Active: {}", details.active));
                ui.label(format!(
                    "Last Schedule: {}",
                    details.last_schedule_time.as_deref().unwrap_or("-")
                ));
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
