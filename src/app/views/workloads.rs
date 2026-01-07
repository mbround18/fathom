use super::super::components::Table;
use super::traits::{ViewAction, ViewComponent};
use crate::app::{ConfirmAction, WorkloadKind};
use crate::protocol::{
    BackendCmd, BackendEvent, DeploymentDetails, DeploymentSummary, ResourceListEvent,
    ResourceReadEvent, WorkloadMetricsAggregate,
};

/// Render aggregated metrics for a workload with per-container breakdown
fn render_workload_metrics(ui: &mut egui::Ui, metrics: &WorkloadMetricsAggregate) {
    ui.group(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("📊 Resource Metrics (Aggregated)").strong());
            ui.separator();

            // Overall aggregate stats
            ui.horizontal(|ui| {
                ui.label("Replicas:");
                ui.monospace(metrics.replica_count.to_string());
            });

            // CPU metrics
            if let Some(cpu_pct) = metrics.total_cpu_usage_percent {
                let color = if cpu_pct > 80.0 {
                    egui::Color32::RED
                } else if cpu_pct > 50.0 {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::GREEN
                };

                ui.horizontal(|ui| {
                    ui.label("Total CPU:");
                    ui.colored_label(color, format!("{:.1}%", cpu_pct));
                });

                if let Some(req) = metrics.total_cpu_request_millicores {
                    ui.horizontal(|ui| {
                        ui.label("  Requested:");
                        ui.monospace(format!("{}m", req));
                    });
                }
                if let Some(lim) = metrics.total_cpu_limit_millicores {
                    ui.horizontal(|ui| {
                        ui.label("  Limited:");
                        ui.monospace(format!("{}m", lim));
                    });
                }
            }

            // Memory metrics
            if let Some(mem_bytes) = metrics.total_memory_usage_bytes {
                let mem_mb = mem_bytes as f64 / 1_048_576.0;
                let color = if let Some(limit_bytes) = metrics.total_memory_limit_bytes {
                    let limit_mb = limit_bytes as f64 / 1_048_576.0;
                    let pct = (mem_bytes as f64 / limit_bytes as f64) * 100.0;
                    if pct > 80.0 {
                        egui::Color32::RED
                    } else if pct > 50.0 {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::GREEN
                    }
                } else {
                    egui::Color32::GRAY
                };

                ui.horizontal(|ui| {
                    ui.label("Total Memory:");
                    ui.colored_label(color, format!("{:.1} MB", mem_mb));
                    if let Some(limit_bytes) = metrics.total_memory_limit_bytes {
                        let limit_mb = limit_bytes as f64 / 1_048_576.0;
                        ui.label(format!("/ {:.1} MB", limit_mb));
                    }
                });

                if let Some(req) = metrics.total_memory_request_bytes {
                    let req_mb = req as f64 / 1_048_576.0;
                    ui.horizontal(|ui| {
                        ui.label("  Requested:");
                        ui.monospace(format!("{:.1} MB", req_mb));
                    });
                }
            }

            ui.separator();
            ui.label(egui::RichText::new("📦 Per-Container Breakdown").strong());

            // Per-container visualization
            for container in &metrics.containers {
                ui.horizontal(|ui| {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&container.name).monospace().strong());

                            // CPU bar
                            if let Some(cpu_pct) = container.cpu_usage_percent {
                                let bar_width = ui.available_width() - 40.0;
                                let fill_fraction = (cpu_pct / 100.0).min(1.0);

                                ui.horizontal(|ui| {
                                    ui.label("CPU:");
                                    let bar_color = if cpu_pct > 80.0 {
                                        egui::Color32::RED
                                    } else if cpu_pct > 50.0 {
                                        egui::Color32::YELLOW
                                    } else {
                                        egui::Color32::GREEN
                                    };

                                    // Simple progress bar-like visualization
                                    ui.colored_label(bar_color, format!("{:3.0}%", cpu_pct));
                                });
                            }

                            // Memory bar
                            if let Some(mem_bytes) = container.memory_usage_bytes {
                                let mem_mb = mem_bytes as f64 / 1_048_576.0;
                                let bar_color = if let Some(limit) = container.memory_limit_bytes {
                                    let pct = (mem_bytes as f64 / limit as f64) * 100.0;
                                    if pct > 80.0 {
                                        egui::Color32::RED
                                    } else if pct > 50.0 {
                                        egui::Color32::YELLOW
                                    } else {
                                        egui::Color32::GREEN
                                    }
                                } else {
                                    egui::Color32::GRAY
                                };

                                ui.horizontal(|ui| {
                                    ui.label("Mem:");
                                    ui.colored_label(
                                        bar_color,
                                        format!(
                                            "{:6.1} MB{}",
                                            mem_mb,
                                            if let Some(lim) = container.memory_limit_bytes {
                                                format!(" / {:.1} MB", lim as f64 / 1_048_576.0)
                                            } else {
                                                String::new()
                                            }
                                        ),
                                    );
                                });
                            }
                        });
                    });
                });
            }

            ui.separator();
            ui.label(egui::RichText::new(format!("Last updated: {}", metrics.timestamp)).small());
        });
    });
}

