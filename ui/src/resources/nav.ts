/**
 * 043 navigator model (spec FR-020 to FR-023): plain-language groups, a one-line description
 * per entry, advanced kinds under a per-group "More", keyed by the backend's section keys
 * (src/protocol/v1/route.rs `section_keys`). Kind names stay as the labels (Principle I): labels
 * for ported kinds come from the descriptor registry; the rest use the fallback labels here.
 *
 * `NAV_MODEL` is the one source for the tree, the palette index and the coverage tests.
 * `NAV_GROUPS` is the flat, render-ready view older callers use.
 */
import type { SectionKey } from '@/protocol/types'

import { descriptorFor } from './index'

/** What an entry's `visible` rule may look at. */
export interface NavVisibilityContext {
  hosted: boolean
  /** Desktop app (Tauri), as opposed to a browser. */
  desktop: boolean
  /** Integration detection for the active context; `undefined` = not detected yet. */
  installed: (key: SectionKey) => boolean | undefined
}

export interface NavEntry {
  section: SectionKey
  label: string
  /** At most 90 characters, plain language, not repeating the label. */
  description: string
  /** Rendered under the group's "More" toggle. */
  advanced?: true
  /** The integration this entry depends on (its own section key). */
  integration?: SectionKey
  /** Hidden when this returns false. */
  visible?: (ctx: NavVisibilityContext) => boolean
}

export interface NavGroup {
  /** Also the persisted `expanded_groups` value, so keep it stable. */
  label: string
  /** Decorative glyph (aria-hidden). */
  kanji: string
  defaultExpanded: boolean
  entries: readonly NavEntry[]
}

type EntryExtra = Partial<Omit<NavEntry, 'section' | 'label' | 'description'>>
type EntrySpec = [SectionKey, string, string, EntryExtra?]

const entry = ([section, fallback, description, extra]: EntrySpec): NavEntry => ({
  section,
  label: descriptorFor(section)?.label ?? fallback,
  description,
  ...extra,
})

const group = (
  label: string,
  kanji: string,
  entries: EntrySpec[],
  defaultExpanded = false,
): NavGroup => ({ label, kanji, defaultExpanded, entries: entries.map(entry) })

const MORE = { advanced: true } as const

export const VELERO_SECTION: SectionKey = 'integrations/velero'

/** Velero is the one integration hidden until it is detected (FR-036); unknown counts as hidden. */
export function integrationVisible(key: SectionKey, installed: boolean | undefined): boolean {
  return key === VELERO_SECTION ? installed === true : true
}

const integration = (key: SectionKey): EntryExtra => ({
  integration: key,
  ...(key === VELERO_SECTION
    ? { visible: (ctx: NavVisibilityContext) => integrationVisible(key, ctx.installed(key)) }
    : {}),
})

