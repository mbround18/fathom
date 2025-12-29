use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};
use crossbeam_channel::{Receiver, Sender};
use std::time::Duration;
use tokio::runtime::Runtime;

mod daemonsets;
mod deployments;
mod events;
mod kube;
mod metrics;
mod pods;
mod replicasets;
mod replicationcontrollers;
mod state;
mod statefulsets;
mod sync;
mod workload_common;

use state::BackendState;

pub fn init(receiver: Receiver<BackendCmd>, sender: Sender<BackendEventEnvelope>) {
    let rt = Runtime::new().expect("Failed to create Tokio runtime");

    let mut state = BackendState::new();

    events::emit(
        state.seq.as_ref(),
        &sender,
        Route::global(section_keys::system()),
        Correlation::None,
        BackendEvent::System(SystemEvent::BackendStarted),
    );
    events::notify_info(
        state.seq.as_ref(),
        &sender,
        "Backend thread started (tokio runtime ready).".to_owned(),
    );

    loop {
        let cmd = match receiver.recv() {
            Ok(cmd) => cmd,
            Err(_) => break,
        };

        let result = rt.block_on(handle_cmd(cmd, &mut state, &sender));
        if let Err(err) = result {
            events::notify_error(state.seq.as_ref(), &sender, err.to_string());
        }
    }
}

