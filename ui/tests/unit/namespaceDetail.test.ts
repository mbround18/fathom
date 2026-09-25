import { describe, expect, it } from 'vitest'

import { selectCmd } from '@/protocol/commands'
import { namespaces } from '@/resources/namespaces'

describe('namespace read path', () => {
  it('maps Namespace to SelectNamespace (cluster-scoped, name only)', () => {
    expect(selectCmd('Namespace', '', 'team-a')).toEqual({
      type: 'SelectNamespace',
      data: { name: 'team-a' },
    })
  })

  it('selects NamespaceDetailsUpdated and exposes raw_json for the YAML tab', () => {
    const data = {
      name: 'team-a',
      status: 'Active',
      labels: { env: 'dev' },
      annotations: {},
      creation_timestamp: '',
      resource_quota_names: [],
      limit_range_names: [],
      raw_json: '{}',
    }
    const detail = namespaces.selectDetail({
      kind: 'Namespace',
      eventType: 'NamespaceDetailsUpdated',
      data,
      updatedAt: 0,
    })
    expect(detail).toEqual(data)
    expect(namespaces.rawJson?.(data)).toBe('{}')
    expect(
      namespaces.selectDetail({
        kind: 'Pod',
        eventType: 'PodDetailsUpdated',
        data: {} as never,
        updatedAt: 0,
      }),
    ).toBeUndefined()
  })
})
