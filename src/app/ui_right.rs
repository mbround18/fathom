use super::*;

impl FathomApp {
    pub(super) fn render_right_panel(&mut self, ctx: &egui::Context) {
        if self.active_section == section_keys::workloads_pods() {
            if self.selected_pod.is_none() {
                return;
            }
            self.render_pod_right_panel(ctx);
            return;
        }

        if self.active_section == section_keys::workloads_deployments() {
            if self.selected_deployment.is_none() {
                return;
            }
            self.render_workload_right_panel(ctx, WorkloadKind::Deployment);
            return;
        }

        if self.active_section == section_keys::workloads_daemonsets() {
            if self.selected_daemonset.is_none() {
                return;
            }
            self.render_workload_right_panel(ctx, WorkloadKind::DaemonSet);
            return;
        }

        if self.active_section == section_keys::workloads_statefulsets() {
            if self.selected_statefulset.is_none() {
                return;
            }
            self.render_workload_right_panel(ctx, WorkloadKind::StatefulSet);
            return;
        }

        if self.active_section == section_keys::workloads_replicasets() {
            if self.selected_replicaset.is_none() {
                return;
            }
            self.render_workload_right_panel(ctx, WorkloadKind::ReplicaSet);
            return;
        }

        if self.active_section == section_keys::workloads_replicationcontrollers() {
            if self.selected_replicationcontroller.is_none() {
                return;
            }
            self.render_workload_right_panel(ctx, WorkloadKind::ReplicationController);
            return;
        }
    }