/// Shared helper for rendering workload details panel
fn render_workload_details(
    ui: &mut egui::Ui,
    workload_type: &str,
    details: &Option<DeploymentDetails>,
    supports_scaling: bool,
    actions: &mut Vec<ViewAction>,
) {
    let Some(details) = details else {
        ui.label(format!(
            "Loading {} details...",
            workload_type.to_lowercase()
        ));
        return;
    };

    ui.monospace(format!("Created: {}", details.creation_timestamp));
    ui.add_space(6.0);

    ui.label(format!("Namespace: {}", details.namespace));
    ui.label(format!("Containers: {}", details.containers.join(", ")));

    if supports_scaling {
        ui.add_space(8.0);
        if ui.button(format!("Scale {}", workload_type)).clicked() {
            // TODO: Open scale modal in Phase 7
            actions.push(ViewAction::NotifyStatus(format!(
                "Scale {} to replicas",
                workload_type.to_lowercase()
            )));
        }
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
        egui::ScrollArea::vertical()
            .max_height(250.0)
            .show(ui, |ui| {
                ui.monospace(&details.raw_json);
            });
    });

    // Show related pods if available
    if !details.pods.is_empty() {
        ui.add_space(10.0);
        ui.collapsing(format!("Pods ({})", details.pods.len()), |ui| {
            for pod in &details.pods {
                ui.monospace(format!(
                    "- {}: {} ({}/{})",
                    pod.name, pod.phase, pod.ready, pod.total
                ));
            }
        });
    }
}

/// Shared helper for rendering workload grids using the Table component
fn render_workload_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<DeploymentSummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    let columns = [
        super::super::components::table::Column {
            header: "Namespace".to_string(),
            width: 100.0,
            sort_value: |w: &DeploymentSummary| w.namespace.clone(),
            render: |w: &DeploymentSummary| w.namespace.clone(),
        },
        super::super::components::table::Column {
            header: "Name".to_string(),
            width: 150.0,
            sort_value: |w: &DeploymentSummary| w.name.clone(),
            render: |w: &DeploymentSummary| w.name.clone(),
        },
        super::super::components::table::Column {
            header: "Ready".to_string(),
            width: 60.0,
            sort_value: |w: &DeploymentSummary| w.ready.to_string(),
            render: |w: &DeploymentSummary| w.ready.to_string(),
        },
        super::super::components::table::Column {
            header: "Updated".to_string(),
            width: 60.0,
            sort_value: |w: &DeploymentSummary| w.updated.to_string(),
            render: |w: &DeploymentSummary| w.updated.to_string(),
        },
        super::super::components::table::Column {
            header: "Available".to_string(),
            width: 60.0,
            sort_value: |w: &DeploymentSummary| w.available.to_string(),
            render: |w: &DeploymentSummary| w.available.to_string(),
        },
        super::super::components::table::Column {
            header: "Desired".to_string(),
            width: 60.0,
            sort_value: |w: &DeploymentSummary| w.desired.to_string(),
            render: |w: &DeploymentSummary| w.desired.to_string(),
        },
    ];

    let search_fn = |w: &DeploymentSummary, query: &str| {
        w.namespace.to_lowercase().contains(query) || w.name.to_lowercase().contains(query)
    };

    // Track selected row to emit selection action
    let mut newly_selected: Option<(String, String)> = None;
    let on_row_clicked = |original_idx: usize| {
        // We'll capture this in the render_table call
        // This is a limitation of the current design - we'll handle this differently
    };

    table.render_table(ui, &columns, search_fn, None);

    // Extract selected from table and convert back to (namespace, name)
    if let Some(selected_idx) = table.selected {
        if let Some(item) = table.items.get(selected_idx) {
            newly_selected = Some((item.namespace.clone(), item.name.clone()));
        }
    }

    newly_selected
}

/// Generic state shared by all workload views
pub struct WorkloadViewState {
    pub summaries: Vec<DeploymentSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<DeploymentDetails>,
    pub metrics: Option<WorkloadMetricsAggregate>,
    pub workload_type: &'static str,
    pub supports_scaling: bool,
    pub table: Table<DeploymentSummary>,
    pub metrics_polling_started: bool,
    pub metrics_interval_secs: u64,
}

