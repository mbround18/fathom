use crate::app::ConfirmAction;
use crate::protocol::BackendCmd;
use egui::Ui;

/// Identifies a workload resource for actions (scale, restart, delete, logs).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WorkloadActionKind {
    Deployment,
    StatefulSet,
    DaemonSet,
    ReplicaSet,
    ReplicationController,
}

impl WorkloadActionKind {
    pub fn display(&self) -> &'static str {
        match self {
            Self::Deployment => "Deployment",
            Self::StatefulSet => "StatefulSet",
            Self::DaemonSet => "DaemonSet",
            Self::ReplicaSet => "ReplicaSet",
            Self::ReplicationController => "ReplicationController",
        }
    }

    /// Check if this workload type supports scaling.
    pub fn supports_scaling(&self) -> bool {
        matches!(
            self,
            Self::Deployment | Self::StatefulSet | Self::ReplicaSet | Self::ReplicationController
        )
    }

    /// Check if this workload type supports restart.
    pub fn supports_restart(&self) -> bool {
        matches!(
            self,
            Self::Deployment | Self::StatefulSet | Self::ReplicaSet | Self::ReplicationController
        )
    }

    /// Check if this workload type supports deletion.
    pub fn supports_delete(&self) -> bool {
        true // All workload types can be deleted
    }

    /// Check if this workload type supports logs.
    pub fn supports_logs(&self) -> bool {
        true // All workload types have pods with logs
    }
}

/// Shared workload actions component.
pub struct WorkloadActions;

impl WorkloadActions {
    /// Render action buttons for a workload (Scale, Restart, Delete).
    /// Returns action(s) to open confirmation modals or trigger commands.
    pub fn render_buttons(
        ui: &mut Ui,
        workload_kind: &WorkloadActionKind,
        namespace: &str,
        name: &str,
        current_replicas: Option<i32>,
    ) -> Vec<ConfirmAction> {
        let mut actions = Vec::new();

        ui.horizontal(|ui| {
            // Scale button (if supported)
            if workload_kind.supports_scaling() {
                if ui.button("Scale").clicked() {
                    match workload_kind {
                        WorkloadActionKind::Deployment => {
                            actions.push(ConfirmAction::ScaleDeployment(
                                namespace.to_string(),
                                name.to_string(),
                                current_replicas.unwrap_or(0),
                            ));
                        }
                        WorkloadActionKind::StatefulSet => {
                            actions.push(ConfirmAction::ScaleStatefulSet(
                                namespace.to_string(),
                                name.to_string(),
                                current_replicas.unwrap_or(0),
                            ));
                        }
                        WorkloadActionKind::ReplicaSet => {
                            actions.push(ConfirmAction::ScaleReplicaSet(
                                namespace.to_string(),
                                name.to_string(),
                                current_replicas.unwrap_or(0),
                            ));
                        }
                        WorkloadActionKind::ReplicationController => {
                            actions.push(ConfirmAction::ScaleReplicationController(
                                namespace.to_string(),
                                name.to_string(),
                                current_replicas.unwrap_or(0),
                            ));
                        }
                        _ => {}
                    }
                }
            }

            // Restart button (if supported)
            if workload_kind.supports_restart() {
                if ui.button("Restart").clicked() {
                    match workload_kind {
                        WorkloadActionKind::Deployment => {
                            actions.push(ConfirmAction::RestartDeployment(
                                namespace.to_string(),
                                name.to_string(),
                            ));
                        }
                        WorkloadActionKind::StatefulSet => {
                            actions.push(ConfirmAction::RestartStatefulSet(
                                namespace.to_string(),
                                name.to_string(),
                            ));
                        }
                        WorkloadActionKind::DaemonSet => {
                            actions.push(ConfirmAction::RestartDaemonSet(
                                namespace.to_string(),
                                name.to_string(),
                            ));
                        }
                        WorkloadActionKind::ReplicaSet => {
                            actions.push(ConfirmAction::RestartReplicaSet(
                                namespace.to_string(),
                                name.to_string(),
                            ));
                        }
                        WorkloadActionKind::ReplicationController => {
                            actions.push(ConfirmAction::RestartReplicationController(
                                namespace.to_string(),
                                name.to_string(),
                            ));
                        }
                    }
                }
            }

            // Delete button (always supported)
            if ui.button("Delete").clicked() {
                match workload_kind {
                    WorkloadActionKind::Deployment => {
                        actions.push(ConfirmAction::DeleteDeployment(
                            namespace.to_string(),
                            name.to_string(),
                        ));
                    }
                    WorkloadActionKind::StatefulSet => {
                        actions.push(ConfirmAction::DeleteStatefulSet(
                            namespace.to_string(),
                            name.to_string(),
                        ));
                    }
                    WorkloadActionKind::DaemonSet => {
                        actions.push(ConfirmAction::DeleteDaemonSet(
                            namespace.to_string(),
                            name.to_string(),
                        ));
                    }
                    WorkloadActionKind::ReplicaSet => {
                        actions.push(ConfirmAction::DeleteReplicaSet(
                            namespace.to_string(),
                            name.to_string(),
                        ));
                    }
                    WorkloadActionKind::ReplicationController => {
                        actions.push(ConfirmAction::DeleteReplicationController(
                            namespace.to_string(),
                            name.to_string(),
                        ));
                    }
                }
            }
        });

        actions
    }

    /// Convert confirm action to a backend command.
    pub fn to_backend_cmd(action: &ConfirmAction) -> Option<BackendCmd> {
        match action {
            ConfirmAction::DeleteDeployment(ns, name) => Some(BackendCmd::DeleteDeployment {
                namespace: ns.clone(),
                name: name.clone(),
            }),
            ConfirmAction::RestartDeployment(ns, name) => Some(BackendCmd::RestartDeployment {
                namespace: ns.clone(),
                name: name.clone(),
            }),
            ConfirmAction::DeleteStatefulSet(ns, name) => Some(BackendCmd::DeleteStatefulSet {
                namespace: ns.clone(),
                name: name.clone(),
            }),
            ConfirmAction::RestartStatefulSet(ns, name) => Some(BackendCmd::RestartStatefulSet {
                namespace: ns.clone(),
                name: name.clone(),
            }),
            ConfirmAction::DeleteDaemonSet(ns, name) => Some(BackendCmd::DeleteDaemonSet {
                namespace: ns.clone(),
                name: name.clone(),
            }),
            ConfirmAction::RestartDaemonSet(ns, name) => Some(BackendCmd::RestartDaemonSet {
                namespace: ns.clone(),
                name: name.clone(),
            }),
            ConfirmAction::DeleteReplicaSet(ns, name) => Some(BackendCmd::DeleteReplicaSet {
                namespace: ns.clone(),
                name: name.clone(),
            }),
            ConfirmAction::RestartReplicaSet(ns, name) => Some(BackendCmd::RestartReplicaSet {
                namespace: ns.clone(),
                name: name.clone(),
            }),
            ConfirmAction::DeleteReplicationController(ns, name) => {
                Some(BackendCmd::DeleteReplicationController {
                    namespace: ns.clone(),
                    name: name.clone(),
                })
            }
            ConfirmAction::RestartReplicationController(ns, name) => {
                Some(BackendCmd::RestartReplicationController {
                    namespace: ns.clone(),
                    name: name.clone(),
                })
            }
            _ => None,
        }
    }
}
