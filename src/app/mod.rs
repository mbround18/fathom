use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, CronJobDetails, CronJobSummary,
    DaemonSetDetails, DaemonSetSummary, DeploymentDetails, DeploymentSummary, JobDetails,
    JobSummary, NotificationLevel, PodDetails, PodMetricsHistory, PodMetricsSummary, PodSummary,
    ReplicaSetDetails, ReplicaSetSummary, ReplicationControllerDetails,
    ReplicationControllerSummary, ResourceListEvent, ScopeKind, SectionKey, StatefulSetDetails,
    StatefulSetSummary, SystemEvent, section_keys,
};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::collections::HashMap;
use views::{
    BoxedView, ContextView, CronJobsView, DaemonSetView, DeploymentView, EventsView,
    IntegrationsView, JobsView, McpIntegrationView, ModalState, ModalView, NamespacesView,
    NavigationView, NodesView, OverviewView, PlaceholderView, PodsView, PortForwardsView,
    ReplicaSetView, ReplicationControllerView, StatefulSetView, StatusView, ViewAction,
    ViewComponent,
};

pub mod components;
mod mailbox;
mod search;
pub mod shell;
mod ui_center;
mod ui_left;
mod ui_modal;
mod ui_right;
mod ui_status;
mod ui_top;
pub mod views;

pub struct FathomApp {
    // Comms
    gui_rx: Receiver<BackendEventEnvelope>,
    backend_tx: Sender<BackendCmd>,

    // Views
    views: HashMap<String, BoxedView>,
    previous_section: Option<SectionKey>,

    // Nav
    active_section: SectionKey,

    // Cluster-level section states
    pub overview_state: views::OverviewState,
    pub events_state: views::EventsState,
    pub nodes_state: views::NodesState,
    pub namespaces_state: views::NamespacesState,

    // Port forwards
    port_forwards: Vec<crate::sections::network::PortForward>,

    // Integrations connection state
    prometheus_namespace: String,
    prometheus_service: String,
    prometheus_port: String,
    prometheus_connected: bool,
    prometheus_testing: bool,
    grafana_url: String,
    grafana_api_key: String,
    grafana_connected: bool,
    grafana_testing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadKind {
    Deployment,
    DaemonSet,
    StatefulSet,
    ReplicaSet,
    ReplicationController,
    Job,
    CronJob,
}

#[derive(Clone, Debug)]
pub enum ConfirmAction {
    DeletePod(String, String),
    DeleteDeployment(String, String),
    DeleteDaemonSet(String, String),
    DeleteStatefulSet(String, String),
    DeleteReplicaSet(String, String),
    DeleteReplicationController(String, String),
    DeleteJob(String, String),
    DeleteCronJob(String, String),
    RestartDeployment(String, String),
    RestartDaemonSet(String, String),
    RestartStatefulSet(String, String),
    RestartReplicaSet(String, String),
    RestartReplicationController(String, String),
    ScaleDeployment(String, String, i32), // namespace, name, current_replicas
    ScaleStatefulSet(String, String, i32), // namespace, name, current_replicas
    ScaleReplicaSet(String, String, i32), // namespace, name, current_replicas
    ScaleReplicationController(String, String, i32), // namespace, name, current_replicas
    BulkDelete(Vec<(String, String)>),    // Vec<(namespace, name)>
    OpenPortForward {
        namespace: String,
        pod: String,
        suggested_ports: Vec<u16>,
        containers: Vec<String>,
    },
    EditPortForward {
        tunnel_id: String,
        namespace: String,
        pod: String,
        local_port: u16,
        remote_port: u16,
        container: String,
    },
}

impl FathomApp {
    pub fn new(gui_rx: Receiver<BackendEventEnvelope>, backend_tx: Sender<BackendCmd>) -> Self {
        let mut app = Self {
            gui_rx,
            backend_tx,
            views: HashMap::new(),
            previous_section: None,
            active_section: section_keys::overview(),
            overview_state: views::OverviewState::default(),
            events_state: views::EventsState::default(),
            nodes_state: views::NodesState::default(),
            namespaces_state: views::NamespacesState::default(),
            port_forwards: Vec::new(),
            prometheus_namespace: "monitoring".to_string(),
            prometheus_service: "prometheus".to_string(),
            prometheus_port: "9090".to_string(),
            prometheus_connected: false,
            prometheus_testing: false,
            grafana_url: String::new(),
            grafana_api_key: String::new(),
            grafana_connected: false,
            grafana_testing: false,
        };

        // Initialize status view and push initial messages
        if let Some(status_view) = app.get_status_view() {
            status_view.push("Initializing: connecting to cluster...");
        }

        // Auto-connect on launch (MVP).
        let _ = app.backend_tx.send(BackendCmd::RefreshContexts);
        let _ = app.backend_tx.send(BackendCmd::Initialize);

        app
    }

