use crate::protocol::{
    BackendCmd, BackendEvent, BackendEventEnvelope, Correlation, Route, SystemEvent, section_keys,
};
use crossbeam_channel::{Receiver, Sender};
use std::time::Duration;
use tokio::runtime::Runtime;

mod cronjobs;
mod daemonsets;
mod deployments;
mod endpoints;
mod events;
mod ingressclasses;
mod ingresses;
pub mod integrations;
mod interactions;
mod jobs;
mod kube;
mod longhorn;
mod metrics;
mod metrics_provider;
mod networkpolicies;
mod nodes;
pub mod persistence;
mod pods;
mod portforward;
mod replicasets;
mod replicationcontrollers;
mod services;
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
            }
        }
    }

    Ok(())
}