    fn render_pod_right_panel(&mut self, ctx: &egui::Context) {
        let Some((ns, name)) = self.selected_pod.clone() else {
            return;
        };

        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Pod");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Delete").clicked() {
                            self.open_confirm(
                                "Delete pod?",
                                format!("Delete {}/{}?", ns, name),
                                "Delete",
                                ConfirmAction::DeletePod {
                                    namespace: ns.clone(),
                                    name: name.clone(),
                                },
                            );
                        }
                        if ui.button("Close").clicked() {
                            self.selected_pod = None;
                            self.pod_details = None;
                            self.pod_metrics = None;
                            self.pod_metrics_pending = false;
                            self.pod_logs.clear();
                            self.send(BackendCmd::StopPodLogs);
                        }
                    });
                });

                ui.label(format!("{}/{}", ns, name));

                ui.separator();

                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.pod_tab, PodTab::Details, "Details");
                    ui.selectable_value(&mut self.pod_tab, PodTab::Logs, "Logs");
                    ui.selectable_value(&mut self.pod_tab, PodTab::Metrics, "Metrics");
                    ui.selectable_value(&mut self.pod_tab, PodTab::Exec, "Exec");
                });

                ui.add_space(8.0);

                match self.pod_tab {
                    PodTab::Details => self.render_pod_details(ui),
                    PodTab::Logs => self.render_pod_logs(ui),
                    PodTab::Metrics => self.render_pod_metrics(ui),
                    PodTab::Exec => self.render_pod_exec(ui),
                }
            });
    }

    fn workload_title(kind: WorkloadKind) -> &'static str {
        match kind {
            WorkloadKind::Deployment => "Deployment",
            WorkloadKind::DaemonSet => "DaemonSet",
            WorkloadKind::StatefulSet => "StatefulSet",
            WorkloadKind::ReplicaSet => "ReplicaSet",
            WorkloadKind::ReplicationController => "ReplicationController",
        }
    }

    fn workload_scale_supported(kind: WorkloadKind) -> bool {
        match kind {
            WorkloadKind::DaemonSet => false,
            WorkloadKind::Deployment
            | WorkloadKind::StatefulSet
            | WorkloadKind::ReplicaSet
            | WorkloadKind::ReplicationController => true,
        }
    }

    fn selected_workload(&self, kind: WorkloadKind) -> Option<(String, String)> {
        match kind {
            WorkloadKind::Deployment => self.selected_deployment.clone(),
            WorkloadKind::DaemonSet => self.selected_daemonset.clone(),
            WorkloadKind::StatefulSet => self.selected_statefulset.clone(),
            WorkloadKind::ReplicaSet => self.selected_replicaset.clone(),
            WorkloadKind::ReplicationController => self.selected_replicationcontroller.clone(),
        }
    }

    fn clear_selected_workload(&mut self, kind: WorkloadKind) {
        match kind {
            WorkloadKind::Deployment => {
                self.selected_deployment = None;
                self.deployment_details = None;
            }
            WorkloadKind::DaemonSet => {
                self.selected_daemonset = None;
                self.daemonset_details = None;
            }
            WorkloadKind::StatefulSet => {
                self.selected_statefulset = None;
                self.statefulset_details = None;
            }
            WorkloadKind::ReplicaSet => {
                self.selected_replicaset = None;
                self.replicaset_details = None;
            }
            WorkloadKind::ReplicationController => {
                self.selected_replicationcontroller = None;
                self.replicationcontroller_details = None;
            }
        }
    }

    fn workload_summaries(&self, kind: WorkloadKind) -> &Vec<DeploymentSummary> {
        match kind {
            WorkloadKind::Deployment => &self.deployments,
            WorkloadKind::DaemonSet => &self.daemonsets,
            WorkloadKind::StatefulSet => &self.statefulsets,
            WorkloadKind::ReplicaSet => &self.replicasets,
            WorkloadKind::ReplicationController => &self.replicationcontrollers,
        }
    }

    fn render_workload_right_panel(&mut self, ctx: &egui::Context, kind: WorkloadKind) {
        let Some((ns, name)) = self.selected_workload(kind) else {
            return;
        };

        let title = Self::workload_title(kind);
        let scale_supported = Self::workload_scale_supported(kind);

        // Best-effort: find the current summary for scale defaults.
        let summary = self
            .workload_summaries(kind)
            .iter()
            .find(|d| d.namespace == ns && d.name == name)
            .cloned();

        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(title);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Delete").clicked() {
                            self.open_confirm(
                                format!("Delete {}?", title.to_lowercase()),
                                format!("Delete {}/{}?", ns, name),
                                "Delete",
                                ConfirmAction::DeleteWorkload {
                                    kind,
                                    namespace: ns.clone(),
                                    name: name.clone(),
                                },
                            );
                        }

                        if ui.button("Restart").clicked() {
                            self.open_confirm(
                                format!("Restart {}?", title.to_lowercase()),
                                format!("This will trigger a restart attempt for {}/{}.", ns, name),
                                "Restart",
                                ConfirmAction::RestartWorkload {
                                    kind,
                                    namespace: ns.clone(),
                                    name: name.clone(),
                                },
                            );
                        }

                        if scale_supported {
                            if ui.button("Scale").clicked() {
                                let (current_ready, desired) = summary
                                    .as_ref()
                                    .map(|d| (d.ready, d.desired))
                                    .unwrap_or((0, 1));
                                self.open_scale_workload(
                                    kind,
                                    ns.clone(),
                                    name.clone(),
                                    current_ready,
                                    desired,
                                );
                            }
                        } else {
                            ui.add_enabled(false, egui::Button::new("Scale"));
                        }

                        if ui.button("Close").clicked() {
                            self.clear_selected_workload(kind);
                        }
                    });
                });

                ui.label(format!("{}/{}", ns, name));
                ui.separator();
                self.render_workload_details(ui, kind);
            });
    }

    fn workload_details(&self, kind: WorkloadKind) -> Option<DeploymentDetails> {
        match kind {
            WorkloadKind::Deployment => self.deployment_details.clone(),
            WorkloadKind::DaemonSet => self.daemonset_details.clone(),
            WorkloadKind::StatefulSet => self.statefulset_details.clone(),
            WorkloadKind::ReplicaSet => self.replicaset_details.clone(),
            WorkloadKind::ReplicationController => self.replicationcontroller_details.clone(),
        }
    }

    fn clear_workload_details(&mut self, kind: WorkloadKind) {
        match kind {
            WorkloadKind::Deployment => self.deployment_details = None,
            WorkloadKind::DaemonSet => self.daemonset_details = None,
            WorkloadKind::StatefulSet => self.statefulset_details = None,
            WorkloadKind::ReplicaSet => self.replicaset_details = None,
            WorkloadKind::ReplicationController => self.replicationcontroller_details = None,
        }
    }

    fn send_select_workload(&mut self, kind: WorkloadKind, namespace: String, name: String) {
        match kind {
            WorkloadKind::Deployment => self.send(BackendCmd::SelectDeployment { namespace, name }),
            WorkloadKind::DaemonSet => self.send(BackendCmd::SelectDaemonSet { namespace, name }),
            WorkloadKind::StatefulSet => {
                self.send(BackendCmd::SelectStatefulSet { namespace, name })
            }
            WorkloadKind::ReplicaSet => self.send(BackendCmd::SelectReplicaSet { namespace, name }),
            WorkloadKind::ReplicationController => {
                self.send(BackendCmd::SelectReplicationController { namespace, name })
            }
        }
    }

    fn render_workload_details(&mut self, ui: &mut egui::Ui, kind: WorkloadKind) {
        let details = self.workload_details(kind);
        let title = Self::workload_title(kind);

        match details {
            Some(details) => {
                ui.monospace(format!("Created: {}", details.creation_timestamp));
                ui.add_space(6.0);

                ui.label("Containers:");
                for c in &details.containers {
                    ui.monospace(format!("- {}", c));
                }

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

                ui.add_space(10.0);
                let mut refresh_clicked = false;
                let mut open_pod: Option<(String, String)> = None;
                let mut delete_pod: Option<(String, String)> = None;

                ui.collapsing("Associated Pods", |ui| {
                    if details.pod_selector.is_empty() {
                        ui.label("No selector derived (missing matchLabels).");
                        return;
                    }

                    ui.monospace(format!("selector: {}", details.pod_selector));
                    ui.add_space(6.0);

                    refresh_clicked = ui.button("Refresh").clicked();

                    ui.add_space(6.0);
                    if details.pods.is_empty() {
                        ui.label("No matching pods.");
                        return;
                    }

                    for p in &details.pods {
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.monospace(format!(
                                "{}  {}  {}/{}",
                                p.name, p.phase, p.ready, p.total
                            ));

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("Delete").clicked() {
                                        delete_pod = Some((p.namespace.clone(), p.name.clone()));
                                    }

                                    if ui.button("Open").clicked() {
                                        open_pod = Some((p.namespace.clone(), p.name.clone()));
                                    }
                                },
                            );
                        });
                    }
                });

                if refresh_clicked {
                    self.clear_workload_details(kind);
                    self.send_select_workload(
                        kind,
                        details.namespace.clone(),
                        details.name.clone(),
                    );
                }

                if let Some((ns, name)) = delete_pod {
                    self.open_confirm(
                        "Delete pod?",
                        format!("Delete {}/{}?", ns, name),
                        "Delete",
                        ConfirmAction::DeletePod {
                            namespace: ns,
                            name,
                        },
                    );
                }

                if let Some((ns, name)) = open_pod {
                    // Reuse the Pods section right panel for full pod actions.
                    self.active_section = section_keys::workloads_pods();
                    self.selected_pod = Some((ns.clone(), name.clone()));
                    self.pod_details = None;
                    self.pod_metrics = None;
                    self.pod_logs.clear();
                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                    self.send(BackendCmd::SelectPod {
                        namespace: ns,
                        name,
                    });
                }

                ui.add_space(10.0);
                ui.collapsing("Raw JSON", |ui| {
                    let height = ui.available_height().max(250.0);
                    egui::ScrollArea::vertical()
                        .max_height(height)
                        .show(ui, |ui| {
                            ui.monospace(&details.raw_json);
                        });
                });
            }
            None => {
                ui.label(format!(
                    "Select a {} to view details.",
                    title.to_lowercase()
                ));
            }
        }
    }

    fn render_pod_details(&mut self, ui: &mut egui::Ui) {
        match &self.pod_details {
            Some(details) => {
                ui.monospace(format!("Created: {}", details.creation_timestamp));
                ui.add_space(6.0);

                ui.label("Containers:");
                for c in &details.containers {
                    ui.monospace(format!("- {}", c));
                }

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

                ui.add_space(10.0);
                ui.collapsing("Raw JSON", |ui| {
                    let height = ui.available_height().max(250.0);
                    egui::ScrollArea::vertical()
                        .max_height(height)
                        .show(ui, |ui| {
                            ui.monospace(&details.raw_json);
                        });
                });
            }
            None => {
                ui.label("Select a pod to view details.");
            }
        }
    }

    fn render_pod_logs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let enabled = self.selected_pod.is_some();
            if ui
                .add_enabled(enabled, egui::Button::new("Start"))
                .clicked()
            {
                if let Some((ns, name)) = &self.selected_pod {
                    self.pod_logs.clear();
                    self.send(BackendCmd::StartPodLogs {
                        namespace: ns.clone(),
                        name: name.clone(),
                        container: None,
                    });
                }
            }

            if ui.button("Stop").clicked() {
                self.send(BackendCmd::StopPodLogs);
            }

            if ui.button("Clear").clicked() {
                self.pod_logs.clear();
            }
        });

        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for line in &self.pod_logs {
                ui.monospace(line);
            }
        });
    }

    fn render_pod_metrics(&mut self, ui: &mut egui::Ui) {
        // Auto-fetch once per selection when opening the Metrics tab.
        if !self.pod_metrics_pending {
            if let Some((ns, name)) = &self.selected_pod {
                // If we have no metrics yet (or they are for a different pod), request them.
                let needs_fetch = self
                    .pod_metrics
                    .as_ref()
                    .is_none_or(|m| m.namespace != *ns || m.name != *name);
                if needs_fetch {
                    self.pod_metrics = None;
                    self.pod_metrics_pending = true;
                    self.send(BackendCmd::FetchPodMetrics {
                        namespace: ns.clone(),
                        name: name.clone(),
                    });
                }
            }
        }

        ui.horizontal(|ui| {
            let enabled = self.selected_pod.is_some();
            if ui
                .add_enabled(enabled, egui::Button::new("Refresh Metrics"))
                .clicked()
            {
                if let Some((ns, name)) = &self.selected_pod {
                    self.pod_metrics = None;
                    self.pod_metrics_pending = true;
                    self.send(BackendCmd::FetchPodMetrics {
                        namespace: ns.clone(),
                        name: name.clone(),
                    });
                }
            }
        });

        ui.add_space(8.0);

        match &self.pod_metrics {
            Some(metrics) => {
                self.pod_metrics_pending = false;
                if metrics.containers.is_empty() {
                    ui.label(
                        "No metrics returned (metrics-server may be missing or RBAC denies access).",
                    );
                } else {
                    ui.label("Containers:");
                    for c in &metrics.containers {
                        ui.monospace(format!("{}  cpu={}  mem={} ", c.name, c.cpu, c.memory));
                    }
                }
            }
            None => {
                if self.selected_pod.is_some() {
                    if self.pod_metrics_pending {
                        ui.label("Fetching metrics...");
                    } else {
                        ui.label("Click Refresh Metrics to retry.");
                    }
                } else {
                    ui.label("Select a pod to view metrics.");
                }
            }
        }
    }

    fn render_pod_exec(&mut self, ui: &mut egui::Ui) {
        ui.label("Terminal/exec UI is planned, not wired yet.");
        ui.add_enabled(false, egui::Button::new("Open Exec Session"));
    }
}
