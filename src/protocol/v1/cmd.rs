use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{
    ActionOutcome, AgentAuthModeInput, AgentObjectRef, ApiKeyInput, CapabilityQuery,
    ClusterSettings, ContextAuthMode, FathomRoleMap, NodeShellPodRef, NodeShellSettings,
    OidcIssuerConfig, OidcSignInMethod, Provider, RbacAccessCheck, RequestId, RoleAssignmentDraft,
    SectionKey, UiConfig,
};

/// Commands sent from a UI client to the backend. Fire-and-forget: results arrive as events.
///
/// Serialized adjacently tagged: `{ "type": "RefreshPods" }`,
/// `{ "type": "SetContext", "data": { "context": "kind-a" } }`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "data")]
pub enum BackendCmdV1 {
    // Global / shell-owned
    Initialize,
    Shutdown,
    RefreshContexts,
    SetContext {
        context: String,
    },
    RefreshNamespaces,
    SetNamespaceFilter {
        namespaces: Vec<String>,
    },
    RefreshNodes,
    SelectNode {
        name: String,
    },
    CordonNode {
        name: String,
    },
    UncordonNode {
        name: String,
    },
    DrainNode {
        name: String,
        ignore_daemonsets: bool,
        force_delete_empty_dir: bool,
    },
    CancelDrainNode {
        name: String,
        request_id: String,
    },
    TaintNode {
        name: String,
        op: TaintOp,
        key: String,
        value: Option<String>,
        effect: String,
    },
    // 037-node-shell: privileged debug pod pinned to a node, with an attached terminal.
    // Terminal I/O reuses `StartPodExec`/`SendPodExecInput`/`ResizePodExec` against the pod.
    /// Read-only `SelfSubjectAccessReview`s for everything Node Shell needs.
    CheckNodeShellAccess {
        namespace: String,
    },
    /// Create the debug pod. Only sent after the operator confirmed the dialog.
    StartNodeShell {
        node: String,
        namespace: String,
        image: String,
        image_pull_secret: Option<String>,
        deadline_seconds: Option<u32>,
    },
    /// Delete the session's pod (also cancels a pending start). Retry after a leftover.
    StopNodeShell {
        session_id: u64,
        /// The pod's namespace, so the delete classifies as a namespaced write.
        namespace: String,
    },
    ListNodeShellOrphans {
        namespace: Option<String>,
    },
    DeleteNodeShellOrphans {
        pods: Vec<NodeShellPodRef>,
    },
    GetNodeShellSettings,
    SetNodeShellSettings {
        settings: NodeShellSettings,
    },
    /// 035-cloud-auth-plugins: allow (or stop allowing) non-allowlisted exec commands for one
    /// context. Persisted additively; re-emits `CloudAuthDetected`.
    SetExecPolicy {
        context: String,
        policy: super::ExecPolicy,
    },
    RefreshEvents,

    // Workloads / Pods section
    RefreshPods,
    SelectPod {
        namespace: String,
        name: String,
    },
    DeletePod {
        namespace: String,
        name: String,
    },
    /// Fetch one container's OCI image manifest/config metadata (010-container-image-inspection).
    /// Sent only when the operator opens that container's "Image" tab (FR-003) — never eagerly.
    InspectContainerImage {
        namespace: String,
        pod: String,
        container: String,
    },
    StartPodLogs {
        namespace: String,
        name: String,
        container: Option<String>,
    },
    StopPodLogs,
    FetchPodMetrics {
        namespace: String,
        name: String,
    },
    StartPodExec {
        namespace: String,
        name: String,
        container: Option<String>,
    },
    SendPodExecInput {
        data: String,
    },
    ResizePodExec {
        cols: u16,
        rows: u16,
    },
    StopPodExec,

    // Workloads / Deployments section
    RefreshDeployments,
    SelectDeployment {
        namespace: String,
        name: String,
    },
    ScaleDeployment {
        namespace: String,
        name: String,
        replicas: i32,
    },
    RestartDeployment {
        namespace: String,
        name: String,
    },
    DeleteDeployment {
        namespace: String,
        name: String,
    },

    // Workloads / DaemonSets section
    RefreshDaemonSets,
    SelectDaemonSet {
        namespace: String,
        name: String,
    },
    RestartDaemonSet {
        namespace: String,
        name: String,
    },
    DeleteDaemonSet {
        namespace: String,
        name: String,
    },

    // Workloads / StatefulSets section
    RefreshStatefulSets,
    SelectStatefulSet {
        namespace: String,
        name: String,
    },
    ScaleStatefulSet {
        namespace: String,
        name: String,
        replicas: i32,
    },
    RestartStatefulSet {
        namespace: String,
        name: String,
    },
    DeleteStatefulSet {
        namespace: String,
        name: String,
    },

    // Workloads / ReplicaSets section
    RefreshReplicaSets,
    SelectReplicaSet {
        namespace: String,
        name: String,
    },
    ScaleReplicaSet {
        namespace: String,
        name: String,
        replicas: i32,
    },
    RestartReplicaSet {
        namespace: String,
        name: String,
    },
    DeleteReplicaSet {
        namespace: String,
        name: String,
    },

    // Workloads / ReplicationControllers section
    RefreshReplicationControllers,
    SelectReplicationController {
        namespace: String,
        name: String,
    },
    ScaleReplicationController {
        namespace: String,
        name: String,
        replicas: i32,
    },
    RestartReplicationController {
        namespace: String,
        name: String,
    },
    DeleteReplicationController {
        namespace: String,
        name: String,
    },
    // Workloads / Jobs section
    RefreshJobs,
    SelectJob {
        namespace: String,
        name: String,
    },
    DeleteJob {
        namespace: String,
        name: String,
    },

