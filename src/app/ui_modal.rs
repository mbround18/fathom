use super::*;

impl FathomApp {
    pub(super) fn render_modal(&mut self, ctx: &egui::Context) {
        let mut next_modal: Option<ModalState> = None;
        let mut confirm_action: Option<ConfirmAction> = None;
        let mut scale_apply: Option<(WorkloadKind, String, String, i32)> = None;

        match self.modal.clone() {
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
                mut desired,
            } => {
                let mut cancel = false;
                let mut apply = false;

                let title = match kind {
                    WorkloadKind::Deployment => "Scale deployment",
                    WorkloadKind::DaemonSet => "Scale daemonset",
                    WorkloadKind::StatefulSet => "Scale statefulset",
                    WorkloadKind::ReplicaSet => "Scale replicaset",
                    WorkloadKind::ReplicationController => "Scale replicationcontroller",
                };

                egui::Window::new(title)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("{}/{}", namespace, name));
                        ui.add_space(6.0);
                        ui.label(format!("Current ready replicas: {}", current));
                        ui.add_space(10.0);

                        let max =
                            std::cmp::max(desired, std::cmp::max(current, 10)).saturating_add(10);
                        ui.add(
                            egui::Slider::new(&mut desired, 0..=max)
                                .text("replicas")
                                .clamping(egui::SliderClamping::Always),
                        );

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
                    scale_apply = Some((kind, namespace, name, desired));
                } else {
                    next_modal = Some(ModalState::ScaleWorkload {
                        kind,
                        namespace,
                        name,
                        current,
                        desired,
                    });
                }
            }
        }

        // Apply state transitions after UI rendering.
        if let Some(next) = next_modal {
            // Preserve the existing modal if we couldn't compute a better next state.
            self.modal = next;
        }

        if let Some(action) = confirm_action {
            self.apply_confirm_action(action);
        }

        if let Some((kind, namespace, name, replicas)) = scale_apply {
            match kind {
                WorkloadKind::Deployment => self.send(BackendCmd::ScaleDeployment {
                    namespace,
                    name,
                    replicas,
                }),
                WorkloadKind::StatefulSet => self.send(BackendCmd::ScaleStatefulSet {
                    namespace,
                    name,
                    replicas,
                }),
                WorkloadKind::ReplicaSet => self.send(BackendCmd::ScaleReplicaSet {
                    namespace,
                    name,
                    replicas,
                }),
                WorkloadKind::ReplicationController => {
                    self.send(BackendCmd::ScaleReplicationController {
                        namespace,
                        name,
                        replicas,
                    })
                }
                WorkloadKind::DaemonSet => {
                    // DaemonSets don't scale by replicas.
                }
            }
        }
    }

    fn apply_confirm_action(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::DeletePod { namespace, name } => {
                // Close right panel and stop streams before delete.
                self.selected_pod = None;
                self.pod_details = None;
                self.pod_metrics = None;
                self.pod_logs.clear();
                self.send(BackendCmd::StopPodLogs);

                self.send(BackendCmd::DeletePod { namespace, name });
            }

            ConfirmAction::DeleteWorkload {
                kind,
                namespace,
                name,
            } => match kind {
                WorkloadKind::Deployment => {
                    self.send(BackendCmd::DeleteDeployment { namespace, name })
                }
                WorkloadKind::DaemonSet => {
                    self.send(BackendCmd::DeleteDaemonSet { namespace, name })
                }
                WorkloadKind::StatefulSet => {
                    self.send(BackendCmd::DeleteStatefulSet { namespace, name })
                }
                WorkloadKind::ReplicaSet => {
                    self.send(BackendCmd::DeleteReplicaSet { namespace, name })
                }
                WorkloadKind::ReplicationController => {
                    self.send(BackendCmd::DeleteReplicationController { namespace, name })
                }
            },

            ConfirmAction::RestartWorkload {
                kind,
                namespace,
                name,
            } => match kind {
                WorkloadKind::Deployment => {
                    self.send(BackendCmd::RestartDeployment { namespace, name })
                }
                WorkloadKind::DaemonSet => {
                    self.send(BackendCmd::RestartDaemonSet { namespace, name })
                }
                WorkloadKind::StatefulSet => {
                    self.send(BackendCmd::RestartStatefulSet { namespace, name })
                }
                WorkloadKind::ReplicaSet => {
                    self.send(BackendCmd::RestartReplicaSet { namespace, name })
                }
                WorkloadKind::ReplicationController => {
                    self.send(BackendCmd::RestartReplicationController { namespace, name })
                }
            },
        }
    }
}
