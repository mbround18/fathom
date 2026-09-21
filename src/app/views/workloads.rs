use super::super::components::Table;
use super::detail_sections::{render_kv_collapsing, render_raw_json_collapsing};
use super::traits::{ViewAction, ViewComponent};
use crate::app::{ConfirmAction, WorkloadKind};
use crate::protocol::{
    BackendCmd, BackendEvent, ResourceListEvent, ResourceReadEvent, WorkloadDetails,
    WorkloadMetricsAggregate, WorkloadSummary,
};

/// Render aggregated metrics for a workload with per-container breakdown
#[allow(clippy::too_many_lines)] // removed by T125 (egui cutover)
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
                    let _limit_mb = limit_bytes as f64 / 1_048_576.0;
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

/// Shared helper for rendering workload grids using the Table component
fn render_workload_grid(
    ui: &mut egui::Ui,
    title: &str,
    table: &mut Table<WorkloadSummary>,
) -> Option<(String, String)> {
    ui.heading(title);
    ui.separator();

    let columns = [
        super::super::components::table::Column {
            header: "Namespace".to_string(),
            width: 100.0,
            sort_value: |w: &WorkloadSummary| w.namespace.clone(),
            render: |w: &WorkloadSummary| w.namespace.clone(),
        },
        super::super::components::table::Column {
            header: "Name".to_string(),
            width: 150.0,
            sort_value: |w: &WorkloadSummary| w.name.clone(),
            render: |w: &WorkloadSummary| w.name.clone(),
        },
        super::super::components::table::Column {
            header: "Ready".to_string(),
            width: 60.0,
            sort_value: |w: &WorkloadSummary| w.ready.to_string(),
            render: |w: &WorkloadSummary| w.ready.to_string(),
        },
        super::super::components::table::Column {
            header: "Updated".to_string(),
            width: 60.0,
            sort_value: |w: &WorkloadSummary| w.updated.to_string(),
            render: |w: &WorkloadSummary| w.updated.to_string(),
        },
        super::super::components::table::Column {
            header: "Available".to_string(),
            width: 60.0,
            sort_value: |w: &WorkloadSummary| w.available.to_string(),
            render: |w: &WorkloadSummary| w.available.to_string(),
        },
        super::super::components::table::Column {
            header: "Desired".to_string(),
            width: 60.0,
            sort_value: |w: &WorkloadSummary| w.desired.to_string(),
            render: |w: &WorkloadSummary| w.desired.to_string(),
        },
    ];

    let search_fn = |w: &WorkloadSummary, query: &str| {
        w.namespace.to_lowercase().contains(query) || w.name.to_lowercase().contains(query)
    };

    table.render_table(ui, &columns, search_fn, None);

    // Extract selected from table and convert back to (namespace, name)
    if let Some(selected_idx) = table.selected
        && let Some(item) = table.items.get(selected_idx)
    {
        return Some((item.namespace.clone(), item.name.clone()));
    }

    None
}

/// Per-kind wiring that differs across the 5 workload kinds (Deployment,
/// DaemonSet, StatefulSet, ReplicaSet, ReplicationController). Everything
/// else — table rendering, selection, details panel layout — is shared by
/// [`WorkloadView`]. Kept as static function pointers (not closures with
/// captures) so each kind is a plain `'static` data table, not a trait object.
pub struct WorkloadKindConfig {
    pub view_id: &'static str,
    pub label: &'static str,
    pub grid_title: &'static str,
    pub kind: WorkloadKind,
    pub supports_scaling: bool,
    pub refresh_cmd: BackendCmd,
    pub select_cmd: fn(String, String) -> BackendCmd,
    pub delete_action: fn(String, String) -> ConfirmAction,
    pub restart_action: fn(String, String) -> ConfirmAction,
    pub match_list: fn(&BackendEvent) -> Option<Vec<WorkloadSummary>>,
    pub match_details: fn(&BackendEvent) -> Option<WorkloadDetails>,
    /// `None` for kinds where the backend doesn't (yet) emit a per-kind
    /// metrics event (ReplicaSet, ReplicationController).
    pub match_metrics: Option<fn(&BackendEvent) -> Option<WorkloadMetricsAggregate>>,
}