export const NAV_MODEL: readonly NavGroup[] = [
  group(
    'Cluster',
    '概',
    [
      ['overview', 'Overview', 'Health, capacity and recent problems for the whole cluster'],
      ['topology', 'Topology', 'A map of how the workloads, services and nodes connect'],
      ['nodes', 'Nodes', 'The machines that run your workloads, with their capacity and health'],
      ['namespaces', 'Namespaces', 'Folders that separate teams and apps inside the cluster'],
      ['events', 'Events', 'Recent warnings and changes Kubernetes reported'],
      ['linter', 'Linter', 'Common misconfigurations found in what is running now'],
      [
        'cluster-history',
        'Cluster history',
        'What changed while Fathom was closed, recorded by the optional daemon',
      ],
    ],
    true,
  ),
  group(
    'Workloads',
    '器',
    [
      ['applications', 'Applications', 'Workloads grouped into the apps they belong to'],
      ['helm/releases', 'Helm Releases', 'Charts installed with Helm, their versions and history'],
      ['workloads/pods', 'Pods', 'The running containers, one or more per app copy'],
      ['workloads/deployments', 'Deployments', 'Keep a set number of identical app copies running'],
      [
        'workloads/statefulsets',
        'StatefulSets',
        'Apps whose copies keep a stable name and storage',
      ],
      ['workloads/daemonsets', 'DaemonSets', 'Run one copy on every node, such as agents'],
      ['workloads/jobs', 'Jobs', 'Tasks that run to completion once'],
      ['workloads/cronjobs', 'CronJobs', 'Tasks that run on a schedule'],
      [
        'workloads/replicasets',
        'ReplicaSets',
        'The copies a Deployment manages; usually read through it',
        MORE,
      ],
      [
        'workloads/replicationcontrollers',
        'ReplicationControllers',
        'The older way to keep copies running, replaced by Deployments',
        MORE,
      ],
    ],
    true,
  ),
  group('Networking', '網', [
    ['network/services', 'Services', 'Stable addresses that route traffic to your pods'],
    ['network/ingress', 'Ingress', 'Rules that bring outside web traffic into the cluster'],
    ['network/networkpolicies', 'NetworkPolicies', 'Firewall rules for which pods may talk'],
    ['network/portforwards', 'Port Forwards', 'Reach a pod or service from this computer'],
    ['network/endpoints', 'Endpoints', 'The pod addresses currently behind each service', MORE],
    [
      'network/ingressclasses',
      'IngressClasses',
      'Which ingress controller handles which rules',
      MORE,
    ],
  ]),
  group('Storage', '蔵', [
    [
      'storage/persistentvolumeclaims',
      'PersistentVolumeClaims',
      'Requests for disk space made by your apps',
    ],
    ['storage/persistentvolumes', 'PersistentVolumes', 'The actual disks that back those requests'],
    ['storage/storageclasses', 'StorageClasses', 'The kinds of disk the cluster can provide'],
  ]),
  group('Configuration', '設', [
    ['config/configmaps', 'ConfigMaps', 'Plain settings and files handed to your apps'],
    ['config/secrets', 'Secrets', 'Passwords, tokens and keys handed to your apps'],
    [
      'config/resourcequotas',
      'Resource Quotas',
      'Caps on how much CPU, memory and objects a namespace may use',
      MORE,
    ],
    ['config/limitranges', 'Limit Ranges', 'Default and maximum sizes for each container', MORE],
    [
      'config/priorityclasses',
      'Priority Classes',
      'Which workloads win when the cluster runs short of room',
      MORE,
    ],
    [
      'config/runtimeclasses',
      'Runtime Classes',
      'Which container runtime a pod is started with',
      MORE,
    ],
    ['config/leases', 'Leases', 'Locks components use to pick a leader and report liveness', MORE],
    [
      'config/mutatingwebhooks',
      'Mutating Webhook Configurations',
      'Services that may change objects as they are created',
      MORE,
    ],
    [
      'config/validatingwebhooks',
      'Validating Webhook Configurations',
      'Services that may reject objects as they are created',
      MORE,
    ],
  ]),
  group('Scaling', '伸', [
    [
      'config/horizontalpodautoscalers',
      'Horizontal Pod Autoscalers',
      'Add or remove app copies as load changes',
    ],
    [
      'config/verticalpodautoscalers',
      'Vertical Pod Autoscalers',
      'Resize the CPU and memory each copy asks for',
    ],
    [
      'config/poddisruptionbudgets',
      'Pod Disruption Budgets',
      'How many copies must stay up during maintenance',
    ],
  ]),
  group('Access Control', '鍵', [
    ['access/roles', 'Roles', 'Sets of permissions inside one namespace'],
    ['access/rolebindings', 'RoleBindings', 'Who gets a role in a namespace'],
    ['access/clusterroles', 'ClusterRoles', 'Sets of permissions that can span the whole cluster'],
    ['access/clusterrolebindings', 'ClusterRoleBindings', 'Who gets a cluster-wide role'],
    [
      'access/namespace-access',
      'Namespace Access',
      'Who can do what in each namespace, at a glance',
    ],
    [
      'access/oidc-config',
      'OIDC Config',
      'How the cluster trusts sign-in from an identity provider',
    ],
  ]),
  group('Custom Resources', '拡', [
    [
      'custom-resources',
      'Custom Resources',
      'Object types added by the tools installed in this cluster',
    ],
  ]),
  group('Integrations', '連', [
    [
      'integrations/argo',
      'Argo',
      'GitOps apps kept in sync with a Git repository',
      integration('integrations/argo'),
    ],
    [
      'integrations/flux',
      'Flux',
      'Git sources and releases synced to the cluster automatically',
      integration('integrations/flux'),
    ],
    [
      'integrations/knative',
      'Knative',
      'Serverless services that scale to zero',
      integration('integrations/knative'),
    ],
    [
      'integrations/keda',
      'KEDA Autoscalers',
      'Scale workloads on queue length and other events',
      integration('integrations/keda'),
    ],
    [
      VELERO_SECTION,
      'Velero',
      'Backups and restores of cluster objects and volumes',
      integration(VELERO_SECTION),
    ],
    [
      'integrations/cert-manager',
      'Certificates',
      'TLS certs that cert-manager issues and renews',
      integration('integrations/cert-manager'),
    ],
    [
      'integrations/gatekeeper',
      'Gatekeeper',
      'Policies that block objects breaking your rules',
      integration('integrations/gatekeeper'),
    ],
    [
      'integrations/hubble',
      'Hubble',
      'Live network flows between pods, from Cilium',
      integration('integrations/hubble'),
    ],
    [
      'integrations/vulnerabilities',
      'Vulnerabilities',
      'Known security flaws in your container images, from Trivy',
      integration('integrations/vulnerabilities'),
    ],
    [
      'integrations/metrics',
      'Metrics',
      'CPU, memory and custom charts from Prometheus',
      integration('integrations/metrics'),
    ],
    [
      'integrations/grafana',
      'Grafana',
      'Dashboards from an in-cluster dashboard server',
      integration('integrations/grafana'),
    ],
  ]),
  group('Settings', '調', [
    [
      'integrations/cluster-settings',
      'Cluster Settings',
      'Options for the selected context, such as metrics and the daemon',
    ],
    ['settings/sign-in', 'Sign-in', 'Identity providers and how each context logs you in'],
    ['team/teams', 'Teams', 'Organizations and the people you share work with'],
    ['team/profiles', 'Connection Profiles', 'Shared, non-secret connection details for your team'],
    ['settings/app', 'App settings', 'Appearance, AI assist and other app-wide options'],
    [
      'settings/local-clusters',
      'Local clusters',
      'Create and remove kind clusters on this computer',
      { visible: (ctx) => ctx.desktop && !ctx.hosted },
    ],
    ['audit-log', 'Audit Log', 'Every change Fathom made to a cluster, and who made it'],
  ]),
]

