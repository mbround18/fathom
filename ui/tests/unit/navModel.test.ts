import { describe, expect, it } from 'vitest'

import { SECTION } from '@/protocol/commands'
import { integrationInstalled } from '@/resources/integrationStatus'
import { NAV_GROUPS, NAV_TOOLS, PANEL_SECTIONS } from '@/resources/nav'
import { initialData } from '@/store'

const keys = NAV_GROUPS.flatMap((g) => g.sections.map((s) => s.key))

describe('nav model', () => {
  it('has unique section keys', () => {
    expect(new Set(keys).size).toBe(keys.length)
  })

  it('lists every implemented integration under Integrations', () => {
    const integ = (NAV_GROUPS.find((g) => g.label === 'Integrations')?.sections ?? []).map(
      (s) => s.key,
    )
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
  })

  it('treats Events and every integration as first-class panel sections', () => {
    expect(PANEL_SECTIONS.has(SECTION.events)).toBe(true)
    for (const k of keys.filter((k) => k.startsWith('integrations/')))
      expect(PANEL_SECTIONS.has(k), k).toBe(true)
  })

  it('keeps only non-integration tools in the Tools list', () => {
    expect(NAV_TOOLS).toEqual(['logs', 'topology', 'linter'])
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