pub static DEPLOYMENT_CONFIG: WorkloadKindConfig = WorkloadKindConfig {
    view_id: "workloads_deployments",
    label: "Deployment",
    grid_title: "Deployments",
    kind: WorkloadKind::Deployment,
    supports_scaling: true,
    refresh_cmd: BackendCmd::RefreshDeployments,
    select_cmd: |namespace, name| BackendCmd::SelectDeployment { namespace, name },
    delete_action: ConfirmAction::DeleteDeployment,
    restart_action: ConfirmAction::RestartDeployment,
    match_list: |event| match event {
        BackendEvent::ResourceList(ResourceListEvent::DeploymentsUpdated(v)) => Some(v.clone()),
        _ => None,
    },
    match_details: |event| match event {
        BackendEvent::ResourceRead(ResourceReadEvent::DeploymentDetailsUpdated(d)) => {
            Some(d.clone())
        }
        _ => None,
    },
    match_metrics: Some(|event| match event {
        BackendEvent::ResourceRead(ResourceReadEvent::DeploymentMetricsUpdated(m)) => {
            Some(m.clone())
        }
        _ => None,
    }),
};

pub static DAEMONSET_CONFIG: WorkloadKindConfig = WorkloadKindConfig {
    view_id: "workloads_daemonsets",
    label: "DaemonSet",
    grid_title: "DaemonSets",
    kind: WorkloadKind::DaemonSet,
    supports_scaling: false,
    refresh_cmd: BackendCmd::RefreshDaemonSets,
    select_cmd: |namespace, name| BackendCmd::SelectDaemonSet { namespace, name },
    delete_action: ConfirmAction::DeleteDaemonSet,
    restart_action: ConfirmAction::RestartDaemonSet,
    match_list: |event| match event {
        BackendEvent::ResourceList(ResourceListEvent::DaemonSetsUpdated(v)) => Some(v.clone()),
        _ => None,
    },
    match_details: |event| match event {
        BackendEvent::ResourceRead(ResourceReadEvent::DaemonSetDetailsUpdated(d)) => {
            Some(d.clone())
        }
        _ => None,
    },
    // Backend does emit DaemonSetMetricsUpdated, but the values are a hardcoded
    // stub today (see backend/metrics.rs) — not wiring it up here so this view
    // doesn't newly display fabricated numbers; matches prior behavior.
    match_metrics: None,
};

pub static STATEFULSET_CONFIG: WorkloadKindConfig = WorkloadKindConfig {
    view_id: "workloads_statefulsets",
    label: "StatefulSet",
    grid_title: "StatefulSets",
    kind: WorkloadKind::StatefulSet,
    supports_scaling: true,
    refresh_cmd: BackendCmd::RefreshStatefulSets,
    select_cmd: |namespace, name| BackendCmd::SelectStatefulSet { namespace, name },
    delete_action: ConfirmAction::DeleteStatefulSet,
    restart_action: ConfirmAction::RestartStatefulSet,
    match_list: |event| match event {
        BackendEvent::ResourceList(ResourceListEvent::StatefulSetsUpdated(v)) => Some(v.clone()),
        _ => None,
    },
    match_details: |event| match event {
        BackendEvent::ResourceRead(ResourceReadEvent::StatefulSetDetailsUpdated(d)) => {
            Some(d.clone())
        }
        _ => None,
    },
    // See DAEMONSET_CONFIG note: metrics values are a stub, not wired up.
    match_metrics: None,
};

pub static REPLICASET_CONFIG: WorkloadKindConfig = WorkloadKindConfig {
    view_id: "workloads_replicasets",
    label: "ReplicaSet",
    grid_title: "ReplicaSets",
    kind: WorkloadKind::ReplicaSet,
    supports_scaling: true,
    refresh_cmd: BackendCmd::RefreshReplicaSets,
    select_cmd: |namespace, name| BackendCmd::SelectReplicaSet { namespace, name },
    delete_action: ConfirmAction::DeleteReplicaSet,
    restart_action: ConfirmAction::RestartReplicaSet,
    match_list: |event| match event {
        BackendEvent::ResourceList(ResourceListEvent::ReplicaSetsUpdated(v)) => Some(v.clone()),
        _ => None,
    },
    match_details: |event| match event {
        BackendEvent::ResourceRead(ResourceReadEvent::ReplicaSetDetailsUpdated(d)) => {
            Some(d.clone())
        }
        _ => None,
    },
    match_metrics: None,
};

