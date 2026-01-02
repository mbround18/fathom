use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{
    BackendCmd, BackendEvent, PodDetails, PodMetricsHistory, PodMetricsSummary, PodSummary,
    ResourceListEvent, ResourceReadEvent, StreamEvent,
};

/// Pod-specific state and rendering for the Pods view
pub struct PodsView {
    // List and selection
    pub pods: Vec<PodSummary>,
    pub selected_pod: Option<(String, String)>,

    // Search and sorting
    pub pod_search: String,
    pub pods_sort: PodsSort,

    // Details panel state
    pub pod_tab: PodTab,
    pub pod_details: Option<PodDetails>,

    // Streaming state
    pub pod_logs: Vec<String>,

    // Metrics polling
    pub pod_metrics: Option<PodMetricsSummary>,
    pub pod_metrics_history: Option<PodMetricsHistory>,
    pub pod_metrics_pending: bool,
    pub pod_metrics_polling_started: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PodTab {
    Details,
    Logs,
    Exec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PodsSortKey {
    Namespace,
    Name,
    Phase,
    Ready,
    Restarts,
    Node,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PodsSort {
    pub key: PodsSortKey,
    pub dir: SortDir,
}

impl Default for PodsView {
    fn default() -> Self {
        Self::new()
    }
}

impl PodsView {
    pub fn new() -> Self {
        Self {
            pods: Vec::new(),
            selected_pod: None,
            pod_search: String::new(),
            pods_sort: PodsSort {
                key: PodsSortKey::Namespace,
                dir: SortDir::Asc,
            },
            pod_tab: PodTab::Details,
            pod_details: None,
            pod_logs: Vec::new(),
            pod_metrics: None,
            pod_metrics_history: None,
            pod_metrics_pending: false,
            pod_metrics_polling_started: false,
        }
    }

    /// Get the search-filtered and sorted pod list
    pub fn filtered_pods(&self) -> Vec<PodSummary> {
        let mut filtered: Vec<_> = self
            .pods
            .iter()
            .filter(|p| self.search_matches(p))
            .cloned()
            .collect();

        // Apply sorting
        let sort_key = self.pods_sort.key;
        let is_asc = self.pods_sort.dir == SortDir::Asc;

        filtered.sort_by(|a, b| {
            let cmp = match sort_key {
                PodsSortKey::Namespace => a.namespace.cmp(&b.namespace),
                PodsSortKey::Name => a.name.cmp(&b.name),
                PodsSortKey::Phase => a.phase.cmp(&b.phase),
                PodsSortKey::Ready => a.ready.cmp(&b.ready),
                PodsSortKey::Restarts => a.restarts.cmp(&b.restarts),
                PodsSortKey::Node => a.node.cmp(&b.node),
            };

            if is_asc { cmp } else { cmp.reverse() }
        });

        filtered
    }

    fn search_matches(&self, pod: &PodSummary) -> bool {
        if self.pod_search.is_empty() {
            return true;
        }

        let search_lower = self.pod_search.to_lowercase();
        pod.name.to_lowercase().contains(&search_lower)
            || pod.namespace.to_lowercase().contains(&search_lower)
    }

    pub fn sort_indicator(&self, key: PodsSortKey) -> &'static str {
        if self.pods_sort.key != key {
            return "";
        }
        match self.pods_sort.dir {
            SortDir::Asc => "▲",
            SortDir::Desc => "▼",
        }
    }

    pub fn toggle_sort(&mut self, key: PodsSortKey) {
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

    /// Render Details tab content
    fn render_pod_details_tab(&mut self, ui: &mut egui::Ui, _actions: &mut Vec<ViewAction>) {
        match &self.pod_details {
            Some(details) => {
                // Show metrics if available
                if let Some(history) = &self.pod_metrics_history {
                    ui.label("Metrics:");
                    ui.add_space(4.0);
                    for container in &history.containers {
                        ui.label(&container.name);
                        ui.label(format!(
                            "  CPU: {} / Memory: {}",
                            container
                                .samples
                                .last()
                                .map(|s| format!("{}m", s.cpu_millicores))
                                .unwrap_or_else(|| "-".to_string()),
                            container
                                .samples
                                .last()
                                .map(|s| format!("{}MB", s.memory_bytes / 1024.0 / 1024.0))
                                .unwrap_or_else(|| "-".to_string())
                        ));
                    }
                    ui.separator();
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
                    egui::ScrollArea::vertical()
                        .max_height(250.0)
                        .show(ui, |ui| {
                            ui.monospace(&details.raw_json);
                        });
                });
            }
            None => {
                ui.label("Loading details...");
            }
        }
    }

    /// Render Logs tab content
    fn render_pod_logs_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<ViewAction>) {
        ui.horizontal(|ui| {
            if ui.button("Start").clicked() {
                if let Some((ns, name)) = &self.selected_pod {
                    self.pod_logs.clear();
                    actions.push(ViewAction::SendCommand(BackendCmd::StartPodLogs {
                        namespace: ns.clone(),
                        name: name.clone(),
                        container: None,
                    }));
                }
            }

            if ui.button("Stop").clicked() {
                actions.push(ViewAction::SendCommand(BackendCmd::StopPodLogs));
            }

            if ui.button("Clear").clicked() {
                self.pod_logs.clear();
            }
        });

        ui.add_space(8.0);

        for line in &self.pod_logs {
            ui.monospace(line);
        }
    }

    /// Render Exec tab content
    fn render_pod_exec_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("Exec functionality coming soon.");
        ui.label("Will provide interactive shell access to containers.");
    }
}

