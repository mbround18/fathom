use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, NotificationLevel, SectionKey, SystemEvent,
    section_keys,
};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::collections::HashMap;
use views::{
    BoxedView, ClusterRoleBindingsView, ClusterRolesView, ConfigMapsView, ContextView,
    CronJobsView, EndpointsView, EventsView, FluxView, HorizontalPodAutoscalersView,
    IngressClassesView, IngressesView, IntegrationsView, JobsView, LeasesView, LimitRangesView,
    McpIntegrationView, ModalView, MutatingWebhooksView, NamespacesView, NavigationView,
    NetworkPoliciesView, NodesView, OverviewView, PersistentVolumeClaimsView,
    PersistentVolumesView, PlaceholderView, PodDisruptionBudgetsView, PodsView, PortForwardsView,
    PriorityClassesView, ResourceQuotasView, RoleBindingsView, RolesView, RuntimeClassesView,
    SecretsView, ServicesView, StatusView, StorageClassesView, ValidatingWebhooksView, ViewAction,
    ViewComponent, daemonset_view, deployment_view, replicaset_view, replicationcontroller_view,
    statefulset_view,
};

pub mod components;
pub mod icons;
mod mailbox;
mod ui_center;
mod ui_left;
mod ui_modal;
mod ui_right;
mod ui_status;
mod ui_top;
pub mod utilities;
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

    // UI state
    show_details_panel: bool,

    // Filtering
    pub selected_namespaces: Vec<String>, // Empty = show all
    pub all_namespaces: Vec<String>,      // Available namespaces for filter dropdown

    // Cluster-level section states
    pub overview_state: views::OverviewState,
    pub events_state: views::EventsState,
    pub nodes_state: views::NodesState,
    pub namespaces_state: views::NamespacesState,

    // Port forwards
    port_forwards: Vec<crate::sections::network::PortForward>,

    // Integrations connection state
    prometheus_connected: bool,
    prometheus_testing: bool,
    grafana_connected: bool,
    grafana_testing: bool,

    // Disk usage
    disk_usage: Vec<crate::protocol::DiskUsage>,
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
    DeleteService(String, String),
    DeleteIngress(String, String),
    DeleteNetworkPolicy(String, String),
    DeletePersistentVolume(String),
    DeletePersistentVolumeClaim(String, String),
    DeleteStorageClass(String),
    DeleteRole(String, String),
    DeleteRoleBinding(String, String),
    DeleteClusterRole(String),
    DeleteClusterRoleBinding(String),
    DeleteConfigMap(String, String),
    DeleteSecret(String, String),
    DeleteResourceQuota(String, String),
    DeleteLimitRange(String, String),
    DeleteHorizontalPodAutoscaler(String, String),
    DeletePodDisruptionBudget(String, String),
    DeletePriorityClass(String),
    DeleteRuntimeClass(String),
    DeleteLease(String, String),
    DeleteMutatingWebhookConfiguration(String),
    DeleteValidatingWebhookConfiguration(String),
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
            show_details_panel: false,
            selected_namespaces: Vec::new(), // Empty = show all
            all_namespaces: Vec::new(),
            overview_state: views::OverviewState::default(),
            events_state: views::EventsState::default(),
            nodes_state: views::NodesState::default(),
            namespaces_state: views::NamespacesState::default(),
            port_forwards: Vec::new(),
            prometheus_connected: false,
            prometheus_testing: false,
            disk_usage: Vec::new(),
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

    /// Apply namespace filter to a view by its ID
    ///
    /// `set_namespace_filter` is a no-op default on `ViewComponent`, overridden
    /// only by views that scope their resource list by namespace — so this can
    /// dispatch through the trait directly instead of downcasting per view type.
    pub fn apply_namespace_filter_to_view(
        view: &mut dyn ViewComponent,
        _view_id: &str,
        filter: &[String],
    ) {
        view.set_namespace_filter(filter.to_vec());
    }

    /// Get or create a view for the given section key.
    #[allow(clippy::cognitive_complexity)] // removed by T125 (egui cutover)
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
                Box::new(deployment_view())
            } else if section_key == &section_keys::workloads_daemonsets() {
                Box::new(daemonset_view())
            } else if section_key == &section_keys::workloads_statefulsets() {
                Box::new(statefulset_view())
            } else if section_key == &section_keys::workloads_replicasets() {
                Box::new(replicaset_view())
            } else if section_key == &section_keys::workloads_replicationcontrollers() {
                Box::new(replicationcontroller_view())
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
                Box::new(FluxView::new())
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
            } else if section_key == &section_keys::network_services() {
                Box::new(ServicesView::new())
            } else if section_key == &section_keys::network_endpoints() {
                Box::new(EndpointsView::new())
            } else if section_key == &section_keys::network_ingress() {
                Box::new(IngressesView::new())
            } else if section_key == &section_keys::network_ingressclasses() {
                Box::new(IngressClassesView::new())
            } else if section_key == &section_keys::network_policies() {
                Box::new(NetworkPoliciesView::new())
            } else if section_key == &section_keys::storage_pv() {
                Box::new(PersistentVolumesView::new())
            } else if section_key == &section_keys::storage_pvc() {
                Box::new(PersistentVolumeClaimsView::new())
            } else if section_key == &section_keys::storage_storageclasses() {
                Box::new(StorageClassesView::new())
            } else if section_key == &section_keys::access_roles() {
                Box::new(RolesView::new())
            } else if section_key == &section_keys::access_rolebindings() {
                Box::new(RoleBindingsView::new())
            } else if section_key == &section_keys::access_clusterroles() {
                Box::new(ClusterRolesView::new())
            } else if section_key == &section_keys::access_clusterrolebindings() {
                Box::new(ClusterRoleBindingsView::new())
            } else if section_key == &section_keys::config_configmaps() {
                Box::new(ConfigMapsView::new())
            } else if section_key == &section_keys::config_secrets() {
                Box::new(SecretsView::new())
            } else if section_key == &section_keys::config_resourcequotas() {
                Box::new(ResourceQuotasView::new())
            } else if section_key == &section_keys::config_limitranges() {
                Box::new(LimitRangesView::new())
            } else if section_key == &section_keys::config_horizontalpodautoscalers() {
                Box::new(HorizontalPodAutoscalersView::new())
            } else if section_key == &section_keys::config_poddisruptionbudgets() {
                Box::new(PodDisruptionBudgetsView::new())
            } else if section_key == &section_keys::config_priorityclasses() {
                Box::new(PriorityClassesView::new())
            } else if section_key == &section_keys::config_runtimeclasses() {
                Box::new(RuntimeClassesView::new())
            } else if section_key == &section_keys::config_leases() {
                Box::new(LeasesView::new())
            } else if section_key == &section_keys::config_mutatingwebhooks() {
                Box::new(MutatingWebhooksView::new())
            } else if section_key == &section_keys::config_validatingwebhooks() {
                Box::new(ValidatingWebhooksView::new())
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

            // Reset details panel visibility on section change
            self.show_details_panel = false;

            // On unmount: call on_unmount on previous view
            if let Some(prev_key) = prev
                && let Some(view) = self.views.get_mut(&prev_key.to_string())
            {
                let _ = view.on_unmount();
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
            .map(|v| v.as_mut() as *mut dyn ViewComponent as *mut StatusView)
            .map(|p| unsafe { &mut *p })
    }

    fn send(&mut self, cmd: BackendCmd) {
        if let Err(e) = self.backend_tx.send(cmd) {
            self.push_status(format!("ERROR sending command: {}", e));
        }
    }

    /// Check if a namespace matches the current filter
    /// Returns true if: no filter selected (show all), or namespace is in selected_namespaces
    pub fn namespace_matches_filter(&self, namespace: &str) -> bool {
        if self.selected_namespaces.is_empty() {
            // No filter = show all
            return true;
        }
        self.selected_namespaces.contains(&namespace.to_string())
    }

    /// Update the list of available namespaces
    pub fn update_available_namespaces(&mut self, namespaces: Vec<String>) {
        self.all_namespaces = namespaces;
    }

    /// Toggle a namespace in the filter
    pub fn toggle_namespace_selection(&mut self, namespace: &str) {
        if let Some(pos) = self.selected_namespaces.iter().position(|n| n == namespace) {
            self.selected_namespaces.remove(pos);
        } else {
            self.selected_namespaces.push(namespace.to_string());
        }
    }

    /// Handle keyboard shortcuts for the current frame.
    /// Skipped when a text field has focus so typing doesn't trigger actions.
    pub(super) fn handle_keyboard_input(&mut self, ctx: &egui::Context) {
        // Don't fire shortcuts while the user is typing in a text field
        if ctx.wants_keyboard_input() {
            return;
        }

        ctx.input(|i| {
            // R — refresh current view
            if i.key_pressed(egui::Key::R)
                && let Some(cmd) = self.refresh_cmd_for_active_section()
            {
                let _ = self.backend_tx.send(cmd);
                self.push_status("Refreshed.".to_string());
            }

            // Escape — close any open modal
            if i.key_pressed(egui::Key::Escape) {
                self.close_modal();
            }
        });
    }

    /// Returns the appropriate Refresh backend command for the currently active section.
    fn refresh_cmd_for_active_section(&self) -> Option<BackendCmd> {
        let s = &self.active_section;
        let cmd = if s == &section_keys::nodes() {
            BackendCmd::RefreshNodes
        } else if s == &section_keys::workloads_pods() {
            BackendCmd::RefreshPods
        } else if s == &section_keys::workloads_deployments() {
            BackendCmd::RefreshDeployments
        } else if s == &section_keys::workloads_daemonsets() {
            BackendCmd::RefreshDaemonSets
        } else if s == &section_keys::workloads_statefulsets() {
            BackendCmd::RefreshStatefulSets
        } else if s == &section_keys::workloads_replicasets() {
            BackendCmd::RefreshReplicaSets
        } else if s == &section_keys::workloads_replicationcontrollers() {
            BackendCmd::RefreshReplicationControllers
        } else if s == &section_keys::workloads_jobs() {
            BackendCmd::RefreshJobs
        } else if s == &section_keys::workloads_cronjobs() {
            BackendCmd::RefreshCronJobs
        } else if s == &section_keys::overview() {
            // Overview refreshes all its data sources
            let _ = self.backend_tx.send(BackendCmd::RefreshNodes);
            let _ = self.backend_tx.send(BackendCmd::RefreshPods);
            BackendCmd::RefreshNamespaces
        } else {
            return None;
        };
        Some(cmd)
    }

    /// Close any open confirmation / scale / port-forward modal.
    pub(super) fn close_modal(&mut self) {
        if let Some(view) = self.views.get_mut(&section_keys::modal().to_string()) {
            let ptr = view.as_mut() as *mut dyn ViewComponent as *mut views::ModalView;
            if let Some(modal_view) = unsafe { ptr.as_mut() } {
                modal_view.state = views::ModalState::None;
            }
        }
    }

    /// Open a bulk-delete confirmation modal for a set of (namespace, name) items.
    pub(super) fn open_bulk_delete(&mut self, items: Vec<(String, String)>) {
        if let Some(view) = self.views.get_mut(&section_keys::modal().to_string()) {
            let ptr = view.as_mut() as *mut dyn ViewComponent as *mut views::ModalView;
            if let Some(modal_view) = unsafe { ptr.as_mut() } {
                modal_view.state = views::ModalState::BulkDelete { items };
            }
        }
    }
}

impl eframe::App for FathomApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.check_mailbox();
        self.handle_keyboard_input(ctx);

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
