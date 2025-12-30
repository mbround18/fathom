use super::traits::{ViewAction, ViewComponent};
use crate::protocol::{
    BackendCmd, BackendEvent, DeploymentDetails, DeploymentSummary, ResourceListEvent,
    ResourceReadEvent,
};

/// Generic state shared by all workload views
pub struct WorkloadViewState {
    pub summaries: Vec<DeploymentSummary>,
    pub selected: Option<(String, String)>,
    pub details: Option<DeploymentDetails>,
    pub workload_type: &'static str,
    pub supports_scaling: bool,
}

impl WorkloadViewState {
    pub fn new(workload_type: &'static str, supports_scaling: bool) -> Self {
        Self {
            summaries: Vec::new(),
            selected: None,
            details: None,
            workload_type,
            supports_scaling,
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

    fn render(&mut self, _ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        Vec::new()
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

    fn render(&mut self, _ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        Vec::new()
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

    fn render(&mut self, _ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        Vec::new()
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

    fn render(&mut self, _ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        Vec::new()
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

    fn render(&mut self, _ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        Vec::new()
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