impl ViewComponent for PodsView {
    fn id(&self) -> &'static str {
        "workloads_pods"
    }

    fn render(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();

        ui.heading("Pods");
        ui.separator();

        // Search and sort controls
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

        // Pod list grid
        egui::ScrollArea::vertical().show(ui, |ui| {
            let pods = self.filtered_pods();

            egui::Grid::new("pods_grid")
                .striped(true)
                .min_col_width(120.0)
                .show(ui, |ui| {
                    // Column headers with sort indicators
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

                    // Pod rows
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

                    // Handle selection change
                    if let Some((ns, name)) = newly_selected {
                        self.selected_pod = Some((ns.clone(), name.clone()));
                        self.pod_details = None;
                        self.pod_metrics = None;
                        self.pod_metrics_pending = false;
                        self.pod_logs.clear();

                        actions.push(ViewAction::SendCommand(BackendCmd::SelectPod {
                            namespace: ns.clone(),
                            name: name.clone(),
                        }));

                        // Fetch metrics immediately for details panel
                        self.pod_metrics_pending = true;
                        actions.push(ViewAction::SendCommand(BackendCmd::FetchPodMetrics {
                            namespace: ns,
                            name,
                        }));

                        ctx.request_repaint();
                    }
                });
        });

        actions
    }

    fn render_details(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        let Some((ns, name)) = self.selected_pod.clone() else {
            return (false, Vec::new());
        };

        let mut actions = Vec::new();

        ui.horizontal(|ui| {
            ui.heading("Pod");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Delete").clicked() {
                    // TODO: Open modal via ViewAction in Phase 7
                    actions.push(ViewAction::NotifyStatus(format!(
                        "Delete pod {}/{}",
                        ns, name
                    )));
                }
                if ui.button("Close").clicked() {
                    self.selected_pod = None;
                    self.pod_details = None;
                    self.pod_logs.clear();
                    self.pod_metrics = None;
                    self.pod_metrics_history = None;
                    self.pod_metrics_pending = false;
                    self.pod_metrics_polling_started = false;
                    actions.push(ViewAction::SendCommand(BackendCmd::StopPodLogs));
                    actions.push(ViewAction::SendCommand(BackendCmd::StopMetricsPolling));
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(&ns);
            ui.label(format!("/{}", name));
        });

        ui.separator();

        // Tabs
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.pod_tab, PodTab::Details, "Details");
            ui.selectable_value(&mut self.pod_tab, PodTab::Logs, "Logs");
            ui.selectable_value(&mut self.pod_tab, PodTab::Exec, "Exec");
        });

        // Auto-start metrics polling when details panel opens
        if !self.pod_metrics_polling_started {
            self.pod_metrics_polling_started = true;
            actions.push(ViewAction::SendCommand(BackendCmd::StartMetricsPolling {
                namespace: ns.clone(),
                name: name.clone(),
                interval_secs: 15,
            }));
        }

        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| match self.pod_tab {
            PodTab::Details => self.render_pod_details_tab(ui, &mut actions),
            PodTab::Logs => self.render_pod_logs_tab(ui, &mut actions),
            PodTab::Exec => self.render_pod_exec_tab(ui),
        });

        ctx.request_repaint();

        (true, actions)
    }

    fn on_event(&mut self, event: &BackendEvent) {
        // Handle pod list updates
        if let BackendEvent::ResourceList(ResourceListEvent::PodsUpdated(pods)) = event {
            self.pods = pods.clone();
        }

        // Handle pod details updates
        if let BackendEvent::ResourceRead(ResourceReadEvent::PodDetailsUpdated(details)) = event {
            if Some((details.namespace.clone(), details.name.clone())) == self.selected_pod {
                self.pod_details = Some(details.clone());
            }
        }

        // Handle pod logs
        if let BackendEvent::Stream(StreamEvent::PodLogsChunk { lines, .. }) = event {
            self.pod_logs.extend(lines.clone());
            // Keep ring buffer bounded
            const MAX_LOGS: usize = 1000;
            if self.pod_logs.len() > MAX_LOGS {
                self.pod_logs.drain(0..self.pod_logs.len() - MAX_LOGS);
            }
        }

        // Handle metrics updates
        if let BackendEvent::ResourceRead(ResourceReadEvent::PodMetricsHistoryUpdated(metrics)) =
            event
        {
            if Some((metrics.namespace.clone(), metrics.name.clone())) == self.selected_pod {
                self.pod_metrics_history = Some(metrics.clone());
            }
        }
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        // When pods view becomes active, request initial fetch
        vec![ViewAction::SendCommand(BackendCmd::RefreshPods)]
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        // When switching away from pods view, stop any active streams
        let mut actions = Vec::new();

        if self.selected_pod.is_some() {
            actions.push(ViewAction::SendCommand(BackendCmd::StopPodLogs));
            actions.push(ViewAction::SendCommand(BackendCmd::StopMetricsPolling));
        }

        self.selected_pod = None;
        self.pod_details = None;
        self.pod_logs.clear();
        self.pod_metrics = None;
        self.pod_metrics_history = None;
        self.pod_metrics_polling_started = false;

        actions
    }
}
