use super::traits::{ViewAction, ViewComponent};
use crate::app::components::{self, LogStreamId, LogStreamState, LogViewer};
use crate::protocol::{
    BackendCmd, BackendEvent, PodDetails, PodMetricsHistory, PodMetricsSummary, PodSummary,
    PrometheusMetrics, ResourceListEvent, ResourceReadEvent, StreamEvent,
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

    // Logs streaming (now using generic LogStreamState)
    pub pod_logs_stream: Option<LogStreamState>,

    // Metrics polling
    pub pod_metrics: Option<PodMetricsSummary>,
    pub pod_metrics_history: Option<PodMetricsHistory>,
    pub pod_prometheus_metrics: Option<PrometheusMetrics>,
    pub pod_metrics_pending: bool,
    pub pod_metrics_polling_started: bool,
    pub pod_metrics_polling_notified: bool,
    pub pod_metrics_interval_secs: u64,
    pub pod_metrics_interval_prev: u64, // Track previous interval to detect changes
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
            pod_logs_stream: None,
            pod_metrics: None,
            pod_metrics_history: None,
            pod_prometheus_metrics: None,
            pod_metrics_pending: false,
            pod_metrics_polling_started: false,
            pod_metrics_polling_notified: false,
            pod_metrics_interval_secs: 15,
            pod_metrics_interval_prev: 15,
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
    fn render_pod_details_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<ViewAction>) {
        match &self.pod_details {
            Some(details) => {
                // Prometheus metrics section (displayed first if available)
                if let Some(prom_metrics) = &self.pod_prometheus_metrics {
                    ui.group(|ui| {
                        ui.label(egui::RichText::new("📊 Prometheus Metrics").strong());
                        ui.separator();

                        // CPU metrics
                        if let Some(cpu_pct) = prom_metrics.cpu.usage_percent {
                            let color = if cpu_pct > 80.0 {
                                egui::Color32::RED
                            } else if cpu_pct > 50.0 {
                                egui::Color32::YELLOW
                            } else {
                                egui::Color32::GREEN
                            };

                            ui.horizontal(|ui| {
                                ui.label("CPU:");
                                ui.colored_label(color, format!("{:.1}%", cpu_pct));
                            });
                        } else {
                            ui.label("CPU: (unavailable)");
                        }

                        // Memory metrics
                        if let Some(mem_bytes) = prom_metrics.memory.usage_bytes {
                            let mem_mb = mem_bytes as f64 / 1_048_576.0;
                            let color = if let Some(limit_bytes) = prom_metrics.memory.limit_bytes {
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
                                ui.label("Memory:");
                                ui.colored_label(color, format!("{:.1} MB", mem_mb));
                                if let Some(limit_bytes) = prom_metrics.memory.limit_bytes {
                                    let limit_mb = limit_bytes as f64 / 1_048_576.0;
                                    ui.label(format!("/ {:.1} MB", limit_mb));
                                }
                            });
                        } else {
                            ui.label("Memory: (unavailable)");
                        }

                        // Network metrics
                        if let Some(rx_bytes) = prom_metrics.network.bytes_in {
                            let rx_mb = rx_bytes as f64 / 1_048_576.0;
                            ui.horizontal(|ui| {
                                ui.label("Network RX:");
                                ui.monospace(format!("{:.2} MB", rx_mb));
                            });
                        }

                        if let Some(tx_bytes) = prom_metrics.network.bytes_out {
                            let tx_mb = tx_bytes as f64 / 1_048_576.0;
                            ui.horizontal(|ui| {
                                ui.label("Network TX:");
                                ui.monospace(format!("{:.2} MB", tx_mb));
                            });
                        }

                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!(
                                "Last updated: {}",
                                prom_metrics.timestamp
                            ))
                            .small(),
                        );
                    });
                    ui.add_space(8.0);
                }

                // Kubernetes metrics section (if available and different from Prometheus)
                if let Some(history) = &self.pod_metrics_history {
                    components::render_metrics_history(
                        ui,
                        &history.containers,
                        "📈 Kubernetes Metrics",
                    );
                }

                // Pod metadata section
                ui.group(|ui| {
                    ui.label(egui::RichText::new("ℹ️ Pod Info").strong());
                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.label("Namespace:");
                        ui.monospace(&details.namespace);
                    });

                    ui.horizontal(|ui| {
                        ui.label("Created:");
                        ui.monospace(&details.creation_timestamp);
                    });
                });
                ui.add_space(8.0);

                // Ownership chain if available
                if details.owner.is_some() || details.resolved_owner.is_some() {
                    ui.group(|ui| {
                        ui.label(egui::RichText::new("🔗 Ownership").strong());
                        ui.separator();

                        if let Some(owner) = &details.owner {
                            ui.horizontal(|ui| {
                                ui.label("Direct owner:");
                                ui.monospace(format!("{} / {}", owner.kind, owner.name));
                            });
                        }

                        if let Some(resolved) = &details.resolved_owner {
                            ui.horizontal(|ui| {
                                ui.label("Resolved owner:");
                                ui.monospace(format!("{} / {}", resolved.kind, resolved.name));
                            });
                        }
                    });
                    ui.add_space(8.0);
                }

                // Containers section
                ui.group(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "📦 Containers ({})",
                            details.containers.len()
                        ))
                        .strong(),
                    );
                    ui.separator();

                    for container in &details.containers {
                        ui.monospace(format!("• {}", container));
                    }
                });
                ui.add_space(8.0);

                // Ports section if any
                if !details.ports.is_empty() {
                    ui.group(|ui| {
                        ui.label(
                            egui::RichText::new(format!("🔌 Ports ({})", details.ports.len()))
                                .strong(),
                        );
                        ui.separator();

                        for port in &details.ports {
                            let name_str = port.name.as_deref().unwrap_or("(unnamed)");
                            ui.horizontal(|ui| {
                                ui.monospace(format!(
                                    "• {} : {} ({})",
                                    name_str, port.port, port.protocol
                                ));
                                if ui.small_button("⚡ Forward").clicked() {
                                    let containers: Vec<String> = details
                                        .containers
                                        .iter()
                                        .map(|c| {
                                            c.split_whitespace().next().unwrap_or(c).to_string()
                                        })
                                        .collect();

                                    actions.push(ViewAction::OpenModal(
                                        crate::app::ConfirmAction::OpenPortForward {
                                            namespace: details.namespace.clone(),
                                            pod: details.name.clone(),
                                            suggested_ports: vec![port.port],
                                            containers,
                                        },
                                    ));
                                }
                            });
                        }
                    });
                    ui.add_space(8.0);
                }

                // Resource requests/limits
                if !details.resource_specs.is_empty() {
                    ui.group(|ui| {
                        ui.label(egui::RichText::new("⚙️ Resources").strong());
                        ui.separator();

                        for spec in &details.resource_specs {
                            ui.label(egui::RichText::new(&spec.container).strong());

                            ui.horizontal(|ui| {
                                ui.label("CPU:");
                                if let Some(req) = spec.cpu_request_millicores {
                                    ui.monospace(format!("{}m", req as i32));
                                }
                                ui.label("/");
                                if let Some(lim) = spec.cpu_limit_millicores {
                                    ui.monospace(format!("{}m", lim as i32));
                                } else {
                                    ui.label("-");
                                }
                            });

                            ui.horizontal(|ui| {
                                ui.label("Memory:");
                                if let Some(req) = spec.memory_request_bytes {
                                    ui.monospace(format!("{:.0}Mi", req / 1024.0 / 1024.0));
                                }
                                ui.label("/");
                                if let Some(lim) = spec.memory_limit_bytes {
                                    ui.monospace(format!("{:.0}Mi", lim / 1024.0 / 1024.0));
                                } else {
                                    ui.label("-");
                                }
                            });
                        }
                    });
                    ui.add_space(8.0);
                }

                // Labels
                if !details.labels.is_empty() {
                    ui.collapsing(
                        egui::RichText::new(format!("🏷️  Labels ({})", details.labels.len()))
                            .strong(),
                        |ui| {
                            for (k, v) in &details.labels {
                                ui.monospace(format!("{} = {}", k, v));
                            }
                        },
                    );
                    ui.add_space(6.0);
                }

                // Annotations
                if !details.annotations.is_empty() {
                    ui.collapsing(
                        egui::RichText::new(format!(
                            "📝 Annotations ({})",
                            details.annotations.len()
                        ))
                        .strong(),
                        |ui| {
                            for (k, v) in &details.annotations {
                                ui.monospace(format!("{}", k));
                                ui.monospace(format!("  {}", v));
                                ui.add_space(2.0);
                            }
                        },
                    );
                    ui.add_space(6.0);
                }

                // Raw JSON
                ui.collapsing("📄 Raw JSON", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            ui.monospace(&details.raw_json);
                        });
                });
            }
            None => {
                ui.label("No pod selected or details still loading...");
            }
        }
    }

    /// Render Logs tab content
    fn render_pod_logs_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<ViewAction>) {
        // Initialize log stream if pod is selected and stream doesn't exist
        if let Some((ns, name)) = &self.selected_pod {
            if self.pod_logs_stream.is_none() {
                let stream_id = LogStreamId::Pod {
                    namespace: ns.clone(),
                    name: name.clone(),
                };
                self.pod_logs_stream = Some(LogStreamState::new(stream_id));
            }
        }

        if let Some(stream) = &mut self.pod_logs_stream {
            // Get available containers from pod details
            let containers = self
                .pod_details
                .as_ref()
                .map(|d| d.containers.clone())
                .unwrap_or_default();

            // Render full log viewer with controls and filter
            let (should_start, should_stop, _should_clear, _container_changed) =
                LogViewer::render_full(ui, stream, &containers);

            // Handle start action
            if should_start {
                if let Some((ns, name)) = &self.selected_pod {
                    actions.push(ViewAction::SendCommand(BackendCmd::StartPodLogs {
                        namespace: ns.clone(),
                        name: name.clone(),
                        container: stream.container.clone(),
                    }));
                }
            }

            // Handle stop action
            if should_stop {
                actions.push(ViewAction::SendCommand(BackendCmd::StopPodLogs));
            }
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
                SortDir::Asc => "▲",
                SortDir::Desc => "▼",
            };
            if ui.button(dir_text).clicked() {
                self.pods_sort.dir = match self.pods_sort.dir {
                    SortDir::Asc => SortDir::Desc,
                    SortDir::Desc => SortDir::Asc,
                };
            }

            if ui.button("Reset Sort").clicked() {
                self.pods_sort.key = PodsSortKey::Namespace;
                self.pods_sort.dir = SortDir::Asc;
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
                        // Stop logs for previous pod if switching pods
                        if let Some((prev_ns, prev_name)) = &self.selected_pod {
                            if (prev_ns, prev_name) != (&ns, &name) {
                                actions.push(ViewAction::SendCommand(BackendCmd::StopPodLogs));
                            }
                        }

                        self.selected_pod = Some((ns.clone(), name.clone()));
                        self.pod_details = None;
                        self.pod_metrics = None;
                        self.pod_metrics_history = None;
                        self.pod_prometheus_metrics = None;
                        self.pod_metrics_pending = false;
                        self.pod_metrics_polling_started = false;
                        self.pod_metrics_polling_notified = false;
                        self.pod_metrics_interval_prev = self.pod_metrics_interval_secs;
                        self.pod_logs_stream = None;

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
                    actions.push(ViewAction::OpenModal(crate::app::ConfirmAction::DeletePod(
                        ns.clone(),
                        name.clone(),
                    )));
                }
                if ui.button("Close").clicked() {
                    self.selected_pod = None;
                    self.pod_details = None;
                    self.pod_logs_stream = None;
                    self.pod_metrics = None;
                    self.pod_prometheus_metrics = None;
                    self.pod_metrics_history = None;
                    self.pod_metrics_pending = false;
                    self.pod_metrics_polling_started = false;
                    self.pod_metrics_polling_notified = false;
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

        // Tabs + metrics interval control
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.pod_tab, PodTab::Details, "Details");
            ui.selectable_value(&mut self.pod_tab, PodTab::Logs, "Logs");
            ui.selectable_value(&mut self.pod_tab, PodTab::Exec, "Exec");

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label("Metrics refresh (sec):");
                let mut interval_str = self.pod_metrics_interval_secs.to_string();
                ui.text_edit_singleline(&mut interval_str);
                if let Ok(parsed) = interval_str.parse::<u64>() {
                    if parsed > 0 && parsed <= 300 {
                        // Allow 1-300 seconds
                        // Only restart if the interval actually changed
                        if parsed != self.pod_metrics_interval_secs
                            && self.pod_metrics_polling_started
                        {
                            self.pod_metrics_interval_secs = parsed;
                            self.pod_metrics_interval_prev = parsed;
                            actions.push(ViewAction::SendCommand(BackendCmd::StopMetricsPolling));
                            actions.push(ViewAction::SendCommand(
                                BackendCmd::StartMetricsPolling {
                                    namespace: ns.clone(),
                                    name: name.clone(),
                                    interval_secs: self.pod_metrics_interval_secs,
                                },
                            ));
                        } else {
                            self.pod_metrics_interval_secs = parsed;
                        }
                    }
                }
            });
        });

        // Auto-start metrics polling when details panel opens (only once per pod selection)
        if !self.pod_metrics_polling_started {
            self.pod_metrics_polling_started = true;
            actions.push(ViewAction::SendCommand(BackendCmd::StartMetricsPolling {
                namespace: ns.clone(),
                name: name.clone(),
                interval_secs: self.pod_metrics_interval_secs,
            }));
        }
        // Mark as notified to suppress spam from backend notification system
        self.pod_metrics_polling_notified = true;

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

        // Handle pod logs streaming
        if let BackendEvent::Stream(StreamEvent::PodLogsChunk { lines, .. }) = event {
            if let Some(stream) = &mut self.pod_logs_stream {
                stream.add_lines(lines.clone());
                // Keep ring buffer bounded
                const MAX_LOGS: usize = 1000;
                if stream.logs.len() > MAX_LOGS {
                    stream.logs.drain(0..stream.logs.len() - MAX_LOGS);
                }
            }
        }

        // Handle metrics updates - merge with existing data to accumulate history
        if let BackendEvent::ResourceRead(ResourceReadEvent::PodMetricsHistoryUpdated(metrics)) =
            event
        {
            if Some((metrics.namespace.clone(), metrics.name.clone())) == self.selected_pod {
                if let Some(ref mut existing) = self.pod_metrics_history {
                    // Merge containers: update each container's samples while preserving old ones
                    for new_container in &metrics.containers {
                        if let Some(existing_container) = existing
                            .containers
                            .iter_mut()
                            .find(|c| c.name == new_container.name)
                        {
                            // Merge samples: keep existing + add new ones
                            // (assumes backend returns recent data)
                            let mut merged_samples = existing_container.samples.clone();
                            for new_sample in &new_container.samples {
                                // Avoid duplicates by timestamp
                                if !merged_samples
                                    .iter()
                                    .any(|s| s.timestamp == new_sample.timestamp)
                                {
                                    merged_samples.push(new_sample.clone());
                                }
                            }
                            // Keep bounded history (max 100 samples)
                            if merged_samples.len() > 100 {
                                merged_samples = merged_samples
                                    .into_iter()
                                    .rev()
                                    .take(100)
                                    .collect::<Vec<_>>();
                                merged_samples.reverse();
                            }
                            existing_container.samples = merged_samples;
                        } else {
                            // New container, add it
                            existing.containers.push(new_container.clone());
                        }
                    }
                } else {
                    // First update, just store the metrics
                    self.pod_metrics_history = Some(metrics.clone());
                }
            }
        }

        // Handle Prometheus metrics updates
        if let BackendEvent::ResourceRead(ResourceReadEvent::PrometheusMetricsUpdated(metrics)) =
            event
        {
            if Some((metrics.namespace.clone(), metrics.name.clone())) == self.selected_pod {
                self.pod_prometheus_metrics = Some(metrics.clone());
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
        self.pod_logs_stream = None;
        self.pod_metrics = None;
        self.pod_prometheus_metrics = None;
        self.pod_metrics_history = None;
        self.pod_metrics_polling_started = false;

        actions
    }
}
