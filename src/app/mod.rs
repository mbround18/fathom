use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, DeploymentDetails, DeploymentSummary,
    NotificationLevel, PodDetails, PodMetricsSummary, PodSummary, ResourceListEvent,
    ResourceReadEvent, ScopeKind, SectionKey, StreamEvent, SystemEvent, section_keys,
};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::collections::BTreeSet;

mod mailbox;
mod search;
mod ui_center;
mod ui_left;
mod ui_modal;
mod ui_right;
mod ui_status;
mod ui_top;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PodTab {
    Details,
    Logs,
    Metrics,
    Exec,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortDir {
    Asc,
    Desc,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PodsSortKey {
    Namespace,
    Name,
    Phase,
    Ready,
    Restarts,
    Node,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PodsSort {
    key: PodsSortKey,
    dir: SortDir,
}

pub struct FathomApp {
    // Comms
    gui_rx: Receiver<BackendEventEnvelope>,
    backend_tx: Sender<BackendCmd>,

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

    // Pod panels
    pod_tab: PodTab,
    pod_details: Option<PodDetails>,
    pod_logs: Vec<String>,
    pod_metrics: Option<PodMetricsSummary>,

    // Status
    status_log: Vec<String>,

    // Modal
    modal: ModalState,
}

#[derive(Clone, Debug)]
enum ConfirmAction {
    DeletePod { namespace: String, name: String },
    DeleteDeployment { namespace: String, name: String },
    RestartDeployment { namespace: String, name: String },
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
    ScaleDeployment {
        namespace: String,
        name: String,
        current: i32,
        desired: i32,
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
            active_section: section_keys::workloads_pods(),
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
            deployments: Vec::new(),
            selected_deployment: None,
            deployment_details: None,
            pod_tab: PodTab::Details,
            pod_details: None,
            pod_logs: Vec::new(),
            pod_metrics: None,
            status_log,
            modal: ModalState::None,
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

    fn open_scale_deployment(
        &mut self,
        namespace: String,
        name: String,
        current_ready: i32,
        desired: i32,
    ) {
        self.modal = ModalState::ScaleDeployment {
            namespace,
            name,
            current: current_ready,
            desired,
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
}