async fn handle_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<()> {
    match cmd {
        BackendCmd::Initialize => {
            // Contexts can be loaded even before we connect.
            let _ = kube::load_and_send_contexts(
                state.seq.as_ref(),
                state.current_context.clone(),
                sender,
            );

            events::notify_info(
                state.seq.as_ref(),
                sender,
                "Connecting to cluster (kubeconfig/in-cluster)...".to_owned(),
            );

            let client = kube::connect_client_for_current_context(state).await?;
            state.client = Some(client);

            // Best-effort: if we didn't have an explicit context selection, treat kubeconfig's
            // current context as the backend-authoritative active context.
            if state.current_context.is_none() {
                state.current_context = kube::read_kubeconfig_current_context();
            }
            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::ContextChanged {
                    current: state.current_context.clone(),
                }),
            );

            events::notify_info(state.seq.as_ref(), sender, "Connected.".to_owned());

            // Discover optional backing services.
            metrics::probe_metrics_api(state, sender).await?;

            sync::refresh_namespaces(state, sender).await?;
            pods::refresh_pods(state, sender).await?;
            deployments::refresh_deployments(state, sender).await?;
            daemonsets::refresh_daemonsets(state, sender).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
            replicasets::refresh_replicasets(state, sender).await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;

            sync::ensure_background_sync(state, sender).await?;
        }

        BackendCmd::RefreshContexts => {
            let _ = kube::load_and_send_contexts(
                state.seq.as_ref(),
                state.current_context.clone(),
                sender,
            );
        }

        BackendCmd::SetContext { context } => {
            // Stop background activity and reconnect with the new context.
            state::stop_logs(state);
            state::abort_task(&mut state.pod_watch_task);
            state::abort_task(&mut state.namespace_sync_task);

            state.current_context = Some(context.clone());
            state.reset_metrics_probe();

            // Context switch invalidates namespace filters; reset to "all" on the backend.
            state.namespace_filter.clear();
            if let Ok(mut shared) = state.namespace_filter_shared.write() {
                shared.clear();
            }
            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::NamespaceFilterChanged {
                    namespaces: Vec::new(),
                }),
            );

            events::notify_info(
                state.seq.as_ref(),
                sender,
                format!("Switching context to '{}'...", context),
            );

            let client = kube::connect_client_for_current_context(state).await?;
            state.client = Some(client);

            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::ContextChanged {
                    current: state.current_context.clone(),
                }),
            );

            // Re-load contexts so UI reflects the selected one.
            let _ = kube::load_and_send_contexts(
                state.seq.as_ref(),
                state.current_context.clone(),
                sender,
            );

            metrics::probe_metrics_api(state, sender).await?;
            sync::refresh_namespaces(state, sender).await?;
            pods::refresh_pods(state, sender).await?;
            deployments::refresh_deployments(state, sender).await?;
            daemonsets::refresh_daemonsets(state, sender).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
            replicasets::refresh_replicasets(state, sender).await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
            sync::ensure_background_sync(state, sender).await?;

            events::notify_info(state.seq.as_ref(), sender, "Context switched.".to_owned());
        }

        BackendCmd::RefreshNamespaces => {
            sync::refresh_namespaces(state, sender).await?;
        }

        BackendCmd::SetNamespaceFilter { namespaces } => {
            state.namespace_filter = namespaces;
            if let Ok(mut shared) = state.namespace_filter_shared.write() {
                *shared = state.namespace_filter.clone();
            }

            events::emit(
                state.seq.as_ref(),
                sender,
                Route::global(section_keys::system()),
                Correlation::None,
                BackendEvent::System(SystemEvent::NamespaceFilterChanged {
                    namespaces: state.namespace_filter.clone(),
                }),
            );
            pods::refresh_pods(state, sender).await?;
            deployments::refresh_deployments(state, sender).await?;
            daemonsets::refresh_daemonsets(state, sender).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
            replicasets::refresh_replicasets(state, sender).await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
        }

        BackendCmd::RefreshPods => {
            pods::refresh_pods(state, sender).await?;
        }

        BackendCmd::DeletePod { namespace, name } => {
            pods::delete_pod(state, sender, &namespace, &name).await?;
            pods::refresh_pods(state, sender).await?;
        }

        BackendCmd::SelectPod { namespace, name } => {
            pods::fetch_pod_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::StartPodLogs {
            namespace,
            name,
            container,
        } => {
            pods::start_logs(state, sender, namespace, name, container).await?;
        }

        BackendCmd::StopPodLogs => {
            state::stop_logs(state);
        }

        BackendCmd::FetchPodMetrics { namespace, name } => {
            metrics::fetch_metrics(state, sender, &namespace, &name).await?;
        }

        BackendCmd::RefreshDeployments => {
            deployments::refresh_deployments(state, sender).await?;
        }

        BackendCmd::SelectDeployment { namespace, name } => {
            deployments::fetch_deployment_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::ScaleDeployment {
            namespace,
            name,
            replicas,
        } => {
            deployments::scale_deployment(state, sender, &namespace, &name, replicas).await?;
            deployments::refresh_deployments(state, sender).await?;
        }

        BackendCmd::RestartDeployment { namespace, name } => {
            deployments::restart_deployment(state, sender, &namespace, &name).await?;
            deployments::refresh_deployments(state, sender).await?;
        }

        BackendCmd::DeleteDeployment { namespace, name } => {
            deployments::delete_deployment(state, sender, &namespace, &name).await?;
            deployments::refresh_deployments(state, sender).await?;
        }

        BackendCmd::RefreshDaemonSets => {
            daemonsets::refresh_daemonsets(state, sender).await?;
        }

        BackendCmd::SelectDaemonSet { namespace, name } => {
            daemonsets::fetch_daemonset_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::RestartDaemonSet { namespace, name } => {
            daemonsets::restart_daemonset(state, sender, &namespace, &name).await?;
            daemonsets::refresh_daemonsets(state, sender).await?;
        }

        BackendCmd::DeleteDaemonSet { namespace, name } => {
            daemonsets::delete_daemonset(state, sender, &namespace, &name).await?;
            daemonsets::refresh_daemonsets(state, sender).await?;
        }

        BackendCmd::RefreshStatefulSets => {
            statefulsets::refresh_statefulsets(state, sender).await?;
        }

        BackendCmd::SelectStatefulSet { namespace, name } => {
            statefulsets::fetch_statefulset_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::ScaleStatefulSet {
            namespace,
            name,
            replicas,
        } => {
            statefulsets::scale_statefulset(state, sender, &namespace, &name, replicas).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
        }

        BackendCmd::RestartStatefulSet { namespace, name } => {
            statefulsets::restart_statefulset(state, sender, &namespace, &name).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
        }

        BackendCmd::DeleteStatefulSet { namespace, name } => {
            statefulsets::delete_statefulset(state, sender, &namespace, &name).await?;
            statefulsets::refresh_statefulsets(state, sender).await?;
        }

        BackendCmd::RefreshReplicaSets => {
            replicasets::refresh_replicasets(state, sender).await?;
        }

        BackendCmd::SelectReplicaSet { namespace, name } => {
            replicasets::fetch_replicaset_details(state, sender, &namespace, &name).await?;
        }

        BackendCmd::ScaleReplicaSet {
            namespace,
            name,
            replicas,
        } => {
            replicasets::scale_replicaset(state, sender, &namespace, &name, replicas).await?;
            replicasets::refresh_replicasets(state, sender).await?;
        }

        BackendCmd::RestartReplicaSet { namespace, name } => {
            replicasets::restart_replicaset(state, sender, &namespace, &name).await?;
            replicasets::refresh_replicasets(state, sender).await?;
        }

        BackendCmd::DeleteReplicaSet { namespace, name } => {
            replicasets::delete_replicaset(state, sender, &namespace, &name).await?;
            replicasets::refresh_replicasets(state, sender).await?;
        }

        BackendCmd::RefreshReplicationControllers => {
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
        }

        BackendCmd::SelectReplicationController { namespace, name } => {
            replicationcontrollers::fetch_replicationcontroller_details(
                state, sender, &namespace, &name,
            )
            .await?;
        }

        BackendCmd::ScaleReplicationController {
            namespace,
            name,
            replicas,
        } => {
            replicationcontrollers::scale_replicationcontroller(
                state, sender, &namespace, &name, replicas,
            )
            .await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
        }

        BackendCmd::RestartReplicationController { namespace, name } => {
            replicationcontrollers::restart_replicationcontroller(state, sender, &namespace, &name)
                .await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
        }

        BackendCmd::DeleteReplicationController { namespace, name } => {
            replicationcontrollers::delete_replicationcontroller(state, sender, &namespace, &name)
                .await?;
            replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
        }

        BackendCmd::SimulateSlowCall => {
            events::notify_info(state.seq.as_ref(), sender, "Simulating slow call...".into());
            tokio::time::sleep(Duration::from_secs(2)).await;
            events::notify_info(state.seq.as_ref(), sender, "Slow call done.".into());
        }

        BackendCmd::SetActiveSection { section } => {
            // Light prefetch: keep current section fresh.
            if section == section_keys::workloads_pods() {
                pods::refresh_pods(state, sender).await?;
            } else if section == section_keys::workloads_deployments() {
                deployments::refresh_deployments(state, sender).await?;
            } else if section == section_keys::workloads_daemonsets() {
                daemonsets::refresh_daemonsets(state, sender).await?;
            } else if section == section_keys::workloads_statefulsets() {
                statefulsets::refresh_statefulsets(state, sender).await?;
            } else if section == section_keys::workloads_replicasets() {
                replicasets::refresh_replicasets(state, sender).await?;
            } else if section == section_keys::workloads_replicationcontrollers() {
                replicationcontrollers::refresh_replicationcontrollers(state, sender).await?;
            }
        }
    }

    Ok(())
}
