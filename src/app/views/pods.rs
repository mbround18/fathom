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
}

impl ViewComponent for PodsView {
    fn id(&self) -> &'static str {
        "workloads_pods"
    }

    fn render(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        // For now, return empty actions. Actual rendering moved from ui_center.rs will go here.
        // This is a placeholder that demonstrates the component can be rendered.
        Vec::new()
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
