//! Workload commands: pods (logs, metrics, bulk delete), deployments, daemonsets,
//! statefulsets, replicasets, replication controllers, jobs and cronjobs.

use crossbeam_channel::Sender;

use crate::backend::state::BackendState;
use crate::backend::{
    cronjobs, daemonsets, deployments, events, jobs, metrics, pods, replicasets,
    replicationcontrollers, statefulsets,
};
use crate::protocol::{BackendCmd, BackendEventEnvelope};

/// Handles workload commands; returns the command back if it belongs to another family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    let Some(cmd) = handle_pod_cmd(cmd, state, sender).await? else {
        return Ok(None);
    };
    let Some(cmd) = handle_deployment_daemonset_cmd(cmd, state, sender).await? else {
        return Ok(None);
    };
    let Some(cmd) = handle_statefulset_replicaset_cmd(cmd, state, sender).await? else {
        return Ok(None);
    };
    handle_controller_job_cmd(cmd, state, sender).await
}

async fn handle_pod_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshPods => pods::refresh_pods(state, sender).await?,
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
        } => pods::start_logs(state, sender, namespace, name, container).await?,
        BackendCmd::StopPodLogs => state.log_stream.stop(),
        BackendCmd::FetchPodMetrics { namespace, name } => {
            metrics::fetch_metrics(state, sender, &namespace, &name).await?;
        }
        BackendCmd::StartMetricsPolling {
            namespace,
            name,
            interval_secs,
        } => start_metrics_polling(state, sender, namespace, name, interval_secs).await?,
        BackendCmd::StopMetricsPolling => metrics::stop_metrics_polling(state),
        BackendCmd::StartWorkloadMetricsPolling {
            namespace,
            workload_name,
            workload_kind,
            pod_selector,
            interval_secs,
        } => {
            metrics::start_workload_metrics_polling(
                state,
                sender,
                namespace,
                workload_name,
                workload_kind,
                pod_selector,
                interval_secs,
            )
            .await?;
        }
        BackendCmd::StopWorkloadMetricsPolling => metrics::stop_workload_metrics_polling(state),
        BackendCmd::BulkDelete { items } => bulk_delete_pods(state, sender, items).await?,
        other => return Ok(Some(other)),
    }
    Ok(None)
}

async fn start_metrics_polling(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
    name: String,
    interval_secs: u64,
) -> anyhow::Result<()> {
    // Fetch pod to get resource specs
    use k8s_openapi::api::core::v1::Pod;
    let client = match &state.cluster.client {
        Some(c) => c,
        None => anyhow::bail!("No Kubernetes client available"),
    };
    let pod_api = ::kube::Api::<Pod>::namespaced(client.clone(), &namespace);
    let pod = pod_api.get(&name).await?;

    // Extract resource specs (reuse logic from pods module)
    let resource_specs = pods::extract_resource_specs(&pod);

    metrics::start_metrics_polling(
        state,
        sender,
        namespace,
        name,
        resource_specs,
        interval_secs,
    )
    .await
}

async fn bulk_delete_pods(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    items: Vec<(String, String)>,
) -> anyhow::Result<()> {
    let mut success_count = 0;
    let mut failed_items = Vec::new();

    for (namespace, name) in items {
        if state.cluster.client.is_some() {
            match pods::delete_pod(state, sender, &namespace, &name).await {
                Ok(_) => success_count += 1,
                Err(e) => {
                    tracing::warn!(namespace = %namespace, name = %name, "Bulk delete failed: {}", e);
                    failed_items.push(format!("{}/{}", namespace, name));
                }
            }
        } else {
            failed_items.push(format!("{}/{}", namespace, name));
        }
    }

    if success_count > 0 {
        events::notify_info(
            state.cluster.seq.as_ref(),
            sender,
            format!("Deleted {} item(s)", success_count),
        );
    }

    if !failed_items.is_empty() {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!("Failed to delete: {}", failed_items.join(", ")),
        );
    }

    // Refresh the relevant sections
    pods::refresh_pods(state, sender).await
}

async fn handle_deployment_daemonset_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshDeployments => deployments::refresh_deployments(state, sender).await?,
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
        BackendCmd::RefreshDaemonSets => daemonsets::refresh_daemonsets(state, sender).await?,
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
        other => return Ok(Some(other)),
    }
    Ok(None)
}

async fn handle_statefulset_replicaset_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
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
        BackendCmd::RefreshReplicaSets => replicasets::refresh_replicasets(state, sender).await?,
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
        other => return Ok(Some(other)),
    }
    Ok(None)
}

async fn handle_controller_job_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
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
        BackendCmd::RefreshJobs => jobs::refresh_jobs(state, sender).await?,
        BackendCmd::SelectJob { namespace, name } => {
            jobs::fetch_job_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteJob { namespace, name } => {
            jobs::delete_job(state, sender, &namespace, &name).await?;
            jobs::refresh_jobs(state, sender).await?;
        }
        other => return handle_cronjob_cmd(other, state, sender).await,
    }
    Ok(None)
}

async fn handle_cronjob_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshCronJobs => cronjobs::refresh_cronjobs(state, sender).await?,
        BackendCmd::SelectCronJob { namespace, name } => {
            cronjobs::fetch_cronjob_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::SuspendCronJob {
            namespace,
            name,
            suspend,
        } => {
            cronjobs::suspend_cronjob(state, sender, &namespace, &name, suspend).await?;
            cronjobs::refresh_cronjobs(state, sender).await?;
        }
        BackendCmd::TriggerCronJob { namespace, name } => {
            cronjobs::trigger_cronjob(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteCronJob { namespace, name } => {
            cronjobs::delete_cronjob(state, sender, &namespace, &name).await?;
            cronjobs::refresh_cronjobs(state, sender).await?;
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}