impl WorkloadViewState {
    pub fn new(workload_type: &'static str, supports_scaling: bool) -> Self {
        Self {
            summaries: Vec::new(),
            selected: None,
            details: None,
            metrics: None,
            workload_type,
            supports_scaling,
            table: Table::new(Vec::new()),
            metrics_polling_started: false,
            metrics_interval_secs: 15,
        }
    }
}

/// Deployment view
pub struct DeploymentView(pub WorkloadViewState);

impl DeploymentView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for DeploymentView {
    fn default() -> Self {
        Self(WorkloadViewState::new("Deployment", true))
    }
}

impl ViewComponent for DeploymentView {
    fn id(&self) -> &'static str {
        "workloads_deployments"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        // Update table data
        self.0.table.set_items(self.0.summaries.clone());

        let newly_selected = render_workload_grid(ui, "Deployments", &mut self.0.table);

        // Only treat as newly selected if it's different from current selection
        if newly_selected != self.0.selected {
            if let Some((ns, name)) = newly_selected {
                self.0.selected = Some((ns.clone(), name.clone()));
                self.0.details = None;
                actions.push(ViewAction::SendCommand(BackendCmd::SelectDeployment {
                    namespace: ns,
                    name,
                }));
            } else {
                self.0.selected = None;
                self.0.details = None;
            }
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::DeploymentsUpdated(deployments)) =
            event
        {
            self.0.summaries = deployments.clone();
        }

        if let BackendEvent::ResourceRead(ResourceReadEvent::DeploymentDetailsUpdated(details)) =
            event
        {
            if Some((details.namespace.clone(), details.name.clone())) == self.0.selected {
                self.0.details = Some(details.clone());
            }
        }

        if let BackendEvent::ResourceRead(ResourceReadEvent::DeploymentMetricsUpdated(metrics)) =
            event
        {
            if Some((metrics.namespace.clone(), metrics.name.clone())) == self.0.selected {
                self.0.metrics = Some(metrics.clone());
            }
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.0.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.0.selected.as_ref().unwrap().clone();

        // Auto-start metrics polling when details panel opens
        if !self.0.metrics_polling_started {
            self.0.metrics_polling_started = true;
            actions.push(ViewAction::SendCommand(
                BackendCmd::StartWorkloadMetricsPolling {
                    namespace: ns.clone(),
                    workload_name: name.clone(),
                    workload_kind: self.0.workload_type.to_string(),
                    interval_secs: self.0.metrics_interval_secs,
                },
            ));
        }

        ui.horizontal(|ui| {
            ui.heading("Deployment");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(
                        crate::app::ConfirmAction::DeleteDeployment(ns.clone(), name.clone()),
                    ));
                }
                if ui.button("Restart").clicked() {
                    actions.push(ViewAction::OpenModal(
                        crate::app::ConfirmAction::RestartDeployment(ns.clone(), name.clone()),
                    ));
                }
                if ui.button("Scale").clicked() {
                    // Get current replica count from the summary
                    let current = self
                        .0
                        .summaries
                        .iter()
                        .find(|s| s.namespace == ns && s.name == name)
                        .map(|s| s.desired)
                        .unwrap_or(0);

                    actions.push(ViewAction::OpenScaleModal(
                        crate::app::WorkloadKind::Deployment,
                        ns.clone(),
                        name.clone(),
                        current,
                    ));
                }
                // Port Forward button - extract ports from deployment template
                if let Some(details) = &self.0.details {
                    if !details.ports.is_empty()
                        && !details.pods.is_empty()
                        && ui.button("Port Forward").clicked()
                    {
                        // Use first available pod
                        let pod = &details.pods[0];
                        let suggested_ports: Vec<u16> =
                            details.ports.iter().map(|p| p.port).collect();
                        let containers: Vec<String> = details
                            .containers
                            .iter()
                            .map(|c| c.split_whitespace().next().unwrap_or(c).to_string())
                            .collect();

                        actions.push(ViewAction::OpenModal(
                            crate::app::ConfirmAction::OpenPortForward {
                                namespace: pod.namespace.clone(),
                                pod: pod.name.clone(),
                                suggested_ports,
                                containers,
                            },
                        ));
                    }
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(&ns);
            ui.label(format!("/{}", &name));
        });