    /// Get or create a view for the given section key.
    pub fn get_or_create_view(&mut self, section_key: &SectionKey) -> &mut dyn ViewComponent {
        let key_str = section_key.to_string();
        if !self.views.contains_key(&key_str) {
            // Create the appropriate view based on section key
            let view: BoxedView = if section_key == &section_keys::context() {
                Box::new(ContextView::new())
            } else if section_key == &section_keys::navigation() {
                Box::new(NavigationView::new())
            } else if section_key == &section_keys::status() {
                Box::new(StatusView::new())
            } else if section_key == &section_keys::modal() {
                Box::new(ModalView::default())
            } else if section_key == &section_keys::workloads_pods() {
                Box::new(PodsView::new())
            } else if section_key == &section_keys::workloads_deployments() {
                Box::new(DeploymentView::new())
            } else if section_key == &section_keys::workloads_daemonsets() {
                Box::new(DaemonSetView::new())
            } else if section_key == &section_keys::workloads_statefulsets() {
                Box::new(StatefulSetView::new())
            } else if section_key == &section_keys::workloads_replicasets() {
                Box::new(ReplicaSetView::new())
            } else if section_key == &section_keys::workloads_replicationcontrollers() {
                Box::new(ReplicationControllerView::new())
            } else if section_key == &section_keys::workloads_jobs() {
                Box::new(JobsView::new())
            } else if section_key == &section_keys::workloads_cronjobs() {
                Box::new(CronJobsView::new())
            } else if section_key == &section_keys::integrations_metrics() {
                Box::new(IntegrationsView::new("Prometheus"))
            } else if section_key == &section_keys::integrations_grafana() {
                Box::new(IntegrationsView::new("Grafana"))
            } else if section_key == &section_keys::integrations_argo() {
                Box::new(IntegrationsView::new("ArgoCD"))
            } else if section_key == &section_keys::integrations_flux() {
                Box::new(IntegrationsView::new("FluxCD"))
            } else if section_key == &section_keys::integrations_mcp() {
                Box::new(McpIntegrationView::new())
            } else if section_key == &section_keys::overview() {
                Box::new(OverviewView::new())
            } else if section_key == &section_keys::nodes() {
                Box::new(NodesView::new())
            } else if section_key == &section_keys::events() {
                Box::new(EventsView::new())
            } else if section_key == &section_keys::namespaces() {
                Box::new(NamespacesView::new())
            } else if section_key == &section_keys::network_portforwards() {
                Box::new(PortForwardsView::new())
            } else {
                // Placeholder for sections not yet implemented
                Box::new(PlaceholderView::new(section_key.clone()))
            };
            self.views.insert(key_str.clone(), view);
        }
        self.views.get_mut(&key_str).unwrap().as_mut()
    }

