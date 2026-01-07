use super::*;

impl FathomApp {
    /// Render right panel (details/tabs)
    ///
    /// Delegates to the active view's render_details() method.
    pub(super) fn render_right_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("right_panel")
            .default_width(400.0)
            .show(ctx, |ui| {
                // Get the active view and render its details panel
                let key = self.active_section.clone();
                let view = self.get_or_create_view(&key);

                let (has_content, actions) = view.render_details(ui, ctx);

                // Show placeholder if view has no content to display
                if !has_content {
                    ui.heading("Details");
                    ui.separator();
                    ui.label("Select a resource to view details.");
                }

                // Process actions returned by details panel
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
                        ViewAction::OpenModal(confirm_action) => {
                            // First check if it's OpenPortForward before doing the complex match
                            match &confirm_action {
                                ConfirmAction::OpenPortForward { namespace, pod, suggested_ports, containers } => {
                                    // Ensure modal view exists
                                    let _ = self.get_or_create_view(&section_keys::modal());
                                    
                                    // Now update its state
                                    if let Some(modal_view) = self.views.get_mut(&section_keys::modal().to_string()) {
                                        let ptr = modal_view.as_mut() as *mut dyn ViewComponent as *mut views::ModalView;
                                        if !ptr.is_null() {
                                            unsafe {
                                                let default_port = suggested_ports.first().copied().unwrap_or(8080);
                                                (*ptr).state = views::ModalState::CreatePortForward {
                                                    namespace: namespace.clone(),
                                                    pod: pod.clone(),
                                                    local_port: default_port.to_string(),
                                                    remote_port: default_port.to_string(),
                                                    container: containers.first().cloned().unwrap_or_default(),
                                                };
                                            }
                                        }
                                    }
                                    return;
                                }
                                _ => {}
                            }
                            
                            // Continue with other action types
                            // Update modal state in the modal view
                            if let Some(modal_view) = self.views.get_mut(&section_keys::modal().to_string()) {
                                // Cast to ModalView and update its state
                                let ptr = modal_view.as_mut() as *mut dyn ViewComponent as *mut views::ModalView;
                                if !ptr.is_null() {
                                    unsafe {
                                        let (title, body, confirm_text) = match &confirm_action {
                                            ConfirmAction::DeletePod(ns, name) => (
                                                "Delete Pod".to_string(),
                                                format!("Are you sure you want to delete pod {} in namespace {}?", name, ns),
                                                "Delete".to_string(),
                                            ),
                                            ConfirmAction::DeleteDeployment(ns, name) => (
                                                "Delete Deployment".to_string(),
                                                format!("Are you sure you want to delete deployment {} in namespace {}?", name, ns),
                                                "Delete".to_string(),
                                            ),
                                            ConfirmAction::DeleteDaemonSet(ns, name) => (
                                                "Delete DaemonSet".to_string(),
                                                format!("Are you sure you want to delete daemonset {} in namespace {}?", name, ns),
                                                "Delete".to_string(),
                                            ),
                                            ConfirmAction::DeleteStatefulSet(ns, name) => (
                                                "Delete StatefulSet".to_string(),
                                                format!("Are you sure you want to delete statefulset {} in namespace {}?", name, ns),
                                                "Delete".to_string(),
                                            ),
                                            ConfirmAction::DeleteReplicaSet(ns, name) => (
                                                "Delete ReplicaSet".to_string(),
                                                format!("Are you sure you want to delete replicaset {} in namespace {}?", name, ns),
                                                "Delete".to_string(),
                                            ),
                                            ConfirmAction::DeleteReplicationController(ns, name) => (
                                                "Delete ReplicationController".to_string(),
                                                format!("Are you sure you want to delete replication controller {} in namespace {}?", name, ns),
                                                "Delete".to_string(),
                                            ),
                                            ConfirmAction::DeleteJob(ns, name) => (
                                                "Delete Job".to_string(),
                                                format!("Are you sure you want to delete job {} in namespace {}?", name, ns),
                                                "Delete".to_string(),
                                            ),
                                            ConfirmAction::DeleteCronJob(ns, name) => (
                                                "Delete CronJob".to_string(),
                                                format!("Are you sure you want to delete cronjob {} in namespace {}?", name, ns),
                                                "Delete".to_string(),
                                            ),
                                            ConfirmAction::RestartDeployment(ns, name) => (
                                                "Restart Deployment".to_string(),
                                                format!("Restart all pods in deployment {} in namespace {}?", name, ns),
                                                "Restart".to_string(),
                                            ),
                                            ConfirmAction::RestartDaemonSet(ns, name) => (
                                                "Restart DaemonSet".to_string(),
                                                format!("Restart all pods in daemonset {} in namespace {}?", name, ns),
                                                "Restart".to_string(),
                                            ),
                                            ConfirmAction::RestartStatefulSet(ns, name) => (
                                                "Restart StatefulSet".to_string(),
                                                format!("Restart all pods in statefulset {} in namespace {}?", name, ns),
                                                "Restart".to_string(),
                                            ),
                                            ConfirmAction::RestartReplicaSet(ns, name) => (
                                                "Restart ReplicaSet".to_string(),
                                                format!("Restart all pods in replicaset {} in namespace {}?", name, ns),
                                                "Restart".to_string(),
                                            ),
                                            ConfirmAction::RestartReplicationController(ns, name) => (
                                                "Restart ReplicationController".to_string(),
                                                format!("Restart all pods in replication controller {} in namespace {}?", name, ns),
                                                "Restart".to_string(),
                                            ),
                                            ConfirmAction::ScaleDeployment(ns, name, current) => {
                                                // Handle scale actions separately - they use ScaleWorkload modal
                                                let modal_state = views::ModalState::ScaleWorkload {
                                                    kind: crate::app::WorkloadKind::Deployment,
                                                    namespace: ns.clone(),
                                                    name: name.clone(),
                                                    current: *current,
                                                    desired: *current,
                                                };
                                                (*ptr).state = modal_state;
                                                return;
                                            }
                                            ConfirmAction::ScaleStatefulSet(ns, name, current) => {
                                                let modal_state = views::ModalState::ScaleWorkload {
                                                    kind: crate::app::WorkloadKind::StatefulSet,
                                                    namespace: ns.clone(),
                                                    name: name.clone(),
                                                    current: *current,
                                                    desired: *current,
                                                };
                                                (*ptr).state = modal_state;
                                                return;
                                            }
                                            ConfirmAction::ScaleReplicaSet(ns, name, current) => {
                                                let modal_state = views::ModalState::ScaleWorkload {
                                                    kind: crate::app::WorkloadKind::ReplicaSet,
                                                    namespace: ns.clone(),
                                                    name: name.clone(),
                                                    current: *current,
                                                    desired: *current,
                                                };
                                                (*ptr).state = modal_state;
                                                return;
                                            }
                                            ConfirmAction::ScaleReplicationController(ns, name, current) => {
                                                let modal_state = views::ModalState::ScaleWorkload {
                                                    kind: crate::app::WorkloadKind::ReplicationController,
                                                    namespace: ns.clone(),
                                                    name: name.clone(),
                                                    current: *current,
                                                    desired: *current,
                                                };
                                                (*ptr).state = modal_state;
                                                return;
                                            }
                                            ConfirmAction::BulkDelete(_) => {
                                                // BulkDelete is handled by OpenBulkDeleteModal, not OpenModal
                                                return;
                                            }
                                            ConfirmAction::OpenPortForward { .. } => {
                                                // OpenPortForward is handled above, should not reach here
                                                unreachable!();
                                            }
                                        };
                                        (*ptr).state = views::ModalState::Confirm {
                                            title,
                                            body,
                                            confirm_text,
                                            action: confirm_action,
                                        };
                                    }
                                }
                            }
                        }
                        ViewAction::OpenScaleModal(kind, namespace, name, current) => {
                            // Update modal state with scale modal
                            if let Some(modal_view) = self.views.get_mut(&section_keys::modal().to_string()) {
                                let ptr = modal_view.as_mut() as *mut dyn ViewComponent as *mut views::ModalView;
                                if !ptr.is_null() {
                                    unsafe {
                                        (*ptr).state = views::ModalState::ScaleWorkload {
                                            kind,
                                            namespace,
                                            name,
                                            current,
                                            desired: current,
                                        };
                                    }
                                }
                            }
                        }
                    }
                }
            });
    }
}