        ui.separator();
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(details) = &self.0.details {
                // Find the summary for replica counts
                let summary = self
                    .0
                    .summaries
                    .iter()
                    .find(|s| s.namespace == ns && s.name == name);

                // 📊 Replicas Info
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Replicas");
                        ui.separator();
                        if let Some(summary) = summary {
                            ui.horizontal(|ui| {
                                ui.label(format!("Desired:    {}", summary.desired));
                                ui.label(format!("Ready:      {}", summary.ready));
                                ui.label(format!("Updated:    {}", summary.updated));
                                ui.label(format!("Available:  {}", summary.available));
                            });
                        } else {
                            ui.label("(loading replica info...)");
                        }
                    });
                });
                ui.add_space(8.0);

                // Aggregated metrics if available
                if let Some(metrics) = &self.0.metrics {
                    render_workload_metrics(ui, metrics);
                    ui.add_space(8.0);
                }

                // ℹ️ Metadata
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("ℹ️ Metadata");
                        ui.separator();
                        ui.monospace(format!("Created: {}", details.creation_timestamp));
                        ui.label(format!("Containers: {}", details.containers.len()));
                        for container in &details.containers {
                            ui.monospace(format!("  - {}", container));
                        }
                    });
                });
                ui.add_space(8.0);

                // 🔍 Selector
                if !details.pod_selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🔍 Pod Selector");
                            ui.separator();
                            ui.monospace(&details.pod_selector);
                        });
                    });
                    ui.add_space(8.0);
                }

                // 🏷️ Labels & Annotations
                ui.group(|ui| {
                    ui.vertical(|ui| {
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
                    });
                });
                ui.add_space(8.0);

                // 📦 Related Pods
                if !details.pods.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.collapsing(format!("📦 Pods ({})", details.pods.len()), |ui| {
                                for pod in &details.pods {
                                    ui.monospace(format!(
                                        "- {}: {} ({}/{})",
                                        pod.name, pod.phase, pod.ready, pod.total
                                    ));
                                }
                            });
                        });
                    });
                    ui.add_space(8.0);
                }

                // 📄 Raw JSON
                ui.collapsing("📄 Raw JSON", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            ui.monospace(&details.raw_json);
                        });
                });
            } else {
                ui.label("Loading deployment details...");
            }
        });

        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshDeployments)]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.0.selected = None;
        self.0.details = None;
        Vec::new()
    }
}

/// DaemonSet view
pub struct DaemonSetView(pub WorkloadViewState);

impl DaemonSetView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for DaemonSetView {
    fn default() -> Self {
        Self(WorkloadViewState::new("DaemonSet", false))
    }
}

impl ViewComponent for DaemonSetView {
    fn id(&self) -> &'static str {
        "workloads_daemonsets"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        // Update table data
        self.0.table.set_items(self.0.summaries.clone());

        let newly_selected = render_workload_grid(ui, "DaemonSets", &mut self.0.table);

        // Only treat as newly selected if it's different from current selection
        if newly_selected != self.0.selected {
            if let Some((ns, name)) = newly_selected {
                self.0.selected = Some((ns.clone(), name.clone()));
                self.0.details = None;
                actions.push(ViewAction::SendCommand(BackendCmd::SelectDaemonSet {
                    namespace: ns,
                    name,
                }));
            } else {
                self.0.selected = None;
                self.0.details = None;
            }
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::DaemonSetsUpdated(daemonsets)) = event
        {
            self.0.summaries = daemonsets.clone();
        }