pub static REPLICATIONCONTROLLER_CONFIG: WorkloadKindConfig = WorkloadKindConfig {
    view_id: "workloads_replicationcontrollers",
    label: "ReplicationController",
    grid_title: "ReplicationControllers",
    kind: WorkloadKind::ReplicationController,
    supports_scaling: true,
    refresh_cmd: BackendCmd::RefreshReplicationControllers,
    select_cmd: |namespace, name| BackendCmd::SelectReplicationController { namespace, name },
    delete_action: ConfirmAction::DeleteReplicationController,
    restart_action: ConfirmAction::RestartReplicationController,
    match_list: |event| match event {
        BackendEvent::ResourceList(ResourceListEvent::ReplicationControllersUpdated(v)) => {
            Some(v.clone())
        }
        _ => None,
    },
    match_details: |event| match event {
        BackendEvent::ResourceRead(ResourceReadEvent::ReplicationControllerDetailsUpdated(d)) => {
            Some(d.clone())
        }
        _ => None,
    },
    match_metrics: None,
};

/// State shared by all workload kinds
pub struct WorkloadViewState {
    pub summaries: Vec<WorkloadSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<WorkloadDetails>,
    pub metrics: Option<WorkloadMetricsAggregate>,
    pub table: Table<WorkloadSummary>,
    pub metrics_polling_started: bool,
    pub metrics_interval_secs: u64,
    pub namespace_filter: Vec<String>, // Empty = show all
}

impl WorkloadViewState {
    fn new() -> Self {
        Self {
            summaries: Vec::new(),
            selected: None,
            details: None,
            metrics: None,
            table: Table::new(Vec::new()),
            metrics_polling_started: false,
            metrics_interval_secs: 15,
            namespace_filter: Vec::new(),
        }
    }
}

/// Single [`ViewComponent`] implementation shared by all 5 workload kinds.
/// Kind-specific behavior (command variants, event matching, scaling support)
/// comes from the `'static` [`WorkloadKindConfig`] rather than a bespoke
/// struct + impl per kind.
pub struct WorkloadView {
    state: WorkloadViewState,
    config: &'static WorkloadKindConfig,
}

impl WorkloadView {
    pub fn new(config: &'static WorkloadKindConfig) -> Self {
        Self {
            state: WorkloadViewState::new(),
            config,
        }
    }
}

pub fn deployment_view() -> WorkloadView {
    WorkloadView::new(&DEPLOYMENT_CONFIG)
}

pub fn daemonset_view() -> WorkloadView {
    WorkloadView::new(&DAEMONSET_CONFIG)
}

pub fn statefulset_view() -> WorkloadView {
    WorkloadView::new(&STATEFULSET_CONFIG)
}

pub fn replicaset_view() -> WorkloadView {
    WorkloadView::new(&REPLICASET_CONFIG)
}

pub fn replicationcontroller_view() -> WorkloadView {
    WorkloadView::new(&REPLICATIONCONTROLLER_CONFIG)
}

