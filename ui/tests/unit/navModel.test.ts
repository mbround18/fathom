/** 043 T008: the navigator model (spec FR-020 to FR-023, SC-005). */
import { describe, expect, it } from 'vitest'

import { SECTION } from '@/protocol/commands'
import { descriptorFor } from '@/resources'
import { integrationInstalled } from '@/resources/integrationStatus'
import {
  DEFAULT_EXPANDED_GROUPS,
  LEGACY_GROUP_LABELS,
  NAV_GROUPS,
  NAV_MODEL,
  NAV_TOOLS,
  PANEL_SECTIONS,
  entryVisible,
  mapLegacyGroups,
  moreKey,
  navEntry,
  sectionLabel,
  type NavVisibilityContext,
} from '@/resources/nav'
import { initialData } from '@/store'

const entries = NAV_MODEL.flatMap((g) => g.entries)
const keys = entries.map((e) => e.section)
const group = (label: string) => NAV_MODEL.find((g) => g.label === label)
const keysOf = (label: string, advanced: boolean) =>
  (group(label)?.entries ?? [])
    .filter((e) => (e.advanced === true) === advanced)
    .map((e) => e.section)

/** Every section key the navigator listed before 043 (FR-013: none may disappear). */
const PRE_043_KEYS = [
  'overview',
  'applications',
  'helm/releases',
  'nodes',
  'namespaces',
  'events',
  'workloads/pods',
  'workloads/deployments',
  'workloads/statefulsets',
  'workloads/daemonsets',
  'workloads/replicasets',
  'workloads/replicationcontrollers',
  'workloads/jobs',
  'workloads/cronjobs',
  'network/services',
  'network/endpoints',
  'network/ingress',
  'network/ingressclasses',
  'network/networkpolicies',
  'network/portforwards',
  'storage/persistentvolumes',
  'storage/persistentvolumeclaims',
  'storage/storageclasses',
  'config/configmaps',
  'config/secrets',
  'config/resourcequotas',
  'config/limitranges',
  'config/horizontalpodautoscalers',
  'config/verticalpodautoscalers',
  'config/poddisruptionbudgets',
  'config/priorityclasses',
  'config/runtimeclasses',
  'config/leases',
  'config/mutatingwebhooks',
  'config/validatingwebhooks',
  'access/roles',
  'access/rolebindings',
  'access/clusterroles',
  'access/clusterrolebindings',
  'access/namespace-access',
  'access/oidc-config',
  'custom-resources',
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
  'team/profiles',
  'team/teams',
  'audit-log',
]