        if let BackendEvent::ResourceRead(ResourceReadEvent::DaemonSetDetailsUpdated(details)) =
            event
        {
            if Some((details.namespace.clone(), details.name.clone())) == self.0.selected {
                self.0.details = Some(details.clone());
            }
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.0.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.0.selected.as_ref().unwrap().clone();

        // Auto-start metrics polling when details panel opens
        if !self.0.metrics_polling_started {
            self.0.metrics_polling_started = true;
            actions.push(ViewAction::SendCommand(
                BackendCmd::StartWorkloadMetricsPolling {
                    namespace: ns.clone(),
                    workload_name: name.clone(),
                    workload_kind: self.0.workload_type.to_string(),
                    interval_secs: self.0.metrics_interval_secs,
                },
            ));
        }

        ui.horizontal(|ui| {
            ui.heading("DaemonSet");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteDaemonSet(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                if ui.button("Restart").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::RestartDaemonSet(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                // Port Forward button - extract ports from daemonset template
                if let Some(details) = &self.0.details {
                    if !details.ports.is_empty()
                        && !details.pods.is_empty()
                        && ui.button("Port Forward").clicked()
                    {
                        // Use first available pod
                        let pod = &details.pods[0];
                        let suggested_ports: Vec<u16> =
                            details.ports.iter().map(|p| p.port).collect();
                        let containers: Vec<String> = details
                            .containers
                            .iter()
                            .map(|c| c.split_whitespace().next().unwrap_or(c).to_string())
                            .collect();

                        actions.push(ViewAction::OpenModal(
                            crate::app::ConfirmAction::OpenPortForward {
                                namespace: pod.namespace.clone(),
                                pod: pod.name.clone(),
                                suggested_ports,
                                containers,
                            },
                        ));
                    }
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
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
            if let Some(details) = &self.0.details {
                // Find the summary for replica counts
                let summary = self
                    .0
                    .summaries
                    .iter()
                    .find(|s| s.namespace == ns && s.name == name);

                // 📊 Status
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Status");
                        ui.separator();
                        if let Some(summary) = summary {
                            ui.monospace(format!("Desired: {}", summary.desired));
                            ui.monospace(format!("Ready: {}", summary.ready));
                            ui.monospace(format!("Updated: {}", summary.updated));
                            ui.monospace(format!("Available: {}", summary.available));
                        } else {
                            ui.label("(loading status...)");
                        }
                    });
                });
                ui.add_space(8.0);

                // ℹ️ Metadata
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("ℹ️ Metadata");
                        ui.separator();
                        ui.monospace(format!("Created: {}", details.creation_timestamp));
                        ui.label(format!("Containers: {}", details.containers.len()));
                        for container in &details.containers {
                            ui.monospace(format!("  - {}", container));
                        }
                    });
                });
                ui.add_space(8.0);

                // 🔍 Selector
                if !details.pod_selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🔍 Pod Selector");
                            ui.separator();
                            ui.monospace(&details.pod_selector);
                        });
                    });
                    ui.add_space(8.0);
                }

                // 🏷️ Labels & Annotations
                ui.group(|ui| {
                    ui.vertical(|ui| {
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
                    });
                });
                ui.add_space(8.0);

                // 📦 Related Pods
                if !details.pods.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.collapsing(format!("📦 Pods ({})", details.pods.len()), |ui| {
                                for pod in &details.pods {
                                    ui.monospace(format!(
                                        "- {}: {} ({}/{})",
                                        pod.name, pod.phase, pod.ready, pod.total
                                    ));
                                }
                            });
                        });
                    });
                    ui.add_space(8.0);
                }

                // 📄 Raw JSON
                ui.collapsing("📄 Raw JSON", |ui| {
                    if let Ok(json_str) = serde_json::to_string_pretty(&details) {
                        ui.code(json_str);
                    }
                });
            } else {
                ui.label("Loading DaemonSet details...");
            }
        });

        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshDaemonSets)]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.0.selected = None;
        self.0.details = None;
        Vec::new()
    }
}

/// StatefulSet view
pub struct StatefulSetView(pub WorkloadViewState);

impl StatefulSetView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for StatefulSetView {
    fn default() -> Self {
        Self(WorkloadViewState::new("StatefulSet", true))
    }
}

impl ViewComponent for StatefulSetView {
    fn id(&self) -> &'static str {
        "workloads_statefulsets"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        // Update table data
        self.0.table.set_items(self.0.summaries.clone());

        let newly_selected = render_workload_grid(ui, "StatefulSets", &mut self.0.table);

        // Only treat as newly selected if it's different from current selection
        if newly_selected != self.0.selected {
            if let Some((ns, name)) = newly_selected {
                self.0.selected = Some((ns.clone(), name.clone()));
                self.0.details = None;
                actions.push(ViewAction::SendCommand(BackendCmd::SelectStatefulSet {
                    namespace: ns,
                    name,
                }));
            } else {
                self.0.selected = None;
                self.0.details = None;
            }
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::StatefulSetsUpdated(statefulsets)) =
            event
        {
            self.0.summaries = statefulsets.clone();
        }