    /// Handle view switching: call on_unmount on previous view, on_mount on new view.
    fn handle_view_switch(&mut self) {
        if self.previous_section.is_none()
            || self.previous_section.as_ref() != Some(&self.active_section)
        {
            let prev = self.previous_section.clone();
            let current = self.active_section.clone();
            self.previous_section = Some(current.clone());

            // On unmount: call on_unmount on previous view
            if let Some(prev_key) = prev {
                if let Some(view) = self.views.get_mut(&prev_key.to_string()) {
                    let _ = view.on_unmount();
                }
            }

            // On mount: call on_mount on new view and process returned actions
            let view = self.get_or_create_view(&current);
            let actions = view.on_mount();

            // Process actions returned from on_mount
            for action in actions {
                match action {
                    ViewAction::SendCommand(cmd) => {
                        self.send(cmd);
                    }
                    ViewAction::NotifyStatus(msg) => {
                        self.push_status(msg);
                    }
                    ViewAction::Navigate(section) => {
                        self.active_section = section;
                    }
                    ViewAction::OpenModal(_) | ViewAction::OpenScaleModal(_, _, _, _) => {
                        // Modal actions from on_mount are ignored - modals are for user interactions in render()
                    }
                    ViewAction::OpenUrl(url) => {
                        if let Err(e) = open::that(&url) {
                            tracing::warn!("Failed to open URL {}: {}", url, e);
                            self.push_status(format!("Failed to open URL: {}", e));
                        }
                    }
                }
            }
        }
    }

    fn push_status(&mut self, msg: impl Into<String>) {
        if let Some(status_view) = self.get_status_view() {
            status_view.push(msg);
        }
    }

    fn get_status_view(&mut self) -> Option<&mut StatusView> {
        let status_key = section_keys::status().to_string();
        if !self.views.contains_key(&status_key) {
            self.views
                .insert(status_key.clone(), Box::new(StatusView::new()));
        }
        self.views
            .get_mut(&status_key)
            .and_then(|v| {
                let ptr = v.as_mut() as *mut dyn ViewComponent as *mut StatusView;
                Some(ptr)
            })
            .map(|p| unsafe { &mut *p })
    }

    fn effective_namespace_filter(&self) -> Vec<String> {
        // This is used by other parts of the app that need to know the current filter
        // Get it from the context view
        if let Some(view) = self.views.get(&section_keys::context().to_string()) {
            let ptr = view.as_ref() as *const dyn ViewComponent as *const ContextView;
            if let Some(context_view) = unsafe { ptr.as_ref() } {
                return context_view.effective_namespace_filter();
            }
        }
        Vec::new() // Default to all namespaces
    }

    fn toggle_namespace_filter_from_link(&mut self, namespace: &str) {
        if let Some(view) = self.views.get_mut(&section_keys::context().to_string()) {
            let ptr = view.as_mut() as *mut dyn ViewComponent as *mut ContextView;
            if let Some(context_view) = unsafe { ptr.as_mut() } {
                if context_view.select_all_namespaces {
                    context_view.select_all_namespaces = false;
                    context_view.selected_namespaces.clear();
                    context_view
                        .selected_namespaces
                        .insert(namespace.to_owned());
                } else if context_view.selected_namespaces.contains(namespace) {
                    // If this was the last selected namespace, revert to All.
                    if context_view.selected_namespaces.len() == 1 {
                        context_view.select_all_namespaces = true;
                        context_view.selected_namespaces.clear();
                    } else {
                        context_view.selected_namespaces.remove(namespace);
                    }
                } else {
                    context_view
                        .selected_namespaces
                        .insert(namespace.to_owned());
                }
                self.send(BackendCmd::SetNamespaceFilter {
                    namespaces: context_view.effective_namespace_filter(),
                });
            }
        }
    }

    fn get_namespaces(&self) -> Vec<String> {
        if let Some(view) = self.views.get(&section_keys::context().to_string()) {
            let ptr = view.as_ref() as *const dyn ViewComponent as *const ContextView;
            if let Some(context_view) = unsafe { ptr.as_ref() } {
                return context_view.namespaces.clone();
            }
        }
        Vec::new()
    }

    fn send(&mut self, cmd: BackendCmd) {
        if let Err(e) = self.backend_tx.send(cmd) {
            self.push_status(format!("ERROR sending command: {}", e));
        }
    }