describe('nav model', () => {
  it('orders the groups as FR-020', () => {
    expect(NAV_MODEL.map((g) => g.label)).toEqual([
      'Cluster',
      'Workloads',
      'Networking',
      'Storage',
      'Configuration',
      'Scaling',
      'Access Control',
      'Custom Resources',
      'Integrations',
      // 048-docker-management: this computer's container runtime, a scene of its own rather
      // than a setting, so it sits above Settings and below the cluster's own groups.
      'Containers',
      'Settings',
    ])
  })

  it('lists each group’s entries and "More" entries as FR-020', () => {
    expect(keysOf('Cluster', false)).toEqual([
      'overview',
      'topology',
      'nodes',
      'namespaces',
      'events',
      'linter',
      'cluster-history',
    ])
    expect(keysOf('Workloads', false)).toEqual([
      'applications',
      'helm/releases',
      'workloads/pods',
      'workloads/deployments',
      'workloads/statefulsets',
      'workloads/daemonsets',
      'workloads/jobs',
      'workloads/cronjobs',
    ])
    expect(keysOf('Workloads', true)).toEqual([
      'workloads/replicasets',
      'workloads/replicationcontrollers',
    ])
    expect(keysOf('Networking', false)).toEqual([
      'network/services',
      'network/ingress',
      'network/networkpolicies',
      'network/portforwards',
    ])
    expect(keysOf('Networking', true)).toEqual(['network/endpoints', 'network/ingressclasses'])
    expect(keysOf('Storage', false)).toEqual([
      'storage/persistentvolumeclaims',
      'storage/persistentvolumes',
      'storage/storageclasses',
    ])
    expect(keysOf('Configuration', false)).toEqual(['config/configmaps', 'config/secrets'])
    expect(keysOf('Configuration', true)).toEqual([
      'config/resourcequotas',
      'config/limitranges',
      'config/priorityclasses',
      'config/runtimeclasses',
      'config/leases',
      'config/mutatingwebhooks',
      'config/validatingwebhooks',
    ])
    expect(keysOf('Scaling', false)).toEqual([
      'config/horizontalpodautoscalers',
      'config/verticalpodautoscalers',
      'config/poddisruptionbudgets',
    ])
    expect(keysOf('Access Control', false)).toEqual([
      'access/roles',
      'access/rolebindings',
      'access/clusterroles',
      'access/clusterrolebindings',
      'access/namespace-access',
      'access/oidc-config',
    ])
    expect(keysOf('Custom Resources', false)).toEqual(['custom-resources'])
    expect(keysOf('Settings', false)).toEqual([
      'integrations/cluster-settings',
      'settings/sign-in',
      'team/teams',
      'team/profiles',
      'settings/app',
      'settings/local-clusters',
      'audit-log',
    ])
    for (const g of ['Cluster', 'Storage', 'Scaling', 'Access Control', 'Integrations', 'Settings'])
      expect(keysOf(g, true), g).toEqual([])
  })

  it('keeps kind names as labels (Principle I)', () => {
    expect(sectionLabel('workloads/pods')).toBe('Pods')
    expect(sectionLabel('config/configmaps')).toBe('ConfigMaps')
    for (const e of entries) {
      const d = descriptorFor(e.section)
      if (d) expect(e.label, e.section).toBe(d.label)
    }
  })

  it('has unique section keys', () => {
    expect(new Set(keys).size).toBe(keys.length)
  })

  it('gives every entry a one-line plain description of at most 90 characters', () => {
    for (const e of entries) {
      expect(e.description.trim().length, e.section).toBeGreaterThan(10)
      expect(e.description.length, e.section).toBeLessThanOrEqual(90)
      expect(e.description, e.section).not.toMatch(/\n/)
      expect(e.description.toLowerCase(), e.section).not.toContain(e.label.toLowerCase())
    }
  })

  it('shows at most 8 entries before "More", except Custom Resources and Integrations (SC-005)', () => {
    for (const g of NAV_MODEL) {
      if (g.label === 'Custom Resources' || g.label === 'Integrations') continue
      expect(g.entries.filter((e) => !e.advanced).length, g.label).toBeLessThanOrEqual(8)
    }
  })

  it('keeps every section the navigator listed before 043 (FR-013)', () => {
    for (const k of PRE_043_KEYS) expect(keys, k).toContain(k)
  })

  it('adds the new sections', () => {
    for (const k of [
      'topology',
      'linter',
      'cluster-history',
      'settings/sign-in',
      'settings/app',
      'settings/local-clusters',
    ])
      expect(keys, k).toContain(k)
  })

  it('lists every implemented integration under Integrations', () => {
    const integ = (group('Integrations')?.entries ?? []).map((e) => e.section)
    for (const k of [
      SECTION.argo,
      SECTION.flux,
      SECTION.knative,
      SECTION.keda,
      SECTION.velero,
      SECTION.certManager,
      SECTION.gatekeeper,
      SECTION.hubble,
      SECTION.vulnerabilities,
      SECTION.metrics,
      SECTION.grafana,
    ])
      expect(integ).toContain(k)
    for (const e of group('Integrations')?.entries ?? []) expect(e.integration).toBe(e.section)
  })

  it('treats Events and every integration as first-class panel sections', () => {
    expect(PANEL_SECTIONS.has(SECTION.events)).toBe(true)
    for (const k of keys.filter((k) => k.startsWith('integrations/')))
      expect(PANEL_SECTIONS.has(k), k).toBe(true)
  })

  it('keeps only non-integration tools in the Tools list', () => {
    expect(NAV_TOOLS).toEqual(['logs'])
  })

  it('opens Cluster and Workloads on first run', () => {
    expect(DEFAULT_EXPANDED_GROUPS).toEqual(['Cluster', 'Workloads'])
  })

  it('resolves labels and entries by section key', () => {
    expect(sectionLabel('cluster-history')).toBe('Cluster history')
    expect(sectionLabel('not-a-section')).toBe('not-a-section')
    expect(navEntry('config/poddisruptionbudgets')?.label).toBe('Pod Disruption Budgets')
  })

  it('hides Local clusters in hosted mode and in a browser', () => {
    const e = navEntry('settings/local-clusters')
    expect(e).toBeDefined()
    if (!e) return
    const ctx = (hosted: boolean, desktop: boolean): NavVisibilityContext => ({
      hosted,
      desktop,
      installed: () => undefined,
    })
    expect(entryVisible(e, ctx(false, true))).toBe(true)
    expect(entryVisible(e, ctx(true, true))).toBe(false)
    expect(entryVisible(e, ctx(false, false))).toBe(false)
  })
})