    // Workloads / CronJobs section
    RefreshCronJobs,
    SelectCronJob {
        namespace: String,
        name: String,
    },
    SuspendCronJob {
        namespace: String,
        name: String,
        suspend: bool,
    },
    TriggerCronJob {
        namespace: String,
        name: String,
    },
    DeleteCronJob {
        namespace: String,
        name: String,
    },
    // Network / Port Forwards section
    CheckPortAvailability {
        port: u16,
    },
    StartPortForward {
        namespace: String,
        pod: String,
        local_port: u16,
        remote_port: u16,
        container: Option<String>,
    },
    StopPortForward {
        tunnel_id: String,
    },
    DeletePortForward {
        tunnel_id: String,
    },
    UpdatePortForward {
        tunnel_id: String,
        local_port: u16,
        remote_port: u16,
        auto_start: bool,
    },
    RefreshPortForwards,

    // Network / Services section
    RefreshServices,
    SelectService {
        namespace: String,
        name: String,
    },
    DeleteService {
        namespace: String,
        name: String,
    },

    // Network / Endpoints section
    RefreshEndpoints,
    SelectEndpoint {
        namespace: String,
        name: String,
    },

    // Network / Ingresses section
    RefreshIngresses,
    SelectIngress {
        namespace: String,
        name: String,
    },
    DeleteIngress {
        namespace: String,
        name: String,
    },

    // Network / Ingress Classes section
    RefreshIngressClasses,
    SelectIngressClass {
        name: String,
    },

    // Network / Network Policies section
    RefreshNetworkPolicies,
    SelectNetworkPolicy {
        namespace: String,
        name: String,
    },
    DeleteNetworkPolicy {
        namespace: String,
        name: String,
    },

    // Integrations / Metrics section
    RefreshIntegrations,
    StartMetricsPolling {
        namespace: String,
        name: String,
        interval_secs: u64,
    },
    StopMetricsPolling,
    StartWorkloadMetricsPolling {
        namespace: String,
        workload_name: String,
        workload_kind: String,
        pod_selector: String,
        interval_secs: u64,
    },
    StopWorkloadMetricsPolling,
    ConfigurePrometheus {
        namespace: String,
        service: String,
        port: u16,
    },
    TestPrometheusConnection {
        namespace: String,
        service: String,
        port: u16,
    },
    ConfigureGrafana {
        base_url: String,
        api_key: String,
    },
    TestGrafanaConnection {
        base_url: String,
        api_key: String,
    },
    RefreshFlux,
    RefreshHubble,
    // Keeps the original Day-1 “proof” button around.
    SimulateSlowCall,
    // Bulk operations
    BulkDelete {
        items: Vec<(String, String)>, // (namespace, name)
    },
    // Future hook: allow UI to hint active section for prefetching (optional).
    SetActiveSection {
        section: SectionKey,
    },

    // Storage / Config / Access resources
    // storage_pv
    RefreshPersistentVolumes,
    SelectPersistentVolume {
        name: String,
    },
    DeletePersistentVolume {
        name: String,
    },

    // storage_pvc
    RefreshPersistentVolumeClaims,
    SelectPersistentVolumeClaim {
        namespace: String,
        name: String,
    },
    DeletePersistentVolumeClaim {
        namespace: String,
        name: String,
    },

    // storage_storageclasses
    RefreshStorageClasses,
    SelectStorageClass {
        name: String,
    },
    DeleteStorageClass {
        name: String,
    },

    // access_roles
    RefreshRoles,
    SelectRole {
        namespace: String,
        name: String,
    },
    DeleteRole {
        namespace: String,
        name: String,
    },

    // access_rolebindings
    RefreshRoleBindings,
    SelectRoleBinding {
        namespace: String,
        name: String,
    },
    DeleteRoleBinding {
        namespace: String,
        name: String,
    },

    // access_clusterroles
    RefreshClusterRoles,
    SelectClusterRole {
        name: String,
    },
    DeleteClusterRole {
        name: String,
    },

    // access_clusterrolebindings
    RefreshClusterRoleBindings,
    SelectClusterRoleBinding {
        name: String,
    },
    DeleteClusterRoleBinding {
        name: String,
    },

    // access_rolebindings / access_clusterrolebindings: live self-permission gating for
    // create/edit/delete actions (FR-007). Read-only; never mutates the cluster.
    RefreshRbacSelfAccess {
        checks: Vec<RbacAccessCheck>,
    },

    // access/namespace-access (read-only, derived from already-fetched RBAC objects)
    ResolveNamespaceAccess {
        subject_kind: String,
        subject_name: String,
        subject_namespace: Option<String>,
    },

    // access/oidc-config: best-effort read-only detection only, never a live config edit
    // (FR-008). The OIDC flag/instruction generation itself is frontend-only and has no
    // protocol command at all.
    RefreshKubeadmDetection,

    // config_configmaps
    RefreshConfigMaps,
    SelectConfigMap {
        namespace: String,
        name: String,
    },
    DeleteConfigMap {
        namespace: String,
        name: String,
    },

    // config_secrets
    RefreshSecrets,
    SelectSecret {
        namespace: String,
        name: String,
    },
    DeleteSecret {
        namespace: String,
        name: String,
    },

    // config_resourcequotas
    RefreshResourceQuotas,
    SelectResourceQuota {
        namespace: String,
        name: String,
    },
    DeleteResourceQuota {
        namespace: String,
        name: String,
    },

    // config_limitranges
    RefreshLimitRanges,
    SelectLimitRange {
        namespace: String,
        name: String,
    },
    DeleteLimitRange {
        namespace: String,
        name: String,
    },

