use super::*;
use crate::app::ConfirmAction;
use crate::app::WorkloadKind;
use crate::protocol::v1::ResourceListEvent;
use crate::protocol::{BackendCmd, BackendEvent};

/// Modal state for confirmations, scaling, and port forwarding
#[derive(Clone, Debug)]
pub enum ModalState {
    None,
    Confirm {
        title: String,
        body: String,
        confirm_text: String,
        action: ConfirmAction,
    },
    BulkDelete {
        items: Vec<(String, String)>, // (namespace, name)
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
    EditPortForward {
        tunnel_id: String,
        namespace: String,
        pod: String,
        local_port: String,
        remote_port: String,
        container: String,
    },
}

impl ModalState {
    pub fn none() -> Self {
        Self::None
    }

    pub fn is_open(&self) -> bool {
        !matches!(self, Self::None)
    }
}

pub struct ModalView {
    pub state: ModalState,
    pub namespaces: Vec<String>,
}

impl Default for ModalView {
    fn default() -> Self {
        Self {
            state: ModalState::None,
            namespaces: Vec::new(),
        }
    }
}

impl ModalView {
    /// Render modal window using egui::Window (called from ui_modal.rs)
    pub fn render_modal_window(&mut self, ctx: &egui::Context) -> Vec<ViewAction> {
        let mut actions = Vec::new();
        let mut next_modal: Option<ModalState> = None;
        let mut confirm_action: Option<ConfirmAction> = None;
        let mut scale_apply: Option<(WorkloadKind, String, String, i32)> = None;
        let mut create_portforward: Option<(String, String, u16, u16, Option<String>)> = None;

        match self.state.clone() {
            ModalState::None => {}

            ModalState::Confirm {
                title,
                body,
                confirm_text,
                action,
            } => {
                let mut cancel = false;
                let mut confirm = false;

                let keep = ModalState::Confirm {
                    title: title.clone(),
                    body: body.clone(),
                    confirm_text: confirm_text.clone(),
                    action: action.clone(),
                };

                egui::Window::new(title)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(body);
                        ui.add_space(10.0);

                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                            if ui.button(confirm_text).clicked() {
                                confirm = true;
                            }
                        });
                    });

