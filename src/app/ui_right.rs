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
                egui::ScrollArea::vertical().show(ui, |ui| {
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
                                self.pod_metrics_history = None;
                                self.pod_metrics_pending = false;
                                self.pod_metrics_polling_started = false;
                                self.pod_logs.clear();
                                self.send(BackendCmd::StopPodLogs);
                                self.send(BackendCmd::StopMetricsPolling);
                            }
                        });
                    });

                    ui.horizontal(|ui| {
                        if ui.link(&ns).clicked() {
                            self.toggle_namespace_filter_from_link(&ns);
                        }
                        ui.label(format!("/{}", name));
                    });

                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.pod_tab, PodTab::Details, "Details");
                        ui.selectable_value(&mut self.pod_tab, PodTab::Logs, "Logs");
                        ui.selectable_value(&mut self.pod_tab, PodTab::Exec, "Exec");
                    });

                    // Auto-start metrics polling when sidebar opens (only once per pod)
                    if !self.pod_metrics_polling_started {
                        self.pod_metrics_polling_started = true;
                        self.send(BackendCmd::StartMetricsPolling {
                            namespace: ns.clone(),
                            name: name.clone(),
                            interval_secs: 15,
                        });
                    }

                    ui.add_space(8.0);

                    match self.pod_tab {
                        PodTab::Details => self.render_pod_details(ui),
                        PodTab::Logs => self.render_pod_logs(ui),
                        PodTab::Exec => self.render_pod_exec(ui),
                    }
                });
            });
    }

    fn workload_title(kind: WorkloadKind) -> &'static str {
        match kind {
            WorkloadKind::Deployment => "Deployment",
            WorkloadKind::DaemonSet => "DaemonSet",
            WorkloadKind::StatefulSet => "StatefulSet",
            WorkloadKind::ReplicaSet => "ReplicaSet",
            WorkloadKind::ReplicationController => "ReplicationController",
            WorkloadKind::Job => "Job",
            WorkloadKind::CronJob => "CronJob",
        }
    }

    fn workload_scale_supported(kind: WorkloadKind) -> bool {
        match kind {
            WorkloadKind::DaemonSet | WorkloadKind::Job | WorkloadKind::CronJob => false,
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
            WorkloadKind::Job => self.selected_job.clone(),
            WorkloadKind::CronJob => self.selected_cronjob.clone(),
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
            WorkloadKind::Job => {
                self.selected_job = None;
                self.job_details = None;
            }
            WorkloadKind::CronJob => {
                self.selected_cronjob = None;
                self.cronjob_details = None;
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
            WorkloadKind::Job | WorkloadKind::CronJob => {
                // Job and CronJob use different summary types and should not call this method
                panic!("workload_summaries called with Job or CronJob kind")
            }
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
                egui::ScrollArea::vertical().show(ui, |ui| {
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

                    ui.horizontal(|ui| {
                        if ui.link(&ns).clicked() {
                            self.toggle_namespace_filter_from_link(&ns);
                        }
                        ui.label(format!("/{}", name));
                    });
                    ui.separator();
                    self.render_workload_details(ui, kind);
                });
            });
    }

    fn workload_details(&self, kind: WorkloadKind) -> Option<DeploymentDetails> {
        match kind {
            WorkloadKind::Deployment => self.deployment_details.clone(),
            WorkloadKind::DaemonSet => self.daemonset_details.clone(),
            WorkloadKind::StatefulSet => self.statefulset_details.clone(),
            WorkloadKind::ReplicaSet => self.replicaset_details.clone(),
            WorkloadKind::ReplicationController => self.replicationcontroller_details.clone(),
            WorkloadKind::Job | WorkloadKind::CronJob => {
                // Job and CronJob use different details types and should not call this method
                panic!("workload_details called with Job or CronJob kind")
            }
        }
    }

    fn clear_workload_details(&mut self, kind: WorkloadKind) {
        match kind {
            WorkloadKind::Deployment => self.deployment_details = None,
            WorkloadKind::DaemonSet => self.daemonset_details = None,
            WorkloadKind::StatefulSet => self.statefulset_details = None,
            WorkloadKind::ReplicaSet => self.replicaset_details = None,
            WorkloadKind::ReplicationController => self.replicationcontroller_details = None,
            WorkloadKind::Job => self.job_details = None,
            WorkloadKind::CronJob => self.cronjob_details = None,
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
            WorkloadKind::Job => self.send(BackendCmd::SelectJob { namespace, name }),
            WorkloadKind::CronJob => self.send(BackendCmd::SelectCronJob { namespace, name }),
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
                    self.pod_metrics_pending = true;
                    self.pod_logs.clear();
                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                    self.send(BackendCmd::SelectPod {
                        namespace: ns,
                        name,
                    });

                    if let Some((mns, mname)) = &self.selected_pod {
                        self.send(BackendCmd::FetchPodMetrics {
                            namespace: mns.clone(),
                            name: mname.clone(),
                        });
                    }
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
                let mut open_owner: Option<(String, String, String)> = None;

                // Display historical metrics with graphs
                if let Some(history) = &self.pod_metrics_history {
                    ui.label("Metrics History:");
                    ui.add_space(4.0);
                    for container in &history.containers {
                        ui.collapsing(&container.name, |ui| {
                            // CPU graph
                            self.render_metric_graph(
                                ui,
                                &container.name,
                                "cpu",
                                &container.samples,
                                container.cpu_request_millicores.map(|v| v as f64),
                                container.cpu_limit_millicores.map(|v| v as f64),
                                "m",
                            );
                            ui.add_space(8.0);

                            // Memory graph
                            self.render_metric_graph(
                                ui,
                                &container.name,
                                "memory",
                                &container.samples,
                                container.memory_request_bytes.map(|v| v as f64),
                                container.memory_limit_bytes.map(|v| v as f64),
                                "MB",
                            );
                        });
                        ui.add_space(12.0);
                    }
                } else {
                    ui.label("Metrics: (fetching...)");
                }

                ui.separator();
                ui.add_space(8.0);

                // Owner interlinking.
                if let Some(owner) = details.resolved_owner.as_ref().or(details.owner.as_ref()) {
                    ui.horizontal(|ui| {
                        ui.label("Owner:");
                        let label = format!("{}/{}", owner.kind, owner.name);
                        let clicked = ui.link(label).clicked();
                        if clicked {
                            open_owner = Some((
                                details.namespace.clone(),
                                owner.kind.clone(),
                                owner.name.clone(),
                            ));
                        }
                    });
                    ui.add_space(8.0);
                }

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

                if let Some((ns, kind, name)) = open_owner {
                    self.open_owner_from_pod(ns, kind, name);
                }
            }
            None => {
                ui.label("Select a pod to view details.");
            }
        }
    }

    fn open_owner_from_pod(&mut self, namespace: String, kind: String, name: String) {
        let target_kind = match kind.as_str() {
            "Deployment" => Some(WorkloadKind::Deployment),
            "DaemonSet" => Some(WorkloadKind::DaemonSet),
            "StatefulSet" => Some(WorkloadKind::StatefulSet),
            "ReplicaSet" => Some(WorkloadKind::ReplicaSet),
            "ReplicationController" => Some(WorkloadKind::ReplicationController),
            "Job" => Some(WorkloadKind::Job),
            "CronJob" => Some(WorkloadKind::CronJob),
            _ => None,
        };

        let Some(target_kind) = target_kind else {
            return;
        };

        // Switch section and select.
        self.active_section = match target_kind {
            WorkloadKind::Deployment => section_keys::workloads_deployments(),
            WorkloadKind::DaemonSet => section_keys::workloads_daemonsets(),
            WorkloadKind::StatefulSet => section_keys::workloads_statefulsets(),
            WorkloadKind::ReplicaSet => section_keys::workloads_replicasets(),
            WorkloadKind::ReplicationController => section_keys::workloads_replicationcontrollers(),
            WorkloadKind::Job => section_keys::workloads_jobs(),
            WorkloadKind::CronJob => section_keys::workloads_cronjobs(),
        };

        // Clear previous selections for consistency.
        self.selected_pod = None;
        self.pod_details = None;
        self.pod_metrics = None;
        self.pod_metrics_pending = false;
        self.pod_logs.clear();
        self.send(BackendCmd::StopPodLogs);

        self.send(BackendCmd::SetActiveSection {
            section: self.active_section.clone(),
        });

        match target_kind {
            WorkloadKind::Deployment => {
                self.selected_deployment = Some((namespace.clone(), name.clone()));
                self.deployment_details = None;
                self.send(BackendCmd::SelectDeployment { namespace, name });
            }
            WorkloadKind::DaemonSet => {
                self.selected_daemonset = Some((namespace.clone(), name.clone()));
                self.daemonset_details = None;
                self.send(BackendCmd::SelectDaemonSet { namespace, name });
            }
            WorkloadKind::StatefulSet => {
                self.selected_statefulset = Some((namespace.clone(), name.clone()));
                self.statefulset_details = None;
                self.send(BackendCmd::SelectStatefulSet { namespace, name });
            }
            WorkloadKind::ReplicaSet => {
                self.selected_replicaset = Some((namespace.clone(), name.clone()));
                self.replicaset_details = None;
                self.send(BackendCmd::SelectReplicaSet { namespace, name });
            }
            WorkloadKind::ReplicationController => {
                self.selected_replicationcontroller = Some((namespace.clone(), name.clone()));
                self.replicationcontroller_details = None;
                self.send(BackendCmd::SelectReplicationController { namespace, name });
            }
            WorkloadKind::Job => {
                self.selected_job = Some((namespace.clone(), name.clone()));
                self.job_details = None;
                self.send(BackendCmd::SelectJob { namespace, name });
            }
            WorkloadKind::CronJob => {
                self.selected_cronjob = Some((namespace.clone(), name.clone()));
                self.cronjob_details = None;
                self.send(BackendCmd::SelectCronJob { namespace, name });
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

            // Start polling button
            let polling_enabled = self.selected_pod.is_some() && self.pod_metrics_history.is_none();
            if ui
                .add_enabled(
                    polling_enabled,
                    egui::Button::new("📊 Start Metrics Polling"),
                )
                .clicked()
            {
                if let Some((ns, name)) = &self.selected_pod {
                    self.send(BackendCmd::StartMetricsPolling {
                        namespace: ns.clone(),
                        name: name.clone(),
                        interval_secs: 15,
                    });
                }
            }

            // Stop polling button
            let stop_enabled = self.pod_metrics_history.is_some();
            if ui
                .add_enabled(stop_enabled, egui::Button::new("⏹ Stop Polling"))
                .clicked()
            {
                self.send(BackendCmd::StopMetricsPolling);
                self.pod_metrics_history = None;
            }
        });

        ui.add_space(8.0);

        // Show current metrics summary
        match &self.pod_metrics {
            Some(metrics) => {
                self.pod_metrics_pending = false;
                if metrics.containers.is_empty() {
                    ui.label(
                        "No metrics returned (metrics-server may be missing or RBAC denies access).",
                    );
                } else {
                    ui.label("Current metrics:");
                    for c in &metrics.containers {
                        ui.monospace(format!("{}  cpu={}  mem={}", c.name, c.cpu, c.memory));
                    }
                    ui.add_space(8.0);
                }
            }
            None => {
                if self.selected_pod.is_some() {
                    if self.pod_metrics_pending {
                        ui.label("Fetching metrics...");
                    } else {
                        ui.label("Click Refresh Metrics to see current values.");
                    }
                } else {
                    ui.label("Select a pod to view metrics.");
                }
            }
        }

        // Show historical graphs if polling is active
        if let Some(history) = &self.pod_metrics_history {
            ui.add_space(8.0);
            ui.separator();
            ui.label(format!(
                "Historical metrics ({} samples)",
                history
                    .containers
                    .first()
                    .map(|c| c.samples.len())
                    .unwrap_or(0)
            ));

            for container_history in &history.containers {
                if container_history.samples.is_empty() {
                    continue;
                }

                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label(format!("Container: {}", container_history.name));

                        // CPU Graph
                        self.render_metric_graph(
                            ui,
                            &container_history.name,
                            "cpu",
                            &container_history.samples,
                            container_history.cpu_request_millicores,
                            container_history.cpu_limit_millicores,
                            "millicores",
                        );

                        ui.add_space(4.0);

                        // Memory Graph
                        self.render_metric_graph(
                            ui,
                            &container_history.name,
                            "memory",
                            &container_history.samples,
                            container_history.memory_request_bytes,
                            container_history.memory_limit_bytes,
                            "bytes",
                        );
                    });
                });
            }
        }
    }

    fn render_metric_graph(
        &self,
        ui: &mut egui::Ui,
        container: &str,
        metric_type: &str,
        samples: &[crate::protocol::MetricsSample],
        request: Option<f64>,
        limit: Option<f64>,
        unit: &str,
    ) {
        use egui_plot::{Line, Plot, PlotPoints};

        if samples.is_empty() {
            return;
        }

        let (_label, values): (String, Vec<[f64; 2]>) = if metric_type == "cpu" {
            (
                "CPU Usage (millicores)".to_string(),
                samples
                    .iter()
                    .enumerate()
                    .map(|(i, s)| [i as f64, s.cpu_millicores])
                    .collect(),
            )
        } else {
            (
                format!("Memory Usage ({})", unit),
                samples
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let val = if unit == "bytes" {
                            s.memory_bytes
                        } else {
                            s.memory_bytes / 1_000_000.0 // Show in MB
                        };
                        [i as f64, val]
                    })
                    .collect(),
            )
        };

        // Get current value before closure captures values
        let current_value = values.last().map(|[_, v]| *v);

        let plot = Plot::new(format!("{}-{}-{}", container, metric_type, unit))
            .height(150.0)
            .show_grid(true)
            .allow_zoom(false);

        plot.show(ui, |plot_ui| {
            // Usage line
            let points: PlotPoints = values.iter().copied().collect();
            let usage_line = Line::new("Usage", points).color(egui::Color32::from_rgb(0, 150, 200));
            plot_ui.line(usage_line);

            // Request line
            if let Some(req) = request {
                let req_points: PlotPoints = vec![[0.0, req], [samples.len() as f64 - 1.0, req]]
                    .into_iter()
                    .collect();
                let request_line = Line::new("Request", req_points)
                    .color(egui::Color32::from_rgb(100, 200, 100))
                    .style(egui_plot::LineStyle::dashed_loose());
                plot_ui.line(request_line);
            }

            // Limit line
            if let Some(lim) = limit {
                let lim_points: PlotPoints = vec![[0.0, lim], [samples.len() as f64 - 1.0, lim]]
                    .into_iter()
                    .collect();
                let limit_line = Line::new("Limit", lim_points)
                    .color(egui::Color32::from_rgb(200, 100, 100))
                    .style(egui_plot::LineStyle::dashed_loose());
                plot_ui.line(limit_line);
            }
        });

        // Show current/request/limit summary
        if let Some(current) = current_value {
            let summary = if metric_type == "cpu" {
                format!(
                    "Current: {:.1}m | Request: {} | Limit: {}",
                    current,
                    request.map(|r| format!("{:.0}m", r)).unwrap_or_default(),
                    limit.map(|l| format!("{:.0}m", l)).unwrap_or_default()
                )
            } else {
                let mb_current = current / 1_000_000.0;
                format!(
                    "Current: {:.1} MB | Request: {} | Limit: {}",
                    mb_current,
                    request
                        .map(|r| format!("{:.0} MB", r / 1_000_000.0))
                        .unwrap_or_default(),
                    limit
                        .map(|l| format!("{:.0} MB", l / 1_000_000.0))
                        .unwrap_or_default()
                )
            };
            ui.small(summary);
        }
    }

    fn render_pod_exec(&mut self, ui: &mut egui::Ui) {
        ui.label("Terminal/exec UI is planned, not wired yet.");
        ui.add_enabled(false, egui::Button::new("Open Exec Session"));
    }
}
