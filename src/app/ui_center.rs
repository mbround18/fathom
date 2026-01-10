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

    pub(super) fn render_center(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.active_section == section_keys::workloads_pods() {
                self.render_pods_center(ui);
                return;
            }

            if self.active_section == section_keys::workloads_deployments() {
                self.render_deployments_center(ui);
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
                        self.pod_logs.clear();

                        self.send(BackendCmd::SelectPod {
                            namespace: ns,
                            name,
                        });
                    }
                });
        });
    }

    fn render_deployments_center(&mut self, ui: &mut egui::Ui) {
        ui.heading("Deployments");
        ui.separator();

        let deployments = self.deployments.clone();

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("deployments_grid")
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

                    let mut newly_selected: Option<(String, String)> = None;
                    for d in &deployments {
                        let is_selected = self
                            .selected_deployment
                            .as_ref()
                            .is_some_and(|(ns, name)| ns == &d.namespace && name == &d.name);

                        let ns_clicked = ui.selectable_label(is_selected, &d.namespace).clicked();
                        let name_clicked = ui.selectable_label(is_selected, &d.name).clicked();
                        let row_clicked = ns_clicked || name_clicked;

                        ui.label(d.ready.to_string());
                        ui.label(d.updated.to_string());
                        ui.label(d.available.to_string());
                        ui.label(d.desired.to_string());
                        ui.end_row();

                        if row_clicked {
                            newly_selected = Some((d.namespace.clone(), d.name.clone()));
                        }
                    }

                    if let Some((ns, name)) = newly_selected {
                        self.selected_deployment = Some((ns.clone(), name.clone()));
                        self.deployment_details = None;
                        self.send(BackendCmd::SelectDeployment {
                            namespace: ns,
                            name,
                        });
                    }
                });
        });
    }
}
