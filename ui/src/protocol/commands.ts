/**
 * Command builders and section keys shared by the store, `useBackend`, and the fake transport.
 *
 * Section keys mirror `section_keys` in `src/protocol/v1/route.rs` (SectionKey is an opaque
 * string on the wire, so ts-rs cannot export the values).
 */
import type { BackendCmd, SectionKey } from './types'

export const SECTION = {
  system: 'system',
  replicationcontrollers: 'workloads/replicationcontrollers',
  endpoints: 'network/endpoints',
  ingresses: 'network/ingress',
  ingressClasses: 'network/ingressclasses',
  networkPolicies: 'network/networkpolicies',
  persistentVolumes: 'storage/persistentvolumes',
  persistentVolumeClaims: 'storage/persistentvolumeclaims',
  storageClasses: 'storage/storageclasses',
  resourceQuotas: 'config/resourcequotas',
  limitRanges: 'config/limitranges',
  horizontalPodAutoscalers: 'config/horizontalpodautoscalers',
  verticalPodAutoscalers: 'config/verticalpodautoscalers',
  priorityClasses: 'config/priorityclasses',
  runtimeClasses: 'config/runtimeclasses',
  leases: 'config/leases',
  mutatingWebhooks: 'config/mutatingwebhooks',
  validatingWebhooks: 'config/validatingwebhooks',
  overview: 'overview',
  helmReleases: 'helm/releases',
  argo: 'integrations/argo',
  metrics: 'integrations/metrics',
  grafana: 'integrations/grafana',
  flux: 'integrations/flux',
  events: 'events',
  namespaces: 'namespaces',
  pods: 'workloads/pods',
  deployments: 'workloads/deployments',
  customResources: 'custom-resources',
  statefulsets: 'workloads/statefulsets',
  daemonsets: 'workloads/daemonsets',
  jobs: 'workloads/jobs',
  cronjobs: 'workloads/cronjobs',
  replicasets: 'workloads/replicasets',
  configmaps: 'config/configmaps',
  secrets: 'config/secrets',
  poddisruptionbudgets: 'config/poddisruptionbudgets',
  nodes: 'nodes',
  applications: 'applications',
  services: 'network/services',
  portForwards: 'network/portforwards',
  /** 020-gatekeeper-policy-pane: read-only OPA Gatekeeper constraint compliance view. */
  gatekeeper: 'integrations/gatekeeper',
  /** 014-teams-organizations: local §10 Cluster Settings + org-level sync affordances. */
  clusterSettings: 'integrations/cluster-settings',
  /** 014-teams-organizations: shared, non-secret connection profiles + team-store status. */
  teamProfiles: 'team/profiles',
  /** 033 US3: IdP groups/users as teams. */
  teams: 'team/teams',
  /** 026-mutation-audit-log-viewer: local mutation history viewer. */
  auditLog: 'audit-log',
  // 011-rbac-oidc-administration
  roles: 'access/roles',
  roleBindings: 'access/rolebindings',
  clusterRoles: 'access/clusterroles',
  clusterRoleBindings: 'access/clusterrolebindings',
  namespaceAccess: 'access/namespace-access',
  oidcConfig: 'access/oidc-config',
} as const satisfies Record<string, SectionKey>

/** Commands sent once the event subscription is up (mirrors egui's startup in src/app/mod.rs). */
export const BOOTSTRAP_CMDS: readonly BackendCmd[] = [
  { type: 'LoadUiConfig' },
  { type: 'RefreshContexts' },
  { type: 'LoadOidcConfig' },
  // Connects to the current context, then emits ContextChanged, namespaces, pods, deployments.
  { type: 'Initialize' },
]

