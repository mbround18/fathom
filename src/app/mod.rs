use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, CronJobDetails, CronJobSummary,
    DaemonSetDetails, DaemonSetSummary, DeploymentDetails, DeploymentSummary, JobDetails,
    JobSummary, NotificationLevel, PodDetails, PodMetricsHistory, PodMetricsSummary, PodSummary,
    ReplicaSetDetails, ReplicaSetSummary, ReplicationControllerDetails,
    ReplicationControllerSummary, ScopeKind, SectionKey, StatefulSetDetails, StatefulSetSummary,
    SystemEvent, section_keys,
};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::collections::{BTreeSet, HashMap};
use views::{
    BoxedView, DaemonSetView, DeploymentView, PodsView, ReplicaSetView, ReplicationControllerView,
    StatefulSetView, ViewComponent,
};

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

    // Namespace filter
    namespaces: Vec<String>,
    selected_namespaces: BTreeSet<String>,
    select_all_namespaces: bool,

    // Kube contexts
    contexts: Vec<String>,
    current_context: Option<String>,

    // Pods
    pods: Vec<PodSummary>,
    selected_pod: Option<(String, String)>,
    pod_search: String,
    pods_sort: PodsSort,

    // Deployments
    deployments: Vec<DeploymentSummary>,
    selected_deployment: Option<(String, String)>,
    deployment_details: Option<DeploymentDetails>,

    // DaemonSets
    daemonsets: Vec<DaemonSetSummary>,
    selected_daemonset: Option<(String, String)>,
    daemonset_details: Option<DaemonSetDetails>,

    // StatefulSets
    statefulsets: Vec<StatefulSetSummary>,
    selected_statefulset: Option<(String, String)>,
    statefulset_details: Option<StatefulSetDetails>,

    // ReplicaSets
    replicasets: Vec<ReplicaSetSummary>,
    selected_replicaset: Option<(String, String)>,
    replicaset_details: Option<ReplicaSetDetails>,

    // ReplicationControllers
    replicationcontrollers: Vec<ReplicationControllerSummary>,
    selected_replicationcontroller: Option<(String, String)>,
    replicationcontroller_details: Option<ReplicationControllerDetails>,

    // Jobs
    jobs: Vec<JobSummary>,
    selected_job: Option<(String, String)>,
    job_details: Option<JobDetails>,

    // CronJobs
    cronjobs: Vec<CronJobSummary>,
    selected_cronjob: Option<(String, String)>,
    cronjob_details: Option<CronJobDetails>,

    // Pod panels
    pod_tab: PodTab,
    pod_details: Option<PodDetails>,
    pod_logs: Vec<String>,
    pod_metrics: Option<PodMetricsSummary>,
    pod_metrics_history: Option<PodMetricsHistory>,
    pod_metrics_pending: bool,
    pod_metrics_polling_started: bool,

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

    // Status
    status_log: Vec<String>,
    status_auto_scroll: bool,

    // Modal
    modal: ModalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PodTab {
    Details,
    Logs,
    Exec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SortDir {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PodsSortKey {
    Namespace,
    Name,
    Phase,
    Ready,
    Restarts,
    Node,
}

#[derive(Clone, Copy, Debug)]
struct PodsSort {
    key: PodsSortKey,
    dir: SortDir,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkloadKind {
    Deployment,
    DaemonSet,
    StatefulSet,
    ReplicaSet,
    ReplicationController,
    Job,
    CronJob,
}

#[derive(Clone, Debug)]
enum ConfirmAction {
    DeletePod {
        namespace: String,
        name: String,
    },
    DeleteWorkload {
        kind: WorkloadKind,
        namespace: String,
        name: String,
    },
    RestartWorkload {
        kind: WorkloadKind,
        namespace: String,
        name: String,
    },
}

#[derive(Clone, Debug)]
enum ModalState {
    None,
    Confirm {
        title: String,
        body: String,
        confirm_text: String,
        action: ConfirmAction,
    },
    ScaleWorkload {
        kind: WorkloadKind,
        namespace: String,
        name: String,
        current: i32,
        desired: i32,
    },
    CreatePortForward {
        namespace: String,
        pod: String,
        local_port: String,
        remote_port: String,
        container: String,
    },
}

impl FathomApp {
    pub fn new(gui_rx: Receiver<BackendEventEnvelope>, backend_tx: Sender<BackendCmd>) -> Self {
        let mut status_log = Vec::new();
        status_log.push("System Ready.".to_owned());

        // Auto-connect on launch (MVP).
        let _ = backend_tx.send(BackendCmd::RefreshContexts);
        let _ = backend_tx.send(BackendCmd::Initialize);
        status_log.push("Initializing: connecting to cluster...".to_owned());

        Self {
            gui_rx,
            backend_tx,
            views: HashMap::new(),
            previous_section: None,
            active_section: section_keys::overview(),
            namespaces: Vec::new(),
            selected_namespaces: BTreeSet::new(),
            select_all_namespaces: true,
            contexts: Vec::new(),
            current_context: None,
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
            deployments: Vec::new(),
            selected_deployment: None,
            deployment_details: None,
            daemonsets: Vec::new(),
            selected_daemonset: None,
            daemonset_details: None,
            statefulsets: Vec::new(),
            selected_statefulset: None,
            statefulset_details: None,
            replicasets: Vec::new(),
            selected_replicaset: None,
            replicaset_details: None,
            replicationcontrollers: Vec::new(),
            selected_replicationcontroller: None,
            replicationcontroller_details: None,
            jobs: Vec::new(),
            selected_job: None,
            job_details: None,
            cronjobs: Vec::new(),
            selected_cronjob: None,
            cronjob_details: None,
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
            status_log,
            status_auto_scroll: true,
            modal: ModalState::None,
        }
    }

    /// Get or create a view for the given section key.
    fn get_or_create_view(&mut self, section_key: &SectionKey) -> &mut dyn ViewComponent {
        let key_str = section_key.to_string();
        if !self.views.contains_key(&key_str) {
            // Create the appropriate view based on section key
            let view: BoxedView = if section_key == &section_keys::workloads_pods() {
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
            } else {
                // For unimplemented views, create an empty Pod view as placeholder
                Box::new(PodsView::new())
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

            // On mount: call on_mount on new view
            let view = self.get_or_create_view(&current);
            let _ = view.on_mount();
        }
    }

    fn push_status(&mut self, msg: impl Into<String>) {
        self.status_log.push(format!("> {}", msg.into()));
        if self.status_log.len() > 500 {
            let drain = self.status_log.len() - 500;
            self.status_log.drain(0..drain);
        }
    }

    fn effective_namespace_filter(&self) -> Vec<String> {
        if self.select_all_namespaces {
            return Vec::new();
        }
        self.selected_namespaces.iter().cloned().collect()
    }

    fn toggle_namespace_filter_from_link(&mut self, namespace: &str) {
        if self.select_all_namespaces {
            self.select_all_namespaces = false;
            self.selected_namespaces.clear();
            self.selected_namespaces.insert(namespace.to_owned());
        } else if self.selected_namespaces.contains(namespace) {
            // If this was the last selected namespace, revert to All.
            if self.selected_namespaces.len() == 1 {
                self.select_all_namespaces = true;
                self.selected_namespaces.clear();
            } else {
                self.selected_namespaces.remove(namespace);
            }
        } else {
            self.selected_namespaces.insert(namespace.to_owned());
        }

        self.send(BackendCmd::SetNamespaceFilter {
            namespaces: self.effective_namespace_filter(),
        });
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
        self.modal = ModalState::Confirm {
            title: title.into(),
            body: body.into(),
            confirm_text: confirm_text.into(),
            action,
        };
    }

    fn open_scale_workload(
        &mut self,
        kind: WorkloadKind,
        namespace: String,
        name: String,
        current_ready: i32,
        desired: i32,
    ) {
        self.modal = ModalState::ScaleWorkload {
            kind,
            namespace,
            name,
            current: current_ready,
            desired,
        };
    }

    fn open_create_port_forward(&mut self) {
        self.modal = ModalState::CreatePortForward {
            namespace: String::new(),
            pod: String::new(),
            local_port: String::new(),
            remote_port: String::new(),
            container: String::new(),
        };
    }

    fn open_create_port_forward_prefill(
        &mut self,
        namespace: String,
        pod: String,
        remote_port: Option<u16>,
        container: Option<String>,
    ) {
        self.modal = ModalState::CreatePortForward {
            namespace,
            pod,
            local_port: String::new(),
            remote_port: remote_port.map(|p| p.to_string()).unwrap_or_default(),
            container: container.unwrap_or_default(),
        };
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
        let _ = self.backend_tx.send(BackendCmd::Shutdown);
    }
}