    // config_horizontalpodautoscalers
    RefreshHorizontalPodAutoscalers,
    SelectHorizontalPodAutoscaler {
        namespace: String,
        name: String,
    },
    DeleteHorizontalPodAutoscaler {
        namespace: String,
        name: String,
    },

    // config_poddisruptionbudgets
    RefreshPodDisruptionBudgets,
    SelectPodDisruptionBudget {
        namespace: String,
        name: String,
    },
    DeletePodDisruptionBudget {
        namespace: String,
        name: String,
    },

    // config_priorityclasses
    RefreshPriorityClasses,
    SelectPriorityClass {
        name: String,
    },
    DeletePriorityClass {
        name: String,
    },

    // config_runtimeclasses
    RefreshRuntimeClasses,
    SelectRuntimeClass {
        name: String,
    },
    DeleteRuntimeClass {
        name: String,
    },

    // config_leases
    RefreshLeases,
    SelectLease {
        namespace: String,
        name: String,
    },
    DeleteLease {
        namespace: String,
        name: String,
    },

    // config_mutatingwebhooks
    RefreshMutatingWebhookConfigurations,
    SelectMutatingWebhookConfiguration {
        name: String,
    },
    DeleteMutatingWebhookConfiguration {
        name: String,
    },

    // config_validatingwebhooks
    RefreshValidatingWebhookConfigurations,
    SelectValidatingWebhookConfiguration {
        name: String,
    },
    DeleteValidatingWebhookConfiguration {
        name: String,
    },

    // UI preferences (~/.fathom/config.json); answered with `SystemEvent::UiConfigLoaded`.
    LoadUiConfig,
    SaveUiConfig {
        ui: UiConfig,
        expanded_groups: Vec<String>,
    },
    // 014-teams-organizations: Cluster Settings (§10, local per-operator) + org sync
    // and shared team connection profiles.
    LoadClusterSettings {
        context: String,
    },
    SaveClusterSettings {
        context: String,
        settings: ClusterSettings,
    },
    SetTeamStorePath {
        path: Option<String>,
    },
    RefreshTeamStoreStatus,
    // 033-rauthy-oidc-teams-users: OIDC identity, IdP directory, capabilities, role assignment.
    // Responses never carry tokens/secrets; `IdpSetAdminKey.api_key` is write-only.
    /// Read the persisted `oidc` config and identities; replies `OidcConfigLoaded`.
    LoadOidcConfig,
    /// Persist (insert or replace by `id`) an issuer config (non-secret).
    OidcSaveIssuer {
        issuer: OidcIssuerConfig,
    },
    /// Remove an issuer config, its tokens and its admin key.
    OidcDeleteIssuer {
        issuer_id: String,
    },
    /// Persist the Fathom-level role-group names.
    OidcSetRoleMap {
        role_map: FathomRoleMap,
    },
    /// Start a sign-in flow; emits `OidcSignInProgress`.
    OidcSignIn {
        issuer_id: String,
        method: OidcSignInMethod,
    },
    /// Abort an in-flight sign-in.
    OidcCancelSignIn {
        issuer_id: String,
    },
    /// Delete tokens; optionally open the RP-initiated logout URL.
    OidcSignOut {
        issuer_id: String,
        end_session: bool,
    },
    /// Persist a context's auth mode and reconnect it.
    SetContextAuthMode {
        context: String,
        mode: ContextAuthMode,
        issuer_id: Option<String>,
    },
    /// Store the read-only IdP admin API key in the OS keyring. Write-only, never echoed.
    IdpSetAdminKey {
        issuer_id: String,
        api_key: String,
    },
    /// Read users and groups from the IdP (read-only); emits `IdpDirectoryUpdated`.
    IdpRefreshDirectory {
        issuer_id: String,
    },
    /// Populate the capability cache for a namespace (`None` = cluster-scoped bucket).
    ResolveCapabilities {
        context: String,
        namespace: Option<String>,
    },
    /// Single verb/resource check (cluster-scoped, or fallback for an incomplete rules review).
    CheckCapability {
        query: CapabilityQuery,
    },
    /// Link an IdP group to a shared team profile (advisory visibility, R8).
    LinkTeamProfile {
        group: String,
        profile_id: String,
    },
    UnlinkTeamProfile {
        group: String,
        profile_id: String,
    },
    /// Render a Role/ClusterRoleBinding YAML for the existing diff view; no cluster write.
    PreviewRoleAssignment {
        draft: RoleAssignmentDraft,
    },
    /// Join current bindings' subjects against the IdP directory; emits `OrphanedSubjectsUpdated`.
    RefreshOrphanedSubjects {
        context: String,
        issuer_id: String,
    },
    RefreshConnectionProfiles,
    PublishConnectionProfile {
        context_alias: String,
        api_server: String,
        default_namespace: Option<String>,
    },
    AdoptConnectionProfile {
        profile_id: String,
    },
    RemoveConnectionProfile {
        profile_id: String,
    },
    PublishOrgClusterSettings {
        context: String,
    },
    AcceptOrgClusterSettings {
        context: String,
    },
    DismissOrgClusterSettings {
        context: String,
    },

    // 015-mcp-agent-exposure: the stdio MCP server's operator toggle and status, surfaced in
    // Cluster Settings' "Available Agents" section. Both answered with
    // `SystemEvent::McpServerStatusChanged`.
    /// Read the current enabled/running state and the connection snippet an external MCP
    /// client should use. Reads nothing from the cluster.
    LoadMcpServerStatus,
    /// Turn the stdio MCP server on or off. Persists `auto_start.mcp_stdio` and starts/stops
    /// the server task in the same handler, so no listener ever exists while off (FR-002,
    /// FR-013).
    SetMcpServerEnabled {
        enabled: bool,
    },