    fn active_scope_kind(&self) -> ScopeKind {
        if self.active_section == section_keys::workloads_pods() {
            return ScopeKind::Namespaced;
        }
        if self.active_section == section_keys::workloads_deployments() {
            return ScopeKind::Namespaced;
        }
        if self.active_section == section_keys::workloads_daemonsets() {
            return ScopeKind::Namespaced;
        }
        if self.active_section == section_keys::workloads_statefulsets() {
            return ScopeKind::Namespaced;
        }
        if self.active_section == section_keys::workloads_replicasets() {
            return ScopeKind::Namespaced;
        }
        if self.active_section == section_keys::workloads_replicationcontrollers() {
            return ScopeKind::Namespaced;
        }
        ScopeKind::Cluster
    }

    fn open_confirm(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        confirm_text: impl Into<String>,
        action: ConfirmAction,
    ) {
        if let Some(view) = self.views.get_mut(&section_keys::modal().to_string()) {
            let ptr = view.as_mut() as *mut dyn ViewComponent as *mut ModalView;
            if let Some(modal_view) = unsafe { ptr.as_mut() } {
                modal_view.state = ModalState::Confirm {
                    title: title.into(),
                    body: body.into(),
                    confirm_text: confirm_text.into(),
                    action,
                };
            }
        }
    }

    fn open_scale_workload(
        &mut self,
        kind: WorkloadKind,
        namespace: String,
        name: String,
        current_ready: i32,
        desired: i32,
    ) {
        if let Some(view) = self.views.get_mut(&section_keys::modal().to_string()) {
            let ptr = view.as_mut() as *mut dyn ViewComponent as *mut ModalView;
            if let Some(modal_view) = unsafe { ptr.as_mut() } {
                modal_view.state = ModalState::ScaleWorkload {
                    kind,
                    namespace,
                    name,
                    current: current_ready,
                    desired,
                };
            }
        }
    }

    fn open_create_port_forward(&mut self) {
        if let Some(view) = self.views.get_mut(&section_keys::modal().to_string()) {
            let ptr = view.as_mut() as *mut dyn ViewComponent as *mut ModalView;
            if let Some(modal_view) = unsafe { ptr.as_mut() } {
                modal_view.state = ModalState::CreatePortForward {
                    namespace: String::new(),
                    pod: String::new(),
                    local_port: String::new(),
                    remote_port: String::new(),
                    container: String::new(),
                };
            }
        }
    }

    fn open_create_port_forward_prefill(
        &mut self,
        namespace: String,
        pod: String,
        remote_port: Option<u16>,
        container: Option<String>,
    ) {
        if let Some(view) = self.views.get_mut(&section_keys::modal().to_string()) {
            let ptr = view.as_mut() as *mut dyn ViewComponent as *mut ModalView;
            if let Some(modal_view) = unsafe { ptr.as_mut() } {
                modal_view.state = ModalState::CreatePortForward {
                    namespace,
                    pod,
                    local_port: String::new(),
                    remote_port: remote_port.map(|p| p.to_string()).unwrap_or_default(),
                    container: container.unwrap_or_default(),
                };
            }
        }
    }
}

impl eframe::App for FathomApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.check_mailbox();

        self.render_top_bar(ctx);
        self.render_left_nav(ctx);
        self.render_right_panel(ctx);
        self.render_center(ctx);
        self.render_bottom_status(ctx);
        self.render_modal(ctx);

        // Keep UI snappy while streams/events are coming in.
        ctx.request_repaint();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Send shutdown command to backend for clean exit
        // This triggers cleanup of all background tasks (port forwards, metrics watches, etc.)
        match self.backend_tx.send(BackendCmd::Shutdown) {
            Ok(_) => tracing::info!("Sent Shutdown command to backend"),
            Err(e) => tracing::error!("Failed to send Shutdown command to backend: {}", e),
        }
    }
}
