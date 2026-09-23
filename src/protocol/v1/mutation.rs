//! What a command may change, used as a guard by shells that restrict writes (e.g. the
//! `fathom-devbridge` sandbox guard) and to route `BackendEventV1::CommandFinished`.
//!
//! Both matches below are exhaustive with no wildcard arm on purpose: adding a
//! `BackendCmdV1` variant must force a decision here at compile time.

#[cfg(test)]
use super::{ActionOutcome, AgentAuthModeInput, AgentObjectRef};
use super::{BackendCmdV1, SectionKey, section_keys};

/// What executing a command can modify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationTarget {
    /// Reads the cluster and/or adjusts transient backend state (selection, polling,
    /// log streams, namespace filter). Persists nothing and writes nothing to the cluster.
    ReadOnly,
    /// Writes namespaced cluster objects in these namespaces (sorted, deduplicated).
    /// A single-object command yields one namespace; `BulkDelete` yields every target
    /// namespace (empty for an empty bulk delete).
    Namespaced { namespaces: Vec<String> },
    /// Writes cluster-scoped objects (PVs, StorageClasses, ClusterRoles, webhooks, ...).
    ClusterScoped,
    /// Changes app-local state only: persisted UI/integration settings, the port-forward
    /// registry, the MCP server, or the backend lifecycle.
    Local,
    /// Switches the backend to another kubeconfig context.
    ContextSwitch { context: String },
}

impl MutationTarget {
    fn namespaced(namespace: &str) -> Self {
        Self::Namespaced {
            namespaces: vec![namespace.to_owned()],
        }
    }

    /// Whether the command can write to the cluster (namespaced or cluster-scoped).
    pub fn writes_cluster(&self) -> bool {
        matches!(self, Self::Namespaced { .. } | Self::ClusterScoped)
    }
}