        if let BackendEvent::ResourceRead(ResourceReadEvent::StatefulSetDetailsUpdated(details)) =
            event
        {
            if Some((details.namespace.clone(), details.name.clone())) == self.0.selected {
                self.0.details = Some(details.clone());
            }
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.0.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.0.selected.as_ref().unwrap().clone();

        // Auto-start metrics polling when details panel opens
        if !self.0.metrics_polling_started {
            self.0.metrics_polling_started = true;
            actions.push(ViewAction::SendCommand(
                BackendCmd::StartWorkloadMetricsPolling {
                    namespace: ns.clone(),
                    workload_name: name.clone(),
                    workload_kind: self.0.workload_type.to_string(),
                    interval_secs: self.0.metrics_interval_secs,
                },
            ));
        }

        ui.horizontal(|ui| {
            ui.heading("StatefulSet");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteStatefulSet(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                if ui.button("Restart").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::RestartStatefulSet(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                if ui.button("Scale").clicked() {
                    // Get current replica count from the summary
                    let current = self
                        .0
                        .summaries
                        .iter()
                        .find(|s| s.namespace == ns && s.name == name)
                        .map(|s| s.desired)
                        .unwrap_or(0);

                    actions.push(ViewAction::OpenScaleModal(
                        WorkloadKind::StatefulSet,
                        ns.clone(),
                        name.clone(),
                        current,
                    ));
                }
                // Port Forward button - extract ports from statefulset template
                if let Some(details) = &self.0.details {
                    if !details.ports.is_empty()
                        && !details.pods.is_empty()
                        && ui.button("Port Forward").clicked()
                    {
                        // Use first available pod
                        let pod = &details.pods[0];
                        let suggested_ports: Vec<u16> =
                            details.ports.iter().map(|p| p.port).collect();
                        let containers: Vec<String> = details
                            .containers
                            .iter()
                            .map(|c| c.split_whitespace().next().unwrap_or(c).to_string())
                            .collect();

                        actions.push(ViewAction::OpenModal(
                            crate::app::ConfirmAction::OpenPortForward {
                                namespace: pod.namespace.clone(),
                                pod: pod.name.clone(),
                                suggested_ports,
                                containers,
                            },
                        ));
                    }
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
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
            if let Some(details) = &self.0.details {
                // Find the summary for replica counts
                let summary = self
                    .0
                    .summaries
                    .iter()
                    .find(|s| s.namespace == ns && s.name == name);

                // 📊 Status
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Status");
                        ui.separator();
                        if let Some(summary) = summary {
                            ui.monospace(format!("Desired: {}", summary.desired));
                            ui.monospace(format!("Ready: {}", summary.ready));
                            ui.monospace(format!("Updated: {}", summary.updated));
                            ui.monospace(format!("Available: {}", summary.available));
                        } else {
                            ui.label("(loading status...)");
                        }
                    });
                });
                ui.add_space(8.0);

                // ℹ️ Metadata
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("ℹ️ Metadata");
                        ui.separator();
                        ui.monospace(format!("Created: {}", details.creation_timestamp));
                        ui.label(format!("Containers: {}", details.containers.len()));
                        for container in &details.containers {
                            ui.monospace(format!("  - {}", container));
                        }
                    });
                });
                ui.add_space(8.0);

                // 🔍 Selector
                if !details.pod_selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🔍 Pod Selector");
                            ui.separator();
                            ui.monospace(&details.pod_selector);
                        });
                    });
                    ui.add_space(8.0);
                }

                // 🏷️ Labels & Annotations
                ui.group(|ui| {
                    ui.vertical(|ui| {
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
                    });
                });
                ui.add_space(8.0);

                // 📦 Related Pods
                if !details.pods.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.collapsing(format!("📦 Pods ({})", details.pods.len()), |ui| {
                                for pod in &details.pods {
                                    ui.monospace(format!(
                                        "- {}: {} ({}/{})",
                                        pod.name, pod.phase, pod.ready, pod.total
                                    ));
                                }
                            });
                        });
                    });
                    ui.add_space(8.0);
                }

                // 📄 Raw JSON
                ui.collapsing("📄 Raw JSON", |ui| {
                    if let Ok(json_str) = serde_json::to_string_pretty(&details) {
                        ui.code(json_str);
                    }
                });
            } else {
                ui.label("Loading StatefulSet details...");
            }
        });

        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshStatefulSets)]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.0.selected = None;
        self.0.details = None;
        Vec::new()
    }
}

/// ReplicaSet view
pub struct ReplicaSetView(pub WorkloadViewState);

impl ReplicaSetView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for ReplicaSetView {
    fn default() -> Self {
        Self(WorkloadViewState::new("ReplicaSet", true))
    }
}

impl ViewComponent for ReplicaSetView {
    fn id(&self) -> &'static str {
        "workloads_replicasets"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        // Update table data
        self.0.table.set_items(self.0.summaries.clone());

        let newly_selected = render_workload_grid(ui, "ReplicaSets", &mut self.0.table);

        // Only treat as newly selected if it's different from current selection
        if newly_selected != self.0.selected {
            if let Some((ns, name)) = newly_selected {
                self.0.selected = Some((ns.clone(), name.clone()));
                self.0.details = None;
                actions.push(ViewAction::SendCommand(BackendCmd::SelectReplicaSet {
                    namespace: ns,
                    name,
                }));
            } else {
                self.0.selected = None;
                self.0.details = None;
            }
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::ReplicaSetsUpdated(replicasets)) =
            event
        {
            self.0.summaries = replicasets.clone();
        }