impl ViewComponent for WorkloadView {
    fn id(&self) -> &'static str {
        self.config.view_id
    }

    fn set_namespace_filter(&mut self, filter: Vec<String>) {
        self.state.namespace_filter = filter;
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        // Filter by namespace (empty filter = show all)
        let filtered: Vec<WorkloadSummary> = self
            .state
            .summaries
            .iter()
            .filter(|s| {
                self.state.namespace_filter.is_empty()
                    || self.state.namespace_filter.contains(&s.namespace)
            })
            .cloned()
            .collect();

        self.state.table.set_items(filtered);

        let newly_selected =
            render_workload_grid(ui, self.config.grid_title, &mut self.state.table);

        // Bulk delete toolbar — shown when 1+ rows are checkbox-selected
        let selected_count = self.state.table.selected_count();
        if selected_count > 0 {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(format!("{} selected", selected_count));
                let btn = egui::Button::new(
                    egui::RichText::new(format!("🗑 Delete {}", selected_count))
                        .color(egui::Color32::from_rgb(220, 70, 70)),
                );
                if ui.add(btn).clicked() {
                    let search_fn = |w: &WorkloadSummary, query: &str| {
                        w.namespace.to_lowercase().contains(query)
                            || w.name.to_lowercase().contains(query)
                    };
                    let items: Vec<(String, String)> = self
                        .state
                        .table
                        .get_selected_items(search_fn)
                        .into_iter()
                        .map(|w| (w.namespace, w.name))
                        .collect();
                    if !items.is_empty() {
                        actions.push(ViewAction::OpenModal(ConfirmAction::BulkDelete(items)));
                    }
                }
                if ui.button("Clear selection").clicked() {
                    self.state.table.deselect_all();
                }
            });
        }

        // Only treat as newly selected if it's different from current selection
        if newly_selected != self.state.selected {
            if let Some((ns, name)) = newly_selected {
                self.state.selected = Some((ns.clone(), name.clone()));
                self.state.details = None;
                actions.push(ViewAction::SendCommand((self.config.select_cmd)(ns, name)));
            } else {
                self.state.selected = None;
                self.state.details = None;
            }
        }

        actions
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let Some(summaries) = (self.config.match_list)(event) {
            self.state.summaries = summaries;
        }

        if let Some(details) = (self.config.match_details)(event)
            && Some((details.namespace.clone(), details.name.clone())) == self.state.selected
        {
            self.state.details = Some(details);
        }

        if let Some(match_metrics) = self.config.match_metrics
            && let Some(metrics) = match_metrics(event)
            && Some((metrics.namespace.clone(), metrics.name.clone())) == self.state.selected
        {
            self.state.metrics = Some(metrics);
        }
    }

    fn has_details(&self) -> bool {
        self.state.selected.is_some()
    }
    #[allow(clippy::too_many_lines)] // removed by T125 (egui cutover)
    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        if self.state.selected.is_none() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let (ns, name) = self.state.selected.as_ref().unwrap().clone();

        // Auto-start metrics polling once details (and the real pod label
        // selector) have loaded, rather than guessing pod ownership by name.
        if !self.state.metrics_polling_started
            && let Some(details) = &self.state.details
        {
            self.state.metrics_polling_started = true;
            actions.push(ViewAction::SendCommand(
                BackendCmd::StartWorkloadMetricsPolling {
                    namespace: ns.clone(),
                    workload_name: name.clone(),
                    workload_kind: self.config.label.to_string(),
                    pod_selector: details.pod_selector.clone(),
                    interval_secs: self.state.metrics_interval_secs,
                },
            ));
        }

        ui.horizontal(|ui| {
            ui.heading(self.config.label);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    actions.push(ViewAction::OpenModal((self.config.delete_action)(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                if ui.button("Restart").clicked() {
                    actions.push(ViewAction::OpenModal((self.config.restart_action)(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                if self.config.supports_scaling && ui.button("Scale").clicked() {
                    let current = self
                        .state
                        .summaries
                        .iter()
                        .find(|s| s.namespace == ns && s.name == name)
                        .map(|s| s.desired)
                        .unwrap_or(0);

                    actions.push(ViewAction::OpenScaleModal(
                        self.config.kind,
                        ns.clone(),
                        name.clone(),
                        current,
                    ));
                }
                if ui.button("Close").clicked() {
                    self.state.selected = None;
                    self.state.details = None;
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
            if let Some(details) = &self.state.details {
                let summary = self
                    .state
                    .summaries
                    .iter()
                    .find(|s| s.namespace == ns && s.name == name);

                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label("📊 Status");
                        ui.separator();
                        if let Some(summary) = summary {
                            ui.horizontal(|ui| {
                                ui.label(format!("Desired:    {}", summary.desired));
                                ui.label(format!("Ready:      {}", summary.ready));
                                ui.label(format!("Updated:    {}", summary.updated));
                                ui.label(format!("Available:  {}", summary.available));
                            });
                        } else {
                            ui.label("(loading status...)");
                        }
                    });
                });
                ui.add_space(8.0);

                if let Some(metrics) = &self.state.metrics {
                    render_workload_metrics(ui, metrics);
                    ui.add_space(8.0);
                }

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

                ui.group(|ui| {
                    ui.vertical(|ui| {
                        render_kv_collapsing(ui, "🏷️ Labels", &details.labels);
                        render_kv_collapsing(ui, "📝 Annotations", &details.annotations);
                    });
                });
                ui.add_space(8.0);

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

                render_raw_json_collapsing(ui, &details.raw_json);
            } else {
                ui.label(format!(
                    "Loading {} details...",
                    self.config.label.to_lowercase()
                ));
            }
        });

        (true, actions)
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        vec![ViewAction::SendCommand(self.config.refresh_cmd.clone())]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.state.selected = None;
        self.state.details = None;
        Vec::new()
    }
}
