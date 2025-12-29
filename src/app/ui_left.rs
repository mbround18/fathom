use super::*;

impl FathomApp {
    pub(super) fn render_left_nav(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("left_nav")
            .resizable(false)
            .default_width(170.0)
            .show(ctx, |ui| {
                ui.heading("Workloads");
                ui.separator();

                let pods_selected = self.active_section == section_keys::workloads_pods();
                if ui.selectable_label(pods_selected, "Pods").clicked() {
                    self.active_section = section_keys::workloads_pods();

                    // Leaving any workload view should close workload side panels.
                    self.selected_deployment = None;
                    self.deployment_details = None;
                    self.selected_daemonset = None;
                    self.daemonset_details = None;
                    self.selected_statefulset = None;
                    self.statefulset_details = None;
                    self.selected_replicaset = None;
                    self.replicaset_details = None;
                    self.selected_replicationcontroller = None;
                    self.replicationcontroller_details = None;

                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                }

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

                    // Close other workload panels.
                    self.selected_daemonset = None;
                    self.daemonset_details = None;
                    self.selected_statefulset = None;
                    self.statefulset_details = None;
                    self.selected_replicaset = None;
                    self.replicaset_details = None;
                    self.selected_replicationcontroller = None;
                    self.replicationcontroller_details = None;

                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                }

                let ds_selected = self.active_section == section_keys::workloads_daemonsets();
                if ui.selectable_label(ds_selected, "DaemonSets").clicked() {
                    self.active_section = section_keys::workloads_daemonsets();

                    self.selected_pod = None;
                    self.pod_details = None;
                    self.pod_metrics = None;
                    self.pod_logs.clear();
                    self.send(BackendCmd::StopPodLogs);

                    self.selected_deployment = None;
                    self.deployment_details = None;
                    self.selected_statefulset = None;
                    self.statefulset_details = None;
                    self.selected_replicaset = None;
                    self.replicaset_details = None;
                    self.selected_replicationcontroller = None;
                    self.replicationcontroller_details = None;

                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                }

                let ss_selected = self.active_section == section_keys::workloads_statefulsets();
                if ui.selectable_label(ss_selected, "StatefulSets").clicked() {
                    self.active_section = section_keys::workloads_statefulsets();

                    self.selected_pod = None;
                    self.pod_details = None;
                    self.pod_metrics = None;
                    self.pod_logs.clear();
                    self.send(BackendCmd::StopPodLogs);

                    self.selected_deployment = None;
                    self.deployment_details = None;
                    self.selected_daemonset = None;
                    self.daemonset_details = None;
                    self.selected_replicaset = None;
                    self.replicaset_details = None;
                    self.selected_replicationcontroller = None;
                    self.replicationcontroller_details = None;

                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                }

                let rs_selected = self.active_section == section_keys::workloads_replicasets();
                if ui.selectable_label(rs_selected, "ReplicaSets").clicked() {
                    self.active_section = section_keys::workloads_replicasets();

                    self.selected_pod = None;
                    self.pod_details = None;
                    self.pod_metrics = None;
                    self.pod_logs.clear();
                    self.send(BackendCmd::StopPodLogs);

                    self.selected_deployment = None;
                    self.deployment_details = None;
                    self.selected_daemonset = None;
                    self.daemonset_details = None;
                    self.selected_statefulset = None;
                    self.statefulset_details = None;
                    self.selected_replicationcontroller = None;
                    self.replicationcontroller_details = None;

                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                }

                let rc_selected =
                    self.active_section == section_keys::workloads_replicationcontrollers();
                if ui
                    .selectable_label(rc_selected, "ReplicationControllers")
                    .clicked()
                {
                    self.active_section = section_keys::workloads_replicationcontrollers();

                    self.selected_pod = None;
                    self.pod_details = None;
                    self.pod_metrics = None;
                    self.pod_logs.clear();
                    self.send(BackendCmd::StopPodLogs);

                    self.selected_deployment = None;
                    self.deployment_details = None;
                    self.selected_daemonset = None;
                    self.daemonset_details = None;
                    self.selected_statefulset = None;
                    self.statefulset_details = None;
                    self.selected_replicaset = None;
                    self.replicaset_details = None;

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