        if let BackendEvent::ResourceRead(ResourceReadEvent::ReplicaSetDetailsUpdated(details)) =
            event
        {
            if Some((details.namespace.clone(), details.name.clone())) == self.0.selected {
                self.0.details = Some(details.clone());
            }
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.0.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.0.selected.as_ref().unwrap().clone();

        // Auto-start metrics polling when details panel opens
        if !self.0.metrics_polling_started {
            self.0.metrics_polling_started = true;
            actions.push(ViewAction::SendCommand(
                BackendCmd::StartWorkloadMetricsPolling {
                    namespace: ns.clone(),
                    workload_name: name.clone(),
                    workload_kind: self.0.workload_type.to_string(),
                    interval_secs: self.0.metrics_interval_secs,
                },
            ));
        }

        ui.horizontal(|ui| {
            ui.heading("ReplicaSet");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::DeleteReplicaSet(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                if ui.button("Restart").clicked() {
                    actions.push(ViewAction::OpenModal(ConfirmAction::RestartReplicaSet(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                if ui.button("Scale").clicked() {
                    // Get current replica count from the summary
                    let current = self
                        .0
                        .summaries
                        .iter()
                        .find(|s| s.namespace == ns && s.name == name)
                        .map(|s| s.desired)
                        .unwrap_or(0);

                    actions.push(ViewAction::OpenScaleModal(
                        WorkloadKind::ReplicaSet,
                        ns.clone(),
                        name.clone(),
                        current,
                    ));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
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
            if let Some(details) = &self.0.details {
                // Find the summary for replica counts
                let summary = self
                    .0
                    .summaries
                    .iter()
                    .find(|s| s.namespace == ns && s.name == name);

                // 📊 Status
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Status");
                        ui.separator();
                        if let Some(summary) = summary {
                            ui.monospace(format!("Desired: {}", summary.desired));
                            ui.monospace(format!("Ready: {}", summary.ready));
                            ui.monospace(format!("Updated: {}", summary.updated));
                            ui.monospace(format!("Available: {}", summary.available));
                        } else {
                            ui.label("(loading status...)");
                        }
                    });
                });
                ui.add_space(8.0);

                // ℹ️ Metadata
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("ℹ️ Metadata");
                        ui.separator();
                        ui.monospace(format!("Created: {}", details.creation_timestamp));
                        ui.label(format!("Containers: {}", details.containers.len()));
                        for container in &details.containers {
                            ui.monospace(format!("  - {}", container));
                        }
                    });
                });
                ui.add_space(8.0);

                // 🔍 Selector
                if !details.pod_selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🔍 Pod Selector");
                            ui.separator();
                            ui.monospace(&details.pod_selector);
                        });
                    });
                    ui.add_space(8.0);
                }

                // 🏷️ Labels & Annotations
                ui.group(|ui| {
                    ui.vertical(|ui| {
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
                    });
                });
                ui.add_space(8.0);

                // 📦 Related Pods
                if !details.pods.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.collapsing(format!("📦 Pods ({})", details.pods.len()), |ui| {
                                for pod in &details.pods {
                                    ui.monospace(format!(
                                        "- {}: {} ({}/{})",
                                        pod.name, pod.phase, pod.ready, pod.total
                                    ));
                                }
                            });
                        });
                    });
                    ui.add_space(8.0);
                }

                // 📄 Raw JSON
                ui.collapsing("📄 Raw JSON", |ui| {
                    if let Ok(json_str) = serde_json::to_string_pretty(&details) {
                        ui.code(json_str);
                    }
                });
            } else {
                ui.label("Loading ReplicaSet details...");
            }
        });

        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(BackendCmd::RefreshReplicaSets)]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.0.selected = None;
        self.0.details = None;
        Vec::new()
    }
}

/// ReplicationController view
pub struct ReplicationControllerView(pub WorkloadViewState);

impl ReplicationControllerView {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for ReplicationControllerView {
    fn default() -> Self {
        Self(WorkloadViewState::new("ReplicationController", true))
    }
}

impl ViewComponent for ReplicationControllerView {
    fn id(&self) -> &'static str {
        "workloads_replicationcontrollers"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        // Update table data
        self.0.table.set_items(self.0.summaries.clone());

        let newly_selected = render_workload_grid(ui, "ReplicationControllers", &mut self.0.table);

