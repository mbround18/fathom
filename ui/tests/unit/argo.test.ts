import { describe, expect, it } from 'vitest'

import { detectArgo, parseArgoRows } from '@/features/argo/argo'
import type { CrdCatalogEntry } from '@/protocol/types'

const c = (group: string, kind: string): CrdCatalogEntry => ({
  group,
  kind,
  plural: kind.toLowerCase() + 's',
  versions: ['v1alpha1'],
  scope: 'Namespaced',
  permission_denied: false,
})

describe('argo', () => {
  it('detects installed and missing kinds', () => {
    const d = detectArgo([c('argoproj.io', 'Application'), c('x.io', 'AppProject')])
    expect(d.found.map((f) => f.kind)).toEqual(['Application'])
    expect(d.missing).toEqual(['ApplicationSet', 'AppProject'])
  })
  it('parses sync/health and skips malformed', () => {
    const rows = parseArgoRows('Application', [
      JSON.stringify({
        metadata: { name: 'a', namespace: 'argocd' },
        status: { sync: { status: 'Synced' }, health: { status: 'Healthy' } },
      }),
      '{bad',
      JSON.stringify({ metadata: { name: 'b' } }),
    ])
    expect(rows).toEqual([
      { kind: 'Application', namespace: 'argocd', name: 'a', sync: 'Synced', health: 'Healthy' },
      { kind: 'Application', namespace: null, name: 'b', sync: '—', health: '—' },
    ])
  })
})
