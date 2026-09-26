//! Workload commands: pods (logs, metrics, bulk delete), deployments, daemonsets,
//! statefulsets, replicasets, replication controllers, jobs and cronjobs.

use crossbeam_channel::Sender;

use crate::backend::state::BackendState;
use crate::backend::{
    cronjobs, daemonsets, deployments, events, exec, images, jobs, metrics, pods, replicasets,
    replicationcontrollers, statefulsets, trivy,
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
        BackendCmd::AddEphemeralContainer {
            namespace,
            pod,
            target_container,
            image,
            ephemeral_container_name,
        } => {
            pods::add_ephemeral_container(
                state,
                sender,
                &namespace,
                &pod,
                target_container.as_deref(),
                &image,
                ephemeral_container_name.as_deref(),
            )
            .await?;
            pods::fetch_pod_details(state, sender, &namespace, &pod).await?;
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
        BackendCmd::StartPodExec {
            namespace,
            name,
            container,
        } => exec::start_exec(state, sender, namespace, name, container).await?,
        BackendCmd::CheckPodExecAccess {
            namespace,
            name,
            container,
        } => exec::check_access(state, sender, namespace, name, container).await?,
        BackendCmd::SendPodExecInput { data } => exec::send_input(state, data).await?,
        BackendCmd::ResizePodExec { cols, rows } => exec::resize(state, cols, rows).await?,
        BackendCmd::StopPodExec => exec::stop(state),
        BackendCmd::InspectContainerImage {
            namespace,
            pod,
            container,
        } => {
            images::inspect_container_image(state, sender, &namespace, &pod, &container).await?;
        }
        BackendCmd::InspectPodVulnerabilities { namespace, pod } => {
            trivy::handle_inspect(state, sender, &namespace, &pod).await?;
        }
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
        BackendCmd::StartNodeMetricsPolling { interval_secs } => {
            metrics::start_node_metrics_polling(state, sender, interval_secs).await?;
        }
        BackendCmd::StopNodeMetricsPolling => metrics::stop_node_metrics_polling(state),
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

/// Delete every `(namespace, name)` pod, then refresh. Failures do not stop the loop;
/// they are reported together in one error notification (one `ns/name: <error>` per
/// line, error text verbatim), which also marks the command's `CommandFinished` failed.
async fn bulk_delete_pods(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    items: Vec<(String, String)>,
) -> anyhow::Result<()> {
    let mut success_count = 0;
    let mut failures = Vec::new();

    for (namespace, name) in items {
        match pods::delete_pod(state, sender, &namespace, &name).await {
            Ok(()) => success_count += 1,
            Err(e) => {
                tracing::warn!(namespace = %namespace, name = %name, "Bulk delete failed: {:#}", e);
                failures.push(format!("{namespace}/{name}: {e:#}"));
            }
        }
    }

    if success_count > 0 {
        events::notify_info(
            state.cluster.seq.as_ref(),
            sender,
            format!("Deleted {} item(s)", success_count),
        );
    }

    if !failures.is_empty() {
        events::notify_error(
            state.cluster.seq.as_ref(),
            sender,
            format!(
                "Failed to delete {} item(s):\n{}",
                failures.len(),
                failures.join("\n")
            ),
        );
    }

    // Refresh the relevant sections
    pods::refresh_pods(state, sender).await
}

/// Post-mutation refresh. The write already succeeded; the follow-up list (cluster-wide when no
/// namespace filter is set) can be forbidden to a namespace-scoped user (hosted per-user RBAC),
/// which must not turn a successful write into a failed command.
async fn refresh_deployments_best_effort(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) {
    if let Err(e) = deployments::refresh_deployments(state, sender).await {
        tracing::warn!("post-mutation deployments refresh failed: {e:#}");
    }
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
            refresh_deployments_best_effort(state, sender).await;
        }
        BackendCmd::RestartDeployment { namespace, name } => {
            deployments::restart_deployment(state, sender, &namespace, &name).await?;
            refresh_deployments_best_effort(state, sender).await;
        }
        BackendCmd::DeleteDeployment { namespace, name } => {
            deployments::delete_deployment(state, sender, &namespace, &name).await?;
            refresh_deployments_best_effort(state, sender).await;
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
