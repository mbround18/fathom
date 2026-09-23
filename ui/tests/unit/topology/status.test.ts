import { describe, expect, it } from 'vitest'

import { nodeSummaryTone, podTone, workloadTone } from '@/topology/status'

describe('topology status tone', () => {
  it('colors a running pod ok, a failed pod err', () => {
    expect(podTone({ phase: 'Running' })).toBe('ok')
    expect(podTone({ phase: 'Failed' })).toBe('err')
  })

  it('colors a fully-ready workload ok, an under-ready one warn', () => {
    expect(workloadTone({ name: 'w', namespace: 'default', desired: 3, ready: 3, updated: 3, available: 3 })).toBe('ok')
    expect(workloadTone({ name: 'w', namespace: 'default', desired: 3, ready: 1, updated: 1, available: 1 })).toBe('warn')
  })

  it('colors a ready node ok', () => {
    expect(
      nodeSummaryTone({
        name: 'node-1',
        status: 'Ready',
        role: 'worker',
        pod_count: 1,
        pod_capacity: 10,
        cpu_allocatable_millicores: 1000,
        memory_allocatable_bytes: 1,
      }),
    ).toBe('ok')
  })
})
