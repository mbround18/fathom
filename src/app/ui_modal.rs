use super::*;
use std::net::TcpListener;
use std::time::Instant;

impl FathomApp {
    pub(super) fn render_modal(&mut self, ctx: &egui::Context) {
        let mut next_modal: Option<ModalState> = None;
        let mut confirm_action: Option<ConfirmAction> = None;
        let mut scale_apply: Option<(WorkloadKind, String, String, i32)> = None;
        let mut create_portforward: Option<(String, String, u16, u16, Option<String>)> = None;

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
                    WorkloadKind::Job => "Scale job",
                    WorkloadKind::CronJob => "Scale cronjob",
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

            ModalState::CreatePortForward {
                mut namespace,
                mut pod,
                mut local_port,
                mut remote_port,
                mut container,
            } => {
                let mut cancel = false;
                let mut create = false;

                egui::Window::new("Create Port Forward")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        render_autocomplete_field(
                            ui,
                            "Namespace",
                            &mut namespace,
                            &self.namespaces,
                            "pf_ns_field",
                        );

                        // Restrict pod suggestions to the chosen namespace when present
                        let pod_options: Vec<String> = self
                            .pods
                            .iter()
                            .filter(|p| namespace.is_empty() || p.namespace == namespace)
                            .map(|p| p.name.clone())
                            .collect();
                        render_autocomplete_field(
                            ui,
                            "Pod",
                            &mut pod,
                            &pod_options,
                            "pf_pod_field",
                        );

                        ui.label("Local port (blank = auto)");
                        ui.text_edit_singleline(&mut local_port);

                        ui.label("Remote port (blank = same as local)");
                        ui.text_edit_singleline(&mut remote_port);

                        ui.label("Container (optional)");
                        ui.text_edit_singleline(&mut container);

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
                    // Parse ports; default to same port if remote empty
                    // Determine ports: local can be auto-selected if blank; remote defaults to local
                    let preferred_local = local_port.trim().parse::<u16>().ok();
                    let remote_from_input = if remote_port.trim().is_empty() {
                        None
                    } else {
                        remote_port.trim().parse::<u16>().ok()
                    };

                    if let Some(lp) = choose_local_port(preferred_local.or(remote_from_input)) {
                        let rp = remote_from_input.unwrap_or(lp);

                        let container_opt = if container.trim().is_empty() {
                            None
                        } else {
                            Some(container.clone())
                        };
                        create_portforward =
                            Some((namespace.clone(), pod.clone(), lp, rp, container_opt));
                        next_modal = Some(ModalState::None);
                    } else {
                        // Keep modal open on parse failure
                        next_modal = Some(ModalState::CreatePortForward {
                            namespace,
                            pod,
                            local_port,
                            remote_port,
                            container,
                        });
                    }
                } else {
                    next_modal = Some(ModalState::CreatePortForward {
                        namespace,
                        pod,
                        local_port,
                        remote_port,
                        container,
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
                WorkloadKind::DaemonSet | WorkloadKind::Job | WorkloadKind::CronJob => {
                    // DaemonSets, Jobs, and CronJobs don't scale by replicas.
                }
            }
        }

        if let Some((namespace, pod, local_port, remote_port, container)) = create_portforward {
            let namespace_clone = namespace.clone();
            let pod_clone = pod.clone();

            self.send(BackendCmd::StartPortForward {
                namespace,
                pod,
                local_port,
                remote_port,
                container,
            });
            self.status_log.push(format!(
                "Starting port forward: 127.0.0.1:{} -> {}/{}:{}",
                local_port, namespace_clone, pod_clone, remote_port
            ));
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
                WorkloadKind::Job => self.send(BackendCmd::DeleteJob { namespace, name }),
                WorkloadKind::CronJob => self.send(BackendCmd::DeleteCronJob { namespace, name }),
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
                WorkloadKind::Job | WorkloadKind::CronJob => {
                    // Jobs and CronJobs don't support restart
                }
            },
        }
    }
}

fn render_autocomplete_field(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    options: &[String],
    id: &str,
) {
    ui.label(label);
    ui.text_edit_singleline(value);
    egui::ComboBox::from_id_source(id)
        .selected_text(if value.is_empty() {
            "(suggestions)".to_owned()
        } else {
            value.clone()
        })
        .show_ui(ui, |ui| {
            let filter = value.to_lowercase();
            for opt in options {
                if filter.is_empty() || opt.to_lowercase().contains(&filter) {
                    if ui.selectable_label(false, opt).clicked() {
                        *value = opt.clone();
                        ui.close_menu();
                    }
                }
            }
        });
    ui.add_space(4.0);
}

fn port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port))
        .map(|l| drop(l))
        .is_ok()
}

fn choose_local_port(preferred: Option<u16>) -> Option<u16> {
    if let Some(p) = preferred {
        if port_available(p) {
            return Some(p);
        }
    }

    // Deterministic-ish starting point to avoid new RNG dependency
    let span = 8000u16; // 21000-28999 inclusive
    let start = 21000u16.saturating_add((Instant::now().elapsed().as_nanos() as u16) % span);

    for offset in 0..span {
        let candidate = 21000u16 + ((start - 21000u16 + offset) % span);
        if port_available(candidate) {
            return Some(candidate);
        }
    }

    None
}
