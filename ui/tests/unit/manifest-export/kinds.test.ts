import { describe, expect, it } from 'vitest'

import {
  EXPORTABLE_KINDS,
  isExportable,
  manifestRefFor,
  NON_EXPORTABLE_KINDS,
} from '@/features/manifest-export/kinds'
import { ALL_DESCRIPTORS, descriptorFor } from '@/resources'
import type { ListRow } from '@/store'

describe('manifest export kinds', () => {
  it('every listed kind is either exportable or explicitly excluded', () => {
    const unmapped = ALL_DESCRIPTORS.map((d) => d.kind).filter(
      (k) => !Object.hasOwn(EXPORTABLE_KINDS, k) && !NON_EXPORTABLE_KINDS.has(k),
    )
    expect(unmapped).toEqual([])
  })

  it('synthetic kinds are not exportable', () => {
    for (const d of ALL_DESCRIPTORS) {
      if (NON_EXPORTABLE_KINDS.has(d.kind)) expect(isExportable(d)).toBe(false)
    }
  })

  it('builds refs with the real kind and scope', () => {
    const deployments = descriptorFor('workloads/deployments')
    expect(deployments).toBeDefined()
    if (!deployments) return
    const row = {
      namespace: 'default',
      name: 'web',
      desired: 1,
      ready: 1,
      updated: 1,
      available: 1,
    }
    expect(manifestRefFor(deployments, row as unknown as ListRow)).toEqual({
      api_version: 'apps/v1',
      kind: 'Deployment',
      namespace: 'default',
      name: 'web',
    })
    const endpoints = ALL_DESCRIPTORS.find((d) => d.kind === 'Endpoint')
    if (endpoints) expect(EXPORTABLE_KINDS[endpoints.kind]?.kind).toBe('Endpoints')
  })
})
