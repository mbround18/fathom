import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { UnavailableSection } from '@/components/panels/UnavailableSection'
import { descriptorFor } from '@/resources'
import { hpaMetrics } from '@/resources/horizontalpodautoscalers'
import { runtimeOverhead } from '@/resources/runtimeclasses'
import { hookViews } from '@/resources/webhooks'

const KEYS = [
  'config/horizontalpodautoscalers',
  'config/priorityclasses',
  'config/runtimeclasses',
  'config/leases',
  'config/mutatingwebhooks',
  'config/validatingwebhooks',
]

describe('batch D descriptors', () => {
  it.each(KEYS)('%s is registered with a delete action gated by access check', (k) => {
    const d = descriptorFor(k)
    expect(d).toBeDefined()
    const del = d?.actions.find((a) => a.id === 'delete')
    expect(del?.destructive).toBe(true)
    expect(typeof del?.accessCheck).toBe('function')
  })

  it('cluster-scoped kinds send name-only commands', () => {
    for (const k of KEYS.filter((k) => !/horizontal|leases/.test(k))) {
      const d = descriptorFor(k)
      expect(d?.namespaced).toBe(false)
      const row = { name: 'x' } as never
      const step = d?.actions[0]?.plan([row], {})[0]
      expect(step?.cmd).toMatchObject({ data: { name: 'x' } })
    }
  })

  it('namespaced kinds delete with namespace and name', () => {
    const step = descriptorFor('config/leases')?.actions[0]?.plan(
      [{ namespace: 'kube-system', name: 'l' } as never],
      {},
    )[0]
    expect(step?.cmd).toEqual({
      type: 'DeleteLease',
      data: { namespace: 'kube-system', name: 'l' },
    })
  })

  it('parses HPA metric targets and current values', () => {
    const raw = JSON.stringify({
      spec: {
        metrics: [
          {
            type: 'Resource',
            resource: { name: 'cpu', target: { type: 'Utilization', averageUtilization: 70 } },
          },
        ],
      },
      status: {
        currentMetrics: [
          { type: 'Resource', resource: { name: 'cpu', current: { averageUtilization: 42 } } },
        ],
      },
    })
    expect(hpaMetrics(raw)).toEqual([{ label: 'Resource: cpu', target: '70%', current: '42%' }])
    expect(hpaMetrics('not json')).toEqual([])
  })

  it('parses webhook service/url targets and rules', () => {
    const raw = JSON.stringify({
      webhooks: [
        {
          name: 'a.example.com',
          clientConfig: { service: { namespace: 'ns', name: 'svc', path: '/m', port: 443 } },
          rules: [
            { operations: ['CREATE'], apiGroups: [''], apiVersions: ['v1'], resources: ['pods'] },
          ],
        },
        { name: 'b', clientConfig: { url: 'https://h/x' } },
      ],
    })
    const v = hookViews(raw)
    expect(v[0]?.target).toBe('service ns/svc:443/m')
    expect(v[0]?.rules[0]).toBe('CREATE core/v1 pods')
    expect(v[1]?.target).toBe('https://h/x')
  })

  it('formats runtime class overhead', () => {
    expect(runtimeOverhead(JSON.stringify({ overhead: { podFixed: { cpu: '250m' } } }))).toBe(
      'cpu: 250m',
    )
    expect(runtimeOverhead('{}')).toBe('None')
  })
})

describe('Vertical Pod Autoscalers', () => {
  it('has no descriptor and renders an honest unavailable state', () => {
    expect(descriptorFor('config/verticalpodautoscalers')).toBeUndefined()
    render(<UnavailableSection label="Vertical Pod Autoscalers" />)
    expect(screen.getByText(/not available in this build/i)).toBeTruthy()
    expect(screen.queryByText(/not ported yet/i)).toBeNull()
  })
})
