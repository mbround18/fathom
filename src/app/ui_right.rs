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
            self.render_deployment_right_panel(ctx);
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

    fn render_deployment_right_panel(&mut self, ctx: &egui::Context) {
        let Some((ns, name)) = self.selected_deployment.clone() else {
            return;
        };

        // Best-effort: find the current summary for scale defaults.
        let summary = self
            .deployments
            .iter()
            .find(|d| d.namespace == ns && d.name == name)
            .cloned();

        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Deployment");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Delete").clicked() {
                            self.open_confirm(
                                "Delete deployment?",
                                format!("Delete {}/{}?", ns, name),
                                "Delete",
                                ConfirmAction::DeleteDeployment {
                                    namespace: ns.clone(),
                                    name: name.clone(),
                                },
                            );
                        }

                        if ui.button("Restart").clicked() {
                            self.open_confirm(
                                "Restart deployment?",
                                format!("This will trigger a rollout for {}/{}.", ns, name),
                                "Restart",
                                ConfirmAction::RestartDeployment {
                                    namespace: ns.clone(),
                                    name: name.clone(),
                                },
                            );
                        }

                        if ui.button("Scale").clicked() {
                            let (current_ready, desired) = summary
                                .as_ref()
                                .map(|d| (d.ready, d.desired))
                                .unwrap_or((0, 1));
                            self.open_scale_deployment(
                                ns.clone(),
                                name.clone(),
                                current_ready,
                                desired,
                            );
                        }

                        if ui.button("Close").clicked() {
                            self.selected_deployment = None;
                            self.deployment_details = None;
                        }
                    });
                });

                ui.label(format!("{}/{}", ns, name));
                ui.separator();
                self.render_deployment_details(ui);
            });
    }

    fn render_deployment_details(&mut self, ui: &mut egui::Ui) {
        let details = self.deployment_details.clone();

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
                    self.deployment_details = None;
                    self.send(BackendCmd::SelectDeployment {
                        namespace: details.namespace.clone(),
                        name: details.name.clone(),
                    });
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
                ui.label("Select a deployment to view details.");
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
        ui.horizontal(|ui| {
            let enabled = self.selected_pod.is_some();
            if ui
                .add_enabled(enabled, egui::Button::new("Refresh Metrics"))
                .clicked()
            {
                if let Some((ns, name)) = &self.selected_pod {
                    self.pod_metrics = None;
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
                ui.label("Select a pod, then click Refresh Metrics.");
            }
        }
    }

    fn render_pod_exec(&mut self, ui: &mut egui::Ui) {
        ui.label("Terminal/exec UI is planned, not wired yet.");
        ui.add_enabled(false, egui::Button::new("Open Exec Session"));
    }
}