impl BackendCmdV1 {
    /// Classify what this command may change. See [`MutationTarget`].
    #[allow(clippy::too_many_lines)] // one arm per protocol variant, by design
    pub fn mutation_target(&self) -> MutationTarget {
        use BackendCmdV1 as C;
        match self {
            // Reads and transient backend state.
            C::Initialize
            | C::RefreshContexts
            | C::RefreshNamespaces
            | C::SetNamespaceFilter { .. }
            | C::RefreshNodes
            | C::SelectNode { .. }
            | C::RefreshEvents
            | C::RefreshPods
            | C::SelectPod { .. }
            | C::InspectContainerImage { .. }
            | C::StartPodLogs { .. }
            | C::StopPodLogs
            | C::FetchPodMetrics { .. }
            | C::StartPodExec { .. }
            | C::SendPodExecInput { .. }
            | C::ResizePodExec { .. }
            | C::StopPodExec
            | C::RefreshDeployments
            | C::SelectDeployment { .. }
            | C::RefreshDaemonSets
            | C::SelectDaemonSet { .. }
            | C::RefreshStatefulSets
            | C::SelectStatefulSet { .. }
            | C::RefreshReplicaSets
            | C::SelectReplicaSet { .. }
            | C::RefreshReplicationControllers
            | C::SelectReplicationController { .. }
            | C::RefreshJobs
            | C::SelectJob { .. }
            | C::RefreshCronJobs
            | C::SelectCronJob { .. }
            | C::CheckPortAvailability { .. }
            | C::RefreshPortForwards
            | C::RefreshServices
            | C::SelectService { .. }
            | C::RefreshEndpoints
            | C::SelectEndpoint { .. }
            | C::RefreshIngresses
            | C::SelectIngress { .. }
            | C::RefreshIngressClasses
            | C::SelectIngressClass { .. }
            | C::RefreshNetworkPolicies
            | C::SelectNetworkPolicy { .. }
            | C::RefreshIntegrations
            | C::StartMetricsPolling { .. }
            | C::StopMetricsPolling
            | C::StartWorkloadMetricsPolling { .. }
            | C::StopWorkloadMetricsPolling
            | C::TestPrometheusConnection { .. }
            | C::TestGrafanaConnection { .. }
            | C::RefreshFlux
            | C::RefreshHubble
            | C::RefreshMcpStatus
            | C::SimulateSlowCall
            | C::SetActiveSection { .. }
            | C::RefreshPersistentVolumes
            | C::SelectPersistentVolume { .. }
            | C::RefreshPersistentVolumeClaims
            | C::SelectPersistentVolumeClaim { .. }
            | C::RefreshStorageClasses
            | C::SelectStorageClass { .. }
            | C::RefreshRoles
            | C::SelectRole { .. }
            | C::RefreshRoleBindings
            | C::SelectRoleBinding { .. }
            | C::RefreshClusterRoles
            | C::SelectClusterRole { .. }
            | C::RefreshClusterRoleBindings
            | C::SelectClusterRoleBinding { .. }
            | C::RefreshConfigMaps
            | C::SelectConfigMap { .. }
            | C::RefreshSecrets
            | C::SelectSecret { .. }
            | C::RefreshResourceQuotas
            | C::SelectResourceQuota { .. }
            | C::RefreshLimitRanges
            | C::SelectLimitRange { .. }
            | C::RefreshHorizontalPodAutoscalers
            | C::SelectHorizontalPodAutoscaler { .. }
            | C::RefreshPodDisruptionBudgets
            | C::SelectPodDisruptionBudget { .. }
            | C::RefreshPriorityClasses
            | C::SelectPriorityClass { .. }
            | C::RefreshRuntimeClasses
            | C::SelectRuntimeClass { .. }
            | C::RefreshLeases
            | C::SelectLease { .. }
            | C::RefreshMutatingWebhookConfigurations
            | C::SelectMutatingWebhookConfiguration { .. }
            | C::RefreshValidatingWebhookConfigurations
            | C::SelectValidatingWebhookConfiguration { .. }
            | C::LoadUiConfig
            | C::RefreshCrds
            | C::SelectCrdKind { .. }
            | C::RefreshCrdInstances { .. }
            | C::SelectCrdInstance { .. }
            | C::RefreshApplications
            | C::SelectApplication { .. }
            | C::RefreshRelatedResources { .. }
            | C::RefreshHelmReleases
            | C::SelectHelmRelease { .. }
            | C::SelectFluxResource { .. }
            // Agent SDK Ops (013): `AgentAsk` calls an external model provider but
            // writes no cluster/persisted state beyond the in-memory/on-disk
            // conversation (classified `Local` below is where persistence writes
            // live); reviewing a suggestion is a pure status read/change, never a
            // cluster call. See mutation.rs test `agent_commands_never_write_cluster`.
            | C::AgentGetAuthMode
            | C::AgentRevealKey
            | C::AgentOpenConversation { .. }
            | C::AgentAsk { .. }
            | C::AgentReviewSuggestedAction { .. }
            // 014-teams-organizations: reads of local/shared team state only.
            | C::LoadClusterSettings { .. }
            | C::RefreshTeamStoreStatus
            | C::RefreshConnectionProfiles
            // 011-rbac-oidc-administration: self-access checks, namespace-access lookup, and
            // kubeadm ConfigMap detection are all read-only.
            | C::RefreshRbacSelfAccess { .. }
            | C::ResolveNamespaceAccess { .. }
            | C::RefreshKubeadmDetection => MutationTarget::ReadOnly,

            // App-local state.
            C::Shutdown
            | C::StartPortForward { .. }
            | C::StopPortForward { .. }
            | C::DeletePortForward { .. }
            | C::UpdatePortForward { .. }
            | C::ConfigurePrometheus { .. }
            | C::ConfigureGrafana { .. }
            | C::StartMcpServer
            | C::StopMcpServer
            | C::OpenMcpConfigDir
            | C::SaveUiConfig { .. }
            | C::SetMcpAutoStart { .. }
            // 014-teams-organizations: writes `cluster-settings.json`/`team.json`, or
            // the self-hosted shared config store — never the cluster itself.
            | C::SaveClusterSettings { .. }
            | C::SetTeamStorePath { .. }
            | C::PublishConnectionProfile { .. }
            | C::AdoptConnectionProfile { .. }
            | C::RemoveConnectionProfile { .. }
            | C::PublishOrgClusterSettings { .. }
            | C::AcceptOrgClusterSettings { .. }
            | C::DismissOrgClusterSettings { .. }
            // Agent SDK Ops (013): writes `credentials.json`'s `agent` section or
            // `agent_conversations.json` only — never the cluster. See the
            // `agent_commands_never_write_cluster` test below.
            | C::AgentSetAuthMode { .. }
            | C::AgentClearConversation { .. }
            | C::AgentDeclineSuggestedAction { .. }
            | C::AgentRecordExecutedAction { .. } => MutationTarget::Local,

            C::SetContext { context } => MutationTarget::ContextSwitch {
                context: context.clone(),
            },

            // Namespaced cluster writes.
            C::DeletePod { namespace, .. }
            | C::ScaleDeployment { namespace, .. }
            | C::RestartDeployment { namespace, .. }
            | C::DeleteDeployment { namespace, .. }
            | C::RestartDaemonSet { namespace, .. }
            | C::DeleteDaemonSet { namespace, .. }
            | C::ScaleStatefulSet { namespace, .. }
            | C::RestartStatefulSet { namespace, .. }
            | C::DeleteStatefulSet { namespace, .. }
            | C::ScaleReplicaSet { namespace, .. }
            | C::RestartReplicaSet { namespace, .. }
            | C::DeleteReplicaSet { namespace, .. }
            | C::ScaleReplicationController { namespace, .. }
            | C::RestartReplicationController { namespace, .. }
            | C::DeleteReplicationController { namespace, .. }
            | C::DeleteJob { namespace, .. }
            | C::SuspendCronJob { namespace, .. }
            | C::TriggerCronJob { namespace, .. }
            | C::DeleteCronJob { namespace, .. }
            | C::DeleteService { namespace, .. }
            | C::DeleteIngress { namespace, .. }
            | C::DeleteNetworkPolicy { namespace, .. }
            | C::DeletePersistentVolumeClaim { namespace, .. }
            | C::DeleteRole { namespace, .. }
            | C::DeleteRoleBinding { namespace, .. }
            | C::DeleteConfigMap { namespace, .. }
            | C::DeleteSecret { namespace, .. }
            | C::DeleteResourceQuota { namespace, .. }
            | C::DeleteLimitRange { namespace, .. }
            | C::DeleteHorizontalPodAutoscaler { namespace, .. }
            | C::DeletePodDisruptionBudget { namespace, .. }
            | C::DeleteLease { namespace, .. } => MutationTarget::namespaced(namespace),

            C::BulkDelete { items } => {
                let mut namespaces: Vec<String> = items.iter().map(|(ns, _)| ns.clone()).collect();
                namespaces.sort();
                namespaces.dedup();
                MutationTarget::Namespaced { namespaces }
            }

            // Cluster-scoped cluster writes.
            C::DeletePersistentVolume { .. }
            | C::DeleteStorageClass { .. }
            | C::DeleteClusterRole { .. }
            | C::DeleteClusterRoleBinding { .. }
            | C::DeletePriorityClass { .. }
            | C::DeleteRuntimeClass { .. }
            | C::DeleteMutatingWebhookConfiguration { .. }
            | C::DeleteValidatingWebhookConfiguration { .. }
            | C::CordonNode { .. }
            | C::UncordonNode { .. }
            | C::DrainNode { .. }
            | C::CancelDrainNode { .. }
            | C::TaintNode { .. } => MutationTarget::ClusterScoped,

            // Full-object apply: scope follows the target object's own scope.
            C::ApplyResourceYaml { namespace, .. } => match namespace {
                Some(ns) => MutationTarget::namespaced(ns),
                None => MutationTarget::ClusterScoped,
            },

            // Create-only sibling of ApplyResourceYaml (011-rbac-oidc-administration):
            // scope follows the target object's own scope, same as apply.
            C::CreateResourceYaml { namespace, .. } => match namespace {
                Some(ns) => MutationTarget::namespaced(ns),
                None => MutationTarget::ClusterScoped,
            },

            // Helm releases (mutate): all namespaced release-storage writes.
            C::PreviewHelmRollback { namespace, .. }
            | C::ApplyHelmRollback { namespace, .. }
            | C::PreviewHelmUpgrade { namespace, .. }
            | C::ApplyHelmUpgrade { namespace, .. }
            | C::UninstallHelmRelease { namespace, .. } => MutationTarget::namespaced(namespace),
        }
    }