    // 015-mcp-agent-exposure US4: the "Launch example chat" affordance. All `Local` — they
    // drive a subprocess on the operator's own machine (`claude` from `PATH`), never the
    // cluster. The session is a singleton, like the pod-exec session, so none of these carry
    // a target: `StopMcpDemoChat`/`SendMcpDemoChatInput`/`ResizeMcpDemoChat` address whatever
    // session is live and are a no-op when none is.
    /// Spawn the locally-detected `claude` CLI, pre-configured to attach to this Fathom's own
    /// stdio MCP server. Refused while the MCP server is off (FR-010), and answered with
    /// `McpDemoChatNotDetected` — never a spawn — when no `claude` is on `PATH` (FR-011).
    StartMcpDemoChat,
    /// Stop the live demo-chat subprocess, if any.
    StopMcpDemoChat,
    /// Base64-encoded keystrokes for the demo chat's stdin — same encoding convention as
    /// `SendPodExecInput`.
    SendMcpDemoChatInput {
        data: String,
    },
    /// The terminal's current size, so the subprocess can wrap its output to it.
    ResizeMcpDemoChat {
        cols: u16,
        rows: u16,
    },

    // Resource YAML edit + apply (any kind; identity fields validated pre-cluster-write).
    ApplyResourceYaml {
        api_version: String,
        kind: String,
        namespace: Option<String>,
        name: String,
        yaml: String,
        expected_resource_version: String,
    },

    // 011-rbac-oidc-administration: create-only sibling of `ApplyResourceYaml`. Never
    // `.replace()`s an existing object — always `.create()`s a brand-new one (RoleBinding/
    // ClusterRoleBinding today; generic over kind like `ApplyResourceYaml` for the same
    // discovery reasons). Reuses `ApplyResourceYaml`'s identity-validation + diff-preview/
    // confirm frontend flow; `ApplyResourceYaml`'s own semantics are unchanged.
    CreateResourceYaml {
        api_version: String,
        kind: String,
        namespace: Option<String>,
        name: String,
        yaml: String,
    },

    // Custom Resource Definition browsing (generic, read-only)
    RefreshCrds,
    SelectCrdKind {
        group: String,
        kind: String,
        version: String,
    },
    RefreshCrdInstances {
        group: String,
        kind: String,
        version: String,
        namespace: Option<String>,
    },
    SelectCrdInstance {
        group: String,
        kind: String,
        version: String,
        namespace: Option<String>,
        name: String,
    },

    // Applications (synthesized, read-only)
    RefreshApplications,
    SelectApplication {
        namespace: String,
        instance: String,
    },

    // Generic related-resources resolver (read-only, any kind)
    RefreshRelatedResources {
        section: SectionKey,
        kind: String,
        namespace: Option<String>,
        name: String,
    },

    // Helm releases (read-only, reads release-storage Secret/ConfigMap objects directly)
    RefreshHelmReleases,
    SelectHelmRelease {
        namespace: String,
        name: String,
    },

    // Flux single-resource lookup (read-only; distinct from RefreshFlux's dashboard aggregate)
    SelectFluxResource {
        kind: String,
        namespace: String,
        name: String,
    },

    // Helm releases (mutate: rollback, upgrade, uninstall — 006)
    /// Compute a rollback diff-preview without mutating anything.
    PreviewHelmRollback {
        namespace: String,
        release: String,
        target_revision: i64,
    },
    /// Apply a previously previewed rollback. `base_revision` MUST match the
    /// preview's `base_revision`; a mismatch fails with `HelmMutationFailed`
    /// (stage: Apply).
    ApplyHelmRollback {
        namespace: String,
        release: String,
        target_revision: i64,
        base_revision: i64,
    },
    /// Compute an upgrade diff-preview via `helm upgrade --dry-run` without
    /// mutating anything. `values_yaml` is the (possibly edited) effective
    /// values; `chart_version` is `None` to keep the current version.
    PreviewHelmUpgrade {
        namespace: String,
        release: String,
        values_yaml: String,
        chart_version: Option<String>,
    },
    /// Apply a previously previewed upgrade. `base_revision` MUST match the
    /// preview's `base_revision`.
    ApplyHelmUpgrade {
        namespace: String,
        release: String,
        values_yaml: String,
        chart_version: Option<String>,
        base_revision: i64,
    },
    /// Uninstall a release. No preview step exists (nothing to preview).
    /// `base_revision` guards against uninstalling a release the operator's
    /// confirm dialog is stale against.
    UninstallHelmRelease {
        namespace: String,
        release: String,
        base_revision: i64,
    },

    // 038-sdk-login-ollama: provider login, secure credentials, Ollama, offline mode.
    // Every command replies with an `Ai*` event (no silent success/failure).
    /// Start a provider login: device flow (GitHub) or the key-prompt state (Claude/OpenAI).
    AiProviderLoginStart {
        provider: Provider,
    },
    /// Inbound-only API key; validated with the provider, then stored. Never echoed.
    AiProviderSubmitApiKey {
        provider: Provider,
        key: ApiKeyInput,
    },
    AiProviderLoginCancel,
    AiProviderSignOut {
        provider: Provider,
    },
    AiSetActiveProvider {
        provider: Option<Provider>,
    },
    AiSetOfflineMode {
        enabled: bool,
    },
    /// Fresh probe every time (no cache). `endpoint` overrides the saved one when present.
    AiOllamaProbe {
        endpoint: Option<String>,
    },
    AiOllamaSelectModel {
        model: String,
    },
    AiSetOllamaEndpoint {
        endpoint: String,
        allow_lan: bool,
    },
    /// Explicit opt-in to the labelled 0600 file store when no OS credential store exists.
    AiEnableFileStore {
        confirm: bool,
    },
    /// Read the current provider status (reply: `AiProviderStatus`).
    AiGetStatus,

