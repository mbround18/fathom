/**
 * Navigator groups (T065), in the egui app's order, keyed by the backend's section keys
 * (src/protocol/v1/route.rs `section_keys`). Labels for ported kinds come from the descriptor
 * registry; the rest use the egui labels until their descriptors land (T068+).
 */
import type { SectionKey } from '@/protocol/types'

import { descriptorFor } from './index'

export interface NavGroup {
  /** Also the persisted `expanded_groups` value, so keep it stable. */
  label: string
  /** Decorative glyph (aria-hidden). */
  kanji: string
  sections: readonly { key: SectionKey; label: string }[]
}

const g = (label: string, kanji: string, sections: [SectionKey, string][]): NavGroup => ({
  label,
  kanji,
  sections: sections.map(([key, fallback]) => ({
    key,
    label: descriptorFor(key)?.label ?? fallback,
  })),
})

export const NAV_GROUPS: readonly NavGroup[] = [
  g('Overview', '概', [
    ['overview', 'Overview'],
    ['applications', 'Applications'],
    ['helm/releases', 'Helm Releases'],
    ['nodes', 'Nodes'],
    ['namespaces', 'Namespaces'],
    ['events', 'Events'],
  ]),
  g('Workloads', '器', [
    ['workloads/pods', 'Pods'],
    ['workloads/deployments', 'Deployments'],
    ['workloads/statefulsets', 'StatefulSets'],
    ['workloads/daemonsets', 'DaemonSets'],
    ['workloads/jobs', 'Jobs'],
    ['workloads/cronjobs', 'CronJobs'],
  ]),
  g('Network', '網', [
    ['network/services', 'Services'],
    ['network/endpoints', 'Endpoints'],
    ['network/ingress', 'Ingress'],
    ['network/ingressclasses', 'IngressClasses'],
    ['network/networkpolicies', 'NetworkPolicies'],
    ['network/portforwards', 'Port Forwards'],
  ]),
  g('Storage', '蔵', [
    ['storage/persistentvolumes', 'PersistentVolumes'],
    ['storage/persistentvolumeclaims', 'PersistentVolumeClaims'],
    ['storage/storageclasses', 'StorageClasses'],
  ]),
  g('Config', '設', [
    ['config/configmaps', 'ConfigMaps'],
    ['config/secrets', 'Secrets'],
    ['config/resourcequotas', 'Resource Quotas'],
    ['config/limitranges', 'Limit Ranges'],
    ['config/horizontalpodautoscalers', 'Horizontal Pod Autoscalers'],
    ['config/verticalpodautoscalers', 'Vertical Pod Autoscalers'],
    ['config/poddisruptionbudgets', 'Pod Disruption Budgets'],
    ['config/priorityclasses', 'Priority Classes'],
    ['config/runtimeclasses', 'Runtime Classes'],
    ['config/leases', 'Leases'],
    ['config/mutatingwebhooks', 'Mutating Webhook Configurations'],
    ['config/validatingwebhooks', 'Validating Webhook Configurations'],
  ]),
  g('Access Control', '鍵', [
    ['access/roles', 'Roles'],
    ['access/rolebindings', 'RoleBindings'],
    ['access/clusterroles', 'ClusterRoles'],
    ['access/clusterrolebindings', 'ClusterRoleBindings'],
    ['access/namespace-access', 'Namespace Access'],
    ['access/oidc-config', 'OIDC Config'],
  ]),
  g('Custom Resources', '拡', [['custom-resources', 'Custom Resources']]),
  g('Integrations', '連', [
    ['integrations/argo', 'Argo'],
    ['integrations/metrics', 'Metrics'],
    ['integrations/grafana', 'Grafana'],
    ['integrations/flux', 'Flux'],
  ]),
  // 014-teams-organizations: local §10 Cluster Settings (+ org-level sync affordances) and
  // shared, non-secret connection profiles.
  g('Team', '団', [
    ['integrations/cluster-settings', 'Cluster Settings'],
    ['team/profiles', 'Connection Profiles'],
    // 026-mutation-audit-log-viewer
    ['audit-log', 'Audit Log'],
  ]),
]

/** Groups open on first run, before `UiConfigLoaded` says otherwise. */
export const DEFAULT_EXPANDED_GROUPS: readonly string[] = ['Overview', 'Workloads']

const LABELS = new Map(NAV_GROUPS.flatMap((grp) => grp.sections.map((s) => [s.key, s.label])))

/** Display label for any section key (falls back to the key itself). */
export function sectionLabel(key: SectionKey): string {
  return LABELS.get(key) ?? key
}