    /// The UI section this command belongs to, if any (`None` = global/system).
    #[allow(clippy::too_many_lines)] // one arm per protocol variant, by design
    pub fn section(&self) -> Option<SectionKey> {
        use BackendCmdV1 as C;
        let key = match self {
            C::Initialize
            | C::Shutdown
            | C::RefreshContexts
            | C::SetContext { .. }
            | C::SetNamespaceFilter { .. }
            | C::SimulateSlowCall
            | C::SetActiveSection { .. }
            | C::LoadUiConfig
            | C::SaveUiConfig { .. }
            | C::SetMcpAutoStart { .. }
            | C::ApplyResourceYaml { .. }
            // Agent SDK Ops (013): not tied to one existing UI section (a conversation
            // is scoped to an arbitrary object ref, not a section key) — global, like
            // `ApplyResourceYaml`.
            | C::AgentGetAuthMode
            | C::AgentSetAuthMode { .. }
            | C::AgentRevealKey
            | C::AgentOpenConversation { .. }
            | C::AgentAsk { .. }
            | C::AgentClearConversation { .. }
            | C::AgentReviewSuggestedAction { .. }
            | C::AgentDeclineSuggestedAction { .. }
            | C::AgentRecordExecutedAction { .. }
            // 011-rbac-oidc-administration: create-pathway apply, likewise global.
            | C::CreateResourceYaml { .. } => return None,
            C::RefreshNamespaces => section_keys::namespaces(),
            C::RefreshNodes
            | C::SelectNode { .. }
            | C::CordonNode { .. }
            | C::UncordonNode { .. }
            | C::DrainNode { .. }
            | C::CancelDrainNode { .. }
            | C::TaintNode { .. } => section_keys::nodes(),
            C::RefreshEvents => section_keys::events(),
            C::RefreshPods
            | C::SelectPod { .. }
            | C::DeletePod { .. }
            | C::InspectContainerImage { .. }
            | C::StartPodLogs { .. }
            | C::StopPodLogs
            | C::FetchPodMetrics { .. }
            | C::StartPodExec { .. }
            | C::SendPodExecInput { .. }
            | C::ResizePodExec { .. }
            | C::StopPodExec
            | C::StartMetricsPolling { .. }
            | C::StopMetricsPolling
            | C::BulkDelete { .. } => section_keys::workloads_pods(),
            C::RefreshDeployments
            | C::SelectDeployment { .. }
            | C::ScaleDeployment { .. }
            | C::RestartDeployment { .. }
            | C::DeleteDeployment { .. }
            | C::StartWorkloadMetricsPolling { .. }
            | C::StopWorkloadMetricsPolling => section_keys::workloads_deployments(),
            C::RefreshDaemonSets
            | C::SelectDaemonSet { .. }
            | C::RestartDaemonSet { .. }
            | C::DeleteDaemonSet { .. } => section_keys::workloads_daemonsets(),
            C::RefreshStatefulSets
            | C::SelectStatefulSet { .. }
            | C::ScaleStatefulSet { .. }
            | C::RestartStatefulSet { .. }
            | C::DeleteStatefulSet { .. } => section_keys::workloads_statefulsets(),
            C::RefreshReplicaSets
            | C::SelectReplicaSet { .. }
            | C::ScaleReplicaSet { .. }
            | C::RestartReplicaSet { .. }
            | C::DeleteReplicaSet { .. } => section_keys::workloads_replicasets(),
            C::RefreshReplicationControllers
            | C::SelectReplicationController { .. }
            | C::ScaleReplicationController { .. }
            | C::RestartReplicationController { .. }
            | C::DeleteReplicationController { .. } => {
                section_keys::workloads_replicationcontrollers()
            }
            C::RefreshJobs | C::SelectJob { .. } | C::DeleteJob { .. } => {
                section_keys::workloads_jobs()
            }
            C::RefreshCronJobs
            | C::SelectCronJob { .. }
            | C::SuspendCronJob { .. }
            | C::TriggerCronJob { .. }
            | C::DeleteCronJob { .. } => section_keys::workloads_cronjobs(),
            C::CheckPortAvailability { .. }
            | C::StartPortForward { .. }
            | C::StopPortForward { .. }
            | C::DeletePortForward { .. }
            | C::UpdatePortForward { .. }
            | C::RefreshPortForwards => section_keys::network_portforwards(),
            C::RefreshServices | C::SelectService { .. } | C::DeleteService { .. } => {
                section_keys::network_services()
            }
            C::RefreshEndpoints | C::SelectEndpoint { .. } => section_keys::network_endpoints(),
            C::RefreshIngresses | C::SelectIngress { .. } | C::DeleteIngress { .. } => {
                section_keys::network_ingress()
            }
            C::RefreshIngressClasses | C::SelectIngressClass { .. } => {
                section_keys::network_ingressclasses()
            }
            C::RefreshNetworkPolicies
            | C::SelectNetworkPolicy { .. }
            | C::DeleteNetworkPolicy { .. } => section_keys::network_policies(),
            C::RefreshIntegrations
            | C::ConfigurePrometheus { .. }
            | C::TestPrometheusConnection { .. } => section_keys::integrations_metrics(),
            C::LoadClusterSettings { .. }
            | C::SaveClusterSettings { .. }
            | C::PublishOrgClusterSettings { .. }
            | C::AcceptOrgClusterSettings { .. }
            | C::DismissOrgClusterSettings { .. } => section_keys::cluster_settings(),
            C::SetTeamStorePath { .. }
            | C::RefreshTeamStoreStatus
            | C::RefreshConnectionProfiles
            | C::PublishConnectionProfile { .. }
            | C::AdoptConnectionProfile { .. }
            | C::RemoveConnectionProfile { .. } => section_keys::team_profiles(),
            C::ConfigureGrafana { .. } | C::TestGrafanaConnection { .. } => {
                section_keys::integrations_grafana()
            }
            C::RefreshFlux => section_keys::integrations_flux(),
            C::RefreshHubble => section_keys::integrations_hubble(),
            C::StartMcpServer | C::StopMcpServer | C::RefreshMcpStatus | C::OpenMcpConfigDir => {
                section_keys::integrations_mcp()
            }
            C::RefreshPersistentVolumes
            | C::SelectPersistentVolume { .. }
            | C::DeletePersistentVolume { .. } => section_keys::storage_pv(),
            C::RefreshPersistentVolumeClaims
            | C::SelectPersistentVolumeClaim { .. }
            | C::DeletePersistentVolumeClaim { .. } => section_keys::storage_pvc(),
            C::RefreshStorageClasses
            | C::SelectStorageClass { .. }
            | C::DeleteStorageClass { .. } => section_keys::storage_storageclasses(),
            C::RefreshRoles | C::SelectRole { .. } | C::DeleteRole { .. } => {
                section_keys::access_roles()
            }
            C::RefreshRoleBindings | C::SelectRoleBinding { .. } | C::DeleteRoleBinding { .. } => {
                section_keys::access_rolebindings()
            }
            C::RefreshClusterRoles | C::SelectClusterRole { .. } | C::DeleteClusterRole { .. } => {
                section_keys::access_clusterroles()
            }
            C::RefreshClusterRoleBindings
            | C::SelectClusterRoleBinding { .. }
            | C::DeleteClusterRoleBinding { .. } => section_keys::access_clusterrolebindings(),
            // Gates whichever binding kind the caller is checking; the section is advisory
            // (used for correlation/routing only, not for scoping the review itself).
            C::RefreshRbacSelfAccess { .. } => section_keys::access_rolebindings(),
            C::ResolveNamespaceAccess { .. } => section_keys::access_namespace_access(),
            C::RefreshKubeadmDetection => section_keys::access_oidc_config(),
            C::RefreshConfigMaps | C::SelectConfigMap { .. } | C::DeleteConfigMap { .. } => {
                section_keys::config_configmaps()
            }
            C::RefreshSecrets | C::SelectSecret { .. } | C::DeleteSecret { .. } => {
                section_keys::config_secrets()
            }
            C::RefreshResourceQuotas
            | C::SelectResourceQuota { .. }
            | C::DeleteResourceQuota { .. } => section_keys::config_resourcequotas(),
            C::RefreshLimitRanges | C::SelectLimitRange { .. } | C::DeleteLimitRange { .. } => {
                section_keys::config_limitranges()
            }
            C::RefreshHorizontalPodAutoscalers
            | C::SelectHorizontalPodAutoscaler { .. }
            | C::DeleteHorizontalPodAutoscaler { .. } => {
                section_keys::config_horizontalpodautoscalers()
            }
            C::RefreshPodDisruptionBudgets
            | C::SelectPodDisruptionBudget { .. }
            | C::DeletePodDisruptionBudget { .. } => section_keys::config_poddisruptionbudgets(),
            C::RefreshPriorityClasses
            | C::SelectPriorityClass { .. }
            | C::DeletePriorityClass { .. } => section_keys::config_priorityclasses(),
            C::RefreshRuntimeClasses
            | C::SelectRuntimeClass { .. }
            | C::DeleteRuntimeClass { .. } => section_keys::config_runtimeclasses(),
            C::RefreshLeases | C::SelectLease { .. } | C::DeleteLease { .. } => {
                section_keys::config_leases()
            }
            C::RefreshMutatingWebhookConfigurations
            | C::SelectMutatingWebhookConfiguration { .. }
            | C::DeleteMutatingWebhookConfiguration { .. } => {
                section_keys::config_mutatingwebhooks()
            }
            C::RefreshValidatingWebhookConfigurations
            | C::SelectValidatingWebhookConfiguration { .. }
            | C::DeleteValidatingWebhookConfiguration { .. } => {
                section_keys::config_validatingwebhooks()
            }
            C::RefreshCrds
            | C::SelectCrdKind { .. }
            | C::RefreshCrdInstances { .. }
            | C::SelectCrdInstance { .. } => section_keys::custom_resources(),
            C::RefreshApplications | C::SelectApplication { .. } => section_keys::applications(),
            C::RefreshRelatedResources { section, .. } => section.clone(),
            C::RefreshHelmReleases
            | C::SelectHelmRelease { .. }
            | C::PreviewHelmRollback { .. }
            | C::ApplyHelmRollback { .. }
            | C::PreviewHelmUpgrade { .. }
            | C::ApplyHelmUpgrade { .. }
            | C::UninstallHelmRelease { .. } => section_keys::helm_releases(),
            C::SelectFluxResource { .. } => section_keys::flux_resource(),
        };
        Some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{SectionKey, UiConfig};

    fn nsn(namespace: &str) -> MutationTarget {
        MutationTarget::Namespaced {
            namespaces: vec![namespace.to_owned()],
        }
    }

    fn s(v: &str) -> String {
        v.to_owned()
    }

    #[test]
    fn read_only_sample() {
        for cmd in [
            BackendCmdV1::Initialize,
            BackendCmdV1::RefreshPods,
            BackendCmdV1::SelectDeployment {
                namespace: s("kube-system"),
                name: s("coredns"),
            },
            BackendCmdV1::StartPodLogs {
                namespace: s("a"),
                name: s("b"),
                container: None,
            },
            BackendCmdV1::SetNamespaceFilter {
                namespaces: vec![s("a")],
            },
            BackendCmdV1::SelectClusterRole { name: s("admin") },
            BackendCmdV1::LoadUiConfig,
            BackendCmdV1::SetActiveSection {
                section: SectionKey::new("workloads/pods"),
            },
        ] {
            assert_eq!(cmd.mutation_target(), MutationTarget::ReadOnly, "{cmd:?}");
            assert!(!cmd.mutation_target().writes_cluster());
        }
    }

    #[test]
    fn local_sample() {
        for cmd in [
            BackendCmdV1::Shutdown,
            BackendCmdV1::SaveUiConfig {
                ui: UiConfig::default(),
                expanded_groups: vec![],
            },
            BackendCmdV1::StartPortForward {
                namespace: s("a"),
                pod: s("p"),
                local_port: 8080,
                remote_port: 80,
                container: None,
            },
            BackendCmdV1::DeletePortForward { tunnel_id: s("t") },
            BackendCmdV1::ConfigureGrafana {
                base_url: s("http://g"),
                api_key: s("k"),
            },
            BackendCmdV1::StartMcpServer,
            BackendCmdV1::SetMcpAutoStart { enabled: true },
        ] {
            assert_eq!(cmd.mutation_target(), MutationTarget::Local, "{cmd:?}");
        }
    }

    #[test]
    fn context_switch() {
        let cmd = BackendCmdV1::SetContext {
            context: s("kind-a"),
        };
        assert_eq!(
            cmd.mutation_target(),
            MutationTarget::ContextSwitch {
                context: s("kind-a")
            }
        );
    }

    #[test]
    fn namespaced_sample() {
        let cases = [
            BackendCmdV1::DeletePod {
                namespace: s("fathom-dev"),
                name: s("p"),
            },
            BackendCmdV1::ScaleDeployment {
                namespace: s("fathom-dev"),
                name: s("d"),
                replicas: 3,
            },
            BackendCmdV1::RestartStatefulSet {
                namespace: s("fathom-dev"),
                name: s("kv"),
            },
            BackendCmdV1::TriggerCronJob {
                namespace: s("fathom-dev"),
                name: s("c"),
            },
            BackendCmdV1::DeleteSecret {
                namespace: s("fathom-dev"),
                name: s("x"),
            },
            BackendCmdV1::DeleteLease {
                namespace: s("fathom-dev"),
                name: s("l"),
            },
        ];
        for cmd in cases {
            assert_eq!(cmd.mutation_target(), nsn("fathom-dev"), "{cmd:?}");
            assert!(cmd.mutation_target().writes_cluster());
        }
    }

    #[test]
    fn bulk_delete_collects_every_namespace() {
        let cmd = BackendCmdV1::BulkDelete {
            items: vec![
                (s("fathom-dev"), s("a")),
                (s("kube-system"), s("b")),
                (s("fathom-dev"), s("c")),
            ],
        };
        assert_eq!(
            cmd.mutation_target(),
            MutationTarget::Namespaced {
                namespaces: vec![s("fathom-dev"), s("kube-system")]
            }
        );
        let empty = BackendCmdV1::BulkDelete { items: vec![] };
        assert_eq!(
            empty.mutation_target(),
            MutationTarget::Namespaced { namespaces: vec![] }
        );
    }

    #[test]
    fn cluster_scoped_sample() {
        for cmd in [
            BackendCmdV1::DeletePersistentVolume { name: s("pv") },
            BackendCmdV1::DeleteStorageClass { name: s("sc") },
            BackendCmdV1::DeleteClusterRole { name: s("cr") },
            BackendCmdV1::DeleteClusterRoleBinding { name: s("crb") },
            BackendCmdV1::DeleteValidatingWebhookConfiguration { name: s("w") },
        ] {
            assert_eq!(
                cmd.mutation_target(),
                MutationTarget::ClusterScoped,
                "{cmd:?}"
            );
            assert!(cmd.mutation_target().writes_cluster());
        }
    }

    #[test]
    fn sections() {
        assert_eq!(BackendCmdV1::Initialize.section(), None);
        assert_eq!(
            BackendCmdV1::ScaleDeployment {
                namespace: s("a"),
                name: s("b"),
                replicas: 1
            }
            .section(),
            Some(section_keys::workloads_deployments())
        );
        assert_eq!(
            BackendCmdV1::BulkDelete { items: vec![] }.section(),
            Some(section_keys::workloads_pods())
        );
    }

    /// Type-level proof backing SC-002: no `Agent*` command can write to the cluster.
    /// Every `Agent*` variant MUST classify as `ReadOnly` or `Local`; if a future
    /// `Agent*` variant is added without an arm here, this test — not just the
    /// exhaustive match above — will still catch a `Namespaced`/`ClusterScoped`
    /// misclassification for any variant added to the sample list below.
    #[test]
    fn agent_commands_never_write_cluster() {
        fn object() -> AgentObjectRef {
            AgentObjectRef {
                kind: s("Pod"),
                namespace: s("default"),
                name: s("api-0"),
            }
        }

        for cmd in [
            BackendCmdV1::AgentGetAuthMode,
            BackendCmdV1::AgentSetAuthMode {
                mode: AgentAuthModeInput::Disabled,
            },
            BackendCmdV1::AgentRevealKey,
            BackendCmdV1::AgentOpenConversation { object: object() },
            BackendCmdV1::AgentAsk {
                object: object(),
                question: s("what's wrong?"),
            },
            BackendCmdV1::AgentClearConversation { object: object() },
            BackendCmdV1::AgentReviewSuggestedAction {
                object: object(),
                suggestion_id: s("sugg-1"),
            },
            BackendCmdV1::AgentDeclineSuggestedAction {
                object: object(),
                suggestion_id: s("sugg-1"),
            },
            BackendCmdV1::AgentRecordExecutedAction {
                object: object(),
                suggestion_id: s("sugg-1"),
                command_summary: s("RestartDeployment default/api"),
                outcome: ActionOutcome::Success,
            },
        ] {
            let target = cmd.mutation_target();
            assert!(
                matches!(target, MutationTarget::ReadOnly | MutationTarget::Local),
                "Agent* command must never write the cluster, got {target:?} for {cmd:?}"
            );
            assert!(!target.writes_cluster(), "{cmd:?}");
        }
    }
}