    // Agent SDK Ops (013): in-app troubleshooting assistant. `AgentAsk` is the only
    // variant that calls out to a model provider; it never issues a cluster-mutating
    // command itself (see `mutation.rs`'s `agent_commands_never_write_cluster` test).
    /// Read the current auth mode (BYOK / Ollama / disabled / unconfigured).
    AgentGetAuthMode,
    /// Set the auth mode. Carries the raw key on the way in for `Anthropic`; never
    /// echoed back except via `AgentRevealKey` (FR-004).
    AgentSetAuthMode {
        mode: AgentAuthModeInput,
    },
    /// Return the currently configured BYOK key in full, once, only in direct response
    /// to this explicit operator action (FR-004, Principle VI).
    AgentRevealKey,
    /// Load (or start) the conversation scoped to `object`. Never triggers a model
    /// call — existing history is inert until the operator asks something new
    /// (FR-017, US4 AS3).
    AgentOpenConversation {
        object: AgentObjectRef,
    },
    /// Ask the assistant a question scoped to `object`. The only `Agent*` command that
    /// calls the configured model provider.
    AgentAsk {
        object: AgentObjectRef,
        question: String,
    },
    /// Permanently remove the persisted conversation for `object` (FR-017).
    AgentClearConversation {
        object: AgentObjectRef,
    },
    /// Mark a suggestion `Reviewed`. Pure status change — never calls a cluster
    /// command (FR-007).
    AgentReviewSuggestedAction {
        object: AgentObjectRef,
        suggestion_id: String,
    },
    /// Mark a suggestion `Declined` (dialog cancelled/closed, or card dismissed).
    AgentDeclineSuggestedAction {
        object: AgentObjectRef,
        suggestion_id: String,
    },
    /// Record the outcome of a confirmed suggestion after its *existing* mutating
    /// command (sent through the ordinary, unmodified dispatch path) has finished.
    /// Local-only: this command itself never touches the cluster.
    AgentRecordExecutedAction {
        object: AgentObjectRef,
        suggestion_id: String,
        command_summary: String,
        outcome: ActionOutcome,
    },

    // 027-fleet-rollup-dashboard: view-scoped multi-context health rollup. Both are idempotent
    // and never touch `ClusterContext.client`/`current_context` (the active context) — the
    // fleet pollers hold their own independent short-lived-per-attempt `kube::Client`s, one per
    // context returned by the same kubeconfig listing `Initialize`/`RefreshContexts` already
    // use. See `src/backend/fleet.rs`.
    /// Sent when the Fleet Dashboard view mounts. Spawns a poller for every kubeconfig context
    /// that does not already have one running; a no-op per-context if already running.
    StartFleetPolling,
    /// Sent when the Fleet Dashboard view unmounts. Aborts every running fleet poller and
    /// clears the summary cache (also invoked from `cleanup_all_tasks()` on shutdown).
    StopFleetPolling,
}

/// Direction of a `TaintNode` mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum TaintOp {
    Add,
    Remove,
}

pub type BackendCmd = BackendCmdV1;

/// A command as it travels from a shell to the backend thread: the command plus the
/// bridge-assigned [`RequestId`]. Every envelope the backend emits while handling it
/// (including the terminal `BackendEventV1::CommandFinished`) carries
/// `Correlation::Request(id)`; with `id: None` they carry `Correlation::None`.
///
/// Not part of the wire protocol: the bridge returns the id from `send_cmd`.
#[derive(Debug, Clone)]
pub struct CmdRequest {
    pub id: Option<RequestId>,
    pub cmd: BackendCmd,
}

impl CmdRequest {
    pub fn new(id: RequestId, cmd: BackendCmd) -> Self {
        Self { id: Some(id), cmd }
    }
}