describe('legacy group labels (FR-023)', () => {
  it('maps every pre-043 group label to the groups that took its entries', () => {
    expect(LEGACY_GROUP_LABELS).toEqual({
      Overview: ['Cluster'],
      Network: ['Networking'],
      Config: ['Configuration', 'Scaling'],
      Team: ['Settings'],
    })
    for (const mapped of Object.values(LEGACY_GROUP_LABELS).flat())
      expect(group(mapped), mapped).toBeDefined()
  })

  it('maps persisted labels in order, keeps current and "More" labels, and dedupes', () => {
    expect(mapLegacyGroups(['Overview', 'Workloads', 'Config', 'Team'])).toEqual([
      'Cluster',
      'Workloads',
      'Configuration',
      'Scaling',
      'Settings',
    ])
    expect(mapLegacyGroups(['Cluster', 'Overview', moreKey('Workloads')])).toEqual([
      'Cluster',
      'Workloads/More',
    ])
    expect(mapLegacyGroups([])).toEqual([])
  })
})

describe('NAV_GROUPS (flat view for older callers)', () => {
  it('keeps model order and lists only sections PaneHost can render', () => {
    const flat = NAV_GROUPS.flatMap((g) => g.sections.map((s) => s.key))
    expect(flat).toEqual(keys.filter((k) => flat.includes(k)))
    for (const k of PRE_043_KEYS) expect(flat, k).toContain(k)
  })

  it('carries each entry’s description and "More" flag', () => {
    const rs = NAV_GROUPS.flatMap((g) => g.sections).find((s) => s.key === 'workloads/replicasets')
    expect(rs).toMatchObject({ advanced: true, label: 'ReplicaSets' })
    expect(rs?.description).toBe(navEntry('workloads/replicasets')?.description)
  })
})

describe('integrationInstalled', () => {
  it('is unknown before any scan and false only when a scan says so', () => {
    const d = initialData()
    expect(integrationInstalled(d, SECTION.flux)).toBeUndefined()
    expect(integrationInstalled(d, SECTION.events)).toBeUndefined()
    d.flux = { overview: { installed: false, sources: [], kustomizations: [], helmreleases: [] } }
    expect(integrationInstalled(d, SECTION.flux)).toBe(false)
    d.flux = { overview: { installed: true, sources: [], kustomizations: [], helmreleases: [] } }
    expect(integrationInstalled(d, SECTION.flux)).toBe(true)
  })

  it('probes Argo CD from the CRD catalog once it is loaded', () => {
    const d = initialData()
    expect(integrationInstalled(d, 'integrations/argo')).toBeUndefined()
    d.crds = { ...d.crds, catalogLoaded: true }
    expect(integrationInstalled(d, 'integrations/argo')).toBe(false)
    d.crds = {
      ...d.crds,
      catalog: [
        {
          group: 'argoproj.io',
          kind: 'Application',
          plural: 'applications',
          versions: ['v1alpha1'],
          scope: 'Namespaced',
          permission_denied: false,
        },
      ],
    }
    expect(integrationInstalled(d, 'integrations/argo')).toBe(true)
  })

  it('probes Prometheus and Grafana from the metrics stack scan', () => {
    const d = initialData()
    expect(integrationInstalled(d, 'integrations/metrics')).toBeUndefined()
    expect(integrationInstalled(d, 'integrations/grafana')).toBeUndefined()
    d.metrics = {
      ...d.metrics,
      stackScan: { tier: 'None', components: [], suggested_endpoint: null },
    }
    expect(integrationInstalled(d, 'integrations/metrics')).toBe(false)
    expect(integrationInstalled(d, 'integrations/grafana')).toBe(false)
    const grafana = {
      kind: 'Grafana' as const,
      namespace: 'grafana',
      service: 'grafana',
      port: 3000,
    }
    d.metrics = {
      ...d.metrics,
      stackScan: { tier: 'Partial', components: [grafana], suggested_endpoint: null },
    }
    expect(integrationInstalled(d, 'integrations/metrics')).toBe(true)
    expect(integrationInstalled(d, 'integrations/grafana')).toBe(true)
  })
})