const REFRESH: Partial<Record<SectionKey, BackendCmd>> = {
  [SECTION.pods]: { type: 'RefreshPods' },
  [SECTION.deployments]: { type: 'RefreshDeployments' },
  // 017-cluster-linter: already-shipped backend refreshes with no descriptor yet (research R2).
  [SECTION.statefulsets]: { type: 'RefreshStatefulSets' },
  [SECTION.daemonsets]: { type: 'RefreshDaemonSets' },
  [SECTION.jobs]: { type: 'RefreshJobs' },
  [SECTION.cronjobs]: { type: 'RefreshCronJobs' },
  [SECTION.replicasets]: { type: 'RefreshReplicaSets' },
  [SECTION.configmaps]: { type: 'RefreshConfigMaps' },
  [SECTION.secrets]: { type: 'RefreshSecrets' },
  [SECTION.poddisruptionbudgets]: { type: 'RefreshPodDisruptionBudgets' },
  [SECTION.events]: { type: 'RefreshEvents' },
  [SECTION.replicationcontrollers]: { type: 'RefreshReplicationControllers' },
  [SECTION.endpoints]: { type: 'RefreshEndpoints' },
  [SECTION.ingresses]: { type: 'RefreshIngresses' },
  [SECTION.ingressClasses]: { type: 'RefreshIngressClasses' },
  [SECTION.networkPolicies]: { type: 'RefreshNetworkPolicies' },
  [SECTION.persistentVolumes]: { type: 'RefreshPersistentVolumes' },
  [SECTION.persistentVolumeClaims]: { type: 'RefreshPersistentVolumeClaims' },
  [SECTION.storageClasses]: { type: 'RefreshStorageClasses' },
  [SECTION.resourceQuotas]: { type: 'RefreshResourceQuotas' },
  [SECTION.limitRanges]: { type: 'RefreshLimitRanges' },
  [SECTION.horizontalPodAutoscalers]: { type: 'RefreshHorizontalPodAutoscalers' },
  [SECTION.verticalPodAutoscalers]: { type: 'RefreshVerticalPodAutoscalers' },
  [SECTION.priorityClasses]: { type: 'RefreshPriorityClasses' },
  [SECTION.runtimeClasses]: { type: 'RefreshRuntimeClasses' },
  [SECTION.leases]: { type: 'RefreshLeases' },
  [SECTION.mutatingWebhooks]: { type: 'RefreshMutatingWebhookConfigurations' },
  [SECTION.validatingWebhooks]: { type: 'RefreshValidatingWebhookConfigurations' },
  [SECTION.namespaces]: { type: 'RefreshNamespaces' },
  // US3: the CRD catalog itself; instance lists/reads are sent directly by
  // CustomResourcesSection (they need a group/kind/version the section chooses).
  [SECTION.customResources]: { type: 'RefreshCrds' },
  [SECTION.nodes]: { type: 'RefreshNodes' },
  [SECTION.applications]: { type: 'RefreshApplications' },
  [SECTION.services]: { type: 'RefreshServices' },
  [SECTION.portForwards]: { type: 'RefreshPortForwards' },
  [SECTION.auditLog]: { type: 'RefreshAuditLog' },
  // 011-rbac-oidc-administration: list-section refreshes (namespace-access/oidc-config are
  // lookup/generator panels, not list sections, so they have no entry here).
  [SECTION.roles]: { type: 'RefreshRoles' },
  [SECTION.roleBindings]: { type: 'RefreshRoleBindings' },
  [SECTION.clusterRoles]: { type: 'RefreshClusterRoles' },
  [SECTION.clusterRoleBindings]: { type: 'RefreshClusterRoleBindings' },
}

/** The refresh command for a list section, if the UI knows one yet. */
export function refreshCmd(section: SectionKey): BackendCmd | undefined {
  return REFRESH[section]
}

/**
 * The command that reads one object's details. The reply is a `ResourceRead` event whose
 * payload carries `raw_json`, which is also the source for the YAML view (there is no separate
 * YAML command in protocol v1).
 */