/** Whether `entry` is shown for `ctx` (Velero until detected, Local clusters off-desktop). */
export function entryVisible(e: NavEntry, ctx: NavVisibilityContext): boolean {
  return e.visible === undefined || e.visible(ctx)
}

/**
 * Persisted `expanded_groups` labels from before 043 and the groups they now mean (FR-023).
 * Old labels are mapped on read and rewritten only on the next change.
 */
export const LEGACY_GROUP_LABELS: Readonly<Record<string, readonly string[]>> = {
  Overview: ['Cluster'],
  Network: ['Networking'],
  Config: ['Configuration', 'Scaling'],
  Team: ['Settings'],
}

/** Map old group labels to the new ones, keeping order and dropping duplicates. */
export function mapLegacyGroups(labels: readonly string[]): string[] {
  const out: string[] = []
  for (const label of labels)
    for (const mapped of LEGACY_GROUP_LABELS[label] ?? [label])
      if (!out.includes(mapped)) out.push(mapped)
  return out
}

/** The persisted `expanded_groups` value for a group's "More" toggle. */
export const moreKey = (groupLabel: string): string => `${groupLabel}/More`

/**
 * Sections PaneHost renders with a dedicated panel component instead of a descriptor table.
 * They are first-class, so the navigator must not style them as "unported". A new section
 * becomes navigable once it is listed here (or has a descriptor): see `NAV_GROUPS`.
 */
export const PANEL_SECTIONS: ReadonlySet<SectionKey> = new Set([
  'overview',
  'events',
  'linter',
  'custom-resources',
  'network/portforwards',
  'access/namespace-access',
  'access/oidc-config',
  'integrations/argo',
  'integrations/flux',
  'integrations/knative',
  'integrations/keda',
  'integrations/velero',
  'integrations/cert-manager',
  'integrations/gatekeeper',
  'integrations/hubble',
  'integrations/vulnerabilities',
  'integrations/metrics',
  'integrations/grafana',
  'integrations/cluster-settings',
  'team/teams',
  'team/profiles',
  'audit-log',
])

/** A section PaneHost can render: it has a descriptor or a dedicated panel. */
export function sectionRenderable(key: SectionKey): boolean {
  return descriptorFor(key) !== undefined || PANEL_SECTIONS.has(key)
}

/** Flat, render-ready group shape used by callers that predate `NAV_MODEL`. */
export interface NavSectionGroup {
  label: string
  kanji: string
  sections: readonly { key: SectionKey; label: string; description: string; advanced: boolean }[]
}

/**
 * `NAV_MODEL` flattened for older callers (tree, palette, nav walks): every entry in model
 * order, "More" entries last in their group. Entries whose section PaneHost cannot render yet
 * are left out, so no caller links to an unfinished pane; they appear once they resolve.
 */
export const NAV_GROUPS: readonly NavSectionGroup[] = NAV_MODEL.map((grp) => ({
  label: grp.label,
  kanji: grp.kanji,
  sections: grp.entries
    .filter((e) => sectionRenderable(e.section))
    .map((e) => ({
      key: e.section,
      label: e.label,
      description: e.description,
      advanced: e.advanced === true,
    })),
})).filter((grp) => grp.sections.length > 0)

/** Tools kept in the navigator's Tools list (integrations now live under Integrations). */
export const NAV_TOOLS: readonly string[] = ['logs', 'topology', 'linter']

/** Groups open on first run, before `UiConfigLoaded` says otherwise. */
export const DEFAULT_EXPANDED_GROUPS: readonly string[] = NAV_MODEL.filter(
  (grp) => grp.defaultExpanded,
).map((grp) => grp.label)

const ENTRIES = new Map(NAV_MODEL.flatMap((grp) => grp.entries.map((e) => [e.section, e])))

/** The model entry for a section key, if the navigator lists it. */
export function navEntry(key: SectionKey): NavEntry | undefined {
  return ENTRIES.get(key)
}

/** Display label for any section key (falls back to the key itself). */
export function sectionLabel(key: SectionKey): string {
  return ENTRIES.get(key)?.label ?? key
}
