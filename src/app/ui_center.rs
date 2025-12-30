use super::*;

impl FathomApp {
    fn sort_indicator(&self, key: PodsSortKey) -> &'static str {
        if self.pods_sort.key != key {
            return "";
        }
        match self.pods_sort.dir {
            SortDir::Asc => " ▲",
            SortDir::Desc => " ▼",
        }
    }

    fn toggle_sort(&mut self, key: PodsSortKey) {
        if self.pods_sort.key == key {
            self.pods_sort.dir = match self.pods_sort.dir {
                SortDir::Asc => SortDir::Desc,
                SortDir::Desc => SortDir::Asc,
            };
        } else {
            self.pods_sort.key = key;
            self.pods_sort.dir = SortDir::Asc;
        }
    }

    fn render_workload_grid(
        ui: &mut egui::Ui,
        title: &str,
        grid_id: &str,
        workloads: &[DeploymentSummary],
        selected: &Option<(String, String)>,
    ) -> Option<(String, String)> {
        ui.heading(title);
        ui.separator();

        let mut newly_selected: Option<(String, String)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new(grid_id)
                .striped(true)
                .min_col_width(120.0)
                .show(ui, |ui| {
                    ui.label("Namespace");
                    ui.label("Name");
                    ui.label("Ready");
                    ui.label("Updated");
                    ui.label("Available");
                    ui.label("Desired");
                    ui.end_row();

                    for w in workloads {
                        let is_selected = selected
                            .as_ref()
                            .is_some_and(|(ns, name)| ns == &w.namespace && name == &w.name);

                        let ns_clicked = ui.selectable_label(is_selected, &w.namespace).clicked();
                        let name_clicked = ui.selectable_label(is_selected, &w.name).clicked();
                        let row_clicked = ns_clicked || name_clicked;

                        ui.label(w.ready.to_string());
                        ui.label(w.updated.to_string());
                        ui.label(w.available.to_string());
                        ui.label(w.desired.to_string());
                        ui.end_row();

                        if row_clicked {
                            newly_selected = Some((w.namespace.clone(), w.name.clone()));
                        }
                    }
                });
        });

        newly_selected
    }

    pub(super) fn render_center(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.active_section == section_keys::overview() {
                self.render_overview_center(ui);
                return;
            }

            if self.active_section == section_keys::workloads_pods() {
                self.render_pods_center(ui);
                return;
            }

            if self.active_section == section_keys::workloads_deployments() {
                self.render_deployments_center(ui);
                return;
            }

            if self.active_section == section_keys::workloads_daemonsets() {
                self.render_daemonsets_center(ui);
                return;
            }

            if self.active_section == section_keys::workloads_statefulsets() {
                self.render_statefulsets_center(ui);
                return;
            }

            if self.active_section == section_keys::workloads_replicasets() {
                self.render_replicasets_center(ui);
                return;
            }

            if self.active_section == section_keys::workloads_replicationcontrollers() {
                self.render_replicationcontrollers_center(ui);
                return;
            }

            if self.active_section == section_keys::workloads_jobs() {
                self.render_jobs_center(ui);
                return;
            }

            if self.active_section == section_keys::workloads_cronjobs() {
                self.render_cronjobs_center(ui);
                return;
            }

            if self.active_section == section_keys::network_portforwards() {
                self.render_portforwards_center(ui);
                return;
            }

            if self.active_section == section_keys::integrations_metrics() {
                self.render_metrics_integration_center(ui);
                return;
            }

            if self.active_section == section_keys::integrations_grafana() {
                self.render_grafana_integration_center(ui);
                return;
            }

            if self.active_section == section_keys::integrations_argo() {
                self.render_argo_integration_center(ui);
                return;
            }

            if self.active_section == section_keys::integrations_flux() {
                self.render_flux_integration_center(ui);
                return;
            }

            ui.heading("Not implemented");
            ui.separator();
            ui.label("This section is stubbed.");
        });
    }

    fn render_pods_center(&mut self, ui: &mut egui::Ui) {
        ui.heading("Pods");
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("Search:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.pod_search)
                    .hint_text("namespace, name, phase, node…"),
            );
            if resp.changed() {
                // purely local filter
            }
            if ui.button("Clear").clicked() {
                self.pod_search.clear();
            }

            ui.add_space(12.0);
            ui.separator();
            ui.label("Sort:");

            egui::ComboBox::from_id_salt("pods_sort_key")
                .selected_text(match self.pods_sort.key {
                    PodsSortKey::Namespace => "Namespace",
                    PodsSortKey::Name => "Name",
                    PodsSortKey::Phase => "Phase",
                    PodsSortKey::Ready => "Ready",
                    PodsSortKey::Restarts => "Restarts",
                    PodsSortKey::Node => "Node",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.pods_sort.key,
                        PodsSortKey::Namespace,
                        "Namespace",
                    );
                    ui.selectable_value(&mut self.pods_sort.key, PodsSortKey::Name, "Name");
                    ui.selectable_value(&mut self.pods_sort.key, PodsSortKey::Phase, "Phase");
                    ui.selectable_value(&mut self.pods_sort.key, PodsSortKey::Ready, "Ready");
                    ui.selectable_value(&mut self.pods_sort.key, PodsSortKey::Restarts, "Restarts");
                    ui.selectable_value(&mut self.pods_sort.key, PodsSortKey::Node, "Node");
                });

            let dir_text = match self.pods_sort.dir {
                SortDir::Asc => "Asc",
                SortDir::Desc => "Desc",
            };
            if ui.button(dir_text).clicked() {
                self.pods_sort.dir = match self.pods_sort.dir {
                    SortDir::Asc => SortDir::Desc,
                    SortDir::Desc => SortDir::Asc,
                };
            }
        });

        ui.add_space(6.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            let search = self.pod_search.clone();
            let sort = self.pods_sort;
            let mut pods: Vec<PodSummary> = self
                .pods
                .iter()
                .filter(|p| self.pod_matches_search(p, &search))
                .cloned()
                .collect();

            pods.sort_by(|a, b| {
                use std::cmp::Ordering;
                let primary = match sort.key {
                    PodsSortKey::Namespace => a.namespace.cmp(&b.namespace),
                    PodsSortKey::Name => a.name.cmp(&b.name),
                    PodsSortKey::Phase => a.phase.cmp(&b.phase),
                    PodsSortKey::Ready => (a.ready, a.total).cmp(&(b.ready, b.total)),
                    PodsSortKey::Restarts => a.restarts.cmp(&b.restarts),
                    PodsSortKey::Node => a.node.cmp(&b.node),
                };

                let primary = match sort.dir {
                    SortDir::Asc => primary,
                    SortDir::Desc => primary.reverse(),
                };

                if primary != Ordering::Equal {
                    return primary;
                }
                a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name))
            });

            egui::Grid::new("pods_grid")
                .striped(true)
                .min_col_width(120.0)
                .show(ui, |ui| {
                    if ui
                        .button(format!(
                            "Namespace{}",
                            self.sort_indicator(PodsSortKey::Namespace)
                        ))
                        .clicked()
                    {
                        self.toggle_sort(PodsSortKey::Namespace);
                    }
                    if ui
                        .button(format!("Name{}", self.sort_indicator(PodsSortKey::Name)))
                        .clicked()
                    {
                        self.toggle_sort(PodsSortKey::Name);
                    }
                    if ui
                        .button(format!("Phase{}", self.sort_indicator(PodsSortKey::Phase)))
                        .clicked()
                    {
                        self.toggle_sort(PodsSortKey::Phase);
                    }
                    if ui
                        .button(format!("Ready{}", self.sort_indicator(PodsSortKey::Ready)))
                        .clicked()
                    {
                        self.toggle_sort(PodsSortKey::Ready);
                    }
                    if ui
                        .button(format!(
                            "Restarts{}",
                            self.sort_indicator(PodsSortKey::Restarts)
                        ))
                        .clicked()
                    {
                        self.toggle_sort(PodsSortKey::Restarts);
                    }
                    if ui
                        .button(format!("Node{}", self.sort_indicator(PodsSortKey::Node)))
                        .clicked()
                    {
                        self.toggle_sort(PodsSortKey::Node);
                    }
                    ui.end_row();

                    let mut newly_selected: Option<(String, String)> = None;
                    for pod in &pods {
                        let is_selected = self
                            .selected_pod
                            .as_ref()
                            .is_some_and(|(ns, name)| ns == &pod.namespace && name == &pod.name);

                        let ns_clicked = ui.selectable_label(is_selected, &pod.namespace).clicked();
                        let name_clicked = ui.selectable_label(is_selected, &pod.name).clicked();
                        let row_clicked = ns_clicked || name_clicked;
                        ui.label(&pod.phase);
                        ui.label(format!("{}/{}", pod.ready, pod.total));
                        ui.label(pod.restarts.to_string());
                        ui.label(&pod.node);
                        ui.end_row();

                        if row_clicked {
                            newly_selected = Some((pod.namespace.clone(), pod.name.clone()));
                        }
                    }

                    if let Some((ns, name)) = newly_selected {
                        self.selected_pod = Some((ns.clone(), name.clone()));
                        self.pod_details = None;
                        self.pod_metrics = None;
                        self.pod_metrics_pending = false;
                        self.pod_logs.clear();

                        self.send(BackendCmd::SelectPod {
                            namespace: ns,
                            name,
                        });

                        // Metrics shown at top of details; fetch immediately.
                        if let Some((mns, mname)) = &self.selected_pod {
                            self.pod_metrics_pending = true;
                            self.send(BackendCmd::FetchPodMetrics {
                                namespace: mns.clone(),
                                name: mname.clone(),
                            });
                        }
                    }
                });
        });
    }

    fn render_deployments_center(&mut self, ui: &mut egui::Ui) {
        let deployments = self.deployments.clone();

        let newly_selected = Self::render_workload_grid(
            ui,
            "Deployments",
            "deployments_grid",
            &deployments,
            &self.selected_deployment,
        );

        if let Some((ns, name)) = newly_selected {
            self.selected_deployment = Some((ns.clone(), name.clone()));
            self.deployment_details = None;
            self.send(BackendCmd::SelectDeployment {
                namespace: ns,
                name,
            });
        }
    }

    fn render_daemonsets_center(&mut self, ui: &mut egui::Ui) {
        let daemonsets = self.daemonsets.clone();
        let newly_selected = Self::render_workload_grid(
            ui,
            "DaemonSets",
            "daemonsets_grid",
            &daemonsets,
            &self.selected_daemonset,
        );

        if let Some((ns, name)) = newly_selected {
            self.selected_daemonset = Some((ns.clone(), name.clone()));
            self.daemonset_details = None;
            self.send(BackendCmd::SelectDaemonSet {
                namespace: ns,
                name,
            });
        }
    }

    fn render_statefulsets_center(&mut self, ui: &mut egui::Ui) {
        let statefulsets = self.statefulsets.clone();
        let newly_selected = Self::render_workload_grid(
            ui,
            "StatefulSets",
            "statefulsets_grid",
            &statefulsets,
            &self.selected_statefulset,
        );

        if let Some((ns, name)) = newly_selected {
            self.selected_statefulset = Some((ns.clone(), name.clone()));
            self.statefulset_details = None;
            self.send(BackendCmd::SelectStatefulSet {
                namespace: ns,
                name,
            });
        }
    }

    fn render_replicasets_center(&mut self, ui: &mut egui::Ui) {
        let replicasets = self.replicasets.clone();
        let newly_selected = Self::render_workload_grid(
            ui,
            "ReplicaSets",
            "replicasets_grid",
            &replicasets,
            &self.selected_replicaset,
        );

        if let Some((ns, name)) = newly_selected {
            self.selected_replicaset = Some((ns.clone(), name.clone()));
            self.replicaset_details = None;
            self.send(BackendCmd::SelectReplicaSet {
                namespace: ns,
                name,
            });
        }
    }

    fn render_replicationcontrollers_center(&mut self, ui: &mut egui::Ui) {
        let rcs = self.replicationcontrollers.clone();
        let newly_selected = Self::render_workload_grid(
            ui,
            "ReplicationControllers",
            "replicationcontrollers_grid",
            &rcs,
            &self.selected_replicationcontroller,
        );

        if let Some((ns, name)) = newly_selected {
            self.selected_replicationcontroller = Some((ns.clone(), name.clone()));
            self.replicationcontroller_details = None;
            self.send(BackendCmd::SelectReplicationController {
                namespace: ns,
                name,
            });
        }
    }

    fn render_jobs_center(&mut self, ui: &mut egui::Ui) {
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
                        let is_selected = self.selected_job.as_ref()
                            == Some(&(job.namespace.clone(), job.name.clone()));
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
            self.selected_job = Some((ns.clone(), name.clone()));
            self.job_details = None;
            self.send(BackendCmd::SelectJob {
                namespace: ns,
                name,
            });
        }
    }

    fn render_cronjobs_center(&mut self, ui: &mut egui::Ui) {
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
                        let is_selected = self.selected_cronjob.as_ref()
                            == Some(&(cj.namespace.clone(), cj.name.clone()));
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
            self.selected_cronjob = Some((ns.clone(), name.clone()));
            self.cronjob_details = None;
            self.send(BackendCmd::SelectCronJob {
                namespace: ns,
                name,
            });
        }
    }

    fn render_overview_center(&mut self, ui: &mut egui::Ui) {
        ui.heading("Overview");
        ui.separator();

        ui.label("Resource status summaries:");
        ui.add_space(12.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            // Pods
            let pod_statuses = self.collect_pod_statuses();
            self.render_pod_status_box(ui, "Pods", &pod_statuses, section_keys::workloads_pods());
            ui.add_space(8.0);

            // Deployments
            let deploy_statuses = self.collect_workload_statuses(&self.deployments);
            self.render_workload_status_box_with_data(
                ui,
                "Deployments",
                &deploy_statuses,
                section_keys::workloads_deployments(),
            );
            ui.add_space(8.0);

            // DaemonSets
            let ds_statuses = self.collect_workload_statuses(&self.daemonsets);
            self.render_workload_status_box_with_data(
                ui,
                "DaemonSets",
                &ds_statuses,
                section_keys::workloads_daemonsets(),
            );
            ui.add_space(8.0);

            // StatefulSets
            let ss_statuses = self.collect_workload_statuses(&self.statefulsets);
            self.render_workload_status_box_with_data(
                ui,
                "StatefulSets",
                &ss_statuses,
                section_keys::workloads_statefulsets(),
            );
            ui.add_space(8.0);

            // ReplicaSets
            let rs_statuses = self.collect_workload_statuses(&self.replicasets);
            self.render_workload_status_box_with_data(
                ui,
                "ReplicaSets",
                &rs_statuses,
                section_keys::workloads_replicasets(),
            );
            ui.add_space(8.0);

            // ReplicationControllers
            let rc_statuses = self.collect_workload_statuses(&self.replicationcontrollers);
            self.render_workload_status_box_with_data(
                ui,
                "ReplicationControllers",
                &rc_statuses,
                section_keys::workloads_replicationcontrollers(),
            );
            ui.add_space(8.0);

            // Jobs
            let job_statuses = self.collect_job_statuses();
            self.render_job_status_box_with_data(
                ui,
                "Jobs",
                &job_statuses,
                section_keys::workloads_jobs(),
            );
            ui.add_space(8.0);

            // CronJobs
            let cj_statuses = self.collect_cronjob_statuses();
            self.render_cronjob_status_box_with_data(
                ui,
                "CronJobs",
                &cj_statuses,
                section_keys::workloads_cronjobs(),
            );
        });
    }

    fn collect_pod_statuses(&self) -> Vec<(String, usize)> {
        // Define stable order for pod statuses
        let status_order = ["Running", "Succeeded", "Pending", "Failed", "Unknown"];

        let mut counts = std::collections::HashMap::new();
        for pod in &self.pods {
            *counts.entry(pod.phase.clone()).or_insert(0) += 1;
        }

        // Build result in stable order, only including non-zero counts
        let mut result = Vec::new();
        for status in &status_order {
            if let Some(&count) = counts.get(*status) {
                if count > 0 {
                    result.push((status.to_string(), count));
                }
            }
        }

        // Add any other statuses not in the predefined order
        for (status, &count) in counts.iter() {
            if count > 0 && !status_order.contains(&status.as_str()) {
                result.push((status.clone(), count));
            }
        }

        result
    }

    fn collect_workload_statuses(
        &self,
        workloads: &[DeploymentSummary],
    ) -> (usize, usize, usize, usize, usize) {
        let total = workloads.len();
        let ready = workloads
            .iter()
            .filter(|w| w.ready >= w.desired && w.desired > 0)
            .count();
        let progressing = workloads
            .iter()
            .filter(|w| w.ready < w.desired && w.desired > 0)
            .count();
        let unavailable = workloads
            .iter()
            .filter(|w| w.ready == 0 && w.desired > 0)
            .count();
        let zero_desired = workloads.iter().filter(|w| w.desired == 0).count();

        (total, ready, progressing, unavailable, zero_desired)
    }

    fn collect_job_statuses(&self) -> (usize, usize, usize, usize) {
        let total = self.jobs.len();
        let succeeded = self
            .jobs
            .iter()
            .filter(|j| j.succeeded > 0 && j.active == 0 && j.failed == 0)
            .count();
        let running = self.jobs.iter().filter(|j| j.active > 0).count();
        let failed = self.jobs.iter().filter(|j| j.failed > 0).count();

        (total, succeeded, running, failed)
    }

    fn collect_cronjob_statuses(&self) -> (usize, usize, usize, usize) {
        let total = self.cronjobs.len();
        let active = self.cronjobs.iter().filter(|cj| cj.active > 0).count();
        let suspended = self.cronjobs.iter().filter(|cj| cj.suspend).count();
        let idle = self
            .cronjobs
            .iter()
            .filter(|cj| cj.active == 0 && !cj.suspend)
            .count();

        (total, active, suspended, idle)
    }

    fn render_pod_status_box(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        status_counts: &[(String, usize)],
        section_key: SectionKey,
    ) {
        let total: usize = status_counts.iter().map(|(_, count)| count).sum();

        ui.group(|ui| {
            ui.horizontal(|ui| {
                // Title with total count (clickable with hover effect)
                let title_response = ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("{} ({})", title, total)).strong(),
                    )
                    .sense(egui::Sense::click()),
                );

                if title_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }

                if title_response.clicked() {
                    self.active_section = section_key;
                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                }

                ui.add_space(20.0);

                // Status breakdown in stable order
                for (status, count) in status_counts.iter() {
                    let color = pod_status_color(status);
                    self.render_status_indicator(ui, status, *count, color);
                }
            });
        });
    }

    fn render_workload_status_box_with_data(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        statuses: &(usize, usize, usize, usize, usize),
        section_key: SectionKey,
    ) {
        let (total, ready, progressing, unavailable, zero_desired) = *statuses;

        ui.group(|ui| {
            ui.horizontal(|ui| {
                // Title with total count (clickable with hover effect)
                let title_response = ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("{} ({})", title, total)).strong(),
                    )
                    .sense(egui::Sense::click()),
                );

                if title_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }

                if title_response.clicked() {
                    self.active_section = section_key;
                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                }

                ui.add_space(20.0);

                // Status breakdown with colored indicators
                if ready > 0 {
                    self.render_status_indicator(
                        ui,
                        "Ready",
                        ready,
                        egui::Color32::from_rgb(46, 204, 113),
                    );
                }
                if progressing > 0 {
                    self.render_status_indicator(
                        ui,
                        "Progressing",
                        progressing,
                        egui::Color32::from_rgb(241, 196, 15),
                    );
                }
                if unavailable > 0 {
                    self.render_status_indicator(
                        ui,
                        "Unavailable",
                        unavailable,
                        egui::Color32::from_rgb(231, 76, 60),
                    );
                }
                if zero_desired > 0 {
                    self.render_status_indicator(
                        ui,
                        "Scaled to 0",
                        zero_desired,
                        egui::Color32::GRAY,
                    );
                }
            });
        });
    }

    fn render_job_status_box_with_data(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        statuses: &(usize, usize, usize, usize),
        section_key: SectionKey,
    ) {
        let (total, succeeded, running, failed) = *statuses;

        ui.group(|ui| {
            ui.horizontal(|ui| {
                // Title with total count (clickable with hover effect)
                let title_response = ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("{} ({})", title, total)).strong(),
                    )
                    .sense(egui::Sense::click()),
                );

                if title_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }

                if title_response.clicked() {
                    self.active_section = section_key;
                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                }

                ui.add_space(20.0);

                // Status breakdown with colored indicators
                if succeeded > 0 {
                    self.render_status_indicator(
                        ui,
                        "Succeeded",
                        succeeded,
                        egui::Color32::from_rgb(46, 204, 113),
                    );
                }
                if running > 0 {
                    self.render_status_indicator(
                        ui,
                        "Running",
                        running,
                        egui::Color32::from_rgb(52, 152, 219),
                    );
                }
                if failed > 0 {
                    self.render_status_indicator(
                        ui,
                        "Failed",
                        failed,
                        egui::Color32::from_rgb(231, 76, 60),
                    );
                }
            });
        });
    }

    fn render_cronjob_status_box_with_data(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        statuses: &(usize, usize, usize, usize),
        section_key: SectionKey,
    ) {
        let (total, active, suspended, idle) = *statuses;

        ui.group(|ui| {
            ui.horizontal(|ui| {
                // Title with total count (clickable with hover effect)
                let title_response = ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("{} ({})", title, total)).strong(),
                    )
                    .sense(egui::Sense::click()),
                );

                if title_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }

                if title_response.clicked() {
                    self.active_section = section_key;
                    self.send(BackendCmd::SetActiveSection {
                        section: self.active_section.clone(),
                    });
                }

                ui.add_space(20.0);

                // Status breakdown with colored indicators
                if active > 0 {
                    self.render_status_indicator(
                        ui,
                        "Active",
                        active,
                        egui::Color32::from_rgb(52, 152, 219),
                    );
                }
                if idle > 0 {
                    self.render_status_indicator(
                        ui,
                        "Idle",
                        idle,
                        egui::Color32::from_rgb(46, 204, 113),
                    );
                }
                if suspended > 0 {
                    self.render_status_indicator(ui, "Suspended", suspended, egui::Color32::GRAY);
                }
            });
        });
    }

    fn render_status_indicator(
        &self,
        ui: &mut egui::Ui,
        label: &str,
        count: usize,
        color: egui::Color32,
    ) {
        let (rect, _response) =
            ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 6.0, color);
        ui.label(format!("{}: {}", label, count));
        ui.add_space(8.0);
    }

    fn render_portforwards_center(&mut self, ui: &mut egui::Ui) {
        ui.heading("Port Forwards");
        ui.separator();

        ui.label("Manage local port forwarding to cluster services and pods.");
        ui.add_space(12.0);

        if ui.button("+ Create Port Forward").clicked() {
            self.open_create_port_forward();
        }

        ui.separator();
        ui.label("Active Tunnels:");
        if self.port_forwards.is_empty() {
            ui.label("(None active)");
        } else {
            egui::Grid::new("port_forwards_grid")
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Namespace");
                    ui.label("Pod");
                    ui.label("Local Port");
                    ui.label("Remote Port");
                    ui.label("Status");
                    ui.label("");
                    ui.end_row();

                    let mut stop_ids = Vec::new();
                    for pf in &self.port_forwards {
                        ui.label(&pf.namespace);
                        ui.label(&pf.pod);
                        ui.label(format!("127.0.0.1:{}", pf.local_port));
                        ui.label(pf.remote_port.to_string());
                        let status_color = match pf.status {
                            crate::sections::network::PortForwardStatus::Active => {
                                egui::Color32::GREEN
                            }
                            crate::sections::network::PortForwardStatus::Connecting => {
                                egui::Color32::YELLOW
                            }
                            crate::sections::network::PortForwardStatus::Error => {
                                egui::Color32::RED
                            }
                            crate::sections::network::PortForwardStatus::Closed => {
                                egui::Color32::GRAY
                            }
                        };
                        ui.colored_label(status_color, format!("{:?}", pf.status));
                        if ui.button("Stop").clicked() {
                            stop_ids.push(pf.id.clone());
                        }
                        ui.end_row();
                    }
                    for tunnel_id in stop_ids {
                        self.send(BackendCmd::StopPortForward { tunnel_id });
                    }
                });
        }
    }

    fn render_metrics_integration_center(&mut self, ui: &mut egui::Ui) {
        ui.heading("Metrics Integration");
        ui.separator();

        ui.label("Configure Prometheus for cluster metrics.");
        ui.add_space(12.0);

        // Namespace
        ui.label("Namespace:");
        ui.text_edit_singleline(&mut self.prometheus_namespace);

        // Service
        ui.label("Service:");
        ui.text_edit_singleline(&mut self.prometheus_service);

        // Port
        ui.label("Port:");
        ui.text_edit_singleline(&mut self.prometheus_port);

        ui.add_space(12.0);

        // Control buttons
        let button_row = |ui: &mut egui::Ui| {
            if ui.button("Save Configuration").clicked() {
                // Parse port
                if let Ok(port) = self.prometheus_port.parse::<u16>() {
                    self.send(BackendCmd::ConfigurePrometheus {
                        namespace: self.prometheus_namespace.clone(),
                        service: self.prometheus_service.clone(),
                        port,
                    });
                    self.status_log.push(format!(
                        "Saved Prometheus configuration: {}:{}/{}",
                        self.prometheus_service, port, self.prometheus_namespace
                    ));
                }
            }

            if ui.button("Connect").clicked() {
                // Parse port and start connection test
                if let Ok(port) = self.prometheus_port.parse::<u16>() {
                    self.prometheus_testing = true;
                    self.send(BackendCmd::TestPrometheusConnection {
                        namespace: self.prometheus_namespace.clone(),
                        service: self.prometheus_service.clone(),
                        port,
                    });
                }
            }

            // Connection status indicator
            if self.prometheus_connected {
                ui.colored_label(egui::Color32::GREEN, "✓ Connected");
            } else if self.prometheus_testing {
                ui.label("⟳ Testing...");
            }
        };

        ui.horizontal(button_row);
    }

    fn render_grafana_integration_center(&mut self, ui: &mut egui::Ui) {
        ui.heading("Grafana Integration");
        ui.separator();

        ui.label("Configure Grafana dashboards for visualization.");
        ui.add_space(12.0);

        ui.label("Base URL:");
        ui.text_edit_singleline(&mut self.grafana_url);

        ui.label("API Key:");
        ui.text_edit_singleline(&mut self.grafana_api_key);

        ui.add_space(12.0);

        // Control buttons
        let button_row = |ui: &mut egui::Ui| {
            if ui.button("Save Configuration").clicked() {
                self.send(BackendCmd::ConfigureGrafana {
                    base_url: self.grafana_url.clone(),
                    api_key: self.grafana_api_key.clone(),
                });
                self.status_log
                    .push(format!("Saved Grafana configuration: {}", self.grafana_url));
            }

            if ui.button("Connect").clicked() {
                if !self.grafana_url.is_empty() && !self.grafana_api_key.is_empty() {
                    self.grafana_testing = true;
                    self.send(BackendCmd::TestGrafanaConnection {
                        base_url: self.grafana_url.clone(),
                        api_key: self.grafana_api_key.clone(),
                    });
                }
            }

            // Connection status indicator
            if self.grafana_connected {
                ui.colored_label(egui::Color32::GREEN, "✓ Connected");
            } else if self.grafana_testing {
                ui.label("⟳ Testing...");
            }
        };

        ui.horizontal(button_row);
    }

    fn render_argo_integration_center(&mut self, ui: &mut egui::Ui) {
        ui.heading("ArgoCD Integration");
        ui.separator();

        ui.label("ArgoCD integration coming soon.");
        ui.label("Track Argo Applications and deployment status.");
    }

    fn render_flux_integration_center(&mut self, ui: &mut egui::Ui) {
        ui.heading("Flux Integration");
        ui.separator();

        ui.label("Flux GitOps integration coming soon.");
        ui.label("Monitor Flux Helmreleases and Kustomizations.");
    }
}

fn pod_status_color(status: &str) -> egui::Color32 {
    match status {
        "Running" => egui::Color32::from_rgb(46, 204, 113), // Green
        "Succeeded" => egui::Color32::from_rgb(26, 188, 156), // Teal
        "Pending" => egui::Color32::from_rgb(241, 196, 15), // Yellow
        "Failed" => egui::Color32::from_rgb(231, 76, 60),   // Red
        "Unknown" => egui::Color32::GRAY,
        _ => egui::Color32::LIGHT_GRAY,
    }
}