                if cancel {
                    next_modal = Some(ModalState::None);
                } else if confirm {
                    next_modal = Some(ModalState::None);
                    confirm_action = Some(action);
                } else {
                    next_modal = Some(keep);
                }
            }

            ModalState::ScaleWorkload {
                kind,
                namespace,
                name,
                current,
                desired,
            } => {
                let mut desired_mut = desired;
                let mut cancel = false;
                let mut apply = false;

                egui::Window::new("Scale Workload")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Workload: {} / {}", namespace, name));
                        ui.label(format!("Kind: {:?}", kind));
                        ui.label(format!("Current replicas: {}", current));

                        ui.horizontal(|ui| {
                            ui.label("Desired replicas:");
                            ui.add(egui::DragValue::new(&mut desired_mut).range(0..=100));
                        });

                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                            if ui.button("Apply").clicked() {
                                apply = true;
                            }
                        });
                    });

                if cancel {
                    next_modal = Some(ModalState::None);
                } else if apply {
                    next_modal = Some(ModalState::None);
                    scale_apply = Some((kind, namespace, name, desired_mut));
                } else {
                    next_modal = Some(ModalState::ScaleWorkload {
                        kind,
                        namespace,
                        name,
                        current,
                        desired: desired_mut,
                    });
                }
            }

            ModalState::BulkDelete { items } => {
                let mut cancel = false;
                let mut confirm = false;

                let keep = ModalState::BulkDelete {
                    items: items.clone(),
                };

                let item_count = items.len();
                let title = format!("Delete {} item(s)?", item_count);
                let mut body = format!(
                    "Are you sure you want to delete these {} items?\n\n",
                    item_count
                );

                // Show first 10 items, then truncate
                for (i, (ns, name)) in items.iter().take(10).enumerate() {
                    if i > 0 {
                        body.push('\n');
                    }
                    body.push_str(&format!("  {}/{}", ns, name));
                }

                if item_count > 10 {
                    body.push_str(&format!("\n  ... and {} more", item_count - 10));
                }

                body.push_str("\n\nThis action cannot be undone.");

                egui::Window::new(&title)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(&body);
                        ui.add_space(10.0);

                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                            if ui.button("Delete All").clicked() {
                                confirm = true;
                            }
                        });
                    });

                if cancel {
                    next_modal = Some(ModalState::None);
                } else if confirm {
                    next_modal = Some(ModalState::None);
                    confirm_action = Some(ConfirmAction::BulkDelete(items));
                } else {
                    next_modal = Some(keep);
                }
            }

            ModalState::CreatePortForward {
                namespace,
                pod,
                local_port,
                remote_port,
                container,
            } => {
                let mut namespace_mut = namespace.clone();
                let mut pod_mut = pod.clone();
                let mut local_port_mut = local_port.clone();
                let mut remote_port_mut = remote_port.clone();
                let mut container_mut = container.clone();
                let mut cancel = false;
                let mut create = false;

                egui::Window::new("Create Port Forward")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        // Passing &mut namespace_mut/&mut pod_mut here (not a
                        // `.clone()`) is load-bearing: this used to pass a
                        // mutable reference to a throwaway clone of
                        // `namespace`/`pod`, so `ui.text_edit_singleline`
                        // inside `render_autocomplete_field` edited a value
                        // that was discarded at the end of the call - the
                        // Namespace and Pod fields could not actually be
                        // typed into.
                        render_autocomplete_field(
                            ui,
                            "Namespace",
                            &mut namespace_mut,
                            &self.namespaces,
                            "pf_ns_field",
                        );

                        // TODO: Pod suggestions need refactor to get from PodsView
                        let pod_options: Vec<String> = Vec::new();
                        render_autocomplete_field(
                            ui,
                            "Pod",
                            &mut pod_mut,
                            &pod_options,
                            "pf_pod_field",
                        );

                        ui.horizontal(|ui| {
                            ui.label("Local port:");
                            ui.text_edit_singleline(&mut local_port_mut);
                        });

                        ui.horizontal(|ui| {
                            ui.label("Remote port:");
                            ui.text_edit_singleline(&mut remote_port_mut);
                        });

                        ui.horizontal(|ui| {
                            ui.label("Container (optional):");
                            ui.text_edit_singleline(&mut container_mut);
                        });

                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                            if ui.button("Create").clicked() {
                                create = true;
                            }
                        });
                    });

                if cancel {
                    next_modal = Some(ModalState::None);
                } else if create {
                    next_modal = Some(ModalState::None);
                    // Parse ports
                    if let (Ok(local), Ok(remote)) = (
                        local_port_mut.parse::<u16>(),
                        remote_port_mut.parse::<u16>(),
                    ) {
                        create_portforward = Some((
                            namespace_mut,
                            pod_mut,
                            local,
                            remote,
                            if container_mut.is_empty() {
                                None
                            } else {
                                Some(container_mut)
                            },
                        ));
                    }
                } else {
                    next_modal = Some(ModalState::CreatePortForward {
                        namespace: namespace_mut,
                        pod: pod_mut,
                        local_port: local_port_mut,
                        remote_port: remote_port_mut,
                        container: container_mut,
                    });
                }
            }

            ModalState::EditPortForward {
                tunnel_id,
                namespace,
                pod,
                local_port,
                remote_port,
                container,
            } => {
                let mut local_port_mut = local_port.clone();
                let mut remote_port_mut = remote_port.clone();
                let mut container_mut = container.clone();
                let mut cancel = false;
                let mut save = false;

                egui::Window::new("Edit Port Forward")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Namespace: {}", namespace));
                        ui.label(format!("Pod: {}", pod));
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            ui.label("Local port:");
                            ui.text_edit_singleline(&mut local_port_mut);
                        });

                        ui.horizontal(|ui| {
                            ui.label("Remote port:");
                            ui.text_edit_singleline(&mut remote_port_mut);
                        });

                        ui.horizontal(|ui| {
                            ui.label("Container (optional):");
                            ui.text_edit_singleline(&mut container_mut);
                        });

                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                            if ui.button("Save").clicked() {
                                save = true;
                            }
                        });
                    });

                if cancel {
                    next_modal = Some(ModalState::None);
                } else if save {
                    next_modal = Some(ModalState::None);
                    // Parse ports and send update command
                    if let (Ok(local), Ok(remote)) = (
                        local_port_mut.parse::<u16>(),
                        remote_port_mut.parse::<u16>(),
                    ) {
                        // First stop the current tunnel
                        actions.push(ViewAction::SendCommand(BackendCmd::StopPortForward {
                            tunnel_id: tunnel_id.clone(),
                        }));
                        // Then delete it
                        actions.push(ViewAction::SendCommand(BackendCmd::DeletePortForward {
                            tunnel_id: tunnel_id.clone(),
                        }));
                        // Create new one with updated ports
                        actions.push(ViewAction::SendCommand(BackendCmd::StartPortForward {
                            namespace: namespace.clone(),
                            pod: pod.clone(),
                            local_port: local,
                            remote_port: remote,
                            container: if container_mut.is_empty() {
                                None
                            } else {
                                Some(container_mut)
                            },
                        }));
                    }
                } else {
                    next_modal = Some(ModalState::EditPortForward {
                        tunnel_id: tunnel_id.clone(),
                        namespace: namespace.clone(),
                        pod: pod.clone(),
                        local_port: local_port_mut,
                        remote_port: remote_port_mut,
                        container: container_mut,
                    });
                }
            }
        }

        // Process collected actions
        if let Some(state) = next_modal {
            self.state = state;
        }

        if let Some(action) = confirm_action {
            match action {
                ConfirmAction::DeletePod(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeletePod {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteDeployment(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteDeployment {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteDaemonSet(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteDaemonSet {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteStatefulSet(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteStatefulSet {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteReplicaSet(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteReplicaSet {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteReplicationController(namespace, name) => {
                    actions.push(ViewAction::SendCommand(
                        BackendCmd::DeleteReplicationController { namespace, name },
                    ));
                }
                ConfirmAction::DeleteJob(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteJob {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteService(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteService {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteIngress(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteIngress {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteNetworkPolicy(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteNetworkPolicy {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeletePersistentVolume(name) => {
                    actions.push(ViewAction::SendCommand(
                        BackendCmd::DeletePersistentVolume { name },
                    ));
                }
                ConfirmAction::DeletePersistentVolumeClaim(namespace, name) => {
                    actions.push(ViewAction::SendCommand(
                        BackendCmd::DeletePersistentVolumeClaim { namespace, name },
                    ));
                }
                ConfirmAction::DeleteStorageClass(name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteStorageClass {
                        name,
                    }));
                }
                ConfirmAction::DeleteRole(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteRole {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteRoleBinding(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteRoleBinding {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteClusterRole(name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteClusterRole {
                        name,
                    }));
                }
                ConfirmAction::DeleteClusterRoleBinding(name) => {
                    actions.push(ViewAction::SendCommand(
                        BackendCmd::DeleteClusterRoleBinding { name },
                    ));
                }
                ConfirmAction::DeleteConfigMap(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteConfigMap {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteSecret(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteSecret {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteResourceQuota(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteResourceQuota {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteLimitRange(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteLimitRange {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteHorizontalPodAutoscaler(namespace, name) => {
                    actions.push(ViewAction::SendCommand(
                        BackendCmd::DeleteHorizontalPodAutoscaler { namespace, name },
                    ));
                }
                ConfirmAction::DeletePodDisruptionBudget(namespace, name) => {
                    actions.push(ViewAction::SendCommand(
                        BackendCmd::DeletePodDisruptionBudget { namespace, name },
                    ));
                }
                ConfirmAction::DeletePriorityClass(name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeletePriorityClass {
                        name,
                    }));
                }
                ConfirmAction::DeleteRuntimeClass(name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteRuntimeClass {
                        name,
                    }));
                }
                ConfirmAction::DeleteLease(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteLease {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::DeleteMutatingWebhookConfiguration(name) => {
                    actions.push(ViewAction::SendCommand(
                        BackendCmd::DeleteMutatingWebhookConfiguration { name },
                    ));
                }
                ConfirmAction::DeleteValidatingWebhookConfiguration(name) => {
                    actions.push(ViewAction::SendCommand(
                        BackendCmd::DeleteValidatingWebhookConfiguration { name },
                    ));
                }
                ConfirmAction::DeleteCronJob(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::DeleteCronJob {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::RestartDeployment(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::RestartDeployment {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::RestartDaemonSet(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::RestartDaemonSet {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::RestartStatefulSet(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::RestartStatefulSet {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::RestartReplicaSet(namespace, name) => {
                    actions.push(ViewAction::SendCommand(BackendCmd::RestartReplicaSet {
                        namespace,
                        name,
                    }));
                }
                ConfirmAction::RestartReplicationController(namespace, name) => {
                    actions.push(ViewAction::SendCommand(
                        BackendCmd::RestartReplicationController { namespace, name },
                    ));
                }
                ConfirmAction::ScaleDeployment(namespace, name, _current) => {
                    // Open ScaleWorkload modal instead of direct action
                    self.state = ModalState::ScaleWorkload {
                        kind: WorkloadKind::Deployment,
                        namespace,
                        name,
                        current: _current,
                        desired: _current,
                    };
                }
                ConfirmAction::ScaleStatefulSet(namespace, name, _current) => {
                    self.state = ModalState::ScaleWorkload {
                        kind: WorkloadKind::StatefulSet,
                        namespace,
                        name,
                        current: _current,
                        desired: _current,
                    };
                }
                ConfirmAction::ScaleReplicaSet(namespace, name, _current) => {
                    self.state = ModalState::ScaleWorkload {
                        kind: WorkloadKind::ReplicaSet,
                        namespace,
                        name,
                        current: _current,
                        desired: _current,
                    };
                }
                ConfirmAction::ScaleReplicationController(namespace, name, _current) => {
                    self.state = ModalState::ScaleWorkload {
                        kind: WorkloadKind::ReplicationController,
                        namespace,
                        name,
                        current: _current,
                        desired: _current,
                    };
                }
                ConfirmAction::BulkDelete(items) => {
                    // For each item, emit delete commands
                    // The items list contains (namespace, name) tuples
                    // We'll delegate the actual deletion to a backend handler
                    actions.push(ViewAction::SendCommand(BackendCmd::BulkDelete { items }));
                }
                ConfirmAction::OpenPortForward { .. } => {
                    // Handled in ui_right.rs - opens CreatePortForward modal instead
                }
                ConfirmAction::EditPortForward { .. } => {
                    // Handled in ui_right.rs - opens EditPortForward modal instead
                }
            }
        }

        if let Some((kind, namespace, name, desired)) = scale_apply {
            let cmd = match kind {
                WorkloadKind::Deployment => BackendCmd::ScaleDeployment {
                    namespace,
                    name,
                    replicas: desired,
                },
                WorkloadKind::DaemonSet => {
                    // DaemonSets cannot be scaled
                    return actions;
                }
                WorkloadKind::StatefulSet => BackendCmd::ScaleStatefulSet {
                    namespace,
                    name,
                    replicas: desired,
                },
                WorkloadKind::ReplicaSet => BackendCmd::ScaleReplicaSet {
                    namespace,
                    name,
                    replicas: desired,
                },
                WorkloadKind::ReplicationController => BackendCmd::ScaleReplicationController {
                    namespace,
                    name,
                    replicas: desired,
                },
                WorkloadKind::Job => {
                    // Jobs cannot be scaled
                    return actions;
                }
                WorkloadKind::CronJob => {
                    // CronJobs cannot be scaled
                    return actions;
                }
            };
            actions.push(ViewAction::SendCommand(cmd));
        }

        if let Some((namespace, pod, local_port, remote_port, container)) = create_portforward {
            actions.push(ViewAction::SendCommand(BackendCmd::StartPortForward {
                namespace,
                pod,
                local_port,
                remote_port,
                container,
            }));
        }

        actions
    }
}

impl ViewComponent for ModalView {
    fn id(&self) -> &'static str {
        "_modal"
    }

    fn render(&mut self, _ui: &mut egui::Ui, _ctx: &egui::Context) -> Vec<ViewAction> {
        // Modals are rendered via render_modal_window which uses egui::Window
        // This method is not used for modals
        Vec::new()
    }

    fn on_event(&mut self, event: &BackendEvent) {
        if let BackendEvent::ResourceList(ResourceListEvent::NamespacesUpdated(namespaces)) = event
        {
            self.namespaces = namespaces.clone();
        }
    }

    fn on_mount(&mut self) -> Vec<ViewAction> {
        Vec::new()
    }

    fn on_unmount(&mut self) -> Vec<ViewAction> {
        self.state = ModalState::None;
        Vec::new()
    }

    fn render_details(
        &mut self,
        _ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) -> (bool, Vec<ViewAction>) {
        (false, Vec::new())
    }
}

/// Render autocomplete field for port forward modal
fn render_autocomplete_field(
    ui: &mut egui::Ui,
    label: &str,
    text: &mut String,
    options: &[String],
    id: &str,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.text_edit_singleline(text);
    });

    if !options.is_empty() && !text.is_empty() {
        let filtered: Vec<_> = options
            .iter()
            .filter(|opt| opt.contains(text.as_str()))
            .cloned()
            .collect();

        if !filtered.is_empty() {
            egui::ComboBox::from_id_salt(id)
                .width(200.0)
                .show_ui(ui, |ui| {
                    for opt in filtered {
                        if ui.selectable_value(text, opt.clone(), &opt).changed() {
                            // Selection changed
                        }
                    }
                });
        }
    }
}
