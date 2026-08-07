use super::*;

impl FathomApp {
    /// Render right panel (details/tabs)
    ///
    /// Delegates to the active view's render_details() method.
    /// The panel is only shown if the active view has content to display.
    pub(super) fn render_right_panel(&mut self, ctx: &egui::Context) {
        // Ask the view itself whether it has a selection, before committing to
        // reserving 400px of screen space for the panel this frame.
        let key = self.active_section.clone();
        if !self.get_or_create_view(&key).has_details() {
            self.show_details_panel = false;
            return;
        }

        let panel = egui::SidePanel::right("right_panel").default_width(400.0);

        // Capture actions emitted during details rendering to process after layout
        let mut captured_actions: Vec<ViewAction> = Vec::new();

        // Only show panel if we actually have content
        let show_panel = panel.show(ctx, |ui| {
            let key = self.active_section.clone();
            let view = self.get_or_create_view(&key);

            let (has_content, actions) = view.render_details(ui, ctx);

            // Update visibility for next frame: show panel if there's content
            self.show_details_panel = has_content;

            // Save actions for processing after the panel is laid out
            captured_actions = actions;

            has_content
        });

        // If no content, don't show the panel next time
        if !show_panel.inner {
            self.show_details_panel = false;
            return;
        }

        // Process actions returned by details panel
        for action in captured_actions {
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
                    // First check if it's OpenPortForward or EditPortForward before doing the complex match
                    match &confirm_action {
                        ConfirmAction::OpenPortForward {
                            namespace,
                            pod,
                            suggested_ports,
                            containers,
                        } => {
                            // Ensure modal view exists
                            let _ = self.get_or_create_view(&section_keys::modal());

                            // Now update its state
                            if let Some(modal_view) =
                                self.views.get_mut(&section_keys::modal().to_string())
                            {
                                let ptr = modal_view.as_mut() as *mut dyn ViewComponent
                                    as *mut views::ModalView;
                                if !ptr.is_null() {
                                    unsafe {
                                        let default_port =
                                            suggested_ports.first().copied().unwrap_or(8080);
                                        (*ptr).state = views::ModalState::CreatePortForward {
                                            namespace: namespace.clone(),
                                            pod: pod.clone(),
                                            local_port: default_port.to_string(),
                                            remote_port: default_port.to_string(),
                                            container: containers
                                                .first()
                                                .cloned()
                                                .unwrap_or_default(),
                                        };
                                    }
                                }
                            }
                            return;
                        }
                        ConfirmAction::EditPortForward {
                            tunnel_id,
                            namespace,
                            pod,
                            local_port,
                            remote_port,
                            container,
                        } => {
                            // Ensure modal view exists
                            let _ = self.get_or_create_view(&section_keys::modal());

                            // Now update its state
                            if let Some(modal_view) =
                                self.views.get_mut(&section_keys::modal().to_string())
                            {
                                let ptr = modal_view.as_mut() as *mut dyn ViewComponent
                                    as *mut views::ModalView;
                                if !ptr.is_null() {
                                    unsafe {
                                        (*ptr).state = views::ModalState::EditPortForward {
                                            tunnel_id: tunnel_id.clone(),
                                            namespace: namespace.clone(),
                                            pod: pod.clone(),
                                            local_port: local_port.to_string(),
                                            remote_port: remote_port.to_string(),
                                            container: container.clone(),
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
                    if let Some(modal_view) = self.views.get_mut(&section_keys::modal().to_string())
                    {
                        // Cast to ModalView and update its state
                        let ptr =
                            modal_view.as_mut() as *mut dyn ViewComponent as *mut views::ModalView;
                        if !ptr.is_null() {
                            unsafe {
                                let (title, body, confirm_text) = match &confirm_action {
                                    ConfirmAction::DeletePod(ns, name) => (
                                        "Delete Pod".to_string(),
                                        format!(
                                            "Are you sure you want to delete pod {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteDeployment(ns, name) => (
                                        "Delete Deployment".to_string(),
                                        format!(
                                            "Are you sure you want to delete deployment {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteDaemonSet(ns, name) => (
                                        "Delete DaemonSet".to_string(),
                                        format!(
                                            "Are you sure you want to delete daemonset {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteStatefulSet(ns, name) => (
                                        "Delete StatefulSet".to_string(),
                                        format!(
                                            "Are you sure you want to delete statefulset {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteReplicaSet(ns, name) => (
                                        "Delete ReplicaSet".to_string(),
                                        format!(
                                            "Are you sure you want to delete replicaset {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteReplicationController(ns, name) => (
                                        "Delete ReplicationController".to_string(),
                                        format!(
                                            "Are you sure you want to delete replication controller {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteService(ns, name) => (
                                        "Delete Service".to_string(),
                                        format!(
                                            "Are you sure you want to delete service {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteIngress(ns, name) => (
                                        "Delete Ingress".to_string(),
                                        format!(
                                            "Are you sure you want to delete ingress {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteNetworkPolicy(ns, name) => (
                                        "Delete Network Policy".to_string(),
                                        format!(
                                            "Are you sure you want to delete network policy {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeletePersistentVolume(name) => (
                                        "Delete Persistent Volume".to_string(),
                                        format!(
                                            "Are you sure you want to delete persistent volume {}?",
                                            name
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeletePersistentVolumeClaim(ns, name) => (
                                        "Delete Persistent Volume Claim".to_string(),
                                        format!(
                                            "Are you sure you want to delete persistent volume claim {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteStorageClass(name) => (
                                        "Delete Storage Class".to_string(),
                                        format!(
                                            "Are you sure you want to delete storage class {}?",
                                            name
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteRole(ns, name) => (
                                        "Delete Role".to_string(),
                                        format!(
                                            "Are you sure you want to delete role {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteRoleBinding(ns, name) => (
                                        "Delete Role Binding".to_string(),
                                        format!(
                                            "Are you sure you want to delete role binding {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteClusterRole(name) => (
                                        "Delete Cluster Role".to_string(),
                                        format!(
                                            "Are you sure you want to delete cluster role {}?",
                                            name
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteClusterRoleBinding(name) => (
                                        "Delete Cluster Role Binding".to_string(),
                                        format!(
                                            "Are you sure you want to delete cluster role binding {}?",
                                            name
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteConfigMap(ns, name) => (
                                        "Delete Config Map".to_string(),
                                        format!(
                                            "Are you sure you want to delete config map {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteSecret(ns, name) => (
                                        "Delete Secret".to_string(),
                                        format!(
                                            "Are you sure you want to delete secret {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteResourceQuota(ns, name) => (
                                        "Delete Resource Quota".to_string(),
                                        format!(
                                            "Are you sure you want to delete resource quota {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteLimitRange(ns, name) => (
                                        "Delete Limit Range".to_string(),
                                        format!(
                                            "Are you sure you want to delete limit range {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteHorizontalPodAutoscaler(ns, name) => (
                                        "Delete Horizontal Pod Autoscaler".to_string(),
                                        format!(
                                            "Are you sure you want to delete horizontal pod autoscaler {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeletePodDisruptionBudget(ns, name) => (
                                        "Delete Pod Disruption Budget".to_string(),
                                        format!(
                                            "Are you sure you want to delete pod disruption budget {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeletePriorityClass(name) => (
                                        "Delete Priority Class".to_string(),
                                        format!(
                                            "Are you sure you want to delete priority class {}?",
                                            name
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteRuntimeClass(name) => (
                                        "Delete Runtime Class".to_string(),
                                        format!(
                                            "Are you sure you want to delete runtime class {}?",
                                            name
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteLease(ns, name) => (
                                        "Delete Lease".to_string(),
                                        format!(
                                            "Are you sure you want to delete lease {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteMutatingWebhookConfiguration(name) => (
                                        "Delete Mutating Webhook Configuration".to_string(),
                                        format!(
                                            "Are you sure you want to delete mutating webhook configuration {}?",
                                            name
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteValidatingWebhookConfiguration(name) => (
                                        "Delete Validating Webhook Configuration".to_string(),
                                        format!(
                                            "Are you sure you want to delete validating webhook configuration {}?",
                                            name
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteJob(ns, name) => (
                                        "Delete Job".to_string(),
                                        format!(
                                            "Are you sure you want to delete job {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::DeleteCronJob(ns, name) => (
                                        "Delete CronJob".to_string(),
                                        format!(
                                            "Are you sure you want to delete cronjob {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Delete".to_string(),
                                    ),
                                    ConfirmAction::RestartDeployment(ns, name) => (
                                        "Restart Deployment".to_string(),
                                        format!(
                                            "Restart all pods in deployment {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Restart".to_string(),
                                    ),
                                    ConfirmAction::RestartDaemonSet(ns, name) => (
                                        "Restart DaemonSet".to_string(),
                                        format!(
                                            "Restart all pods in daemonset {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Restart".to_string(),
                                    ),
                                    ConfirmAction::RestartStatefulSet(ns, name) => (
                                        "Restart StatefulSet".to_string(),
                                        format!(
                                            "Restart all pods in statefulset {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Restart".to_string(),
                                    ),
                                    ConfirmAction::RestartReplicaSet(ns, name) => (
                                        "Restart ReplicaSet".to_string(),
                                        format!(
                                            "Restart all pods in replicaset {} in namespace {}?",
                                            name, ns
                                        ),
                                        "Restart".to_string(),
                                    ),
                                    ConfirmAction::RestartReplicationController(ns, name) => (
                                        "Restart ReplicationController".to_string(),
                                        format!(
                                            "Restart all pods in replication controller {} in namespace {}?",
                                            name, ns
                                        ),
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
                                    ConfirmAction::ScaleReplicationController(
                                        ns,
                                        name,
                                        current,
                                    ) => {
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
                                    ConfirmAction::EditPortForward { .. } => {
                                        // EditPortForward is handled above, should not reach here
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
                    if let Some(modal_view) = self.views.get_mut(&section_keys::modal().to_string())
                    {
                        let ptr =
                            modal_view.as_mut() as *mut dyn ViewComponent as *mut views::ModalView;
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