export function selectCmd(kind: string, namespace: string, name: string): BackendCmd | undefined {
  switch (kind) {
    case 'Pod':
      return { type: 'SelectPod', data: { namespace, name } }
    case 'Deployment':
      return { type: 'SelectDeployment', data: { namespace, name } }
    case 'StatefulSet':
      return { type: 'SelectStatefulSet', data: { namespace, name } }
    case 'DaemonSet':
      return { type: 'SelectDaemonSet', data: { namespace, name } }
    case 'ReplicaSet':
      return { type: 'SelectReplicaSet', data: { namespace, name } }
    case 'PodDisruptionBudget':
      return { type: 'SelectPodDisruptionBudget', data: { namespace, name } }
    case 'Node':
      return { type: 'SelectNode', data: { name } }
    case 'Service':
      return { type: 'SelectService', data: { namespace, name } }
    // 034 batch0: remaining kinds with backend Select* commands.
    case 'ReplicationController':
      return { type: 'SelectReplicationController', data: { namespace, name } }
    case 'Job':
      return { type: 'SelectJob', data: { namespace, name } }
    case 'CronJob':
      return { type: 'SelectCronJob', data: { namespace, name } }
    case 'Endpoint':
      return { type: 'SelectEndpoint', data: { namespace, name } }
    case 'Ingress':
      return { type: 'SelectIngress', data: { namespace, name } }
    case 'IngressClass':
      return { type: 'SelectIngressClass', data: { name } }
    case 'NetworkPolicy':
      return { type: 'SelectNetworkPolicy', data: { namespace, name } }
    case 'PersistentVolume':
      return { type: 'SelectPersistentVolume', data: { name } }
    case 'PersistentVolumeClaim':
      return { type: 'SelectPersistentVolumeClaim', data: { namespace, name } }
    case 'StorageClass':
      return { type: 'SelectStorageClass', data: { name } }
    case 'ConfigMap':
      return { type: 'SelectConfigMap', data: { namespace, name } }
    case 'Secret':
      return { type: 'SelectSecret', data: { namespace, name } }
    case 'ResourceQuota':
      return { type: 'SelectResourceQuota', data: { namespace, name } }
    case 'LimitRange':
      return { type: 'SelectLimitRange', data: { namespace, name } }
    case 'HorizontalPodAutoscaler':
      return { type: 'SelectHorizontalPodAutoscaler', data: { namespace, name } }
    case 'VerticalPodAutoscaler':
      return { type: 'SelectVerticalPodAutoscaler', data: { namespace, name } }
    case 'PriorityClass':
      return { type: 'SelectPriorityClass', data: { name } }
    case 'RuntimeClass':
      return { type: 'SelectRuntimeClass', data: { name } }
    case 'Lease':
      return { type: 'SelectLease', data: { namespace, name } }
    case 'MutatingWebhookConfiguration':
      return { type: 'SelectMutatingWebhookConfiguration', data: { name } }
    case 'ValidatingWebhookConfiguration':
      return { type: 'SelectValidatingWebhookConfiguration', data: { name } }
    // 011-rbac-oidc-administration: cross-link support (e.g. an Event's involvedObject) for
    // the four RBAC kinds; the descriptors themselves build these same commands directly in
    // their own `read()` (see `resources/roles.tsx` etc.), this is only the generic fallback.
    case 'Role':
      return { type: 'SelectRole', data: { namespace, name } }
    case 'RoleBinding':
      return { type: 'SelectRoleBinding', data: { namespace, name } }
    case 'ClusterRole':
      return { type: 'SelectClusterRole', data: { name } }
    case 'ClusterRoleBinding':
      return { type: 'SelectClusterRoleBinding', data: { name } }
    default:
      return undefined
  }
}

/** `null` means all namespaces, which the backend encodes as an empty filter. */
export function namespaceFilterCmd(ns: string | null): BackendCmd {
  return { type: 'SetNamespaceFilter', data: { namespaces: ns === null ? [] : [ns] } }
}