        // Only treat as newly selected if it's different from current selection
        if newly_selected != self.0.selected {
            if let Some((ns, name)) = newly_selected {
                self.0.selected = Some((ns.clone(), name.clone()));
                self.0.details = None;
                actions.push(ViewAction::SendCommand(
                    BackendCmd::SelectReplicationController {
                        namespace: ns,
                        name,
                    },
                ));
            } else {
                self.0.selected = None;
                self.0.details = None;
            }
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::ReplicationControllersUpdated(rcs)) =
            event
        {
            self.0.summaries = rcs.clone();
        }

        if let BackendEvent::ResourceRead(ResourceReadEvent::ReplicationControllerDetailsUpdated(
            details,
        )) = event
        {
            if Some((details.namespace.clone(), details.name.clone())) == self.0.selected {
                self.0.details = Some(details.clone());
            }
        }
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.0.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.0.selected.as_ref().unwrap().clone();

        // Auto-start metrics polling when details panel opens
        if !self.0.metrics_polling_started {
            self.0.metrics_polling_started = true;
            actions.push(ViewAction::SendCommand(
                BackendCmd::StartWorkloadMetricsPolling {
                    namespace: ns.clone(),
                    workload_name: name.clone(),
                    workload_kind: self.0.workload_type.to_string(),
                    interval_secs: self.0.metrics_interval_secs,
                },
            ));
        }

        ui.horizontal(|ui| {
            ui.heading("ReplicationController");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal(
                        ConfirmAction::DeleteReplicationController(ns.clone(), name.clone()),
                    ));
                }
                if ui.button("Restart").clicked() {
                    actions.push(ViewAction::OpenModal(
                        ConfirmAction::RestartReplicationController(ns.clone(), name.clone()),
                    ));
                }
                if ui.button("Scale").clicked() {
                    // Get current replica count from the summary
                    let current = self
                        .0
                        .summaries
                        .iter()
                        .find(|s| s.namespace == ns && s.name == name)
                        .map(|s| s.desired)
                        .unwrap_or(0);

                    actions.push(ViewAction::OpenScaleModal(
                        WorkloadKind::ReplicationController,
                        ns.clone(),
                        name.clone(),
                        current,
                    ));
                }
                if ui.button("Close").clicked() {
                    self.0.selected = None;
                    self.0.details = None;
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
            if let Some(details) = &self.0.details {
                // Find the summary for replica counts
                let summary = self
                    .0
                    .summaries
                    .iter()
                    .find(|s| s.namespace == ns && s.name == name);

                // 📊 Status
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Status");
                        ui.separator();
                        if let Some(summary) = summary {
                            ui.monospace(format!("Desired: {}", summary.desired));
                            ui.monospace(format!("Ready: {}", summary.ready));
                            ui.monospace(format!("Updated: {}", summary.updated));
                            ui.monospace(format!("Available: {}", summary.available));
                        } else {
                            ui.label("(loading status...)");
                        }
                    });
                });
                ui.add_space(8.0);

                // ℹ️ Metadata
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("ℹ️ Metadata");
                        ui.separator();
                        ui.monospace(format!("Created: {}", details.creation_timestamp));
                        ui.label(format!("Containers: {}", details.containers.len()));
                        for container in &details.containers {
                            ui.monospace(format!("  - {}", container));
                        }
                    });
                });
                ui.add_space(8.0);

                // 🔍 Selector
                if !details.pod_selector.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.label("🔍 Pod Selector");
                            ui.separator();
                            ui.monospace(&details.pod_selector);
                        });
                    });
                    ui.add_space(8.0);
                }

                // 🏷️ Labels & Annotations
                ui.group(|ui| {
                    ui.vertical(|ui| {
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
                    });
                });
                ui.add_space(8.0);

                // 📦 Related Pods
                if !details.pods.is_empty() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.collapsing(format!("📦 Pods ({})", details.pods.len()), |ui| {
                                for pod in &details.pods {
                                    ui.monospace(format!(
                                        "- {}: {} ({}/{})",
                                        pod.name, pod.phase, pod.ready, pod.total
                                    ));
                                }
                            });
                        });
                    });
                    ui.add_space(8.0);
                }

                // 📄 Raw JSON
                ui.collapsing("📄 Raw JSON", |ui| {
                    if let Ok(json_str) = serde_json::to_string_pretty(&details) {
                        ui.code(json_str);
                    }
                });
            } else {
                ui.label("Loading ReplicationController details...");
            }
        });

        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(
            BackendCmd::RefreshReplicationControllers,
        )]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.0.selected = None;
        self.0.details = None;
        Vec::new()
    }
}