/// Uncorrelated (the legacy egui app, the MCP-only shell).
impl From<BackendCmd> for CmdRequest {
    fn from(cmd: BackendCmd) -> Self {
        Self { id: None, cmd }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(cmd: BackendCmdV1) {
        let json = serde_json::to_string(&cmd).unwrap();
        let back: BackendCmdV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{back:?}"), format!("{cmd:?}"));
    }

    #[test]
    fn start_pod_exec_roundtrip() {
        roundtrip(BackendCmdV1::StartPodExec {
            namespace: "default".to_owned(),
            name: "web-0".to_owned(),
            container: Some("app".to_owned()),
        });
        roundtrip(BackendCmdV1::StartPodExec {
            namespace: "default".to_owned(),
            name: "web-0".to_owned(),
            container: None,
        });
    }

    #[test]
    fn send_pod_exec_input_roundtrip() {
        roundtrip(BackendCmdV1::SendPodExecInput {
            data: "bHM=".to_owned(),
        });
    }

    #[test]
    fn resize_pod_exec_roundtrip() {
        roundtrip(BackendCmdV1::ResizePodExec { cols: 80, rows: 24 });
    }

    #[test]
    fn node_shell_cmds_roundtrip() {
        roundtrip(BackendCmdV1::CheckNodeShellAccess {
            namespace: "default".to_owned(),
        });
        roundtrip(BackendCmdV1::StartNodeShell {
            node: "n1".to_owned(),
            namespace: "default".to_owned(),
            image: "busybox:stable".to_owned(),
            image_pull_secret: Some("regcred".to_owned()),
            deadline_seconds: Some(600),
        });
        roundtrip(BackendCmdV1::StartNodeShell {
            node: "n1".to_owned(),
            namespace: "default".to_owned(),
            image: "busybox".to_owned(),
            image_pull_secret: None,
            deadline_seconds: None,
        });
        roundtrip(BackendCmdV1::StopNodeShell {
            session_id: 3,
            namespace: "default".to_owned(),
        });
        roundtrip(BackendCmdV1::ListNodeShellOrphans { namespace: None });
        roundtrip(BackendCmdV1::ListNodeShellOrphans {
            namespace: Some("kube-system".to_owned()),
        });
        roundtrip(BackendCmdV1::DeleteNodeShellOrphans {
            pods: vec![NodeShellPodRef {
                namespace: "default".to_owned(),
                name: "fathom-node-shell-abc".to_owned(),
            }],
        });
        roundtrip(BackendCmdV1::GetNodeShellSettings);
        roundtrip(BackendCmdV1::SetNodeShellSettings {
            settings: NodeShellSettings::default(),
        });
    }

    #[test]
    fn stop_pod_exec_roundtrip() {
        roundtrip(BackendCmdV1::StopPodExec);
    }

    #[test]
    fn apply_resource_yaml_roundtrip() {
        roundtrip(BackendCmdV1::ApplyResourceYaml {
            api_version: "apps/v1".to_owned(),
            kind: "Deployment".to_owned(),
            namespace: Some("default".to_owned()),
            name: "web".to_owned(),
            yaml: "spec:\n  replicas: 3\n".to_owned(),
            expected_resource_version: "12345".to_owned(),
        });
        roundtrip(BackendCmdV1::ApplyResourceYaml {
            api_version: "v1".to_owned(),
            kind: "PersistentVolume".to_owned(),
            namespace: None,
            name: "pv-1".to_owned(),
            yaml: "spec: {}\n".to_owned(),
            expected_resource_version: "1".to_owned(),
        });
    }

    #[test]
    fn create_resource_yaml_roundtrip() {
        roundtrip(BackendCmdV1::CreateResourceYaml {
            api_version: "rbac.authorization.k8s.io/v1".to_owned(),
            kind: "RoleBinding".to_owned(),
            namespace: Some("default".to_owned()),
            name: "read-pods".to_owned(),
            yaml: "roleRef:\n  kind: Role\n  name: pod-reader\n".to_owned(),
        });
        roundtrip(BackendCmdV1::CreateResourceYaml {
            api_version: "rbac.authorization.k8s.io/v1".to_owned(),
            kind: "ClusterRoleBinding".to_owned(),
            namespace: None,
            name: "cluster-read-pods".to_owned(),
            yaml: "roleRef:\n  kind: ClusterRole\n  name: view\n".to_owned(),
        });
    }

    #[test]
    fn cordon_uncordon_node_roundtrip() {
        roundtrip(BackendCmdV1::CordonNode {
            name: "node-1".to_owned(),
        });
        roundtrip(BackendCmdV1::UncordonNode {
            name: "node-1".to_owned(),
        });
    }

    #[test]
    fn drain_node_roundtrip() {
        roundtrip(BackendCmdV1::DrainNode {
            name: "node-1".to_owned(),
            ignore_daemonsets: true,
            force_delete_empty_dir: false,
        });
    }

    #[test]
    fn cancel_drain_node_roundtrip() {
        roundtrip(BackendCmdV1::CancelDrainNode {
            name: "node-1".to_owned(),
            request_id: "req-1".to_owned(),
        });
    }

    #[test]
    fn taint_node_roundtrip() {
        roundtrip(BackendCmdV1::TaintNode {
            name: "node-1".to_owned(),
            op: TaintOp::Add,
            key: "dedicated".to_owned(),
            value: Some("gpu".to_owned()),
            effect: "NoSchedule".to_owned(),
        });
        roundtrip(BackendCmdV1::TaintNode {
            name: "node-1".to_owned(),
            op: TaintOp::Remove,
            key: "dedicated".to_owned(),
            value: None,
            effect: "NoSchedule".to_owned(),
        });
    }

    #[test]
    fn crd_commands_roundtrip() {
        roundtrip(BackendCmdV1::RefreshCrds);
        roundtrip(BackendCmdV1::SelectCrdKind {
            group: "source.toolkit.fluxcd.io".to_owned(),
            kind: "GitRepository".to_owned(),
            version: "v1".to_owned(),
        });
        roundtrip(BackendCmdV1::RefreshCrdInstances {
            group: "source.toolkit.fluxcd.io".to_owned(),
            kind: "GitRepository".to_owned(),
            version: "v1".to_owned(),
            namespace: Some("flux-system".to_owned()),
        });
        roundtrip(BackendCmdV1::SelectCrdInstance {
            group: "source.toolkit.fluxcd.io".to_owned(),
            kind: "GitRepository".to_owned(),
            version: "v1".to_owned(),
            namespace: Some("flux-system".to_owned()),
            name: "podinfo".to_owned(),
        });
    }

    #[test]
    fn application_commands_roundtrip() {
        roundtrip(BackendCmdV1::RefreshApplications);
        roundtrip(BackendCmdV1::SelectApplication {
            namespace: "default".to_owned(),
            instance: "redis".to_owned(),
        });
    }

    #[test]
    fn helm_release_commands_roundtrip() {
        roundtrip(BackendCmdV1::RefreshHelmReleases);
        roundtrip(BackendCmdV1::SelectHelmRelease {
            namespace: "cache".to_owned(),
            name: "redis".to_owned(),
        });
    }

    #[test]
    fn select_flux_resource_roundtrip() {
        roundtrip(BackendCmdV1::SelectFluxResource {
            kind: "HelmRelease".to_owned(),
            namespace: "flux-system".to_owned(),
            name: "redis".to_owned(),
        });
    }

    #[test]
    fn refresh_related_resources_roundtrip() {
        roundtrip(BackendCmdV1::RefreshRelatedResources {
            section: crate::protocol::section_keys::nodes(),
            kind: "Node".to_owned(),
            namespace: None,
            name: "node-1".to_owned(),
        });
        roundtrip(BackendCmdV1::RefreshRelatedResources {
            section: crate::protocol::section_keys::workloads_pods(),
            kind: "Pod".to_owned(),
            namespace: Some("default".to_owned()),
            name: "web-0".to_owned(),
        });
    }

    fn sample_object() -> AgentObjectRef {
        AgentObjectRef {
            kind: "Pod".to_owned(),
            namespace: "default".to_owned(),
            name: "api-0".to_owned(),
        }
    }

    #[test]
    fn ai_commands_roundtrip() {
        roundtrip(BackendCmdV1::AiProviderLoginStart {
            provider: Provider::Github,
        });
        roundtrip(BackendCmdV1::AiProviderLoginCancel);
        roundtrip(BackendCmdV1::AiProviderSignOut {
            provider: Provider::Openai,
        });
        roundtrip(BackendCmdV1::AiSetActiveProvider { provider: None });
        roundtrip(BackendCmdV1::AiSetActiveProvider {
            provider: Some(Provider::Ollama),
        });
        roundtrip(BackendCmdV1::AiSetOfflineMode { enabled: true });
        roundtrip(BackendCmdV1::AiOllamaProbe { endpoint: None });
        roundtrip(BackendCmdV1::AiOllamaProbe {
            endpoint: Some("http://127.0.0.1:11434".to_owned()),
        });
        roundtrip(BackendCmdV1::AiOllamaSelectModel {
            model: "llama3.2".to_owned(),
        });
        roundtrip(BackendCmdV1::AiSetOllamaEndpoint {
            endpoint: "http://10.0.0.5:11434".to_owned(),
            allow_lan: true,
        });
        roundtrip(BackendCmdV1::AiEnableFileStore { confirm: true });
        roundtrip(BackendCmdV1::AiGetStatus);
    }

    #[test]
    fn ai_submit_key_roundtrips_inbound_but_debug_is_redacted() {
        let cmd = BackendCmdV1::AiProviderSubmitApiKey {
            provider: Provider::Claude,
            key: ApiKeyInput("sk-ant-supersecret".to_owned()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(
            json.contains("sk-ant-supersecret"),
            "inbound wire carries the key"
        );
        let back: BackendCmdV1 = serde_json::from_str(&json).unwrap();
        match back {
            BackendCmdV1::AiProviderSubmitApiKey { provider, key } => {
                assert_eq!(provider, Provider::Claude);
                assert_eq!(key.0, "sk-ant-supersecret");
            }
            other => panic!("wrong variant {other:?}"),
        }
        assert!(!format!("{cmd:?}").contains("supersecret"));
    }

    #[test]
    fn agent_get_auth_mode_roundtrip() {
        roundtrip(BackendCmdV1::AgentGetAuthMode);
    }

    #[test]
    fn agent_set_auth_mode_roundtrip() {
        roundtrip(BackendCmdV1::AgentSetAuthMode {
            mode: AgentAuthModeInput::Anthropic {
                key: "sk-ant-test".to_owned(),
            },
        });
        roundtrip(BackendCmdV1::AgentSetAuthMode {
            mode: AgentAuthModeInput::Ollama {
                endpoint: "http://localhost:11434".to_owned(),
                model: "llama3.1".to_owned(),
            },
        });
        roundtrip(BackendCmdV1::AgentSetAuthMode {
            mode: AgentAuthModeInput::Disabled,
        });
    }

    #[test]
    fn agent_reveal_key_roundtrip() {
        roundtrip(BackendCmdV1::AgentRevealKey);
    }

    #[test]
    fn agent_open_conversation_roundtrip() {
        roundtrip(BackendCmdV1::AgentOpenConversation {
            object: sample_object(),
        });
    }

    #[test]
    fn agent_ask_roundtrip() {
        roundtrip(BackendCmdV1::AgentAsk {
            object: sample_object(),
            question: "What's wrong with this pod?".to_owned(),
        });
    }

    #[test]
    fn agent_clear_conversation_roundtrip() {
        roundtrip(BackendCmdV1::AgentClearConversation {
            object: sample_object(),
        });
    }

    #[test]
    fn agent_review_and_decline_suggested_action_roundtrip() {
        roundtrip(BackendCmdV1::AgentReviewSuggestedAction {
            object: sample_object(),
            suggestion_id: "sugg-1".to_owned(),
        });
        roundtrip(BackendCmdV1::AgentDeclineSuggestedAction {
            object: sample_object(),
            suggestion_id: "sugg-1".to_owned(),
        });
    }

    #[test]
    fn agent_record_executed_action_roundtrip() {
        roundtrip(BackendCmdV1::AgentRecordExecutedAction {
            object: sample_object(),
            suggestion_id: "sugg-1".to_owned(),
            command_summary: "RestartDeployment default/api".to_owned(),
            outcome: ActionOutcome::Success,
        });
        roundtrip(BackendCmdV1::AgentRecordExecutedAction {
            object: sample_object(),
            suggestion_id: "sugg-1".to_owned(),
            command_summary: "RestartDeployment default/api".to_owned(),
            outcome: ActionOutcome::Error {
                message: "deployments.apps \"api\" not found".to_owned(),
            },
        });
    }

    #[test]
    fn oidc_cmd_roundtrips() {
        use crate::protocol::{RoleAssignmentRoleRef, RoleAssignmentSubject, RoleRefKind};
        roundtrip(BackendCmdV1::LoadOidcConfig);
        let issuer: OidcIssuerConfig = serde_json::from_str(
            r#"{"id":"rauthy","issuer_url":"https://i/","client_id":"fathom-desktop"}"#,
        )
        .unwrap();
        roundtrip(BackendCmdV1::OidcSaveIssuer { issuer });
        roundtrip(BackendCmdV1::OidcDeleteIssuer {
            issuer_id: "rauthy".to_owned(),
        });
        roundtrip(BackendCmdV1::OidcSetRoleMap {
            role_map: FathomRoleMap::default(),
        });
        for method in [OidcSignInMethod::Browser, OidcSignInMethod::DeviceCode] {
            roundtrip(BackendCmdV1::OidcSignIn {
                issuer_id: "rauthy".to_owned(),
                method,
            });
        }
        roundtrip(BackendCmdV1::OidcCancelSignIn {
            issuer_id: "rauthy".to_owned(),
        });
        roundtrip(BackendCmdV1::OidcSignOut {
            issuer_id: "rauthy".to_owned(),
            end_session: true,
        });
        roundtrip(BackendCmdV1::SetContextAuthMode {
            context: "prod".to_owned(),
            mode: ContextAuthMode::Oidc,
            issuer_id: Some("rauthy".to_owned()),
        });
        roundtrip(BackendCmdV1::SetContextAuthMode {
            context: "prod".to_owned(),
            mode: ContextAuthMode::Kubeconfig,
            issuer_id: None,
        });
        roundtrip(BackendCmdV1::IdpSetAdminKey {
            issuer_id: "rauthy".to_owned(),
            api_key: "k".to_owned(),
        });
        roundtrip(BackendCmdV1::IdpRefreshDirectory {
            issuer_id: "rauthy".to_owned(),
        });
        roundtrip(BackendCmdV1::ResolveCapabilities {
            context: "prod".to_owned(),
            namespace: Some("default".to_owned()),
        });
        roundtrip(BackendCmdV1::ResolveCapabilities {
            context: "prod".to_owned(),
            namespace: None,
        });
        roundtrip(BackendCmdV1::CheckCapability {
            query: CapabilityQuery {
                context: "prod".to_owned(),
                namespace: None,
                verb: "create".to_owned(),
                resource: "clusterrolebindings".to_owned(),
                api_group: Some("rbac.authorization.k8s.io".to_owned()),
                subresource: None,
            },
        });
        roundtrip(BackendCmdV1::LinkTeamProfile {
            group: "platform".to_owned(),
            profile_id: "p1".to_owned(),
        });
        roundtrip(BackendCmdV1::UnlinkTeamProfile {
            group: "platform".to_owned(),
            profile_id: "p1".to_owned(),
        });
        roundtrip(BackendCmdV1::PreviewRoleAssignment {
            draft: RoleAssignmentDraft {
                issuer_id: "rauthy".to_owned(),
                subject: RoleAssignmentSubject::Group {
                    name: "dev".to_owned(),
                },
                role_ref: RoleAssignmentRoleRef {
                    kind: RoleRefKind::ClusterRole,
                    name: "view".to_owned(),
                },
                namespace: None,
                binding_name: Some("dev-view".to_owned()),
            },
        });
        roundtrip(BackendCmdV1::RefreshOrphanedSubjects {
            context: "prod".to_owned(),
            issuer_id: "rauthy".to_owned(),
        });
    }

    #[test]
    fn team_cmd_roundtrips() {
        roundtrip(BackendCmdV1::LoadClusterSettings {
            context: "prod".to_owned(),
        });
        roundtrip(BackendCmdV1::SaveClusterSettings {
            context: "prod".to_owned(),
            settings: ClusterSettings::default(),
        });
        roundtrip(BackendCmdV1::SetTeamStorePath {
            path: Some("/mnt/team-fathom".to_owned()),
        });
        roundtrip(BackendCmdV1::SetTeamStorePath { path: None });
        roundtrip(BackendCmdV1::RefreshTeamStoreStatus);
        roundtrip(BackendCmdV1::RefreshConnectionProfiles);
        roundtrip(BackendCmdV1::PublishConnectionProfile {
            context_alias: "prod-cluster".to_owned(),
            api_server: "https://cluster.example.com:6443".to_owned(),
            default_namespace: Some("default".to_owned()),
        });
        roundtrip(BackendCmdV1::AdoptConnectionProfile {
            profile_id: "abc123".to_owned(),
        });
        roundtrip(BackendCmdV1::RemoveConnectionProfile {
            profile_id: "abc123".to_owned(),
        });
        roundtrip(BackendCmdV1::PublishOrgClusterSettings {
            context: "prod".to_owned(),
        });
        roundtrip(BackendCmdV1::AcceptOrgClusterSettings {
            context: "prod".to_owned(),
        });
        roundtrip(BackendCmdV1::DismissOrgClusterSettings {
            context: "prod".to_owned(),
        });
    }

    #[test]
    fn mcp_server_cmd_roundtrips() {
        roundtrip(BackendCmdV1::LoadMcpServerStatus);
        roundtrip(BackendCmdV1::SetMcpServerEnabled { enabled: true });
        roundtrip(BackendCmdV1::SetMcpServerEnabled { enabled: false });
    }

    #[test]
    fn fleet_cmd_roundtrips() {
        roundtrip(BackendCmdV1::StartFleetPolling);
        roundtrip(BackendCmdV1::StopFleetPolling);
    }
}
