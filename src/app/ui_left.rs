use super::*;

impl FathomApp {
    pub(super) fn render_left_nav(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("left_nav")
            .resizable(false)
            .default_width(170.0)
            .show(ctx, |ui| {
                ui.heading("Workloads");
                ui.separator();

                let deploy_selected = self.active_section == section_keys::workloads_deployments();
                if ui
                    .selectable_label(deploy_selected, "Deployments")
                    .clicked()
                {
                    self.active_section = section_keys::workloads_deployments();

                    // Leaving the pods view should close the pods side panel.
                    self.selected_pod = None;
                    self.pod_details = None;
                    self.pod_metrics = None;
                    self.pod_logs.clear();
                    self.send(BackendCmd::StopPodLogs);

                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                }

                let pods_selected = self.active_section == section_keys::workloads_pods();
                if ui.selectable_label(pods_selected, "Pods").clicked() {
                    self.active_section = section_keys::workloads_pods();

                    // Leaving deployments should close the deployments side panel.
                    self.selected_deployment = None;
                    self.deployment_details = None;

                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                }

                ui.add_space(12.0);
                ui.separator();

                if ui.button("Simulate Slow Call").clicked() {
                    self.send(BackendCmd::SimulateSlowCall);
                }
            });
    }
}
