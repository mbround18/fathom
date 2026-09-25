import { describe, expect, it } from 'vitest'

import { descriptorFor } from '@/resources'
import { hpaMetrics } from '@/resources/horizontalpodautoscalers'
import { runtimeOverhead } from '@/resources/runtimeclasses'
import { vpaRecommendationLine, vpaTargetSummary } from '@/resources/verticalpodautoscalers'
import { hookViews } from '@/resources/webhooks'

import { VPA_ROW } from '../fixtures/batchD'

const KEYS = [
  'config/horizontalpodautoscalers',
  'config/verticalpodautoscalers',
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
    for (const k of KEYS.filter((k) => !/horizontal|vertical|leases/.test(k))) {
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
  it('is registered as a namespaced descriptor', () => {
    expect(descriptorFor('config/verticalpodautoscalers')?.namespaced).toBe(true)
  })

  it('formats per-container lower/target/upper recommendations', () => {
    const rec = VPA_ROW.recommendations[0]
    if (!rec) throw new Error('fixture has no recommendation')
    expect(vpaRecommendationLine(rec)).toBe(
      'app: lower cpu 25m / mem 100Mi · target cpu 50m / mem 200Mi · upper cpu 1 / mem 1Gi',
    )
  })

  it('summarizes the first target and counts extra containers', () => {
    expect(vpaTargetSummary([])).toBe('—')
    const rec = VPA_ROW.recommendations[0]
    if (!rec) throw new Error('fixture has no recommendation')
    expect(vpaTargetSummary([rec])).toBe('50m / 200Mi')
    expect(vpaTargetSummary([rec, rec])).toBe('50m / 200Mi +1')
  })
})
