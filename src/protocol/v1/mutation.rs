//! What a command may change, used as a guard by shells that restrict writes (e.g. the
//! `fathom-devbridge` sandbox guard) and to route `BackendEventV1::CommandFinished`.
//!
//! Both matches below are exhaustive with no wildcard arm on purpose: adding a
//! `BackendCmdV1` variant must force a decision here at compile time.

#[cfg(test)]
use super::{ActionOutcome, AgentAuthModeInput, AgentObjectRef};
use super::{AuditActionKind, AuditActionSummary, BackendCmdV1, SectionKey, section_keys};

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
            | C::StartNodeMetricsPolling { .. }
            | C::StopNodeMetricsPolling
            | C::SelectNode { .. }
            | C::SelectNamespace { .. }
            | C::RefreshEvents
            | C::RefreshPods
            | C::SelectPod { .. }
            | C::InspectContainerImage { .. }
            | C::InspectPodVulnerabilities { .. }
            | C::StartPodLogs { .. }
            | C::StopPodLogs
            | C::FetchPodMetrics { .. }
            | C::SendPodExecInput { .. }
            | C::ResizePodExec { .. }
            | C::StopPodExec
            // 037-node-shell: access review, orphan listing and settings read.
            | C::CheckNodeShellAccess { .. }
            | C::ListNodeShellOrphans { .. }
            | C::GetNodeShellSettings
            // 024-pod-exec-terminal: the proactive `pods/exec` permission pre-check is a
            // read-only `SelfSubjectAccessReview`, shown before the exec confirmation.
            | C::CheckPodExecAccess { .. }
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
            | C::GetObservabilitySettings
            | C::RefreshFlux
            | C::RefreshVelero
            | C::RefreshTrivyOverview
            | C::RefreshHubble
            | C::RefreshGatekeeper
            | C::RefreshCertManager
            | C::SelectCertificateChain { .. }
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
            | C::RefreshVerticalPodAutoscalers
            | C::SelectVerticalPodAutoscaler { .. }
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
            | C::RefreshKnative
            | C::RefreshKeda
            | C::SelectScaledObject { .. }
            | C::SelectScaledJob { .. }
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
            // 038-sdk-login-ollama: status read and the Ollama probe (a GET to the
            // operator's Ollama endpoint) never write anything.
            | C::AiGetStatus
            | C::AiOllamaProbe { .. }
            // 014-teams-organizations: reads of local/shared team state only.
            | C::LoadClusterSettings { .. }
            | C::RefreshTeamStoreStatus
            | C::RefreshConnectionProfiles
            // 033-rauthy-oidc-teams-users: identity/directory/capability reads and YAML
            // rendering only; the actual RBAC write is `ApplyResourceYaml`.
            | C::LoadOidcConfig
            | C::IdpRefreshDirectory { .. }
            | C::ResolveCapabilities { .. }
            | C::CheckCapability { .. }
            | C::PreviewRoleAssignment { .. }
            | C::RefreshOrphanedSubjects { .. }
            // 015-mcp-agent-exposure: reads the persisted toggle and the derived connection
            // snippet only; never the cluster, never a write.
            | C::LoadMcpServerStatus
            // 030-kind-cluster-provisioning: host tool/runtime probes and `kind get clusters`.
            | C::RefreshKindPrerequisites
            | C::RefreshKindClusters
            // 011-rbac-oidc-administration: self-access checks, namespace-access lookup, and
            // kubeadm ConfigMap detection are all read-only.
            | C::RefreshRbacSelfAccess { .. }
            // 018-rbac-aware-mutation-ui: discovery + SelfSubjectAccessReview, read-only.
            | C::CheckResourceMutationAccess { .. }
            | C::ResolveNamespaceAccess { .. }
            | C::RefreshKubeadmDetection
            // 026-mutation-audit-log-viewer: reads the persisted, bounded local audit log
            // only; never mutates anything.
            | C::RefreshAuditLog => MutationTarget::ReadOnly,

            // App-local state.
            C::Shutdown
            | C::StartPortForward { .. }
            | C::StartServicePortForward { .. }
            | C::StopPortForward { .. }
            | C::DeletePortForward { .. }
            | C::UpdatePortForward { .. }
            | C::ConfigurePrometheus { .. }
            | C::ConfigureGrafana { .. }
            | C::ClearGrafanaKey
            | C::SaveUiConfig { .. }
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
            // 033-rauthy-oidc-teams-users: `config.json` `oidc`, the OS keyring, or the team
            // store — never the cluster.
            | C::OidcSaveIssuer { .. }
            | C::OidcDeleteIssuer { .. }
            | C::OidcSetRoleMap { .. }
            | C::OidcSignIn { .. }
            | C::OidcCancelSignIn { .. }
            | C::OidcSignOut { .. }
            | C::SetContextAuthMode { .. }
            | C::IdpSetAdminKey { .. }
            | C::LinkTeamProfile { .. }
            | C::UnlinkTeamProfile { .. }
            // Agent SDK Ops (013): writes `credentials.json`'s `agent` section or
            // `agent_conversations.json` only — never the cluster. See the
            // `agent_commands_never_write_cluster` test below.
            | C::AgentSetAuthMode { .. }
            | C::AgentClearConversation { .. }
            | C::AgentDeclineSuggestedAction { .. }
            | C::AgentRecordExecutedAction { .. }
            // 038-sdk-login-ollama: OS credential store, `config.json`'s `ai_assist`
            // section and (opt-in) the 0600 credentials file — never the cluster.
            | C::AiProviderLoginStart { .. }
            | C::AiProviderSubmitApiKey { .. }
            | C::AiProviderLoginCancel
            | C::AiProviderSignOut { .. }
            | C::AiSetActiveProvider { .. }
            | C::AiSetOfflineMode { .. }
            | C::AiOllamaSelectModel { .. }
            | C::AiSetOllamaEndpoint { .. }
            | C::AiEnableFileStore { .. }
            // 015-mcp-agent-exposure: writes `config.json`'s `auto_start.mcp_stdio` and
            // starts/stops the local stdio server task — never the cluster itself. (The MCP
            // server's own tools re-enter the backend's existing guarded mutation paths.)
            | C::SetMcpServerEnabled { .. }
            // 030-kind-cluster-provisioning: creates/deletes local kind clusters and merges
            // their contexts into the kubeconfig; never touches an existing cluster's objects.
            | C::CreateKindCluster { .. }
            | C::CancelCreateKindCluster { .. }
            | C::DeleteKindCluster { .. }
            // 015-mcp-agent-exposure US4: the demo chat is a subprocess on the operator's own
            // machine (`claude` from `PATH`) talking to Fathom's stdio MCP server; nothing here
            // reaches the cluster except through that server's own guarded tools.
            | C::StartMcpDemoChat
            | C::StopMcpDemoChat
            | C::SendMcpDemoChatInput { .. }
            | C::ResizeMcpDemoChat { .. }
            // 027-fleet-rollup-dashboard: view-only polling, never mutates a cluster.
            | C::StartFleetPolling
            | C::StopFleetPolling
            | C::FederateContext { .. }
            | C::UnfederateContext { .. }
            | C::RequestFederationSnapshot
            | C::SetDaemonPreference { .. }
            | C::RefreshDaemon { .. }
            | C::RequestDaemonTimeline { .. }
            | C::RequestDaemonTopology { .. }
            | C::RequestDaemonMetrics { .. }
            // 037-node-shell: persists per-context settings only.
            | C::SetNodeShellSettings { .. }
            // 035-cloud-auth-plugins: persists a per-context exec policy only.
            | C::SetExecPolicy { .. } => MutationTarget::Local,

            C::SetContext { context } => MutationTarget::ContextSwitch {
                context: context.clone(),
            },

            // Namespaced cluster writes.
            C::AddEphemeralContainer { namespace, .. }
            //
            // 024-pod-exec-terminal: `StartPodExec` is not a CRUD mutation, but opening an
            // interactive shell into a container is effectively RCE-into-pod, so it is
            // classified here on purpose — this is the enforceable gate (e.g.
            // `tools/devbridge/src/guard.rs`'s sandbox guard treats it exactly like any
            // other namespaced write). `SendPodExecInput`/`ResizePodExec`/`StopPodExec` stay
            // `ReadOnly` above: only the act of opening a session is gated.
            | C::StartPodExec { namespace, .. }
            | C::DeletePod { namespace, .. }
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
            | C::DeleteVerticalPodAutoscaler { namespace, .. }
            | C::DeletePodDisruptionBudget { namespace, .. }
            | C::DeleteLease { namespace, .. } => MutationTarget::namespaced(namespace),

            // 037-node-shell: creates / deletes a privileged pod in the given namespace.
            C::StartNodeShell { namespace, .. } | C::StopNodeShell { namespace, .. } => {
                MutationTarget::namespaced(namespace)
            }
            C::DeleteNodeShellOrphans { pods } => {
                let mut namespaces: Vec<String> =
                    pods.iter().map(|p| p.namespace.clone()).collect();
                namespaces.sort();
                namespaces.dedup();
                MutationTarget::Namespaced { namespaces }
            }

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
            // 030-kind-cluster-provisioning: host-level, not tied to a UI section.
            | C::RefreshKindPrerequisites
            | C::RefreshKindClusters
            | C::CreateKindCluster { .. }
            | C::CancelCreateKindCluster { .. }
            | C::DeleteKindCluster { .. }
            | C::RefreshContexts
            | C::SetContext { .. }
            | C::SetNamespaceFilter { .. }
            | C::SimulateSlowCall
            | C::SetActiveSection { .. }
            | C::LoadUiConfig
            | C::StartFleetPolling
            | C::StopFleetPolling
            | C::FederateContext { .. }
            | C::UnfederateContext { .. }
            | C::RequestFederationSnapshot
            | C::SetDaemonPreference { .. }
            | C::RefreshDaemon { .. }
            | C::RequestDaemonTimeline { .. }
            | C::RequestDaemonTopology { .. }
            | C::RequestDaemonMetrics { .. }
            | C::SaveUiConfig { .. }
            | C::ApplyResourceYaml { .. }
            // Agent SDK Ops (013): not tied to one existing UI section (a conversation
            // is scoped to an arbitrary object ref, not a section key) — global, like
            // `ApplyResourceYaml`.
            | C::AiProviderLoginStart { .. }
            | C::AiProviderSubmitApiKey { .. }
            | C::AiProviderLoginCancel
            | C::AiProviderSignOut { .. }
            | C::AiSetActiveProvider { .. }
            | C::AiSetOfflineMode { .. }
            | C::AiOllamaProbe { .. }
            | C::AiOllamaSelectModel { .. }
            | C::AiSetOllamaEndpoint { .. }
            | C::AiEnableFileStore { .. }
            | C::AiGetStatus
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
            | C::CreateResourceYaml { .. }
            // 033-rauthy-oidc-teams-users: identity is global (per issuer, not per section);
            // capability and role-assignment routing is by the requesting context.
            | C::LoadOidcConfig
            | C::OidcSaveIssuer { .. }
            | C::OidcDeleteIssuer { .. }
            | C::OidcSetRoleMap { .. }
            | C::OidcSignIn { .. }
            | C::OidcCancelSignIn { .. }
            | C::OidcSignOut { .. }
            | C::SetContextAuthMode { .. }
            | C::IdpSetAdminKey { .. }
            | C::IdpRefreshDirectory { .. }
            | C::ResolveCapabilities { .. }
            | C::CheckCapability { .. }
            | C::PreviewRoleAssignment { .. }
            | C::RefreshOrphanedSubjects { .. } => return None,
            C::RefreshNamespaces | C::SelectNamespace { .. } => section_keys::namespaces(),
            C::RefreshNodes
            | C::StartNodeMetricsPolling { .. }
            | C::StopNodeMetricsPolling
            | C::SelectNode { .. }
            | C::CordonNode { .. }
            | C::UncordonNode { .. }
            | C::DrainNode { .. }
            | C::CancelDrainNode { .. }
            | C::TaintNode { .. } => section_keys::nodes(),
            C::RefreshEvents => section_keys::events(),
            // 037-node-shell: launched from the Node inspector.
            C::CheckNodeShellAccess { .. }
            | C::StartNodeShell { .. }
            | C::StopNodeShell { .. }
            | C::ListNodeShellOrphans { .. }
            | C::DeleteNodeShellOrphans { .. }
            | C::GetNodeShellSettings
            | C::SetNodeShellSettings { .. } => section_keys::nodes(),
            C::SetExecPolicy { .. } => section_keys::system(),
            C::RefreshPods
            | C::SelectPod { .. }
            | C::DeletePod { .. }
            | C::AddEphemeralContainer { .. }
            | C::InspectContainerImage { .. }
            | C::InspectPodVulnerabilities { .. }
            | C::StartPodLogs { .. }
            | C::StopPodLogs
            | C::FetchPodMetrics { .. }
            | C::StartPodExec { .. }
            | C::SendPodExecInput { .. }
            | C::ResizePodExec { .. }
            | C::StopPodExec
            | C::CheckPodExecAccess { .. }
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
            | C::StartServicePortForward { .. }
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
            | C::DismissOrgClusterSettings { .. }
            // 015-mcp-agent-exposure: the "Available Agents" section lives inside Cluster
            // Settings, so its commands route to the same section key.
            | C::LoadMcpServerStatus
            | C::SetMcpServerEnabled { .. }
            | C::StartMcpDemoChat
            | C::StopMcpDemoChat
            | C::SendMcpDemoChatInput { .. }
            | C::ResizeMcpDemoChat { .. } => section_keys::cluster_settings(),
            C::SetTeamStorePath { .. }
            | C::RefreshTeamStoreStatus
            | C::RefreshConnectionProfiles
            | C::PublishConnectionProfile { .. }
            | C::AdoptConnectionProfile { .. }
            | C::RemoveConnectionProfile { .. }
            | C::LinkTeamProfile { .. }
            | C::UnlinkTeamProfile { .. } => section_keys::team_profiles(),
            C::ConfigureGrafana { .. }
            | C::ClearGrafanaKey
            | C::GetObservabilitySettings
            | C::TestGrafanaConnection { .. } => {
                section_keys::integrations_grafana()
            }
            C::RefreshFlux => section_keys::integrations_flux(),
            C::RefreshKnative => section_keys::integrations_knative(),
            C::RefreshVelero => section_keys::integrations_velero(),
            C::RefreshTrivyOverview => section_keys::integrations_vulnerabilities(),
            C::RefreshHubble => section_keys::integrations_hubble(),
            C::RefreshGatekeeper => section_keys::integrations_gatekeeper(),
            C::RefreshCertManager => section_keys::integrations_cert_manager(),
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
            C::RefreshRbacSelfAccess { .. } | C::CheckResourceMutationAccess { .. } => {
                section_keys::access_rolebindings()
            }
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
            C::RefreshVerticalPodAutoscalers
            | C::SelectVerticalPodAutoscaler { .. }
            | C::DeleteVerticalPodAutoscaler { .. } => {
                section_keys::config_verticalpodautoscalers()
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
            C::SelectCertificateChain { .. } => section_keys::certificate_chain(),
            C::RefreshAuditLog => section_keys::audit_log(),
            C::RefreshKeda | C::SelectScaledObject { .. } | C::SelectScaledJob { .. } => {
                section_keys::autoscalers()
            }
        };
        Some(key)
    }

    /// Classify a mutating command for the local audit log (026-mutation-audit-log-viewer).
    /// `None` for every `ReadOnly`/`ContextSwitch` command and for pure session/UI mechanics
    /// that carry no meaningful target (backend shutdown, the demo-chat subprocess
    /// lifecycle) — see `research.md` R1. Exhaustive with no wildcard arm, same discipline as
    /// [`Self::mutation_target`].
    #[allow(clippy::too_many_lines)] // one arm per protocol variant, by design
    pub fn audit_summary(&self) -> Option<AuditActionSummary> {
        use BackendCmdV1 as C;

        fn summary(
            kind: AuditActionKind,
            target_kind: Option<&str>,
            target_namespace: Option<&str>,
            target_name: Option<&str>,
        ) -> Option<AuditActionSummary> {
            Some(AuditActionSummary {
                kind,
                target_kind: target_kind.map(str::to_owned),
                target_namespace: target_namespace.map(str::to_owned),
                target_name: target_name.map(str::to_owned),
            })
        }

        match self {
            // Reads, transient backend state, and context switches: nothing to audit.
            C::Initialize
            // 033-038 identity, federation, node-shell, AI-login and daemon commands have no
            // audit kind of their own yet: not audited.
            | C::CheckNodeShellAccess { .. }
            | C::StartNodeShell { .. }
            | C::StopNodeShell { .. }
            | C::ListNodeShellOrphans { .. }
            | C::DeleteNodeShellOrphans { .. }
            | C::GetNodeShellSettings
            | C::SetNodeShellSettings { .. }
            | C::SetExecPolicy { .. }
            | C::LoadOidcConfig
            | C::OidcSaveIssuer { .. }
            | C::OidcDeleteIssuer { .. }
            | C::OidcSetRoleMap { .. }
            | C::OidcSignIn { .. }
            | C::OidcCancelSignIn { .. }
            | C::OidcSignOut { .. }
            | C::SetContextAuthMode { .. }
            | C::IdpSetAdminKey { .. }
            | C::IdpRefreshDirectory { .. }
            | C::ResolveCapabilities { .. }
            | C::CheckCapability { .. }
            | C::LinkTeamProfile { .. }
            | C::UnlinkTeamProfile { .. }
            | C::PreviewRoleAssignment { .. }
            | C::RefreshOrphanedSubjects { .. }
            | C::AiProviderLoginStart { .. }
            | C::AiProviderSubmitApiKey { .. }
            | C::AiProviderLoginCancel
            | C::AiProviderSignOut { .. }
            | C::AiSetActiveProvider { .. }
            | C::AiSetOfflineMode { .. }
            | C::AiOllamaProbe { .. }
            | C::AiOllamaSelectModel { .. }
            | C::AiSetOllamaEndpoint { .. }
            | C::AiEnableFileStore { .. }
            | C::AiGetStatus
            | C::FederateContext { .. }
            | C::UnfederateContext { .. }
            | C::RequestFederationSnapshot
            | C::SetDaemonPreference { .. }
            | C::RefreshDaemon { .. }
            | C::RequestDaemonTimeline { .. }
            | C::RequestDaemonTopology { .. }
            | C::RequestDaemonMetrics { .. }
            | C::RefreshContexts
            | C::RefreshNamespaces
            | C::SetNamespaceFilter { .. }
            | C::RefreshNodes
            | C::StartNodeMetricsPolling { .. }
            | C::StopNodeMetricsPolling
            | C::SelectNode { .. }
            | C::SelectNamespace { .. }
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
            | C::GetObservabilitySettings
            | C::RefreshFlux
            | C::RefreshKnative
            | C::RefreshHubble
            | C::RefreshKindPrerequisites
            | C::RefreshKindClusters
            | C::CancelCreateKindCluster { .. }
            | C::StartFleetPolling
            | C::StopFleetPolling
            | C::CheckPodExecAccess { .. }
            | C::CheckResourceMutationAccess { .. }
            | C::RefreshKeda
            | C::SelectScaledObject { .. }
            | C::SelectScaledJob { .. }
            | C::RefreshGatekeeper
            | C::RefreshVelero
            | C::RefreshCertManager
            | C::RefreshTrivyOverview
            | C::InspectPodVulnerabilities { .. }
            | C::SelectCertificateChain { .. }
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
            | C::RefreshVerticalPodAutoscalers
            | C::SelectVerticalPodAutoscaler { .. }
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
            | C::AgentGetAuthMode
            | C::AgentRevealKey
            | C::AgentOpenConversation { .. }
            | C::AgentAsk { .. }
            | C::AgentReviewSuggestedAction { .. }
            | C::LoadClusterSettings { .. }
            | C::RefreshTeamStoreStatus
            | C::RefreshConnectionProfiles
            | C::LoadMcpServerStatus
            | C::RefreshRbacSelfAccess { .. }
            | C::ResolveNamespaceAccess { .. }
            | C::RefreshKubeadmDetection
            | C::RefreshAuditLog
            | C::SetContext { .. }
            // Pure session/UI mechanics with no meaningful target to audit.
            | C::Shutdown
            | C::StartMcpDemoChat
            | C::StopMcpDemoChat
            | C::SendMcpDemoChatInput { .. }
            | C::ResizeMcpDemoChat { .. }
            | C::AgentClearConversation { .. }
            | C::AgentDeclineSuggestedAction { .. }
            | C::AgentRecordExecutedAction { .. } => None,

            // Local settings/state changes worth recording.
            C::SaveUiConfig { .. } => {
                summary(AuditActionKind::SettingsChange, None, None, None)
            }
            C::ConfigurePrometheus { namespace, service, .. } => summary(
                AuditActionKind::SettingsChange,
                Some("Service"),
                Some(namespace),
                Some(service),
            ),
            C::ConfigureGrafana { .. } | C::ClearGrafanaKey => {
                summary(AuditActionKind::SettingsChange, None, None, None)
            }
            C::SaveClusterSettings { context, .. } => summary(
                AuditActionKind::SettingsChange,
                Some("ClusterSettings"),
                None,
                Some(context),
            ),
            C::SetMcpServerEnabled { .. } => {
                summary(AuditActionKind::McpToggle, None, None, None)
            }
            C::StartPortForward { namespace, pod, .. } => summary(
                AuditActionKind::PortForwardChange,
                Some("Pod"),
                Some(namespace),
                Some(pod),
            ),
            C::StartServicePortForward {
                namespace, service, ..
            } => summary(
                AuditActionKind::PortForwardChange,
                Some("Service"),
                Some(namespace),
                Some(service),
            ),
            C::StopPortForward { tunnel_id }
            | C::DeletePortForward { tunnel_id }
            | C::UpdatePortForward { tunnel_id, .. } => summary(
                AuditActionKind::PortForwardChange,
                Some("PortForward"),
                None,
                Some(tunnel_id),
            ),
            C::SetTeamStorePath { .. } => {
                summary(AuditActionKind::TeamStoreChange, None, None, None)
            }
            C::PublishConnectionProfile { context_alias, .. } => summary(
                AuditActionKind::TeamStoreChange,
                Some("ConnectionProfile"),
                None,
                Some(context_alias),
            ),
            C::AdoptConnectionProfile { profile_id }
            | C::RemoveConnectionProfile { profile_id } => summary(
                AuditActionKind::TeamStoreChange,
                Some("ConnectionProfile"),
                None,
                Some(profile_id),
            ),
            C::PublishOrgClusterSettings { context }
            | C::AcceptOrgClusterSettings { context }
            | C::DismissOrgClusterSettings { context } => summary(
                AuditActionKind::TeamStoreChange,
                Some("OrgClusterSettings"),
                None,
                Some(context),
            ),
            C::AgentSetAuthMode { .. } => {
                summary(AuditActionKind::AgentAction, None, None, None)
            }

            C::CreateKindCluster { name, .. } => summary(
                AuditActionKind::KindClusterCreate,
                Some("KindCluster"),
                None,
                Some(name),
            ),
            C::DeleteKindCluster { name, .. } => summary(
                AuditActionKind::KindClusterDelete,
                Some("KindCluster"),
                None,
                Some(name),
            ),

            // Namespaced cluster writes: deletes.
            C::AddEphemeralContainer {
                namespace, pod, ..
            } => summary(
                AuditActionKind::EphemeralContainer,
                Some("Pod"),
                Some(namespace),
                Some(pod),
            ),

            C::DeletePod { namespace, name } => {
                summary(AuditActionKind::Delete, Some("Pod"), Some(namespace), Some(name))
            }
            C::DeleteDeployment { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("Deployment"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteDaemonSet { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("DaemonSet"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteStatefulSet { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("StatefulSet"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteReplicaSet { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("ReplicaSet"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteReplicationController { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("ReplicationController"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteJob { namespace, name } => {
                summary(AuditActionKind::Delete, Some("Job"), Some(namespace), Some(name))
            }
            C::DeleteCronJob { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("CronJob"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteService { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("Service"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteIngress { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("Ingress"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteNetworkPolicy { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("NetworkPolicy"),
                Some(namespace),
                Some(name),
            ),
            C::DeletePersistentVolumeClaim { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("PersistentVolumeClaim"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteRole { namespace, name } => {
                summary(AuditActionKind::Delete, Some("Role"), Some(namespace), Some(name))
            }
            C::DeleteRoleBinding { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("RoleBinding"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteConfigMap { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("ConfigMap"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteSecret { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("Secret"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteResourceQuota { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("ResourceQuota"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteLimitRange { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("LimitRange"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteHorizontalPodAutoscaler { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("HorizontalPodAutoscaler"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteVerticalPodAutoscaler { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("VerticalPodAutoscaler"),
                Some(namespace),
                Some(name),
            ),
            C::DeletePodDisruptionBudget { namespace, name } => summary(
                AuditActionKind::Delete,
                Some("PodDisruptionBudget"),
                Some(namespace),
                Some(name),
            ),
            C::DeleteLease { namespace, name } => {
                summary(AuditActionKind::Delete, Some("Lease"), Some(namespace), Some(name))
            }

            C::BulkDelete { items } => Some(AuditActionSummary {
                kind: AuditActionKind::BulkDelete,
                target_kind: None,
                target_namespace: None,
                target_name: Some(format!("{} object(s)", items.len())),
            }),

            // Cluster-scoped deletes.
            C::DeletePersistentVolume { name } => {
                summary(AuditActionKind::Delete, Some("PersistentVolume"), None, Some(name))
            }
            C::DeleteStorageClass { name } => {
                summary(AuditActionKind::Delete, Some("StorageClass"), None, Some(name))
            }
            C::DeleteClusterRole { name } => {
                summary(AuditActionKind::Delete, Some("ClusterRole"), None, Some(name))
            }
            C::DeleteClusterRoleBinding { name } => summary(
                AuditActionKind::Delete,
                Some("ClusterRoleBinding"),
                None,
                Some(name),
            ),
            C::DeletePriorityClass { name } => {
                summary(AuditActionKind::Delete, Some("PriorityClass"), None, Some(name))
            }
            C::DeleteRuntimeClass { name } => {
                summary(AuditActionKind::Delete, Some("RuntimeClass"), None, Some(name))
            }
            C::DeleteMutatingWebhookConfiguration { name } => summary(
                AuditActionKind::Delete,
                Some("MutatingWebhookConfiguration"),
                None,
                Some(name),
            ),
            C::DeleteValidatingWebhookConfiguration { name } => summary(
                AuditActionKind::Delete,
                Some("ValidatingWebhookConfiguration"),
                None,
                Some(name),
            ),

            // Node lifecycle.
            C::CordonNode { name } => {
                summary(AuditActionKind::NodeCordon, Some("Node"), None, Some(name))
            }
            C::UncordonNode { name } => {
                summary(AuditActionKind::NodeUncordon, Some("Node"), None, Some(name))
            }
            C::DrainNode { name, .. } | C::CancelDrainNode { name, .. } => {
                summary(AuditActionKind::NodeDrain, Some("Node"), None, Some(name))
            }
            C::TaintNode { name, .. } => {
                summary(AuditActionKind::NodeTaint, Some("Node"), None, Some(name))
            }

            // Scale / restart / cronjob control.
            C::ScaleDeployment { namespace, name, .. } => {
                summary(AuditActionKind::Scale, Some("Deployment"), Some(namespace), Some(name))
            }
            C::ScaleStatefulSet { namespace, name, .. } => {
                summary(AuditActionKind::Scale, Some("StatefulSet"), Some(namespace), Some(name))
            }
            C::ScaleReplicaSet { namespace, name, .. } => {
                summary(AuditActionKind::Scale, Some("ReplicaSet"), Some(namespace), Some(name))
            }
            C::ScaleReplicationController { namespace, name, .. } => summary(
                AuditActionKind::Scale,
                Some("ReplicationController"),
                Some(namespace),
                Some(name),
            ),
            C::RestartDeployment { namespace, name } => summary(
                AuditActionKind::Restart,
                Some("Deployment"),
                Some(namespace),
                Some(name),
            ),
            C::RestartDaemonSet { namespace, name } => summary(
                AuditActionKind::Restart,
                Some("DaemonSet"),
                Some(namespace),
                Some(name),
            ),
            C::RestartStatefulSet { namespace, name } => summary(
                AuditActionKind::Restart,
                Some("StatefulSet"),
                Some(namespace),
                Some(name),
            ),
            C::RestartReplicaSet { namespace, name } => summary(
                AuditActionKind::Restart,
                Some("ReplicaSet"),
                Some(namespace),
                Some(name),
            ),
            C::RestartReplicationController { namespace, name } => summary(
                AuditActionKind::Restart,
                Some("ReplicationController"),
                Some(namespace),
                Some(name),
            ),
            C::SuspendCronJob { namespace, name, .. }
            | C::TriggerCronJob { namespace, name } => summary(
                AuditActionKind::CronJobControl,
                Some("CronJob"),
                Some(namespace),
                Some(name),
            ),

            // Full-object apply/create: scope follows the target object's own scope, but the
            // audit entry always records the object identity regardless of scope.
            C::ApplyResourceYaml {
                kind,
                namespace,
                name,
                ..
            }
            | C::CreateResourceYaml {
                kind,
                namespace,
                name,
                ..
            } => Some(AuditActionSummary {
                kind: AuditActionKind::YamlApply,
                target_kind: Some(kind.clone()),
                target_namespace: namespace.clone(),
                target_name: Some(name.clone()),
            }),

            // Helm releases.
            C::PreviewHelmRollback { .. } | C::PreviewHelmUpgrade { .. } => None,
            C::ApplyHelmRollback { namespace, release, .. } => summary(
                AuditActionKind::HelmRollback,
                Some("HelmRelease"),
                Some(namespace),
                Some(release),
            ),
            C::ApplyHelmUpgrade { namespace, release, .. } => summary(
                AuditActionKind::HelmUpgrade,
                Some("HelmRelease"),
                Some(namespace),
                Some(release),
            ),
            C::UninstallHelmRelease { namespace, release, .. } => summary(
                AuditActionKind::HelmUninstall,
                Some("HelmRelease"),
                Some(namespace),
                Some(release),
            ),
        }
    }
}

/// Why a command is refused in hosted mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedDenial {
    /// Local kind cluster provisioning.
    Kind,
    /// Kubeconfig context listing.
    Contexts,
    /// Switching to a context other than the hosted home context.
    ContextSwitch,
    /// stdio MCP server / demo chat.
    Mcp,
    /// Multi-cluster fleet polling (reads the kubeconfig).
    Fleet,
    /// Reads/writes the state directory shared by every user of the process
    /// (`config.json`, `integrations.json`, team store, cluster settings).
    SharedState,
    /// Agent SDK Ops (server-held credentials and shared conversations).
    Agent,
    /// Makes the server call arbitrary URLs, or is a diagnostic hook.
    ServerSideProbe,
    /// Backend lifecycle is owned by the server, not the client.
    Lifecycle,
}

/// Hosted-mode verdict for one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedPolicy {
    Allowed,
    Denied(HostedDenial),
}

impl BackendCmdV1 {
    /// Hosted-mode policy (spec 031). Exhaustive with no wildcard on purpose: a new variant
    /// must be classified here. `home` is the hosted home context; `SetContext` is allowed
    /// only to it. Cluster writes are NOT decided here: the API server enforces the user's RBAC.
    #[allow(clippy::too_many_lines)] // one arm per protocol variant, by design
    pub fn hosted_policy(&self, home: &str) -> HostedPolicy {
        use BackendCmdV1 as C;
        match self {
            C::SetContext { context } if context == home => HostedPolicy::Allowed,
            C::SetContext { .. } => HostedPolicy::Denied(HostedDenial::ContextSwitch),
            C::RefreshKindPrerequisites
            | C::RefreshKindClusters
            | C::CreateKindCluster { .. }
            | C::CancelCreateKindCluster { .. }
            | C::DeleteKindCluster { .. } => HostedPolicy::Denied(HostedDenial::Kind),
            C::RefreshContexts => HostedPolicy::Denied(HostedDenial::Contexts),
            C::LoadMcpServerStatus
            | C::SetMcpServerEnabled { .. }
            | C::StartMcpDemoChat
            | C::StopMcpDemoChat
            | C::SendMcpDemoChatInput { .. }
            | C::ResizeMcpDemoChat { .. } => HostedPolicy::Denied(HostedDenial::Mcp),
            C::StartFleetPolling | C::StopFleetPolling => HostedPolicy::Denied(HostedDenial::Fleet),
            C::ConfigurePrometheus { .. }
            | C::ConfigureGrafana { .. }
            | C::ClearGrafanaKey
            | C::GetObservabilitySettings
            | C::LoadUiConfig
            | C::SaveUiConfig { .. }
            | C::LoadClusterSettings { .. }
            | C::SaveClusterSettings { .. }
            | C::SetTeamStorePath { .. }
            | C::RefreshTeamStoreStatus
            | C::RefreshConnectionProfiles
            | C::PublishConnectionProfile { .. }
            | C::AdoptConnectionProfile { .. }
            | C::RemoveConnectionProfile { .. }
            | C::PublishOrgClusterSettings { .. }
            | C::AcceptOrgClusterSettings { .. }
            | C::DismissOrgClusterSettings { .. } => {
                HostedPolicy::Denied(HostedDenial::SharedState)
            }
            C::AgentGetAuthMode
            | C::AgentSetAuthMode { .. }
            | C::AgentRevealKey
            | C::AgentOpenConversation { .. }
            | C::AgentAsk { .. }
            | C::AgentClearConversation { .. }
            | C::AgentReviewSuggestedAction { .. }
            | C::AgentDeclineSuggestedAction { .. }
            | C::AgentRecordExecutedAction { .. } => HostedPolicy::Denied(HostedDenial::Agent),
            C::TestPrometheusConnection { .. }
            | C::TestGrafanaConnection { .. }
            | C::SimulateSlowCall => HostedPolicy::Denied(HostedDenial::ServerSideProbe),
            C::Shutdown => HostedPolicy::Denied(HostedDenial::Lifecycle),
            // 033-038 additions. Per-user cluster operations stay allowed (the API server
            // enforces the user's RBAC); everything that touches server-shared state or
            // server-held credentials is refused.
            C::GetNodeShellSettings
            | C::SetNodeShellSettings { .. }
            | C::SetExecPolicy { .. }
            | C::LoadOidcConfig
            | C::OidcSaveIssuer { .. }
            | C::OidcDeleteIssuer { .. }
            | C::OidcSetRoleMap { .. }
            | C::OidcSignIn { .. }
            | C::OidcCancelSignIn { .. }
            | C::OidcSignOut { .. }
            | C::SetContextAuthMode { .. }
            | C::IdpSetAdminKey { .. }
            | C::IdpRefreshDirectory { .. }
            | C::LinkTeamProfile { .. }
            | C::UnlinkTeamProfile { .. }
            | C::PreviewRoleAssignment { .. }
            | C::RefreshOrphanedSubjects { .. }
            | C::SetDaemonPreference { .. }
            | C::RefreshDaemon { .. }
            | C::RequestDaemonTimeline { .. }
            | C::RequestDaemonTopology { .. }
            | C::RequestDaemonMetrics { .. } => HostedPolicy::Denied(HostedDenial::SharedState),
            C::AiProviderLoginStart { .. }
            | C::AiProviderSubmitApiKey { .. }
            | C::AiProviderLoginCancel
            | C::AiProviderSignOut { .. }
            | C::AiSetActiveProvider { .. }
            | C::AiSetOfflineMode { .. }
            | C::AiOllamaProbe { .. }
            | C::AiOllamaSelectModel { .. }
            | C::AiSetOllamaEndpoint { .. }
            | C::AiEnableFileStore { .. }
            | C::AiGetStatus => HostedPolicy::Denied(HostedDenial::Agent),
            C::FederateContext { .. }
            | C::UnfederateContext { .. }
            | C::RequestFederationSnapshot => HostedPolicy::Denied(HostedDenial::Fleet),
            C::CheckNodeShellAccess { .. }
            | C::StartNodeShell { .. }
            | C::StopNodeShell { .. }
            | C::ListNodeShellOrphans { .. }
            | C::DeleteNodeShellOrphans { .. }
            | C::ResolveCapabilities { .. }
            | C::CheckCapability { .. }
            | C::Initialize
            | C::RefreshNamespaces
            | C::SetNamespaceFilter { .. }
            | C::RefreshNodes
            | C::StartNodeMetricsPolling { .. }
            | C::StopNodeMetricsPolling
            | C::SelectNode { .. }
            | C::SelectNamespace { .. }
            | C::CordonNode { .. }
            | C::UncordonNode { .. }
            | C::DrainNode { .. }
            | C::CancelDrainNode { .. }
            | C::TaintNode { .. }
            | C::RefreshEvents
            | C::RefreshPods
            | C::SelectPod { .. }
            | C::DeletePod { .. }
            | C::AddEphemeralContainer { .. }
            | C::InspectContainerImage { .. }
            | C::InspectPodVulnerabilities { .. }
            | C::StartPodLogs { .. }
            | C::StopPodLogs
            | C::FetchPodMetrics { .. }
            | C::StartPodExec { .. }
            | C::SendPodExecInput { .. }
            | C::ResizePodExec { .. }
            | C::StopPodExec
            | C::CheckPodExecAccess { .. }
            | C::RefreshDeployments
            | C::SelectDeployment { .. }
            | C::ScaleDeployment { .. }
            | C::RestartDeployment { .. }
            | C::DeleteDeployment { .. }
            | C::RefreshDaemonSets
            | C::SelectDaemonSet { .. }
            | C::RestartDaemonSet { .. }
            | C::DeleteDaemonSet { .. }
            | C::RefreshStatefulSets
            | C::SelectStatefulSet { .. }
            | C::ScaleStatefulSet { .. }
            | C::RestartStatefulSet { .. }
            | C::DeleteStatefulSet { .. }
            | C::RefreshReplicaSets
            | C::SelectReplicaSet { .. }
            | C::ScaleReplicaSet { .. }
            | C::RestartReplicaSet { .. }
            | C::DeleteReplicaSet { .. }
            | C::RefreshReplicationControllers
            | C::SelectReplicationController { .. }
            | C::ScaleReplicationController { .. }
            | C::RestartReplicationController { .. }
            | C::DeleteReplicationController { .. }
            | C::RefreshJobs
            | C::SelectJob { .. }
            | C::DeleteJob { .. }
            | C::RefreshCronJobs
            | C::SelectCronJob { .. }
            | C::SuspendCronJob { .. }
            | C::TriggerCronJob { .. }
            | C::DeleteCronJob { .. }
            | C::CheckPortAvailability { .. }
            | C::StartPortForward { .. }
            | C::StartServicePortForward { .. }
            | C::StopPortForward { .. }
            | C::DeletePortForward { .. }
            | C::UpdatePortForward { .. }
            | C::RefreshPortForwards
            | C::RefreshServices
            | C::SelectService { .. }
            | C::DeleteService { .. }
            | C::RefreshEndpoints
            | C::SelectEndpoint { .. }
            | C::RefreshIngresses
            | C::SelectIngress { .. }
            | C::DeleteIngress { .. }
            | C::RefreshIngressClasses
            | C::SelectIngressClass { .. }
            | C::RefreshNetworkPolicies
            | C::SelectNetworkPolicy { .. }
            | C::DeleteNetworkPolicy { .. }
            | C::RefreshIntegrations
            | C::StartMetricsPolling { .. }
            | C::StopMetricsPolling
            | C::StartWorkloadMetricsPolling { .. }
            | C::StopWorkloadMetricsPolling
            | C::RefreshFlux
            | C::RefreshKnative
            | C::RefreshVelero
            | C::RefreshTrivyOverview
            | C::RefreshKeda
            | C::SelectScaledObject { .. }
            | C::SelectScaledJob { .. }
            | C::RefreshHubble
            | C::RefreshGatekeeper
            | C::RefreshCertManager
            | C::BulkDelete { .. }
            | C::SetActiveSection { .. }
            | C::RefreshPersistentVolumes
            | C::SelectPersistentVolume { .. }
            | C::DeletePersistentVolume { .. }
            | C::RefreshPersistentVolumeClaims
            | C::SelectPersistentVolumeClaim { .. }
            | C::DeletePersistentVolumeClaim { .. }
            | C::RefreshStorageClasses
            | C::SelectStorageClass { .. }
            | C::DeleteStorageClass { .. }
            | C::RefreshRoles
            | C::SelectRole { .. }
            | C::DeleteRole { .. }
            | C::RefreshRoleBindings
            | C::SelectRoleBinding { .. }
            | C::DeleteRoleBinding { .. }
            | C::RefreshClusterRoles
            | C::SelectClusterRole { .. }
            | C::DeleteClusterRole { .. }
            | C::RefreshClusterRoleBindings
            | C::SelectClusterRoleBinding { .. }
            | C::DeleteClusterRoleBinding { .. }
            | C::RefreshRbacSelfAccess { .. }
            | C::CheckResourceMutationAccess { .. }
            | C::ResolveNamespaceAccess { .. }
            | C::RefreshKubeadmDetection
            | C::RefreshConfigMaps
            | C::SelectConfigMap { .. }
            | C::DeleteConfigMap { .. }
            | C::RefreshSecrets
            | C::SelectSecret { .. }
            | C::DeleteSecret { .. }
            | C::RefreshResourceQuotas
            | C::SelectResourceQuota { .. }
            | C::DeleteResourceQuota { .. }
            | C::RefreshLimitRanges
            | C::SelectLimitRange { .. }
            | C::DeleteLimitRange { .. }
            | C::RefreshHorizontalPodAutoscalers
            | C::SelectHorizontalPodAutoscaler { .. }
            | C::DeleteHorizontalPodAutoscaler { .. }
            | C::RefreshVerticalPodAutoscalers
            | C::SelectVerticalPodAutoscaler { .. }
            | C::DeleteVerticalPodAutoscaler { .. }
            | C::RefreshPodDisruptionBudgets
            | C::SelectPodDisruptionBudget { .. }
            | C::DeletePodDisruptionBudget { .. }
            | C::RefreshPriorityClasses
            | C::SelectPriorityClass { .. }
            | C::DeletePriorityClass { .. }
            | C::RefreshRuntimeClasses
            | C::SelectRuntimeClass { .. }
            | C::DeleteRuntimeClass { .. }
            | C::RefreshLeases
            | C::SelectLease { .. }
            | C::DeleteLease { .. }
            | C::RefreshMutatingWebhookConfigurations
            | C::SelectMutatingWebhookConfiguration { .. }
            | C::DeleteMutatingWebhookConfiguration { .. }
            | C::RefreshValidatingWebhookConfigurations
            | C::SelectValidatingWebhookConfiguration { .. }
            | C::DeleteValidatingWebhookConfiguration { .. }
            | C::ApplyResourceYaml { .. }
            | C::CreateResourceYaml { .. }
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
            | C::SelectCertificateChain { .. }
            | C::PreviewHelmRollback { .. }
            | C::ApplyHelmRollback { .. }
            | C::PreviewHelmUpgrade { .. }
            | C::ApplyHelmUpgrade { .. }
            | C::UninstallHelmRelease { .. }
            | C::RefreshAuditLog => HostedPolicy::Allowed,
        }
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
    fn audit_summary_none_for_reads_some_for_mutations() {
        assert!(BackendCmdV1::RefreshPods.audit_summary().is_none());
        assert!(BackendCmdV1::RefreshAuditLog.audit_summary().is_none());
        let a = BackendCmdV1::CordonNode { name: s("n1") }
            .audit_summary()
            .expect("cordon is audited");
        assert_eq!(a.kind, AuditActionKind::NodeCordon);
        assert_eq!(a.target_kind.as_deref(), Some("Node"));
        assert_eq!(a.target_name.as_deref(), Some("n1"));
        assert_eq!(a.target_namespace, None);
        let y = BackendCmdV1::ApplyResourceYaml {
            api_version: s("v1"),
            kind: s("ConfigMap"),
            namespace: Some(s("default")),
            name: s("cm"),
            yaml: s(""),
            expected_resource_version: s("1"),
        }
        .audit_summary()
        .expect("apply is audited");
        assert_eq!(y.kind, AuditActionKind::YamlApply);
        assert_eq!(y.target_namespace.as_deref(), Some("default"));
    }

    #[test]
    fn read_only_sample() {
        for cmd in [
            BackendCmdV1::Initialize,
            BackendCmdV1::RefreshKindPrerequisites,
            BackendCmdV1::RefreshKindClusters,
            BackendCmdV1::RefreshPods,
            BackendCmdV1::RefreshVelero,
            BackendCmdV1::GetObservabilitySettings,
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
            BackendCmdV1::StartServicePortForward {
                namespace: s("grafana"),
                service: s("grafana"),
                port: 3000,
            },
            BackendCmdV1::DeletePortForward { tunnel_id: s("t") },
            BackendCmdV1::CreateKindCluster {
                request_id: s("r"),
                name: s("dev"),
                profile: crate::protocol::KindClusterProfileId::Bare,
                custom_yaml: None,
            },
            BackendCmdV1::CancelCreateKindCluster { request_id: s("r") },
            BackendCmdV1::DeleteKindCluster {
                request_id: s("r"),
                name: s("dev"),
            },
            BackendCmdV1::ConfigureGrafana {
                base_url: s("http://g"),
                api_key: s("k"),
            },
            BackendCmdV1::ClearGrafanaKey,
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
            // 024-pod-exec-terminal: opening an exec session is gated like a mutation
            // even though it writes no Kubernetes object (see the comment on this arm in
            // `mutation_target()`).
            BackendCmdV1::StartPodExec {
                namespace: s("fathom-dev"),
                name: s("p"),
                container: None,
            },
        ];
        for cmd in cases {
            assert_eq!(cmd.mutation_target(), nsn("fathom-dev"), "{cmd:?}");
            assert!(cmd.mutation_target().writes_cluster());
        }
    }

    /// 024-pod-exec-terminal: `StartPodExec` MUST be gated like a mutation (contracts/
    /// protocol.md's reclassification contract), while every other exec command stays a
    /// pure read.
    #[test]
    fn start_pod_exec_is_namespaced_not_read_only() {
        let start = BackendCmdV1::StartPodExec {
            namespace: s("fathom-dev"),
            name: s("p"),
            container: Some(s("app")),
        };
        assert_eq!(start.mutation_target(), nsn("fathom-dev"));
        assert!(start.mutation_target().writes_cluster());

        for cmd in [
            BackendCmdV1::SendPodExecInput { data: s("bHM=") },
            BackendCmdV1::ResizePodExec { cols: 80, rows: 24 },
            BackendCmdV1::StopPodExec,
            BackendCmdV1::CheckPodExecAccess {
                namespace: s("fathom-dev"),
                name: s("p"),
                container: None,
            },
        ] {
            assert_eq!(cmd.mutation_target(), MutationTarget::ReadOnly, "{cmd:?}");
        }
    }

    #[test]
    fn add_ephemeral_container_is_namespaced_write() {
        let cmd = BackendCmdV1::AddEphemeralContainer {
            namespace: s("fathom-dev"),
            pod: s("web-0"),
            target_container: Some(s("app")),
            image: s("busybox"),
            ephemeral_container_name: None,
        };
        assert_eq!(cmd.mutation_target(), nsn("fathom-dev"));
        assert!(cmd.mutation_target().writes_cluster());
    }

    #[test]
    fn node_shell_classification() {
        use crate::protocol::v1::{NodeShellPodRef, NodeShellSettings};
        for cmd in [
            BackendCmdV1::CheckNodeShellAccess { namespace: s("d") },
            BackendCmdV1::ListNodeShellOrphans { namespace: None },
            BackendCmdV1::GetNodeShellSettings,
        ] {
            assert_eq!(cmd.mutation_target(), MutationTarget::ReadOnly, "{cmd:?}");
        }
        assert_eq!(
            BackendCmdV1::SetNodeShellSettings {
                settings: NodeShellSettings::default()
            }
            .mutation_target(),
            MutationTarget::Local
        );
        let start = BackendCmdV1::StartNodeShell {
            node: s("n"),
            namespace: s("kube-system"),
            image: s("busybox"),
            image_pull_secret: None,
            deadline_seconds: None,
        };
        assert_eq!(start.mutation_target(), nsn("kube-system"));
        assert!(start.mutation_target().writes_cluster());
        let stop = BackendCmdV1::StopNodeShell {
            session_id: 1,
            namespace: s("kube-system"),
        };
        assert_eq!(stop.mutation_target(), nsn("kube-system"));
        let orphans = BackendCmdV1::DeleteNodeShellOrphans {
            pods: vec![
                NodeShellPodRef {
                    namespace: s("b"),
                    name: s("x"),
                },
                NodeShellPodRef {
                    namespace: s("a"),
                    name: s("y"),
                },
                NodeShellPodRef {
                    namespace: s("b"),
                    name: s("z"),
                },
            ],
        };
        assert_eq!(
            orphans.mutation_target(),
            MutationTarget::Namespaced {
                namespaces: vec![s("a"), s("b")]
            }
        );
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
        for cmd in [
            BackendCmdV1::GetObservabilitySettings,
            BackendCmdV1::ClearGrafanaKey,
            BackendCmdV1::ConfigureGrafana {
                base_url: s("http://g"),
                api_key: s("k"),
            },
        ] {
            assert_eq!(cmd.section(), Some(section_keys::integrations_grafana()));
        }
        assert!(
            BackendCmdV1::GetObservabilitySettings
                .audit_summary()
                .is_none()
        );
        assert!(BackendCmdV1::ClearGrafanaKey.audit_summary().is_some());
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
            BackendCmdV1::AiProviderLoginStart {
                provider: crate::protocol::v1::Provider::Github,
            },
            BackendCmdV1::AiSetOfflineMode { enabled: true },
            BackendCmdV1::AiOllamaProbe { endpoint: None },
            BackendCmdV1::AiGetStatus,
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
